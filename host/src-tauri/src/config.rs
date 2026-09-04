use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jsonschema::validator_for;
use reqwest::{blocking::Client, Url};
use serde::{de::Deserializer, Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::Digest;

use app_host_kernel::ids::{AppId, SecretName, SecretRef};
use app_host_kernel::kernel::Kernel;
use app_host_kernel::manifest::AppManifest;
use app_host_kernel::JsonObject;

use crate::atomic_json::{
    persist_json_document, standard_writer, AtomicFileWriter, AtomicJsonError,
};

// -- Secret Storage ----------------------------------------------------------

/// Port for host secret storage, designed for a future OS keyring adapter.
/// Every operation is keyed by `SecretRef` (owner app + local name).
/// Mutation failures must leave the stored value unchanged.
pub trait SecretStorage: Send {
    fn read(&self, ref_: &SecretRef) -> Result<Option<String>, String>;
    fn write(&mut self, ref_: &SecretRef, value: String) -> Result<(), String>;
    fn check(&self, ref_: &SecretRef) -> Result<bool, String>;
    fn clear(&mut self, ref_: &SecretRef) -> Result<(), String>;
    /// Return all stored entries for bootstrap/rehydration.
    fn all(&self) -> Result<Vec<(SecretRef, String)>, String>;
}

fn restore_secret_values(
    secrets: &mut dyn SecretStorage,
    cleared: Vec<(SecretRef, Option<String>)>,
) -> Vec<String> {
    cleared
        .into_iter()
        .filter_map(|(ref_, value)| {
            value.and_then(|value| {
                secrets
                    .write(&ref_, value)
                    .err()
                    .map(|error| format!("{}/{}: {error}", ref_.owner, ref_.name))
            })
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretReferenceEntry {
    owner: String,
    name: String,
    status: SecretReferenceStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum SecretReferenceStatus {
    Stored,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretReferenceDocument {
    version: u32,
    secrets: Vec<SecretReferenceEntry>,
}

trait CredentialBackend: Send + Sync {
    fn read(&self, account: &str) -> Result<Option<String>, String>;
    fn write(&self, account: &str, value: &str) -> Result<(), String>;
    fn clear(&self, account: &str) -> Result<(), String>;
}

#[cfg(not(test))]
struct OsCredentialBackend;

#[cfg(not(test))]
impl OsCredentialBackend {
    fn entry(account: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new("ai-app-host", account)
            .map_err(|error| format!("open OS credential failed: {error}"))
    }
}

#[cfg(not(test))]
impl CredentialBackend for OsCredentialBackend {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        match Self::entry(account)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(format!("read OS credential failed: {error}")),
        }
    }

    fn write(&self, account: &str, value: &str) -> Result<(), String> {
        Self::entry(account)?
            .set_password(value)
            .map_err(|error| format!("write OS credential failed: {error}"))
    }

    fn clear(&self, account: &str) -> Result<(), String> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("delete OS credential failed: {error}")),
        }
    }
}

#[cfg(test)]
struct OsCredentialBackend;

#[cfg(test)]
impl CredentialBackend for OsCredentialBackend {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        Ok(test_credentials().lock().unwrap().get(account).cloned())
    }

    fn write(&self, account: &str, value: &str) -> Result<(), String> {
        test_credentials()
            .lock()
            .unwrap()
            .insert(account.to_string(), value.to_string());
        Ok(())
    }

    fn clear(&self, account: &str) -> Result<(), String> {
        test_credentials().lock().unwrap().remove(account);
        Ok(())
    }
}

#[cfg(test)]
fn test_credentials() -> &'static std::sync::Mutex<BTreeMap<String, String>> {
    static CREDENTIALS: std::sync::OnceLock<std::sync::Mutex<BTreeMap<String, String>>> =
        std::sync::OnceLock::new();
    CREDENTIALS.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
}

/// Status-only JSON index backed by Windows Credential Manager, macOS
/// Keychain, or Linux Secret Service. No secret value is serialized here.
pub struct OsProtectedSecretStore {
    path: PathBuf,
    namespace: String,
    refs: BTreeSet<SecretRef>,
    backend: Box<dyn CredentialBackend>,
}

impl OsProtectedSecretStore {
    pub fn new(path: PathBuf) -> Result<Self, String> {
        let namespace = path.display().to_string();
        Self::with_namespace(path, namespace)
    }

    pub fn with_namespace(path: PathBuf, namespace: String) -> Result<Self, String> {
        Self::with_backend(path, namespace, Box::new(OsCredentialBackend))
    }

    fn with_backend(
        path: PathBuf,
        namespace: String,
        backend: Box<dyn CredentialBackend>,
    ) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create secret store directory failed: {error}"))?;
        }
        let mut store = Self {
            path: path.clone(),
            namespace,
            refs: BTreeSet::new(),
            backend,
        };
        if !path.exists() {
            store.persist().map_err(AtomicJsonError::into_message)?;
            return Ok(store);
        }
        let raw = fs::read_to_string(&path)
            .map_err(|error| format!("read secret reference store failed: {error}"))?;
        let value: Value = serde_json::from_str(&raw).map_err(|error| {
            format!(
                "parse secret reference store failed; preserved '{}': {error}",
                path.display()
            )
        })?;
        match value.get("version").and_then(Value::as_u64) {
            Some(2) => {
                let document: SecretReferenceDocument = serde_json::from_value(value)
                    .map_err(|error| format!("parse secret reference store failed: {error}"))?;
                for entry in document.secrets {
                    store.refs.insert(SecretRef {
                        owner: AppId::new(entry.owner),
                        name: SecretName::new(entry.name),
                    });
                }
            }
            other => {
                return Err(format!(
                    "unsupported secret reference store version: {other:?}"
                ))
            }
        }
        Ok(store)
    }

    fn persist(&self) -> Result<(), AtomicJsonError> {
        let entries: Vec<SecretReferenceEntry> = self
            .refs
            .iter()
            .map(|ref_| SecretReferenceEntry {
                owner: ref_.owner.as_str().to_string(),
                name: ref_.name.as_str().to_string(),
                status: SecretReferenceStatus::Stored,
            })
            .collect();
        persist_json_document(
            &self.path,
            &SecretReferenceDocument {
                version: 2,
                secrets: entries,
            },
            "secret store",
            standard_writer().as_ref(),
        )
    }

    fn account(&self, ref_: &SecretRef) -> String {
        let identity = format!("{}\0{}\0{}", self.namespace, ref_.owner, ref_.name);
        format!("secret-{:x}", sha2::Sha256::digest(identity.as_bytes()))
    }
}

impl SecretStorage for OsProtectedSecretStore {
    fn read(&self, ref_: &SecretRef) -> Result<Option<String>, String> {
        if !self.refs.contains(ref_) {
            return Ok(None);
        }
        self.backend.read(&self.account(ref_))
    }

    fn write(&mut self, ref_: &SecretRef, value: String) -> Result<(), String> {
        let account = self.account(ref_);
        let previous = self.backend.read(&account)?;
        let was_indexed = self.refs.contains(ref_);
        self.backend.write(&account, &value)?;
        self.refs.insert(ref_.clone());
        if let Err(error) = self.persist() {
            if error.is_indeterminate() {
                return Err(error.into_message());
            }
            // Compensation is best effort. Propagating its failure with `?`
            // discarded the original persist error and returned before the
            // index was put back, leaving the on-disk index and the OS keyring
            // disagreeing about whether the secret exists.
            if !was_indexed {
                self.refs.remove(ref_);
            }
            let rollback = match previous {
                Some(previous) => self.backend.write(&account, &previous),
                None => self.backend.clear(&account),
            };
            let message = error.into_message();
            return Err(match rollback {
                Ok(()) => message,
                Err(rollback_error) => format!(
                    "{message}; restoring the previous credential also failed: {rollback_error}"
                ),
            });
        }
        Ok(())
    }

    fn check(&self, ref_: &SecretRef) -> Result<bool, String> {
        Ok(self.refs.contains(ref_) && self.backend.read(&self.account(ref_))?.is_some())
    }

    fn clear(&mut self, ref_: &SecretRef) -> Result<(), String> {
        let account = self.account(ref_);
        let previous = self.backend.read(&account)?;
        let was_indexed = self.refs.contains(ref_);
        self.backend.clear(&account)?;
        self.refs.remove(ref_);
        if let Err(error) = self.persist() {
            if error.is_indeterminate() {
                return Err(error.into_message());
            }
            // Restore the index first, then the credential: a `?` on the
            // credential write used to skip the index restore entirely, so a
            // failed rollback left the index claiming the secret was gone
            // while the keyring still held it.
            if was_indexed {
                self.refs.insert(ref_.clone());
            }
            let rollback = match previous {
                Some(value) => self.backend.write(&account, &value),
                None => Ok(()),
            };
            let message = error.into_message();
            return Err(match rollback {
                Ok(()) => message,
                Err(rollback_error) => format!(
                    "{message}; restoring the cleared credential also failed: {rollback_error}"
                ),
            });
        }
        Ok(())
    }

    fn all(&self) -> Result<Vec<(SecretRef, String)>, String> {
        // A ref can be indexed without a live OS credential — e.g. a connector
        // whose key was cleared while its index entry lingered, or a
        // credential the OS store lost. Skip those rather than failing the
        // whole rehydration: one stale entry must not block bootstrap of every
        // other secret (which in turn gates third-party app reactivation). A
        // genuine backend read error still propagates.
        let mut entries = Vec::new();
        for ref_ in &self.refs {
            if let Some(value) = self.backend.read(&self.account(ref_))? {
                entries.push((ref_.clone(), value));
            }
        }
        Ok(entries)
    }
}

pub(crate) fn clear_all_indexed_secrets(path: PathBuf, namespace: String) -> Result<(), String> {
    let mut store = OsProtectedSecretStore::with_namespace(path, namespace)?;
    let refs: Vec<SecretRef> = store.refs.iter().cloned().collect();
    for ref_ in refs {
        store.clear(&ref_).map_err(|error| {
            format!(
                "clear protected secret '{}/{}' failed: {error}",
                ref_.owner, ref_.name
            )
        })?;
    }
    Ok(())
}

/// In-memory secret store for tests and `HostConfigService::default()`.
#[derive(Debug)]
pub struct InMemorySecretStore {
    secrets: BTreeMap<SecretRef, String>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self {
            secrets: BTreeMap::new(),
        }
    }
}

impl Default for InMemorySecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStorage for InMemorySecretStore {
    fn read(&self, ref_: &SecretRef) -> Result<Option<String>, String> {
        Ok(self.secrets.get(ref_).cloned())
    }

    fn write(&mut self, ref_: &SecretRef, value: String) -> Result<(), String> {
        self.secrets.insert(ref_.clone(), value);
        Ok(())
    }

    fn check(&self, ref_: &SecretRef) -> Result<bool, String> {
        Ok(self.secrets.contains_key(ref_))
    }

    fn clear(&mut self, ref_: &SecretRef) -> Result<(), String> {
        self.secrets.remove(ref_);
        Ok(())
    }

    fn all(&self) -> Result<Vec<(SecretRef, String)>, String> {
        Ok(self
            .secrets
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }
}

// -- Host config --------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    pub version: u32,
    pub host: HostDefaults,
    pub apps: BTreeMap<String, AppConfigEntry>,
    pub connectors: BTreeMap<String, ConnectorConfig>,
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    pub mcp_exports: BTreeMap<String, McpExportProfile>,
    /// Desired lifecycle changes that were durably started but not yet
    /// reconciled with the kernel. Startup resumes these idempotently.
    pub mcp_export_transitions: BTreeMap<String, bool>,
    pub mcp_gateway: McpGatewaySettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostDefaults {
    pub default_llm_provider: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub default_llm_profile: Option<String>,
    pub cloud_llm_egress_accepted_profiles: Vec<String>,
    pub app_data_backup_retention: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfigEntry {
    pub settings: JsonObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectorKind {
    Ollama,
    OpenAiCompatible,
    Openai,
    Anthropic,
    AnthropicOauth,
    OpenaiCodex,
    GithubCopilot,
    Openrouter,
    Google,
    Mistral,
    AmazonBedrock,
}

pub struct ConnectorKindDefaults {
    pub base_url: &'static str,
    pub api_key_required: bool,
    pub oauth_credential_required: bool,
}

impl ConnectorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::OpenAiCompatible => "open-ai-compatible",
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::AnthropicOauth => "anthropic-oauth",
            Self::OpenaiCodex => "openai-codex",
            Self::GithubCopilot => "github-copilot",
            Self::Openrouter => "openrouter",
            Self::Google => "google",
            Self::Mistral => "mistral",
            Self::AmazonBedrock => "amazon-bedrock",
        }
    }

    pub const fn defaults(self) -> ConnectorKindDefaults {
        match self {
            Self::Ollama => ConnectorKindDefaults {
                base_url: "http://localhost:11434",
                api_key_required: false,
                oauth_credential_required: false,
            },
            Self::OpenAiCompatible => ConnectorKindDefaults {
                base_url: "https://api.openai.com/v1",
                api_key_required: false,
                oauth_credential_required: false,
            },
            Self::Openai => ConnectorKindDefaults {
                base_url: "https://api.openai.com/v1",
                api_key_required: true,
                oauth_credential_required: false,
            },
            Self::Anthropic => ConnectorKindDefaults {
                base_url: "https://api.anthropic.com",
                api_key_required: true,
                oauth_credential_required: false,
            },
            Self::AnthropicOauth => ConnectorKindDefaults {
                base_url: "https://api.anthropic.com",
                api_key_required: false,
                oauth_credential_required: true,
            },
            Self::OpenaiCodex => ConnectorKindDefaults {
                base_url: "https://chatgpt.com/backend-api",
                api_key_required: false,
                oauth_credential_required: true,
            },
            Self::GithubCopilot => ConnectorKindDefaults {
                base_url: "https://api.githubcopilot.com",
                api_key_required: false,
                oauth_credential_required: true,
            },
            Self::Openrouter => ConnectorKindDefaults {
                base_url: "https://openrouter.ai/api/v1",
                api_key_required: true,
                oauth_credential_required: false,
            },
            Self::Google => ConnectorKindDefaults {
                base_url: "https://generativelanguage.googleapis.com",
                api_key_required: true,
                oauth_credential_required: false,
            },
            Self::Mistral => ConnectorKindDefaults {
                base_url: "https://api.mistral.ai/v1",
                api_key_required: true,
                oauth_credential_required: false,
            },
            Self::AmazonBedrock => ConnectorKindDefaults {
                base_url: "https://bedrock-runtime.us-east-1.amazonaws.com",
                api_key_required: true,
                oauth_credential_required: false,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelVariant {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl ModelVariant {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextVerbosity {
    Low,
    Medium,
    High,
}

impl TextVerbosity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorConfig {
    pub kind: ConnectorKind,
    pub base_url: String,
    pub default_model: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub default_variant: Option<ModelVariant>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub default_text_verbosity: Option<TextVerbosity>,
    pub secret_refs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorConfigView {
    pub id: String,
    pub kind: ConnectorKind,
    pub base_url: String,
    pub default_model: String,
    #[serde(default)]
    pub default_variant: Option<ModelVariant>,
    #[serde(default)]
    pub default_text_verbosity: Option<TextVerbosity>,
    #[serde(default)]
    pub secret_refs: BTreeMap<String, String>,
}

/// How to reach a configured MCP server. Mirrors the adapter's transports;
/// stored in host-owned config, never inside the kernel. Configuring a
/// server records it — nothing connects or installs until the user says so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum McpTransportConfig {
    Stdio {
        command: String,
        args: Vec<String>,
    },
    StreamableHttp {
        url: String,
        authentication: McpHttpAuthentication,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum McpHttpAuthentication {
    None,
    StaticHeader {
        header_name: String,
        value_prefix: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfig {
    pub display_name: String,
    pub transport: McpTransportConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfigView {
    pub id: String,
    pub display_name: String,
    pub transport: McpTransportConfig,
}

// -- MCP export profiles (outbound gateway) -----------------------------------

/// One capability a profile exports, named exactly. Remote clients never
/// submit these; they are resolved host-side from the profile.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpExportedCapability {
    pub provider: String,
    pub capability: String,
}

/// How much local interaction each remote call requires. This becomes the
/// grant condition on the virtual principal's grants, so local trusted
/// chrome stays authoritative — especially for `RequiresApproval`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum McpExportInteraction {
    #[default]
    RequiresApproval,
    Notify,
    Silent,
}

/// One outbound export: which capabilities a remote MCP client may reach,
/// under which policy. Nothing is exported by default — profiles start
/// disabled, hold no token, and export an explicit capability list only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpExportProfile {
    pub display_name: String,
    pub enabled: bool,
    pub capabilities: Vec<McpExportedCapability>,
    pub interaction: McpExportInteraction,
    /// Grant lifetime in seconds; absent = non-expiring (each use may still
    /// require local approval per `interaction`).
    #[serde(deserialize_with = "deserialize_required_option")]
    pub expires_after_seconds: Option<std::num::NonZeroU32>,
    /// Ceiling on remote `tools/call` requests per minute for this profile.
    pub rate_limit_per_minute: u32,
    /// Return capability result payloads to the remote client. When false,
    /// remote clients only learn that the call completed.
    pub expose_results: bool,
    /// Return produced artifact summaries (id, type, title) to the remote
    /// client. Artifact content never leaves the host either way.
    pub expose_artifacts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpExportProfileView {
    pub id: String,
    #[serde(flatten)]
    pub profile: McpExportProfile,
}

/// Listener settings for the outbound MCP gateway. Loopback-only by design:
/// public exposure goes through a local tunnel (e.g. Cloudflare Tunnel),
/// which forwards to this listener but is NOT authentication — bearer
/// tokens are checked on every request regardless.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpGatewaySettings {
    pub enabled: bool,
    pub bind_address: String,
    /// Origin header values accepted in addition to localhost origins.
    pub allowed_origins: Vec<String>,
    /// OAuth 2.1 protected-resource metadata + audience validation is staged
    /// but NOT implemented; validation rejects `true` until it is correct.
    pub oauth_enabled: bool,
}

impl Default for McpGatewaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_address: default_gateway_bind(),
            allowed_origins: Vec::new(),
            oauth_enabled: false,
        }
    }
}

fn default_gateway_bind() -> String {
    "127.0.0.1:8137".to_string()
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LlmProfileRuntime {
    pub connector_id: String,
    pub kind: ConnectorKind,
    pub base_url: String,
    pub default_model: String,
    pub default_variant: Option<ModelVariant>,
    pub default_text_verbosity: Option<TextVerbosity>,
    pub api_key_secret_ref: Option<String>,
    pub oauth_secret_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionTestResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: Option<String>,
    pub variants: Vec<ModelVariant>,
    pub text_verbosity: Vec<TextVerbosity>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelListResult {
    pub models: Vec<ModelInfo>,
    pub message: String,
}

/// Everything a connector HTTP probe needs, captured under the config lock.
/// The probe itself runs without the lock so a slow provider can never block
/// commands that need config access (see the lock-ordering note in lib.rs).
#[derive(Debug, Clone)]
pub struct ConnectorProbe {
    pub kind: ConnectorKind,
    pub url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostConfigStoreDocument {
    version: u32,
    config: HostConfig,
}

pub struct HostConfigService {
    path: Option<PathBuf>,
    secrets_path: Option<PathBuf>,
    document: HostConfig,
    secrets: Box<dyn SecretStorage>,
    writer: Arc<dyn AtomicFileWriter>,
}

impl Default for HostConfigService {
    fn default() -> Self {
        Self {
            path: None,
            secrets_path: None,
            document: default_host_config(),
            secrets: Box::new(InMemorySecretStore::new()),
            writer: standard_writer(),
        }
    }
}

impl HostConfigService {
    pub(crate) fn validate_persisted_documents(
        config_path: &Path,
        secrets_path: &Path,
    ) -> Result<(), String> {
        if config_path.exists() {
            let value: Value = serde_json::from_slice(&fs::read(config_path).map_err(|error| {
                format!(
                    "read host config '{}' failed: {error}",
                    config_path.display()
                )
            })?)
            .map_err(|error| format!("parse host config failed: {error}"))?;
            let document: HostConfigStoreDocument = serde_json::from_value(value)
                .map_err(|error| format!("parse host config failed: {error}"))?;
            if document.version != 3 {
                return Err(format!(
                    "unsupported host config storage version: {}",
                    document.version
                ));
            }
            validate_host_config(&document.config)?;
        }
        if secrets_path.exists() {
            let document: SecretReferenceDocument =
                serde_json::from_slice(&fs::read(secrets_path).map_err(|error| {
                    format!(
                        "read secret reference store '{}' failed: {error}",
                        secrets_path.display()
                    )
                })?)
                .map_err(|error| format!("parse secret reference store failed: {error}"))?;
            if document.version != 2 {
                return Err(format!(
                    "unsupported secret reference store version: {}",
                    document.version
                ));
            }
            let mut identities = BTreeSet::new();
            for secret in document.secrets {
                if secret.owner.is_empty() || secret.name.is_empty() {
                    return Err("secret reference identity cannot be empty".into());
                }
                if !identities.insert((secret.owner, secret.name)) {
                    return Err("secret reference store contains a duplicate identity".into());
                }
            }
        }
        Ok(())
    }

    /// Load from disk. The stored document is config-only (v2); secret values
    /// live in OS credentials behind the sidecar `host-secrets.json` index.
    pub fn new(config_path: PathBuf) -> Result<Self, String> {
        let secret_namespace = config_path.display().to_string();
        Self::with_writer_and_namespace(config_path, secret_namespace, standard_writer())
    }

    pub fn new_with_namespace(
        config_path: PathBuf,
        secret_namespace: String,
    ) -> Result<Self, String> {
        Self::with_writer_and_namespace(config_path, secret_namespace, standard_writer())
    }

    fn with_writer_and_namespace(
        config_path: PathBuf,
        secret_namespace: String,
        writer: Arc<dyn AtomicFileWriter>,
    ) -> Result<Self, String> {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create host config directory failed: {error}"))?;
        }

        let secrets_path = secret_store_path(&config_path);
        let fresh_document = (!config_path.exists()).then(|| HostConfigStoreDocument {
            version: 3,
            config: fresh_host_config(),
        });

        // Read the raw JSON to handle version detection without schema lock-in.
        let raw = if config_path.exists() {
            fs::read_to_string(&config_path)
                .map_err(|error| format!("read host config failed: {error}"))?
        } else {
            serde_json::to_string_pretty(
                fresh_document
                    .as_ref()
                    .expect("fresh document exists when config file is absent"),
            )
            .map_err(|error| format!("serialize fresh host config failed: {error}"))?
        };

        let parsed: Value = serde_json::from_str(&raw)
            .map_err(|error| format!("parse host config failed: {error}"))?;
        let version = parsed.get("version").and_then(Value::as_u64).unwrap_or(0) as u32;

        let (document, secrets_store) = match version {
            3 => {
                let stored: HostConfigStoreDocument = serde_json::from_value(parsed)
                    .map_err(|error| format!("parse host config failed: {error}"))?;
                let config = stored.config;
                validate_host_config(&config)?;
                let store = OsProtectedSecretStore::with_namespace(
                    secrets_path.clone(),
                    secret_namespace.clone(),
                )?;
                (config, Box::new(store) as Box<dyn SecretStorage>)
            }
            other => {
                return Err(format!("unsupported host config storage version: {other}"));
            }
        };

        if let Some(document) = &fresh_document {
            persist_json_document(&config_path, document, "fresh host config", writer.as_ref())
                .map_err(AtomicJsonError::into_message)?;
        }

        let service = Self {
            path: Some(config_path),
            secrets_path: Some(secrets_path),
            document,
            secrets: secrets_store,
            writer,
        };
        service.validate_active_llm_credential(&service.document)?;
        Ok(service)
    }

    pub fn get_host_config(&self) -> HostConfig {
        self.document.clone()
    }

    pub fn secret_storage_path(&self) -> Option<&Path> {
        self.secrets_path.as_deref()
    }

    pub fn update_host_config(&mut self, patch: JsonObject) -> Result<HostConfig, String> {
        let mut current = serde_json::to_value(&self.document)
            .map_err(|error| format!("serialize host config failed: {error}"))?;
        merge_value(&mut current, Value::Object(patch));
        let next: HostConfig = serde_json::from_value(current)
            .map_err(|error| format!("invalid host config patch: {error}"))?;
        validate_host_config(&next)?;
        validate_cloud_llm_activation_change(&self.document, &next)?;
        self.validate_active_llm_credential(&next)?;
        let previous = self.document.clone();
        self.document = next;
        self.persist_current(previous)?;
        Ok(self.document.clone())
    }

    pub fn get_app_config(&self, app_id: &str) -> JsonObject {
        self.document
            .apps
            .get(app_id)
            .map(|entry| entry.settings.clone())
            .unwrap_or_default()
    }

    pub fn update_app_config(
        &mut self,
        app_id: &str,
        manifest: &AppManifest,
        config: JsonObject,
    ) -> Result<JsonObject, String> {
        validate_app_config(manifest, app_id, &config)?;
        let mut candidate = self.document.clone();
        let entry = candidate
            .apps
            .entry(app_id.to_string())
            .or_insert_with(|| AppConfigEntry {
                settings: Map::new(),
            });
        entry.settings = config;
        let updated = entry.settings.clone();
        self.persist_candidate(candidate)?;
        Ok(updated)
    }

    pub fn validate_candidate_app_config(
        &self,
        app_id: &str,
        manifest: &AppManifest,
        config: JsonObject,
    ) -> Result<JsonObject, String> {
        validate_app_config(manifest, app_id, &config)?;
        Ok(config)
    }

    /// Remove an app's stored config entry (its "app data"). Used by
    /// uninstall when the user chooses to purge data. Absent entry is a no-op.
    pub fn remove_app_config(&mut self, app_id: &str) -> Result<(), String> {
        if !self.document.apps.contains_key(app_id) {
            return Ok(());
        }
        let previous = self.document.clone();
        self.document.apps.remove(app_id);
        self.persist_current(previous)?;
        Ok(())
    }

    pub fn list_connector_configs(&self) -> Vec<ConnectorConfigView> {
        self.document
            .connectors
            .iter()
            .map(|(id, connector)| ConnectorConfigView {
                id: id.clone(),
                kind: connector.kind,
                base_url: connector.base_url.clone(),
                default_model: connector.default_model.clone(),
                default_variant: connector.default_variant,
                default_text_verbosity: connector.default_text_verbosity,
                secret_refs: connector.secret_refs.clone(),
            })
            .collect()
    }

    pub fn upsert_connector_config(
        &mut self,
        connector: ConnectorConfigView,
    ) -> Result<ConnectorConfigView, String> {
        self.upsert_connector_config_inner(connector, false)
    }

    pub fn upsert_connector_config_with_egress_acknowledgement(
        &mut self,
        connector: ConnectorConfigView,
    ) -> Result<ConnectorConfigView, String> {
        self.upsert_connector_config_inner(connector, true)
    }

    fn upsert_connector_config_inner(
        &mut self,
        connector: ConnectorConfigView,
        acknowledge_data_egress: bool,
    ) -> Result<ConnectorConfigView, String> {
        validate_connector_view(&connector)?;
        // A secret ref is addressed by (owner, local_name); two connectors
        // sharing one would make rotating or clearing one silently change the
        // other. Reject creating that collision at upsert time. (Existing state
        // is deliberately not re-validated at load, so a previously-saved
        // config still starts.)
        for key_kind in ["api_key", "oauth"] {
            let Some(incoming_ref) = connector.secret_refs.get(key_kind) else {
                continue;
            };
            for (existing_id, existing) in &self.document.connectors {
                if existing_id != &connector.id
                    && existing.secret_refs.get(key_kind) == Some(incoming_ref)
                {
                    return Err(format!(
                        "the {key_kind} secret name '{incoming_ref}' is already used by connector '{existing_id}'; choose a different name"
                    ));
                }
            }
        }
        let mut next = self.document.clone();
        next.connectors.insert(
            connector.id.clone(),
            ConnectorConfig {
                kind: connector.kind,
                base_url: connector.base_url.clone(),
                default_model: connector.default_model.clone(),
                default_variant: connector.default_variant,
                default_text_verbosity: connector.default_text_verbosity,
                secret_refs: connector.secret_refs.clone(),
            },
        );
        if acknowledge_data_egress {
            let active_connector_id = active_llm_connector_id(&next);
            if active_connector_id.as_deref() != Some(connector.id.as_str()) {
                return Err(
                    "data-egress acknowledgement is only valid for the default LLM profile".into(),
                );
            }
            let updated_connector = next
                .connectors
                .get(&connector.id)
                .expect("the connector was inserted above");
            if !connector_is_cloud(updated_connector) {
                return Err(
                    "data-egress acknowledgement is only valid for a cloud LLM profile".into(),
                );
            }
            if !cloud_profile_acknowledged(&next, &connector.id) {
                next.host
                    .cloud_llm_egress_accepted_profiles
                    .push(connector.id.clone());
            }
        }
        validate_cloud_llm_activation_change(&self.document, &next)?;
        self.validate_active_llm_credential(&next)?;
        let previous = self.document.clone();
        self.document = next;
        self.persist_current(previous)?;
        Ok(connector)
    }

    pub fn delete_connector_config(&mut self, connector_id: &str) -> Result<(), String> {
        if active_llm_connector_id(&self.document).as_deref() == Some(connector_id) {
            return Err(format!(
                "cannot delete the default LLM profile '{connector_id}'; clear it or choose a new default first"
            ));
        }
        if !self.document.connectors.contains_key(connector_id) {
            return Err(format!("unknown connector profile: {connector_id}"));
        }
        let previous = self.document.clone();
        let mut next = previous.clone();
        next.connectors.remove(connector_id);
        // A deleted profile's egress acknowledgment must not carry over to a
        // later profile re-created under the same id.
        next.host
            .cloud_llm_egress_accepted_profiles
            .retain(|accepted| accepted != connector_id);
        // Drop the stored credentials too, for the same reason the egress
        // acknowledgment above is dropped: secret names are derived
        // deterministically from the connector id, so a key left behind is
        // silently adopted by the next profile created under that id — even
        // one pointing at a different `base_url`. Deleting a profile is how a
        // user revokes a leaked key, so it has to actually remove it.
        //
        let removed_refs: Vec<String> = previous
            .connectors
            .get(connector_id)
            .map(|removed| removed.secret_refs.values().cloned().collect())
            .unwrap_or_default();
        let secrets_to_clear: Vec<(SecretRef, Option<String>)> = removed_refs
            .into_iter()
            .filter(|secret_ref| {
                !next.connectors.values().any(|connector| {
                    connector
                        .secret_refs
                        .values()
                        .any(|name| name == secret_ref)
                })
            })
            .map(|secret_ref| {
                // Connector credentials live under the `llm-provider` owner,
                // the same one bootstrap and profile reads/writes use.
                let ref_ = SecretRef {
                    owner: AppId::new("llm-provider"),
                    name: SecretName::new(secret_ref),
                };
                self.secrets.read(&ref_).map(|value| (ref_, value))
            })
            .collect::<Result<_, _>>()?;
        let mut cleared_secrets = Vec::new();
        for (ref_, previous_value) in secrets_to_clear {
            if let Err(error) = self.secrets.clear(&ref_) {
                let rollback_errors = restore_secret_values(self.secrets.as_mut(), cleared_secrets);
                let failure = format!(
                    "connector profile '{connector_id}' was not removed because clearing its \
                     stored credential failed: {error}"
                );
                return if rollback_errors.is_empty() {
                    Err(failure)
                } else {
                    Err(format!(
                        "{failure}; restoring cleared credentials failed: {}",
                        rollback_errors.join(", ")
                    ))
                };
            }
            cleared_secrets.push((ref_, previous_value));
        }

        self.document = next;
        if let Err(error) = self.persist() {
            if error.is_indeterminate() {
                return Err(error.into_message());
            }
            self.document = previous;
            let error = error.into_message();
            let rollback_errors = restore_secret_values(self.secrets.as_mut(), cleared_secrets);
            return if rollback_errors.is_empty() {
                Err(error)
            } else {
                Err(format!(
                    "{error}; restoring cleared credentials failed: {}",
                    rollback_errors.join(", ")
                ))
            };
        }
        Ok(())
    }

    pub fn list_mcp_servers(&self) -> Vec<McpServerConfigView> {
        self.document
            .mcp_servers
            .iter()
            .map(|(id, server)| McpServerConfigView {
                id: id.clone(),
                display_name: server.display_name.clone(),
                transport: server.transport.clone(),
            })
            .collect()
    }

    pub fn mcp_server(&self, server_id: &str) -> Option<McpServerConfig> {
        self.document.mcp_servers.get(server_id).cloned()
    }

    pub fn mcp_http_auth_header(
        &self,
        server_id: &str,
    ) -> Result<Option<(String, String)>, String> {
        let server = self
            .document
            .mcp_servers
            .get(server_id)
            .ok_or_else(|| format!("unknown MCP server: {server_id}"))?;
        let McpTransportConfig::StreamableHttp { authentication, .. } = &server.transport else {
            return Ok(None);
        };
        let McpHttpAuthentication::StaticHeader {
            header_name,
            value_prefix,
        } = authentication
        else {
            return Ok(None);
        };
        let secret = self
            .secrets
            .read(&mcp_http_auth_ref(server_id))?
            .ok_or_else(|| {
                format!(
                    "MCP server '{server_id}' needs an HTTP authentication credential; add it in Tool server settings"
                )
            })?;
        Ok(Some((
            header_name.clone(),
            format!("{value_prefix}{secret}"),
        )))
    }

    pub fn put_mcp_http_auth_secret(
        &mut self,
        server_id: &str,
        value: String,
    ) -> Result<(), String> {
        if value.trim().is_empty() {
            return Err("MCP HTTP authentication credential must not be empty".into());
        }
        if value.len() > 64 * 1024 {
            return Err("MCP HTTP authentication credential is too large".into());
        }
        let server = self
            .document
            .mcp_servers
            .get(server_id)
            .ok_or_else(|| format!("unknown MCP server: {server_id}"))?;
        if !matches!(
            server.transport,
            McpTransportConfig::StreamableHttp {
                authentication: McpHttpAuthentication::StaticHeader { .. },
                ..
            }
        ) {
            return Err(format!(
                "MCP server '{server_id}' is not configured for HTTP authentication"
            ));
        }
        self.secrets.write(&mcp_http_auth_ref(server_id), value)
    }

    pub fn clear_mcp_http_auth_secret(&mut self, server_id: &str) -> Result<(), String> {
        if !self.document.mcp_servers.contains_key(server_id) {
            return Err(format!("unknown MCP server: {server_id}"));
        }
        self.secrets.clear(&mcp_http_auth_ref(server_id))
    }

    pub fn has_mcp_http_auth_secret(&self, server_id: &str) -> Result<bool, String> {
        if !self.document.mcp_servers.contains_key(server_id) {
            return Err(format!("unknown MCP server: {server_id}"));
        }
        self.secrets.check(&mcp_http_auth_ref(server_id))
    }

    pub fn upsert_mcp_server(
        &mut self,
        server: McpServerConfigView,
    ) -> Result<McpServerConfigView, String> {
        validate_mcp_server_view(&server)?;
        let previous = self.document.clone();
        let previous_secret = if self
            .document
            .mcp_servers
            .get(&server.id)
            .is_some_and(|existing| existing.transport != server.transport)
        {
            let ref_ = mcp_http_auth_ref(&server.id);
            let value = self.secrets.read(&ref_)?;
            if value.is_some() {
                self.secrets.clear(&ref_)?;
            }
            value
        } else {
            None
        };
        self.document.mcp_servers.insert(
            server.id.clone(),
            McpServerConfig {
                display_name: server.display_name.clone(),
                transport: server.transport.clone(),
            },
        );
        if let Err(error) = self.persist() {
            if error.is_indeterminate() {
                return Err(error.into_message());
            }
            self.document = previous;
            let error = error.into_message();
            if let Some(value) = previous_secret {
                self.secrets
                    .write(&mcp_http_auth_ref(&server.id), value)
                    .map_err(|restore_error| {
                        format!("{error}; restoring MCP HTTP credential failed: {restore_error}")
                    })?;
            }
            return Err(error);
        }
        Ok(server)
    }

    pub fn delete_mcp_server(&mut self, server_id: &str) -> Result<(), String> {
        if !self.document.mcp_servers.contains_key(server_id) {
            return Err(format!("unknown MCP server: {server_id}"));
        }
        let previous = self.document.clone();
        let ref_ = mcp_http_auth_ref(server_id);
        let previous_secret = self.secrets.read(&ref_)?;
        if previous_secret.is_some() {
            self.secrets.clear(&ref_)?;
        }
        self.document.mcp_servers.remove(server_id);
        if let Err(error) = self.persist() {
            if error.is_indeterminate() {
                return Err(error.into_message());
            }
            self.document = previous;
            let error = error.into_message();
            if let Some(value) = previous_secret {
                self.secrets.write(&ref_, value).map_err(|restore_error| {
                    format!("{error}; restoring MCP HTTP credential failed: {restore_error}")
                })?;
            }
            return Err(error);
        }
        Ok(())
    }

    pub fn list_mcp_export_profiles(&self) -> Vec<McpExportProfileView> {
        self.document
            .mcp_exports
            .iter()
            .map(|(id, profile)| McpExportProfileView {
                id: id.clone(),
                profile: profile.clone(),
            })
            .collect()
    }

    pub fn mcp_export_profile(&self, profile_id: &str) -> Option<McpExportProfile> {
        self.document.mcp_exports.get(profile_id).cloned()
    }

    pub fn upsert_mcp_export_profile(
        &mut self,
        view: McpExportProfileView,
    ) -> Result<McpExportProfileView, String> {
        validate_mcp_export_profile(&view.id, &view.profile)?;
        let previous = self.document.clone();
        self.document
            .mcp_exports
            .insert(view.id.clone(), view.profile.clone());
        self.persist_current(previous)?;
        Ok(view)
    }

    pub fn set_mcp_export_enabled(
        &mut self,
        profile_id: &str,
        enabled: bool,
    ) -> Result<(), String> {
        let previous = self.document.clone();
        let profile = self
            .document
            .mcp_exports
            .get_mut(profile_id)
            .ok_or_else(|| format!("unknown MCP export profile: {profile_id}"))?;
        profile.enabled = enabled;
        self.persist_current(previous)?;
        Ok(())
    }

    pub fn begin_mcp_export_transition(
        &mut self,
        profile_id: &str,
        enabled: bool,
    ) -> Result<(), String> {
        if !self.document.mcp_exports.contains_key(profile_id) {
            return Err(format!("unknown MCP export profile: {profile_id}"));
        }
        let previous = self.document.clone();
        self.document
            .mcp_export_transitions
            .insert(profile_id.to_string(), enabled);
        self.persist_current(previous)?;
        Ok(())
    }

    pub fn complete_mcp_export_transition(&mut self, profile_id: &str) -> Result<(), String> {
        let Some(enabled) = self
            .document
            .mcp_export_transitions
            .get(profile_id)
            .copied()
        else {
            return Ok(());
        };
        let previous = self.document.clone();
        let Some(profile) = self.document.mcp_exports.get_mut(profile_id) else {
            self.document = previous;
            return Err(format!("unknown MCP export profile: {profile_id}"));
        };
        profile.enabled = enabled;
        self.document.mcp_export_transitions.remove(profile_id);
        self.persist_current(previous)?;
        Ok(())
    }

    pub fn mcp_export_transition(&self, profile_id: &str) -> Option<bool> {
        self.document
            .mcp_export_transitions
            .get(profile_id)
            .copied()
    }

    pub fn delete_mcp_export_profile(&mut self, profile_id: &str) -> Result<(), String> {
        if !self.document.mcp_exports.contains_key(profile_id) {
            return Err(format!("unknown MCP export profile: {profile_id}"));
        }
        let previous = self.document.clone();
        self.document.mcp_exports.remove(profile_id);
        self.document.mcp_export_transitions.remove(profile_id);
        self.persist_current(previous)?;
        // Only now revoke the bearer token. Clearing it first meant a failed
        // `persist()` restored the profile with its credential already
        // destroyed: `has_mcp_export_token` would report false and every
        // remote client using that token would stop authenticating against a
        // profile the user still has.
        self.secrets
            .clear(&mcp_export_token_ref(profile_id))
            .map_err(|error| {
                format!(
                    "MCP export profile '{profile_id}' was removed, but revoking its bearer \
                     token failed: {error}"
                )
            })?;
        Ok(())
    }

    pub fn mcp_gateway_settings(&self) -> McpGatewaySettings {
        self.document.mcp_gateway.clone()
    }

    /// Mint (or replace) the bearer token remote clients must present for
    /// this profile. The value is returned exactly once — afterwards only
    /// the gateway reads it, never the frontend.
    pub fn rotate_mcp_export_token(&mut self, profile_id: &str) -> Result<String, String> {
        if !self.document.mcp_exports.contains_key(profile_id) {
            return Err(format!("unknown MCP export profile: {profile_id}"));
        }
        let token = format!("mcp_{}", uuid::Uuid::new_v4().simple());
        self.secrets
            .write(&mcp_export_token_ref(profile_id), token.clone())?;
        Ok(token)
    }

    pub fn has_mcp_export_token(&self, profile_id: &str) -> bool {
        self.secrets
            .check(&mcp_export_token_ref(profile_id))
            .unwrap_or(false)
    }

    pub fn revoke_mcp_export_token(&mut self, profile_id: &str) -> Result<(), String> {
        if !self.document.mcp_exports.contains_key(profile_id) {
            return Err(format!("unknown MCP export profile: {profile_id}"));
        }
        self.secrets.clear(&mcp_export_token_ref(profile_id))
    }

    /// Gateway-side token lookup. Backend-only: no Tauri command exposes it.
    pub fn mcp_export_token(&self, profile_id: &str) -> Option<String> {
        self.secrets
            .read(&mcp_export_token_ref(profile_id))
            .ok()
            .flatten()
    }

    pub fn put_secret(
        &mut self,
        kernel: &mut Kernel,
        owner: &AppId,
        local_name: &str,
        value: String,
    ) -> Result<(), String> {
        if owner == &mcp_http_auth_owner() {
            return Err("MCP HTTP credentials use the dedicated host credential path".into());
        }
        if local_name.trim().is_empty() {
            return Err("secret local name must not be empty".into());
        }
        let ref_ = SecretRef {
            owner: owner.clone(),
            name: SecretName::new(local_name),
        };
        self.secrets.write(&ref_, value.clone())?;
        kernel.put_secret(ref_, value);
        self.refresh_active_llm_secret(kernel);
        Ok(())
    }

    pub fn clear_secret(
        &mut self,
        kernel: &mut Kernel,
        owner: &AppId,
        local_name: &str,
    ) -> Result<(), String> {
        if owner == &mcp_http_auth_owner() {
            return Err("MCP HTTP credentials use the dedicated host credential path".into());
        }
        if local_name.trim().is_empty() {
            return Err("secret local name must not be empty".into());
        }
        let ref_ = SecretRef {
            owner: owner.clone(),
            name: SecretName::new(local_name),
        };
        self.secrets.clear(&ref_)?;
        kernel.clear_secret(&ref_);
        self.refresh_active_llm_secret(kernel);
        Ok(())
    }

    /// Remove a persisted app-owned secret after the kernel has already
    /// revoked its broker copy. Kept separate so uninstall file cleanup does
    /// not need to hold the kernel mutex.
    pub fn clear_secret_persisted(
        &mut self,
        owner: &AppId,
        local_name: &str,
    ) -> Result<(), String> {
        if local_name.trim().is_empty() {
            return Err("secret local name must not be empty".into());
        }
        let ref_ = SecretRef {
            owner: owner.clone(),
            name: SecretName::new(local_name),
        };
        self.secrets.clear(&ref_)
    }

    pub fn has_secret(&self, owner: &AppId, local_name: &str) -> Result<bool, String> {
        if owner == &mcp_http_auth_owner() {
            return Err("MCP HTTP credentials use the dedicated host credential path".into());
        }
        let ref_ = SecretRef {
            owner: owner.clone(),
            name: SecretName::new(local_name),
        };
        self.secrets.check(&ref_)
    }

    pub fn current_llm_profile(&self) -> Result<Option<LlmProfileRuntime>, String> {
        let Some(connector_id) = active_llm_connector_id(&self.document) else {
            return Ok(None);
        };
        self.llm_profile(&connector_id)
            .map(Some)
            .map_err(|_| format!("missing default LLM profile: {connector_id}"))
    }

    /// Resolve a specific connector profile by its full connector id
    /// (e.g. `llm-provider/local-ollama`). Chat pins the active profile per
    /// message and passes it through `llm.generate`, so one message never
    /// silently splits across providers when the default changes mid-run.
    pub fn llm_profile(&self, connector_id: &str) -> Result<LlmProfileRuntime, String> {
        let connector = self
            .document
            .connectors
            .get(connector_id)
            .ok_or_else(|| format!("unknown LLM profile: {connector_id}"))?;
        Ok(LlmProfileRuntime {
            connector_id: connector_id.to_string(),
            kind: connector.kind,
            base_url: runtime_base_url(connector.kind, &connector.base_url),
            default_model: connector.default_model.clone(),
            default_variant: connector.default_variant,
            default_text_verbosity: connector.default_text_verbosity,
            api_key_secret_ref: connector.secret_refs.get("api_key").cloned(),
            oauth_secret_ref: connector.secret_refs.get("oauth").cloned(),
        })
    }

    /// Resolve a profile that Chat may pin without widening the invocation's
    /// broker-authorized credential snapshot. Credential-free local profiles
    /// can be selected directly; a credential-bearing profile must remain the
    /// active default because only that profile owns the `active_api_key`
    /// invocation alias.
    pub fn selectable_chat_llm_profile(
        &self,
        connector_id: &str,
    ) -> Result<LlmProfileRuntime, String> {
        let profile = self.llm_profile(connector_id)?;
        let needs_credential =
            profile.api_key_secret_ref.is_some() || profile.oauth_secret_ref.is_some();
        if needs_credential
            && self
                .current_llm_profile()?
                .is_none_or(|active| active.connector_id != connector_id)
        {
            return Err(format!(
                "LLM profile '{connector_id}' uses a credential and must be selected as Default for Chat before a model profile can use it"
            ));
        }
        Ok(profile)
    }

    /// Read an OAuth credential for a configured LLM profile. Backend-only:
    /// callers receive the serialized credential only inside trusted Rust.
    pub fn read_llm_profile_oauth_credential(
        &self,
        connector_id: &str,
    ) -> Result<Option<String>, String> {
        let profile = self.llm_profile(connector_id)?;
        let local_name = profile
            .oauth_secret_ref
            .ok_or_else(|| format!("LLM profile '{connector_id}' has no OAuth secret reference"))?;
        self.secrets.read(&SecretRef {
            owner: AppId::new("llm-provider"),
            name: SecretName::new(local_name),
        })
    }

    /// Persist a profile OAuth credential without exposing it to a frontend.
    /// Kernel alias synchronization is deliberately separate for handler-side
    /// rotations, which execute without access to the kernel mutex.
    pub fn write_llm_profile_oauth_credential_persisted(
        &mut self,
        connector_id: &str,
        credential: String,
    ) -> Result<(), String> {
        let credential =
            crate::llm_client::OAuthCredential::parse_serialized(&credential)?.serialize()?;
        let profile = self.llm_profile(connector_id)?;
        let local_name = profile
            .oauth_secret_ref
            .ok_or_else(|| format!("LLM profile '{connector_id}' has no OAuth secret reference"))?;
        self.secrets.write(
            &SecretRef {
                owner: AppId::new("llm-provider"),
                name: SecretName::new(local_name),
            },
            credential,
        )
    }

    pub fn write_llm_profile_oauth_credential(
        &mut self,
        kernel: &mut Kernel,
        connector_id: &str,
        credential: String,
    ) -> Result<(), String> {
        self.write_llm_profile_oauth_credential_persisted(connector_id, credential)?;
        self.refresh_active_llm_secret(kernel);
        Ok(())
    }

    pub fn sync_active_llm_secret(&self, kernel: &mut Kernel) -> Result<(), String> {
        let Some(profile) = self.current_llm_profile()? else {
            kernel.clear_secret(&SecretRef {
                owner: AppId::new("llm-provider"),
                name: active_llm_api_key_secret(),
            });
            return Ok(());
        };
        let Some(local_name) = profile.api_key_secret_ref.or(profile.oauth_secret_ref) else {
            return Ok(());
        };
        let owner = AppId::new("llm-provider");
        let ref_ = SecretRef {
            owner: owner.clone(),
            name: SecretName::new(&local_name),
        };
        let Some(secret_value) = self.secrets.read(&ref_)? else {
            return Err(format!(
                "active LLM profile '{}' requires stored secret '{}'",
                profile.connector_id, local_name
            ));
        };
        let active_ref = SecretRef {
            owner: AppId::new("llm-provider"),
            name: active_llm_api_key_secret(),
        };
        kernel.put_secret(active_ref, secret_value);
        Ok(())
    }

    /// Keep the broker's active LLM alias current. A profile may validly have
    /// no API key until the user enters one; handlers requiring it fail when
    /// invoked, while callers that require it up front use `sync_active_llm_secret`.
    pub fn refresh_active_llm_secret(&self, kernel: &mut Kernel) {
        let active_ref = SecretRef {
            owner: AppId::new("llm-provider"),
            name: active_llm_api_key_secret(),
        };
        // Resolve the active profile's key BEFORE mutating the broker. The old
        // clear-then-maybe-reput order left the alias empty whenever any step
        // failed — including a transient store read — silently breaking chat
        // even though the key was on disk. Decide first, mutate once.
        let local_name = match self.current_llm_profile() {
            Ok(Some(profile)) => profile.api_key_secret_ref.or(profile.oauth_secret_ref),
            Ok(None) => {
                kernel.clear_secret(&active_ref);
                return;
            }
            // No usable active profile: the alias must not linger.
            Err(_) => {
                kernel.clear_secret(&active_ref);
                return;
            }
        };
        let Some(local_name) = local_name else {
            // The active profile intentionally has no key (e.g. a local model).
            kernel.clear_secret(&active_ref);
            return;
        };
        let ref_ = SecretRef {
            owner: AppId::new("llm-provider"),
            name: SecretName::new(&local_name),
        };
        match self.secrets.read(&ref_) {
            // Key present: publish it under the active alias.
            Ok(Some(secret_value)) => kernel.put_secret(active_ref, secret_value),
            // Credential genuinely absent: reflect that the key is unavailable.
            Ok(None) => kernel.clear_secret(&active_ref),
            // Transient store error: keep whatever the broker already holds
            // rather than destroying a working key.
            Err(_) => {}
        }
    }

    /// Rehydrate all persisted secrets into the kernel broker and sync the
    /// active LLM key. Called at startup so the broker starts from the same
    /// state as the on-disk store, not just the synthetic active key.
    pub fn bootstrap_secrets(&self, kernel: &mut Kernel) -> Result<(), String> {
        for (ref_, value) in self.secrets.all()? {
            if ref_.owner == mcp_http_auth_owner() {
                continue;
            }
            kernel.put_secret(ref_, value);
        }
        // Diagnostic (quiet when healthy): the active-LLM alias `active_api_key`
        // is broker-only and re-derived here on every launch. If chat later
        // reports "secret 'active_api_key' has not been stored", a warning here
        // names the failing branch — and the *absence* of any warning means
        // this rehydration was skipped entirely (i.e. phased startup returned
        // early before `bootstrap_secrets` ran).
        match self.current_llm_profile() {
            Ok(Some(profile)) => {
                if let Some(local_name) = profile.api_key_secret_ref.or(profile.oauth_secret_ref) {
                    let ref_ = SecretRef {
                        owner: AppId::new("llm-provider"),
                        name: SecretName::new(&local_name),
                    };
                    match self.secrets.read(&ref_) {
                        Ok(Some(_)) => {}
                        Ok(None) => eprintln!(
                            "[secret-bootstrap] active profile '{}' key '{}' MISSING from secret store; active_api_key will be empty",
                            profile.connector_id, local_name
                        ),
                        Err(error) => eprintln!(
                            "[secret-bootstrap] active profile '{}' key '{}' read FAILED: {error}",
                            profile.connector_id, local_name
                        ),
                    }
                }
            }
            Ok(None) => {}
            Err(error) => eprintln!("[secret-bootstrap] no resolvable active LLM profile: {error}"),
        }
        self.refresh_active_llm_secret(kernel);
        Ok(())
    }

    pub fn test_connector_config(
        &self,
        connector_id: &str,
    ) -> Result<ConnectionTestResult, String> {
        run_connector_test(&self.connector_probe(connector_id)?)
    }

    pub fn connector_probe(&self, connector_id: &str) -> Result<ConnectorProbe, String> {
        let connector = self
            .document
            .connectors
            .get(connector_id)
            .ok_or_else(|| format!("unknown connector profile: {connector_id}"))?;
        Ok(self.draft_probe(
            connector.kind,
            &connector.base_url,
            connector.secret_refs.get("api_key").map(String::as_str),
        ))
    }

    fn validate_active_llm_credential(&self, config: &HostConfig) -> Result<(), String> {
        let Some(connector_id) = active_llm_connector_id(config) else {
            return Ok(());
        };
        let connector = config
            .connectors
            .get(&connector_id)
            .ok_or_else(|| format!("missing default LLM profile: {connector_id}"))?;
        if !connector.kind.defaults().oauth_credential_required {
            return Ok(());
        }
        let local_name = connector
            .secret_refs
            .get("oauth")
            .ok_or_else(|| format!("{connector_id} OAuth secret reference is required"))?;
        let credential = self.secrets.read(&SecretRef {
            owner: AppId::new("llm-provider"),
            name: SecretName::new(local_name),
        })?;
        let credential = credential.ok_or_else(|| {
            format!("OAuth LLM profile '{connector_id}' cannot be selected until login completes")
        })?;
        crate::llm_client::OAuthCredential::parse_serialized(&credential)
            .map(|_| ())
            .map_err(|error| {
                format!("OAuth LLM profile '{connector_id}' has an invalid credential: {error}")
            })
    }

    /// Build a probe from unsaved draft fields, so model discovery can run
    /// before the profile is complete enough to save (a default model is
    /// required to save, but discovery is how you find one).
    pub fn draft_probe(
        &self,
        kind: ConnectorKind,
        base_url: &str,
        api_key_secret_name: Option<&str>,
    ) -> ConnectorProbe {
        let owner = AppId::new("llm-provider");
        let api_key = api_key_secret_name.and_then(|name| {
            let ref_ = SecretRef {
                owner,
                name: SecretName::new(name),
            };
            self.secrets.read(&ref_).ok().flatten()
        });
        match kind {
            ConnectorKind::Ollama => ConnectorProbe {
                kind: ConnectorKind::Ollama,
                url: Some(format!("{}/api/tags", base_url.trim_end_matches('/'))),
                api_key: None,
            },
            ConnectorKind::OpenAiCompatible
            | ConnectorKind::Openai
            | ConnectorKind::Openrouter
            | ConnectorKind::Mistral => ConnectorProbe {
                kind,
                url: Some(format!("{}/models", base_url.trim_end_matches('/'))),
                api_key,
            },
            ConnectorKind::Anthropic
            | ConnectorKind::AnthropicOauth
            | ConnectorKind::OpenaiCodex
            | ConnectorKind::GithubCopilot
            | ConnectorKind::Google
            | ConnectorKind::AmazonBedrock => ConnectorProbe {
                kind,
                url: None,
                api_key,
            },
        }
    }

    fn persist(&self) -> Result<(), AtomicJsonError> {
        self.persist_document(&self.document)
    }

    fn persist_current(&mut self, previous: HostConfig) -> Result<(), String> {
        match self.persist() {
            Ok(()) => Ok(()),
            Err(error) if error.is_indeterminate() => Err(error.into_message()),
            Err(error) => {
                self.document = previous;
                Err(error.into_message())
            }
        }
    }

    fn persist_candidate(&mut self, candidate: HostConfig) -> Result<(), String> {
        match self.persist_document(&candidate) {
            Ok(()) => {
                self.document = candidate;
                Ok(())
            }
            Err(error) if error.is_indeterminate() => {
                self.document = candidate;
                Err(error.into_message())
            }
            Err(error) => Err(error.into_message()),
        }
    }

    fn persist_document(&self, document: &HostConfig) -> Result<(), AtomicJsonError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        persist_json_document(
            path,
            &HostConfigStoreDocument {
                version: 3,
                config: document.clone(),
            },
            "host config",
            self.writer.as_ref(),
        )
    }
}

/// Token secrets live under a synthetic owner so they are cleared with the
/// profile and can never collide with a real app's secrets.
fn mcp_export_token_ref(profile_id: &str) -> SecretRef {
    SecretRef {
        owner: AppId::new("mcp-export"),
        name: SecretName::new(format!("token-{profile_id}")),
    }
}

fn secret_store_path(config_path: &Path) -> PathBuf {
    if config_path.file_name().and_then(|name| name.to_str()) == Some("host-config.json") {
        return config_path.with_file_name("host-secrets.json");
    }
    let stem = config_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("host-config");
    config_path.with_file_name(format!("{stem}-secrets.json"))
}

pub fn run_connector_test(probe: &ConnectorProbe) -> Result<ConnectionTestResult, String> {
    let response = send_probe(probe, "connection failed")?;
    if response.status().is_success() {
        Ok(ConnectionTestResult {
            ok: true,
            message: "Connection succeeded".into(),
        })
    } else {
        Err(format!("provider returned {}", response.status()))
    }
}

fn send_probe(
    probe: &ConnectorProbe,
    failure_label: &str,
) -> Result<reqwest::blocking::Response, String> {
    let url = probe
        .url
        .as_ref()
        .ok_or_else(|| unsupported_direct_probe(probe.kind))?;
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| format!("http client setup failed: {error}"))?;
    let mut request = client.get(url);
    if let Some(key) = &probe.api_key {
        request = request.bearer_auth(key);
    }
    request
        .send()
        .map_err(|error| format!("{failure_label}: {error}"))
}

fn unsupported_direct_probe(kind: ConnectorKind) -> String {
    let kind = match kind {
        ConnectorKind::Ollama => "ollama",
        ConnectorKind::OpenAiCompatible => "open-ai-compatible",
        ConnectorKind::Openai => "openai",
        ConnectorKind::Anthropic => "anthropic",
        ConnectorKind::AnthropicOauth => "anthropic-oauth",
        ConnectorKind::OpenaiCodex => "openai-codex",
        ConnectorKind::GithubCopilot => "github-copilot",
        ConnectorKind::Openrouter => "openrouter",
        ConnectorKind::Google => "google",
        ConnectorKind::Mistral => "mistral",
        ConnectorKind::AmazonBedrock => "amazon-bedrock",
    };
    format!(
        "direct HTTP connection testing is unsupported for '{}' profiles; use worker model capabilities instead",
        kind
    )
}

fn default_host_config() -> HostConfig {
    let mut apps = BTreeMap::new();
    apps.insert(
        "chat".into(),
        AppConfigEntry {
            settings: Map::from_iter([
                (String::from("max_iterations"), Value::from(10)),
                (String::from("show_metadata"), Value::from(false)),
                (String::from("show_thinking"), Value::from(false)),
                (String::from("record_injected_context"), Value::from(false)),
            ]),
        },
    );
    apps.insert(
        "llm-provider".into(),
        AppConfigEntry {
            settings: Map::new(),
        },
    );

    HostConfig {
        version: 2,
        host: HostDefaults {
            default_llm_provider: "llm-provider".into(),
            default_llm_profile: None,
            cloud_llm_egress_accepted_profiles: vec![],
            app_data_backup_retention: 1,
        },
        apps,
        connectors: BTreeMap::new(),
        mcp_servers: BTreeMap::new(),
        mcp_exports: BTreeMap::new(),
        mcp_export_transitions: BTreeMap::new(),
        mcp_gateway: McpGatewaySettings::default(),
    }
}

pub(crate) const KESTRAL_GITMCP_SERVER_ID: &str = "kestral-docs";

fn fresh_host_config() -> HostConfig {
    let mut config = default_host_config();
    // This is a discoverable shortcut, not startup work. Saved MCP servers
    // remain inert until the owner explicitly chooses Connect.
    config.mcp_servers.insert(
        KESTRAL_GITMCP_SERVER_ID.into(),
        McpServerConfig {
            display_name: "Kestral documentation".into(),
            transport: McpTransportConfig::StreamableHttp {
                url: "https://gitmcp.io/ManuelZierl/kestral".into(),
                authentication: McpHttpAuthentication::None,
            },
        },
    );
    config
}

fn validate_host_config(config: &HostConfig) -> Result<(), String> {
    if config.version != 2 {
        return Err(format!("unsupported config version: {}", config.version));
    }
    if config.host.default_llm_provider.trim().is_empty() {
        return Err("default_llm_provider must not be empty".into());
    }
    if config
        .host
        .default_llm_profile
        .as_ref()
        .is_some_and(|profile| profile.trim().is_empty())
    {
        return Err("default_llm_profile must be null or non-empty".into());
    }
    if config.host.app_data_backup_retention == 0 {
        return Err("app_data_backup_retention must be at least 1".into());
    }
    for connector_id in &config.host.cloud_llm_egress_accepted_profiles {
        if connector_id.trim().is_empty() {
            return Err("cloud_llm_egress_accepted_profiles must not contain empty ids".into());
        }
    }
    if let Some(active_connector_id) = active_llm_connector_id(config) {
        if !config.connectors.contains_key(&active_connector_id) {
            return Err(format!(
                "missing default LLM profile: {active_connector_id}"
            ));
        }
    }
    for (id, connector) in &config.connectors {
        validate_connector_view(&ConnectorConfigView {
            id: id.clone(),
            kind: connector.kind,
            base_url: connector.base_url.clone(),
            default_model: connector.default_model.clone(),
            default_variant: connector.default_variant,
            default_text_verbosity: connector.default_text_verbosity,
            secret_refs: connector.secret_refs.clone(),
        })?;
    }
    for (id, server) in &config.mcp_servers {
        validate_mcp_server_view(&McpServerConfigView {
            id: id.clone(),
            display_name: server.display_name.clone(),
            transport: server.transport.clone(),
        })?;
    }
    for (id, profile) in &config.mcp_exports {
        validate_mcp_export_profile(id, profile)?;
    }
    validate_mcp_gateway_settings(&config.mcp_gateway)?;
    Ok(())
}

fn validate_mcp_export_profile(id: &str, profile: &McpExportProfile) -> Result<(), String> {
    if id.trim().is_empty() || id.contains(char::is_whitespace) {
        return Err("MCP export profile id must be non-empty and contain no whitespace".into());
    }
    if profile.display_name.trim().is_empty() {
        return Err(format!("MCP export profile '{id}' needs a display name"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for capability in &profile.capabilities {
        if capability.provider.trim().is_empty() || capability.capability.trim().is_empty() {
            return Err(format!(
                "MCP export profile '{id}' lists a capability with an empty provider or name"
            ));
        }
        if !seen.insert((capability.provider.as_str(), capability.capability.as_str())) {
            return Err(format!(
                "MCP export profile '{id}' lists '{}/{}' twice",
                capability.provider, capability.capability
            ));
        }
    }
    if profile.rate_limit_per_minute == 0 {
        return Err(format!(
            "MCP export profile '{id}' needs a rate limit of at least 1 per minute"
        ));
    }
    Ok(())
}

fn validate_mcp_gateway_settings(settings: &McpGatewaySettings) -> Result<(), String> {
    let address: std::net::SocketAddr = settings.bind_address.parse().map_err(|_| {
        format!(
            "invalid MCP gateway bind address: {}",
            settings.bind_address
        )
    })?;
    if !address.ip().is_loopback() {
        return Err(format!(
            "MCP gateway must bind a loopback address (got {}); expose it publicly through a \
             local tunnel, which is transport only — not authentication",
            settings.bind_address
        ));
    }
    if settings.oauth_enabled {
        return Err(
            "MCP gateway OAuth 2.1 protected-resource support is staged but not yet \
             implemented; keep oauth_enabled=false and use bearer-token auth"
                .into(),
        );
    }
    Ok(())
}

fn validate_mcp_server_view(server: &McpServerConfigView) -> Result<(), String> {
    if server.id.trim().is_empty() || server.id.contains(char::is_whitespace) {
        return Err("MCP server id must be non-empty and contain no whitespace".into());
    }
    if server.display_name.trim().is_empty() {
        return Err(format!("MCP server '{}' needs a display name", server.id));
    }
    match &server.transport {
        McpTransportConfig::Stdio { command, .. } => {
            if command.trim().is_empty() {
                return Err(format!(
                    "MCP server '{}' has an empty stdio command",
                    server.id
                ));
            }
        }
        McpTransportConfig::StreamableHttp {
            url,
            authentication,
        } => {
            let headers = match authentication {
                McpHttpAuthentication::None => Vec::new(),
                McpHttpAuthentication::StaticHeader {
                    header_name,
                    value_prefix,
                } => vec![(header_name.clone(), format!("{value_prefix}credential"))],
            };
            mcp_adapter::StreamableHttpTransport::validate_settings(url, &headers).map_err(
                |error| {
                    format!(
                        "MCP server '{}' has invalid HTTP settings: {error}",
                        server.id
                    )
                },
            )?;
        }
    }
    Ok(())
}

fn mcp_http_auth_ref(server_id: &str) -> SecretRef {
    SecretRef {
        owner: mcp_http_auth_owner(),
        name: SecretName::new(format!("{server_id}/http-auth")),
    }
}

fn mcp_http_auth_owner() -> AppId {
    AppId::new("com.ma-zierl.host.mcp-client")
}

fn active_llm_connector_id(config: &HostConfig) -> Option<String> {
    config
        .host
        .default_llm_profile
        .as_ref()
        .map(|profile| format!("{}/{}", config.host.default_llm_provider, profile))
}

fn connector_is_cloud(connector: &ConnectorConfig) -> bool {
    match connector.kind {
        ConnectorKind::Ollama => false,
        ConnectorKind::OpenAiCompatible => !is_local_base_url(&connector.base_url),
        ConnectorKind::Openai
        | ConnectorKind::Anthropic
        | ConnectorKind::AnthropicOauth
        | ConnectorKind::OpenaiCodex
        | ConnectorKind::GithubCopilot
        | ConnectorKind::Openrouter
        | ConnectorKind::Google
        | ConnectorKind::Mistral
        | ConnectorKind::AmazonBedrock => true,
    }
}

fn is_local_base_url(base_url: &str) -> bool {
    // Keep local-endpoint detection aligned with
    // host/src/lib/settings/connectorProfiles.ts.
    let Ok(url) = Url::parse(base_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host == "127.0.0.1"
        || host == "0.0.0.0"
        || host == "::1"
        || host.ends_with(".local")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host
            .strip_prefix("172.")
            .and_then(|suffix| suffix.split('.').next())
            .and_then(|octet| octet.parse::<u8>().ok())
            .map(|octet| (16..=31).contains(&octet))
            .unwrap_or(false)
}

fn cloud_profile_acknowledged(config: &HostConfig, connector_id: &str) -> bool {
    config
        .host
        .cloud_llm_egress_accepted_profiles
        .iter()
        .any(|accepted| accepted == connector_id)
}

fn validate_cloud_llm_activation_change(
    current: &HostConfig,
    next: &HostConfig,
) -> Result<(), String> {
    let Some(next_connector_id) = active_llm_connector_id(next) else {
        return Ok(());
    };
    let Some(next_connector) = next.connectors.get(&next_connector_id) else {
        return Err(format!("missing default LLM profile: {next_connector_id}"));
    };
    if !connector_is_cloud(next_connector) || cloud_profile_acknowledged(next, &next_connector_id) {
        return Ok(());
    }

    let current_connector_id = active_llm_connector_id(current);
    let current_is_same_unacknowledged_cloud = current_connector_id.as_deref()
        == Some(next_connector_id.as_str())
        && current_connector_id
            .as_ref()
            .and_then(|id| current.connectors.get(id))
            .map(connector_is_cloud)
            .unwrap_or(false)
        && current_connector_id
            .as_ref()
            .is_some_and(|id| !cloud_profile_acknowledged(current, id));
    if current_is_same_unacknowledged_cloud {
        return Ok(());
    }

    Err(format!(
        "cloud LLM profile '{next_connector_id}' may send chat context, tool inputs/results, and selected artifacts to an external provider. Chat's llm.generate grant is app-static today, so acknowledge the data-egress policy in Settings before making this the default profile"
    ))
}

fn validate_connector_view(connector: &ConnectorConfigView) -> Result<(), String> {
    let mut parts = connector.id.split('/');
    let provider = parts.next().unwrap_or_default().trim();
    let profile = parts.next().unwrap_or_default().trim();
    if provider.is_empty() || profile.is_empty() || parts.next().is_some() {
        return Err("connector id must be '<provider>/<profile>'".into());
    }
    if connector.base_url.trim().is_empty() {
        return Err(format!("{} base URL is required", connector.id));
    }
    if connector.default_model.trim().is_empty() {
        return Err(format!("{} default model is required", connector.id));
    }
    if connector.kind.defaults().api_key_required
        && connector
            .secret_refs
            .get("api_key")
            .is_none_or(|secret_ref| secret_ref.trim().is_empty())
    {
        return Err(format!(
            "{} API key secret reference is required",
            connector.id
        ));
    }
    if connector.kind.defaults().oauth_credential_required
        && connector
            .secret_refs
            .get("oauth")
            .is_none_or(|secret_ref| secret_ref.trim().is_empty())
    {
        return Err(format!(
            "{} OAuth secret reference is required",
            connector.id
        ));
    }
    if connector.kind.defaults().oauth_credential_required
        && connector
            .secret_refs
            .get("api_key")
            .is_some_and(|secret_ref| !secret_ref.trim().is_empty())
    {
        return Err(format!(
            "{} cannot configure both API key and OAuth secret references",
            connector.id
        ));
    }
    Ok(())
}

pub(crate) fn runtime_base_url(kind: ConnectorKind, base_url: &str) -> String {
    match kind {
        ConnectorKind::Ollama => {
            let trimmed = base_url.trim_end_matches('/');
            if trimmed.ends_with("/v1") {
                trimmed.to_string()
            } else {
                format!("{trimmed}/v1")
            }
        }
        ConnectorKind::OpenaiCodex => ConnectorKind::OpenaiCodex.defaults().base_url.into(),
        ConnectorKind::OpenAiCompatible
        | ConnectorKind::Openai
        | ConnectorKind::Anthropic
        | ConnectorKind::AnthropicOauth
        | ConnectorKind::GithubCopilot
        | ConnectorKind::Openrouter
        | ConnectorKind::Google
        | ConnectorKind::Mistral
        | ConnectorKind::AmazonBedrock => base_url.trim_end_matches('/').to_string(),
    }
}

fn validate_app_config(
    manifest: &AppManifest,
    app_id: &str,
    config: &JsonObject,
) -> Result<(), String> {
    match manifest.config_declarations.as_slice() {
        [] => Err(format!("app '{app_id}' declares no configurable settings")),
        [declaration] => validate_json_against_schema(
            &Value::Object(config.clone()),
            &declaration.json_schema,
            &format!("config declaration '{}' for app '{app_id}'", declaration.name),
        ),
        _ => Err(format!(
            "app '{app_id}' declares multiple config sections; host storage currently supports exactly one"
        )),
    }
}

fn validate_json_against_schema(
    instance: &Value,
    schema: &JsonObject,
    described_as: &str,
) -> Result<(), String> {
    let validator = validator_for(&Value::Object(schema.clone()))
        .map_err(|error| format!("invalid schema for {described_as}: {error}"))?;
    let mut messages: Vec<String> = validator
        .iter_errors(instance)
        .map(|error| format!("{}: {}", error.instance_path, error))
        .collect();
    if messages.is_empty() {
        return Ok(());
    }
    messages.sort();
    Err(format!("invalid {described_as}: {}", messages.join("; ")))
}

fn merge_value(target: &mut Value, patch: Value) {
    match (target, patch) {
        (Value::Object(target_object), Value::Object(patch_object)) => {
            for (key, patch_value) in patch_object {
                match target_object.get_mut(&key) {
                    Some(target_value) => merge_value(target_value, patch_value),
                    None => {
                        target_object.insert(key, patch_value);
                    }
                }
            }
        }
        (target_slot, patch_value) => *target_slot = patch_value,
    }
}

pub fn active_llm_api_key_secret() -> SecretName {
    SecretName::new("active_api_key")
}

#[cfg(test)]
mod tests;
