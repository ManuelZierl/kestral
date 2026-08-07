//! Installable app packages (`docs/writing-apps.md`): reading, inspection, and translation.
//!
//! A package is a directory with an `app.json` plus optional `ui/` and
//! `backend/` payload. This module parses and verifies `app.json`, produces a
//! static **inspection** (everything the manager shows before install), and
//! **translates** an approved package into a kernel `AppManifest` plus the
//! sandboxed surface UI bundles the host serves.
//!
//! Hard rule: nothing here executes package code. Inspection reads and hashes
//! files only; a backend process is spawned solely at activation, after the
//! user confirms (see `app_manager`). The kernel never sees a package — it
//! receives an ordinary sealed manifest and generic handlers.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use app_host_kernel::ids::{AppId, ArtifactTypeName, CapabilityName, EventTopic, SurfaceName};
use app_host_kernel::manifest::{
    seal, AgentDeclaration, AppManifest, ArtifactTypeDeclaration, AssistantProfileDeclaration,
    AutomationDeclaration, ConfigDeclaration, ConnectorDeclaration, ExtensionContribution,
    ExtensionPointDeclaration, GrantRequest, SealedManifest, SkillDeclaration,
};
use app_host_kernel::primitives::capability::{
    CapabilityDeclaration, CapabilityEffect, CapabilityRef,
};
use app_host_kernel::primitives::grant::{DataScope, GrantCondition, GrantDuration, GrantScope};
use app_host_kernel::primitives::surface::{SurfaceDeclaration, SurfaceKind};

use crate::publisher_trust::{PackageSignatureDocument, PublisherTrustStore, SignatureState};
use crate::surface_ui::SurfaceUiBundle;

/// Package-format generation this host understands.
pub const SUPPORTED_FORMAT_VERSION: u32 = 1;

/// This host's version, for `min_host_version` compatibility checks.
pub const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const APP_JSON: &str = "app.json";
pub(crate) const MAX_MANAGED_DATA_BATCH_OPERATIONS: u32 = 2048;

const MAX_APP_DOCUMENT_BYTES: usize = 1024 * 1024;
const MAX_SIGNATURE_DOCUMENT_BYTES: usize = 64 * 1024;
const MAX_SURFACE_UI_BYTES: usize = 32 * 1024 * 1024;
const MANAGED_DATA_UUID_PATTERN: &str =
    "^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$";
pub(crate) const MAX_PROPOSAL_PAYLOAD_BYTES: usize = 1024 * 1024;

pub fn resolve_package_directory(source: &Path) -> Result<PathBuf, String> {
    if source.join(APP_JSON).is_file() {
        return Ok(source.to_path_buf());
    }
    let built_package = source.join("dist");
    if built_package.join(APP_JSON).is_file() {
        return Ok(built_package);
    }
    Err(format!(
        "no app package found at '{}' (expected app.json or dist/app.json)",
        source.display()
    ))
}

// -- app.json shape -----------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageDocument {
    pub format_version: u32,
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    #[serde(default)]
    pub publisher: Option<Publisher>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub icon: Option<PackageIcon>,
    #[serde(default)]
    pub theme_colors: Vec<AppThemeColor>,
    pub min_host_version: String,
    pub manifest: PackageManifestBody,
    /// Grants this package asks the host to issue to another installed app so
    /// that consumer can use capabilities provided by this package.
    #[serde(default)]
    pub consumer_grant_requests: Vec<ConsumerGrantRequest>,
    pub backend: Backend,
    pub data: AppData,
    pub integrity: Integrity,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppThemeColor {
    pub name: String,
    pub title: String,
    pub description: String,
    pub light: String,
    pub dark: String,
}

#[derive(Debug, Clone)]
pub struct AppPresentationView {
    pub icon: Option<AppIconView>,
    pub theme_colors: Vec<AppThemeColor>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged, deny_unknown_fields)]
pub enum PackageIcon {
    Asset(String),
    Kestral {
        kind: KestralIconKind,
        name: KestralIconName,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum KestralIconKind {
    Kestral,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum KestralIconName {
    Activity,
    AppGrid,
    ArtifactBox,
    BookOpen,
    ChatBubble,
    CheckSquare,
    PencilRuler,
    Settings,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AppIconView {
    Asset {
        media_type: String,
        data_base64: String,
    },
    Kestral {
        name: KestralIconName,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerGrantRequest {
    pub holder: AppId,
    pub request: GrantRequest,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publisher {
    pub name: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub key_id: Option<String>,
}

/// Mirrors the kernel `AppManifest` contribution fields (identity is supplied
/// at the top level). Reuses the kernel types directly, so the wire shape is
/// exactly what the kernel expects — except surfaces, which carry an optional
/// host-only `ui` binding stripped before sealing.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PackageManifestBody {
    #[serde(default)]
    pub capabilities: Vec<CapabilityDeclaration>,
    #[serde(default)]
    pub surfaces: Vec<PackageSurface>,
    #[serde(default)]
    pub agents: Vec<AgentDeclaration>,
    #[serde(default)]
    pub skills: Vec<SkillDeclaration>,
    #[serde(default)]
    pub assistant_profiles: Vec<AssistantProfileDeclaration>,
    #[serde(default)]
    pub automations: Vec<AutomationDeclaration>,
    #[serde(default)]
    pub connectors: Vec<ConnectorDeclaration>,
    #[serde(default)]
    pub config_declarations: Vec<ConfigDeclaration>,
    #[serde(default)]
    pub artifact_types: Vec<ArtifactTypeDeclaration>,
    #[serde(default)]
    pub extension_points: Vec<ExtensionPointDeclaration>,
    #[serde(default)]
    pub extension_contributions: Vec<ExtensionContribution>,
    #[serde(default)]
    pub grant_requests: Vec<GrantRequest>,
    #[serde(default)]
    pub event_subscriptions: Vec<EventTopic>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum BackendAuthorityMode {
    Sandboxed,
    Unsandboxed,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSurface {
    pub name: SurfaceName,
    pub kind: SurfaceKind,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub intents: Vec<CapabilityRef>,
    #[serde(default)]
    pub ui: Option<PackageSurfaceUi>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSurfaceUi {
    pub entry: String,
    /// Rejected at translate time: the CSP is always host-authored
    /// (deny-by-default). Kept in the schema so a package that sets it fails
    /// with a clear error rather than a generic unknown-field one. Use
    /// `connect_src` to widen network access.
    #[serde(default)]
    pub csp: Option<String>,
    #[serde(default)]
    pub connect_src: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Backend {
    None,
    McpStdio {
        authority_mode: BackendAuthorityMode,
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    McpStreamableHttp {
        url: String,
    },
    Executable {
        authority_mode: BackendAuthorityMode,
        protocol: String,
        #[serde(default)]
        args: Vec<String>,
        platforms: BTreeMap<String, String>,
    },
    AgentWorker {
        authority_mode: BackendAuthorityMode,
        protocol_version: u32,
        entry: String,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AppData {
    None,
    Versioned {
        format_version: u32,
        migration: AppDataMigration,
    },
    HostManaged {
        contract_version: u32,
        collections: BTreeMap<String, ManagedDataCollection>,
        #[serde(default)]
        documents: BTreeMap<String, ManagedDocumentCollection>,
        limits: ManagedDataStoreLimits,
        exports: Vec<ManagedDataExport>,
        #[serde(default)]
        proposals: Vec<ManagedDataProposal>,
    },
}

impl AppData {
    pub fn format_version(&self) -> Option<u32> {
        match self {
            Self::None => None,
            Self::Versioned { format_version, .. } => Some(*format_version),
            Self::HostManaged { .. } => None,
        }
    }

    pub fn summary(&self) -> AppDataSummary {
        match self {
            Self::None => AppDataSummary {
                kind: "none".into(),
                format_version: None,
                migration_protocol_version: None,
                transitions: Vec::new(),
                contract_version: None,
                total_bytes: None,
                batch_operations: None,
                collections: Vec::new(),
                documents: Vec::new(),
                proposals: Vec::new(),
            },
            Self::Versioned {
                format_version,
                migration,
            } => AppDataSummary {
                kind: "versioned".into(),
                format_version: Some(*format_version),
                migration_protocol_version: Some(migration.protocol_version),
                transitions: migration.transitions.clone(),
                contract_version: None,
                total_bytes: None,
                batch_operations: None,
                collections: Vec::new(),
                documents: Vec::new(),
                proposals: Vec::new(),
            },
            Self::HostManaged {
                contract_version,
                collections,
                documents,
                limits,
                proposals,
                ..
            } => AppDataSummary {
                kind: "host-managed".into(),
                format_version: None,
                migration_protocol_version: None,
                transitions: Vec::new(),
                contract_version: Some(*contract_version),
                total_bytes: Some(limits.total_bytes),
                batch_operations: limits.batch_operations,
                collections: collections
                    .iter()
                    .map(|(name, declaration)| ManagedDataCollectionSummary {
                        name: name.clone(),
                        schema: declaration.schema.clone(),
                        operations: declaration.operations.iter().copied().collect(),
                        records: declaration.limits.records,
                        record_bytes: declaration.limits.record_bytes,
                        query_results: declaration.limits.query_results,
                        indexes: declaration
                            .indexes
                            .iter()
                            .map(|index| index.name.clone())
                            .collect(),
                        unique_indexes: declaration
                            .indexes
                            .iter()
                            .filter(|index| index.unique)
                            .map(|index| index.name.clone())
                            .collect(),
                    })
                    .collect(),
                documents: documents
                    .iter()
                    .map(|(name, declaration)| ManagedDocumentCollectionSummary {
                        name: name.clone(),
                        metadata_schema: declaration.metadata_schema.clone(),
                        operations: declaration.operations.iter().copied().collect(),
                        documents: declaration.limits.documents,
                        metadata_bytes: declaration.limits.metadata_bytes,
                        content_bytes: declaration.limits.content_bytes,
                    })
                    .collect(),
                proposals: proposals
                    .iter()
                    .map(ManagedDataProposalSummary::from)
                    .collect(),
            },
        }
    }

    pub fn host_managed(&self) -> Option<HostManagedDataRef<'_>> {
        match self {
            Self::HostManaged {
                contract_version,
                collections,
                documents,
                limits,
                exports,
                proposals,
            } => Some(HostManagedDataRef {
                contract_version: *contract_version,
                collections,
                documents,
                limits,
                exports,
                proposals,
            }),
            Self::None | Self::Versioned { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HostManagedDataRef<'a> {
    pub contract_version: u32,
    pub collections: &'a BTreeMap<String, ManagedDataCollection>,
    pub documents: &'a BTreeMap<String, ManagedDocumentCollection>,
    pub limits: &'a ManagedDataStoreLimits,
    pub exports: &'a [ManagedDataExport],
    pub proposals: &'a [ManagedDataProposal],
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDataCollection {
    pub schema: app_host_kernel::JsonObject,
    #[serde(default)]
    pub indexes: Vec<ManagedDataIndex>,
    pub operations: BTreeSet<ManagedDataOperation>,
    pub limits: ManagedDataCollectionLimits,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDocumentCollection {
    pub metadata_schema: app_host_kernel::JsonObject,
    pub operations: BTreeSet<ManagedDocumentOperation>,
    pub limits: ManagedDocumentCollectionLimits,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedDocumentOperation {
    Get,
    List,
    Create,
    Replace,
    UpdateMetadata,
    Delete,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDocumentCollectionLimits {
    pub documents: u32,
    pub metadata_bytes: u32,
    pub content_bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDataIndex {
    pub name: String,
    pub field: String,
    pub value_schema: app_host_kernel::JsonObject,
    #[serde(default)]
    pub unique: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedDataOperation {
    Get,
    List,
    Create,
    Replace,
    Delete,
    Transaction,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDataCollectionLimits {
    pub records: u32,
    pub record_bytes: u32,
    pub query_results: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDataStoreLimits {
    pub total_bytes: u64,
    pub transaction_operations: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_operations: Option<u32>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedDataExportOperation {
    Get,
    List,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedDataExportHostInput {
    CurrentChatThreadId,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDataExport {
    pub capability: CapabilityName,
    pub operation: ManagedDataExportOperation,
    pub collection: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals_host_input: Option<ManagedDataExportHostInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ManagedDataProposalTarget {
    Collection { collection: String },
    Record { collection: String },
    Document { document_collection: String },
}

impl ManagedDataProposalTarget {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Collection { .. } => "collection",
            Self::Record { .. } => "record",
            Self::Document { .. } => "document",
        }
    }

    pub fn collection(&self) -> &str {
        match self {
            Self::Collection { collection } | Self::Record { collection } => collection,
            Self::Document {
                document_collection,
            } => document_collection,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDataProposal {
    pub capability: CapabilityName,
    pub artifact_type: ArtifactTypeName,
    pub title: String,
    pub description: String,
    pub target: ManagedDataProposalTarget,
    pub payload_schema: app_host_kernel::JsonObject,
    pub max_payload_bytes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDataProposalSummary {
    pub capability: String,
    pub artifact_type: String,
    pub title: String,
    pub description: String,
    pub target_kind: String,
    pub collection: String,
    pub max_payload_bytes: u32,
    pub payload_schema: app_host_kernel::JsonObject,
}

impl From<&ManagedDataProposal> for ManagedDataProposalSummary {
    fn from(proposal: &ManagedDataProposal) -> Self {
        Self {
            capability: proposal.capability.to_string(),
            artifact_type: proposal.artifact_type.to_string(),
            title: proposal.title.clone(),
            description: proposal.description.clone(),
            target_kind: proposal.target.kind().into(),
            collection: proposal.target.collection().into(),
            max_payload_bytes: proposal.max_payload_bytes,
            payload_schema: proposal.payload_schema.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppDataMigration {
    pub protocol_version: u32,
    pub command: String,
    pub entry: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub transitions: Vec<AppDataTransition>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppDataTransition {
    pub from: u32,
    pub to: u32,
    pub destructive: bool,
}

impl Backend {
    pub fn kind_label(&self) -> &'static str {
        match self {
            Backend::None => "none",
            Backend::McpStdio { .. } => "mcp-stdio",
            Backend::McpStreamableHttp { .. } => "mcp-streamable-http",
            Backend::Executable { .. } => "executable",
            Backend::AgentWorker { .. } => "agent-worker",
        }
    }

    pub fn authority_mode(&self) -> Option<BackendAuthorityMode> {
        match self {
            Backend::McpStdio { authority_mode, .. }
            | Backend::Executable { authority_mode, .. }
            | Backend::AgentWorker { authority_mode, .. } => Some(*authority_mode),
            Backend::None | Backend::McpStreamableHttp { .. } => None,
        }
    }

    fn detail(&self) -> String {
        match self {
            Backend::None => "No backend process — UI, data, and cross-app actions only".into(),
            Backend::McpStdio {
                authority_mode,
                command,
                ..
            } => format!(
                "MCP over stdio: {command} ({})",
                authority_mode_label(*authority_mode)
            ),
            Backend::McpStreamableHttp { url } => format!("MCP Streamable HTTP: {url}"),
            Backend::Executable {
                authority_mode,
                platforms,
                ..
            } => {
                format!(
                    "Packaged executable (MCP stdio), {} platform(s), {}",
                    platforms.len(),
                    authority_mode_label(*authority_mode),
                )
            }
            Backend::AgentWorker {
                authority_mode,
                protocol_version,
                ..
            } => format!(
                "Agent worker protocol v{protocol_version} ({})",
                authority_mode_label(*authority_mode)
            ),
        }
    }
}

pub fn authority_mode_label(mode: BackendAuthorityMode) -> &'static str {
    match mode {
        BackendAuthorityMode::Sandboxed => "sandboxed",
        BackendAuthorityMode::Unsandboxed => "unsandboxed",
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Integrity {
    pub algorithm: String,
    pub assets: BTreeMap<String, String>,
}

// -- Inspection views (serialized to the manager UI) --------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageInspection {
    /// Opaque host-owned staging identity. Installation accepts this value,
    /// never the mutable source path.
    pub staged_id: String,
    pub package_digest: String,
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub publisher: Option<PublisherView>,
    pub license: Option<String>,
    pub signature: SignatureState,
    pub signature_public_key: Option<String>,
    pub backend_kind: String,
    pub backend_detail: String,
    pub backend_authority_mode: Option<BackendAuthorityMode>,
    pub data: AppDataSummary,
    pub min_host_version: String,
    pub host_version: String,
    pub host_compatible: bool,
    pub capabilities: Vec<CapabilitySummary>,
    pub grant_requests: Vec<GrantRequestSummary>,
    pub extension_contributions: Vec<ExtensionContributionSummary>,
    pub surfaces: Vec<SurfaceSummary>,
    pub config: Vec<ConfigSummary>,
    pub secrets: Vec<SecretSummary>,
    pub artifact_types: Vec<String>,
    pub event_subscriptions: Vec<String>,
    pub integrity_ok: bool,
    pub integrity_error: Option<String>,
    pub warnings: Vec<String>,
    /// True only if the package can be installed as-is (format, host version,
    /// id rules, integrity, and signature trust all pass).
    pub installable: bool,
    pub blocking_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppDataSummary {
    pub kind: String,
    pub format_version: Option<u32>,
    pub migration_protocol_version: Option<u32>,
    pub transitions: Vec<AppDataTransition>,
    pub contract_version: Option<u32>,
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub batch_operations: Option<u32>,
    pub collections: Vec<ManagedDataCollectionSummary>,
    pub documents: Vec<ManagedDocumentCollectionSummary>,
    pub proposals: Vec<ManagedDataProposalSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDataCollectionSummary {
    pub name: String,
    pub schema: app_host_kernel::JsonObject,
    pub operations: Vec<ManagedDataOperation>,
    pub records: u32,
    pub record_bytes: u32,
    pub query_results: u32,
    pub indexes: Vec<String>,
    pub unique_indexes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDocumentCollectionSummary {
    pub name: String,
    pub metadata_schema: app_host_kernel::JsonObject,
    pub operations: Vec<ManagedDocumentOperation>,
    pub documents: u32,
    pub metadata_bytes: u32,
    pub content_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherView {
    pub name: String,
    pub homepage: Option<String>,
    pub key_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySummary {
    pub name: String,
    pub description: String,
    pub effect: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GrantRequestSummary {
    pub scope_label: String,
    pub data_scope_label: String,
    pub condition: String,
    pub reason: String,
    pub duration_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionContributionSummary {
    pub target_app: String,
    pub extension_point: String,
    pub contract_version: u32,
    pub surface: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceSummary {
    pub name: String,
    pub kind: String,
    pub title: String,
    pub has_custom_ui: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigSummary {
    pub name: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecretSummary {
    pub connector: String,
    pub name: String,
    pub description: String,
}

// -- Reading & parsing --------------------------------------------------------

/// Read and parse a package's `app.json`. Runs no package code.
pub fn read_document(package_dir: &Path) -> Result<PackageDocument, String> {
    let path = package_dir.join(APP_JSON);
    let raw = read_bounded_utf8(&path, APP_JSON, MAX_APP_DOCUMENT_BYTES)?;
    let value: Value =
        serde_json::from_str(&raw).map_err(|error| format!("invalid {APP_JSON}: {error}"))?;
    validate_public_schema(&value)?;
    let document: PackageDocument =
        serde_json::from_value(value).map_err(|error| format!("invalid {APP_JSON}: {error}"))?;
    Ok(document)
}

pub fn read_signature_document(
    package_dir: &Path,
) -> Result<Option<PackageSignatureDocument>, String> {
    let path = package_dir.join("app.signature.json");
    if !path.is_file() {
        return Ok(None);
    }
    let raw = read_bounded_utf8(&path, "app.signature.json", MAX_SIGNATURE_DOCUMENT_BYTES)?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid app.signature.json: {error}"))?;
    validate_signature_schema(&value)?;
    let document: PackageSignatureDocument = serde_json::from_value(value)
        .map_err(|error| format!("invalid app.signature.json: {error}"))?;
    Ok(Some(document))
}

fn validate_public_schema(value: &Value) -> Result<(), String> {
    let schema: Value = serde_json::from_str(include_str!("../../../schemas/app.schema.json"))
        .map_err(|error| format!("bundled app package schema is invalid JSON: {error}"))?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|error| format!("bundled app package schema is invalid: {error}"))?;
    let mut errors: Vec<String> = validator
        .iter_errors(value)
        .map(|error| format!("{}: {error}", error.instance_path))
        .collect();
    if errors.is_empty() {
        return Ok(());
    }
    errors.sort();
    Err(format!(
        "{APP_JSON} does not match the public schema: {}",
        errors.join("; ")
    ))
}

fn validate_signature_schema(value: &Value) -> Result<(), String> {
    let schema: Value = serde_json::from_str(include_str!("../../../schemas/app.schema.json"))
        .map_err(|error| format!("bundled app package schema is invalid JSON: {error}"))?;
    let schema = schema
        .pointer("/$defs/SignatureDocument")
        .cloned()
        .ok_or_else(|| {
            "bundled app package schema is missing definition 'SignatureDocument'".to_string()
        })?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|error| format!("bundled app package schema is invalid: {error}"))?;
    let mut errors: Vec<String> = validator
        .iter_errors(value)
        .map(|error| format!("{}: {error}", error.instance_path))
        .collect();
    if errors.is_empty() {
        return Ok(());
    }
    errors.sort();
    Err(format!(
        "app.signature.json does not match the public schema: {}",
        errors.join("; ")
    ))
}

fn read_bounded_utf8(path: &Path, label: &str, max_bytes: usize) -> Result<String, String> {
    let mut file = open_regular_nofollow(path, label)?;
    let length = file
        .metadata()
        .map_err(|error| format!("inspect {label} failed: {error}"))?
        .len();
    if length > max_bytes as u64 {
        return Err(format!(
            "{label} is {length} bytes; the maximum is {max_bytes}"
        ));
    }

    let mut bytes = Vec::with_capacity(length as usize);
    file.by_ref()
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {label} failed: {error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!(
            "{label} is larger than the maximum of {max_bytes} bytes"
        ));
    }
    String::from_utf8(bytes).map_err(|_| format!("{label} is not valid UTF-8"))
}

/// Validate the format-independent structural rules a package must satisfy
/// before it can be installed. Returns a blocking error string if any fail.
pub(crate) fn structural_error(document: &PackageDocument) -> Option<String> {
    if document.format_version != SUPPORTED_FORMAT_VERSION {
        return Some(format!(
            "unsupported package format version {} (this host supports {SUPPORTED_FORMAT_VERSION})",
            document.format_version
        ));
    }
    if !id_is_valid(&document.id) {
        return Some(format!(
            "invalid app id '{}': must be reverse-DNS (contain a dot) and not start with 'mcp-'",
            document.id
        ));
    }
    if let Err(error) = Version::parse(&document.version) {
        return Some(format!(
            "version '{}' is not strict semver: {error}",
            document.version
        ));
    }
    if let Err(error) = Version::parse(&document.min_host_version) {
        return Some(format!(
            "min_host_version '{}' is not strict semver: {error}",
            document.min_host_version
        ));
    }
    if let Err(error) = Version::parse(HOST_VERSION) {
        return Some(format!(
            "host version '{HOST_VERSION}' is not strict semver: {error}"
        ));
    }
    if document.integrity.algorithm != "sha256" {
        return Some(format!(
            "unsupported integrity algorithm '{}'",
            document.integrity.algorithm
        ));
    }
    let mut theme_color_names = BTreeSet::new();
    for color in &document.theme_colors {
        if !theme_color_names.insert(color.name.as_str()) {
            return Some(format!(
                "theme_colors contains duplicate name '{}'",
                color.name
            ));
        }
        if !theme_color_value_is_valid(&color.light) || !theme_color_value_is_valid(&color.dark) {
            return Some(format!(
                "theme color '{}' must provide valid HEX, rgb(), or rgba() light and dark values",
                color.name
            ));
        }
    }
    // A `none` backend contributes capabilities only through fixed host-managed
    // exports or proposals. No publisher code runs behind those declarations.
    if matches!(document.backend, Backend::None)
        && !document.manifest.capabilities.is_empty()
        && !matches!(document.data, AppData::HostManaged { .. })
    {
        return Some(
            "a 'none' backend must not declare capabilities (nothing would back them)".into(),
        );
    }
    if let AppData::HostManaged {
        contract_version,
        collections,
        documents,
        limits,
        exports,
        proposals,
    } = &document.data
    {
        if !matches!(document.backend, Backend::None) {
            return Some("host-managed data contract v1 requires a 'none' backend".into());
        }
        if let Err(error) = validate_host_managed_data(
            *contract_version,
            collections,
            documents,
            limits,
            exports,
            proposals,
            &document.manifest.capabilities,
            &document.manifest.artifact_types,
            &document.id,
        ) {
            return Some(error);
        }
    }
    if let AppData::Versioned {
        format_version,
        migration,
    } = &document.data
    {
        if *format_version == 0 {
            return Some("versioned app data requires a positive format_version".into());
        }
        if !matches!(
            document.backend,
            Backend::McpStdio { .. } | Backend::Executable { .. }
        ) {
            return Some(
                "versioned app data requires an mcp-stdio or executable local backend".into(),
            );
        }
        if migration.protocol_version != 1 {
            return Some(format!(
                "unsupported app-data migration protocol version {}",
                migration.protocol_version
            ));
        }
        if !is_safe_relative(&migration.entry) {
            return Some("app-data migration entry is not a safe package path".into());
        }
        if !document.integrity.assets.contains_key(&migration.entry) {
            return Some(format!(
                "app-data migration entry '{}' is not declared in integrity.assets",
                migration.entry
            ));
        }
        if migration.command != migration.entry
            && !migration
                .args
                .iter()
                .any(|argument| argument == &migration.entry)
        {
            return Some(
                "app-data migration command must execute its declared package entry".into(),
            );
        }
        let mut transitions = BTreeSet::new();
        for transition in &migration.transitions {
            if transition.from == 0 || transition.to == 0 || transition.from == transition.to {
                return Some(
                    "app-data migration transitions require distinct positive versions".into(),
                );
            }
            if transition.from != *format_version && transition.to != *format_version {
                return Some(format!(
                    "app-data migration transition {} -> {} is unrelated to declared format {}",
                    transition.from, transition.to, format_version
                ));
            }
            if !transitions.insert((transition.from, transition.to)) {
                return Some(format!(
                    "duplicate app-data migration transition {} -> {}",
                    transition.from, transition.to
                ));
            }
        }
    }
    if matches!(document.backend, Backend::AgentWorker { .. }) {
        let capabilities: Vec<&str> = document
            .manifest
            .capabilities
            .iter()
            .map(|capability| capability.name.as_str())
            .collect();
        if capabilities != ["agent.run"] {
            return Some(
                "an 'agent-worker' backend must declare exactly the 'agent.run' capability".into(),
            );
        }
        if !document
            .manifest
            .artifact_types
            .iter()
            .any(|artifact| artifact.name.as_str() == "agent-transcript")
        {
            return Some(
                "an 'agent-worker' backend must declare the 'agent-transcript' artifact type"
                    .into(),
            );
        }
    }
    for consumer in &document.consumer_grant_requests {
        if consumer.holder.as_str() == document.id {
            return Some(
                "consumer_grant_requests must name another app; use manifest.grant_requests for the package itself"
                    .into(),
            );
        }
        if consumer.request.scope.provider().as_str() != document.id {
            return Some(
                "consumer_grant_requests may only expose capabilities provided by this package"
                    .into(),
            );
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn validate_host_managed_data(
    contract_version: u32,
    collections: &BTreeMap<String, ManagedDataCollection>,
    documents: &BTreeMap<String, ManagedDocumentCollection>,
    limits: &ManagedDataStoreLimits,
    exports: &[ManagedDataExport],
    proposals: &[ManagedDataProposal],
    capabilities: &[CapabilityDeclaration],
    artifact_types: &[ArtifactTypeDeclaration],
    app_id: &str,
) -> Result<(), String> {
    if !matches!(contract_version, 1 | 2) {
        return Err(format!(
            "unsupported host-managed data contract version {contract_version}"
        ));
    }
    if collections.len() > 64 {
        return Err("host-managed data exceeds the 64-collection limit".into());
    }
    if contract_version == 1 && collections.is_empty() {
        return Err("host-managed contract v1 requires 1-64 collections".into());
    }
    if contract_version == 2 && collections.is_empty() && documents.is_empty() {
        return Err(
            "host-managed contract v2 requires at least one record or document collection".into(),
        );
    }
    if !(1024..=64 * 1024 * 1024).contains(&limits.total_bytes) {
        return Err("host-managed total_bytes must be between 1024 and 67108864".into());
    }
    if !(1..=64).contains(&limits.transaction_operations) {
        return Err("host-managed transaction_operations must be between 1 and 64".into());
    }
    if contract_version == 1 {
        if limits.batch_operations.is_some() {
            return Err("host-managed contract v1 must not declare batch_operations".into());
        }
    } else {
        let Some(batch_operations) = limits.batch_operations else {
            return Err("host-managed contract v2 requires batch_operations".into());
        };
        if !(1..=MAX_MANAGED_DATA_BATCH_OPERATIONS).contains(&batch_operations) {
            return Err(format!(
                "host-managed batch_operations must be between 1 and {MAX_MANAGED_DATA_BATCH_OPERATIONS}"
            ));
        }
    }

    for (name, collection) in collections {
        validate_managed_data_name(name, "collection")?;
        if collection.schema.get("type").and_then(Value::as_str) != Some("object")
            || collection.schema.get("additionalProperties") != Some(&Value::Bool(false))
        {
            return Err(format!(
                "host-managed collection '{name}' schema must have type 'object' and additionalProperties false"
            ));
        }
        jsonschema::validator_for(&Value::Object(collection.schema.clone())).map_err(|error| {
            format!("host-managed collection '{name}' schema is invalid: {error}")
        })?;
        if collection.operations.is_empty() {
            return Err(format!(
                "host-managed collection '{name}' must declare at least one operation"
            ));
        }
        if !(1..=100_000).contains(&collection.limits.records)
            || !(2..=1024 * 1024).contains(&collection.limits.record_bytes)
            || !(1..=1000).contains(&collection.limits.query_results)
        {
            return Err(format!(
                "host-managed collection '{name}' has limits outside host bounds"
            ));
        }
        if collection.indexes.len() > 16 {
            return Err(format!(
                "host-managed collection '{name}' exceeds the 16-index limit"
            ));
        }
        let properties = collection
            .schema
            .get("properties")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                format!("host-managed collection '{name}' schema must declare properties")
            })?;
        let mut index_names = BTreeSet::new();
        for index in &collection.indexes {
            validate_managed_data_name(&index.name, "index")?;
            validate_managed_data_name(&index.field, "index field")?;
            if !index_names.insert(index.name.as_str()) {
                return Err(format!(
                    "host-managed collection '{name}' contains duplicate index '{}'",
                    index.name
                ));
            }
            let property_schema = properties.get(&index.field).ok_or_else(|| {
                format!(
                    "host-managed collection '{name}' index '{}' references unknown field '{}'",
                    index.name, index.field
                )
            })?;
            if property_schema != &Value::Object(index.value_schema.clone()) {
                return Err(format!(
                    "host-managed collection '{name}' index '{}' value_schema must equal the schema for field '{}'",
                    index.name, index.field
                ));
            }
            jsonschema::validator_for(&Value::Object(index.value_schema.clone())).map_err(
                |error| {
                    format!(
                        "host-managed collection '{name}' index '{}' schema is invalid: {error}",
                        index.name
                    )
                },
            )?;
        }
    }

    if contract_version == 1 && !documents.is_empty() {
        return Err("host-managed contract v1 must not declare document collections".into());
    }
    if contract_version == 1 && !proposals.is_empty() {
        return Err("host-managed contract v1 must not declare proposals".into());
    }
    if documents.len() > 32 {
        return Err("host-managed data exceeds the 32-document-collection limit".into());
    }
    for (name, collection) in documents {
        validate_managed_data_name(name, "document collection")?;
        if collection
            .metadata_schema
            .get("type")
            .and_then(Value::as_str)
            != Some("object")
            || collection.metadata_schema.get("additionalProperties") != Some(&Value::Bool(false))
        {
            return Err(format!(
                "host-managed document collection '{name}' metadata_schema must have type 'object' and additionalProperties false"
            ));
        }
        jsonschema::validator_for(&Value::Object(collection.metadata_schema.clone())).map_err(
            |error| {
                format!(
                    "host-managed document collection '{name}' metadata schema is invalid: {error}"
                )
            },
        )?;
        if collection.operations.is_empty()
            || !(1..=10_000).contains(&collection.limits.documents)
            || !(2..=64 * 1024).contains(&collection.limits.metadata_bytes)
            || !(1..=8 * 1024 * 1024).contains(&collection.limits.content_bytes)
        {
            return Err(format!(
                "host-managed document collection '{name}' has limits or operations outside host bounds"
            ));
        }
    }

    if exports.len() + proposals.len() > 64 {
        return Err("host-managed data exceeds the 64-export limit".into());
    }
    let capability_by_name: BTreeMap<&CapabilityName, &CapabilityDeclaration> = capabilities
        .iter()
        .map(|capability| (&capability.name, capability))
        .collect();
    let mut exported_names = BTreeSet::new();
    for export in exports {
        if !exported_names.insert(&export.capability) {
            return Err(format!(
                "host-managed capability '{}' is exported more than once",
                export.capability
            ));
        }
        let collection = collections.get(&export.collection).ok_or_else(|| {
            format!(
                "host-managed export '{}' references unknown collection '{}'",
                export.capability, export.collection
            )
        })?;
        let required_operation = match export.operation {
            ManagedDataExportOperation::Get => ManagedDataOperation::Get,
            ManagedDataExportOperation::List => ManagedDataOperation::List,
        };
        if !collection.operations.contains(&required_operation) {
            return Err(format!(
                "host-managed export '{}' uses an operation not enabled for collection '{}'",
                export.capability, export.collection
            ));
        }
        let index = match (&export.operation, &export.index) {
            (ManagedDataExportOperation::Get, Some(_)) => {
                return Err(format!(
                    "host-managed get export '{}' must not declare an index",
                    export.capability
                ))
            }
            (ManagedDataExportOperation::List, Some(name)) => Some(
                collection
                    .indexes
                    .iter()
                    .find(|index| index.name == *name)
                    .ok_or_else(|| {
                        format!(
                            "host-managed list export '{}' references unknown index '{name}'",
                            export.capability
                        )
                    })?,
            ),
            _ => None,
        };
        if export.equals_host_input.is_some()
            && (!matches!(export.operation, ManagedDataExportOperation::List) || index.is_none())
        {
            return Err(format!(
                "host-managed export '{}' may bind equality host input only for an indexed list",
                export.capability
            ));
        }
        if export.equals_host_input == Some(ManagedDataExportHostInput::CurrentChatThreadId)
            && index
                .and_then(|index| index.value_schema.get("type"))
                .and_then(Value::as_str)
                != Some("string")
        {
            return Err(format!(
                "host-managed export '{}' current Chat thread binding requires a string index",
                export.capability
            ));
        }
        let capability = capability_by_name.get(&export.capability).ok_or_else(|| {
            format!(
                "host-managed export '{}' has no matching manifest capability",
                export.capability
            )
        })?;
        if capability.effect != CapabilityEffect::ReadOnly {
            return Err(format!(
                "host-managed export '{}' must declare effect 'read-only'",
                export.capability
            ));
        }
        let expected_input = managed_export_input_schema(
            export.operation,
            collection,
            index,
            export.equals_host_input,
        );
        let expected_output = managed_export_output_schema(export.operation, collection);
        if capability.input_schema != expected_input
            || capability.output_schema.as_ref() != Some(&expected_output)
        {
            return Err(format!(
                "host-managed export '{}' capability schemas do not match its fixed host operation",
                export.capability
            ));
        }
    }
    let mut artifact_by_name: BTreeMap<&ArtifactTypeName, &ArtifactTypeDeclaration> =
        artifact_types
            .iter()
            .map(|artifact| (&artifact.name, artifact))
            .collect();
    for proposal in proposals {
        if !exported_names.insert(&proposal.capability) {
            return Err(format!(
                "host-managed capability '{}' is exported or proposed more than once",
                proposal.capability
            ));
        }
        if proposal.title.trim().is_empty() || proposal.title.chars().count() > 256 {
            return Err(format!(
                "host-managed proposal '{}' title must contain 1-256 characters",
                proposal.capability
            ));
        }
        if proposal.description.trim().is_empty() || proposal.description.chars().count() > 2000 {
            return Err(format!(
                "host-managed proposal '{}' description must contain 1-2000 characters",
                proposal.capability
            ));
        }
        if !(1..=MAX_PROPOSAL_PAYLOAD_BYTES as u32).contains(&proposal.max_payload_bytes) {
            return Err(format!(
                "host-managed proposal '{}' max_payload_bytes must be between 1 and {MAX_PROPOSAL_PAYLOAD_BYTES}",
                proposal.capability
            ));
        }
        validate_proposal_payload_schema(&proposal.capability, &proposal.payload_schema)?;
        match &proposal.target {
            ManagedDataProposalTarget::Collection { collection }
            | ManagedDataProposalTarget::Record { collection } => {
                if !collections.contains_key(collection) {
                    return Err(format!(
                        "host-managed proposal '{}' references unknown collection '{}'",
                        proposal.capability, collection
                    ));
                }
            }
            ManagedDataProposalTarget::Document {
                document_collection,
            } => {
                if !documents.contains_key(document_collection) {
                    return Err(format!(
                        "host-managed proposal '{}' references unknown document collection '{}'",
                        proposal.capability, document_collection
                    ));
                }
            }
        }
        let capability = capability_by_name
            .get(&proposal.capability)
            .ok_or_else(|| {
                format!(
                    "host-managed proposal '{}' has no matching manifest capability",
                    proposal.capability
                )
            })?;
        if capability.effect != CapabilityEffect::LocalWrite {
            return Err(format!(
                "host-managed proposal '{}' must declare effect 'local-write'",
                proposal.capability
            ));
        }
        let artifact = artifact_by_name
            .remove(&proposal.artifact_type)
            .ok_or_else(|| {
                format!(
                    "host-managed proposal '{}' has no matching manifest artifact type '{}'",
                    proposal.capability, proposal.artifact_type
                )
            })?;
        let input_schema = managed_proposal_input_schema(proposal);
        let envelope_schema = managed_proposal_artifact_schema(&AppId::new(app_id), proposal);
        if capability.input_schema != input_schema
            || capability.output_schema.as_ref() != Some(&envelope_schema)
        {
            return Err(format!(
                "host-managed proposal '{}' capability schemas do not match its fixed host operation",
                proposal.capability
            ));
        }
        if artifact.json_schema != envelope_schema {
            return Err(format!(
                "host-managed proposal '{}' artifact type schema does not match its fixed host envelope",
                proposal.capability
            ));
        }
    }
    if capabilities.len() != exported_names.len()
        || capabilities
            .iter()
            .any(|capability| !exported_names.contains(&capability.name))
    {
        return Err(
            "a host-managed backend-free app may declare only capabilities listed in data.exports or data.proposals"
                .into(),
        );
    }
    Ok(())
}

fn validate_managed_data_name(value: &str, label: &str) -> Result<(), String> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(format!(
            "host-managed {label} name '{value}' must be 1-64 lowercase ASCII letters, digits, or '-' and start with a letter"
        ))
    }
}

pub(crate) fn managed_export_input_schema(
    operation: ManagedDataExportOperation,
    collection: &ManagedDataCollection,
    index: Option<&ManagedDataIndex>,
    equals_host_input: Option<ManagedDataExportHostInput>,
) -> app_host_kernel::JsonObject {
    let value = match operation {
        ManagedDataExportOperation::Get => serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["id"],
            "properties": {
                "id": { "type": "string", "pattern": MANAGED_DATA_UUID_PATTERN }
            }
        }),
        ManagedDataExportOperation::List => {
            let mut properties = serde_json::Map::from_iter([
                (
                    "after".to_string(),
                    serde_json::json!({
                        "type": ["string", "null"],
                        "pattern": MANAGED_DATA_UUID_PATTERN
                    }),
                ),
                (
                    "limit".to_string(),
                    serde_json::json!({
                        "type": "integer",
                        "minimum": 1,
                        "maximum": collection.limits.query_results
                    }),
                ),
            ]);
            let required = if let Some(index) = index {
                let mut equals_schema = index.value_schema.clone();
                if equals_host_input == Some(ManagedDataExportHostInput::CurrentChatThreadId) {
                    equals_schema.insert(
                        crate::tool_mapping::HOST_INPUT_ANNOTATION.into(),
                        Value::String(crate::tool_mapping::CURRENT_CHAT_THREAD_ID.into()),
                    );
                }
                properties.insert("equals".into(), Value::Object(equals_schema));
                serde_json::json!(["equals"])
            } else {
                serde_json::json!([])
            };
            serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": required,
                "properties": properties
            })
        }
    };
    value
        .as_object()
        .cloned()
        .expect("managed input schema is an object")
}

pub(crate) fn managed_export_output_schema(
    operation: ManagedDataExportOperation,
    collection: &ManagedDataCollection,
) -> app_host_kernel::JsonObject {
    let record = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "revision", "created_at", "updated_at", "value"],
        "properties": {
            "id": { "type": "string", "pattern": MANAGED_DATA_UUID_PATTERN },
            "revision": { "type": "integer", "minimum": 1 },
            "created_at": { "type": "string", "format": "date-time" },
            "updated_at": { "type": "string", "format": "date-time" },
            "value": Value::Object(collection.schema.clone())
        }
    });
    let value = match operation {
        ManagedDataExportOperation::Get => serde_json::json!({
            "oneOf": [record, { "type": "null" }]
        }),
        ManagedDataExportOperation::List => serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["records", "next_after"],
            "properties": {
                "records": {
                    "type": "array",
                    "maxItems": collection.limits.query_results,
                    "items": record
                },
                "next_after": { "type": ["string", "null"] }
            }
        }),
    };
    value
        .as_object()
        .cloned()
        .expect("managed output schema is an object")
}

pub(crate) fn managed_proposal_input_schema(
    proposal: &ManagedDataProposal,
) -> app_host_kernel::JsonObject {
    let (target_property, target_schema) = match &proposal.target {
        ManagedDataProposalTarget::Collection { collection } => (
            "targetGeneration",
            serde_json::json!({
                "type": "integer",
                "minimum": 0,
                crate::tool_mapping::MANAGED_DATA_SCOPE_ANNOTATION: {
                    "kind": "collection",
                    "collection": collection
                }
            }),
        ),
        ManagedDataProposalTarget::Record { collection }
        | ManagedDataProposalTarget::Document {
            document_collection: collection,
        } => (
            "targetId",
            serde_json::json!({
                "type": "string",
                "pattern": MANAGED_DATA_UUID_PATTERN,
                crate::tool_mapping::MANAGED_DATA_SCOPE_ANNOTATION: {
                    "kind": proposal.target.kind(),
                    "collection": collection
                }
            }),
        ),
    };
    let mut properties = serde_json::Map::new();
    properties.insert(target_property.into(), target_schema);
    if !matches!(
        proposal.target,
        ManagedDataProposalTarget::Collection { .. }
    ) {
        properties.insert(
            "targetRevision".into(),
            serde_json::json!({"type": "integer", "minimum": 1}),
        );
    }
    properties.insert(
        "payload".into(),
        Value::Object(proposal.payload_schema.clone()),
    );
    let required = if matches!(
        proposal.target,
        ManagedDataProposalTarget::Collection { .. }
    ) {
        vec!["targetGeneration", "payload"]
    } else {
        vec!["targetId", "targetRevision", "payload"]
    };
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": properties,
        crate::tool_mapping::MANAGED_DATA_PROPOSAL_ANNOTATION: true
    })
    .as_object()
    .cloned()
    .expect("proposal input schema is an object")
}

pub(crate) fn managed_proposal_artifact_schema(
    app_id: &AppId,
    proposal: &ManagedDataProposal,
) -> app_host_kernel::JsonObject {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["targetAppId", "targetKind", "collection", "resourceId", "targetGeneration", "targetRevision", "payload"],
        "properties": {
            "targetAppId": {"const": app_id.as_str()},
            "targetKind": {"const": proposal.target.kind()},
            "collection": {"const": proposal.target.collection()},
            "resourceId": {"type": "string", "minLength": 1, "maxLength": 256},
            "targetGeneration": {"type": "integer", "minimum": 0},
            "targetRevision": {"type": ["integer", "null"], "minimum": 1},
            "payload": Value::Object(proposal.payload_schema.clone())
        }
    })
    .as_object()
    .cloned()
    .expect("proposal artifact schema is an object")
}

fn validate_proposal_payload_schema(
    capability: &CapabilityName,
    schema: &app_host_kernel::JsonObject,
) -> Result<(), String> {
    if schema.get("type").and_then(Value::as_str) != Some("object")
        || schema.get("additionalProperties") != Some(&Value::Bool(false))
    {
        return Err(format!(
            "host-managed proposal '{}' payload_schema must have type 'object' and additionalProperties false",
            capability
        ));
    }
    jsonschema::validator_for(&Value::Object(schema.clone())).map_err(|error| {
        format!(
            "host-managed proposal '{}' payload schema is invalid: {error}",
            capability
        )
    })?;
    Ok(())
}

fn theme_color_value_is_valid(value: &str) -> bool {
    let value = value.trim();
    if value.len() > 80 {
        return false;
    }
    if let Some(hex) = value.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8)
            && hex.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    let Some(open) = value.find('(') else {
        return false;
    };
    if !value.ends_with(')') {
        return false;
    }
    let function = &value[..open];
    let alpha = function.eq_ignore_ascii_case("rgba");
    if !alpha && !function.eq_ignore_ascii_case("rgb") {
        return false;
    }
    let parts: Vec<&str> = value[open + 1..value.len() - 1]
        .split(',')
        .map(str::trim)
        .collect();
    if parts.len() != if alpha { 4 } else { 3 } {
        return false;
    }
    let channels_valid = parts[..3]
        .iter()
        .all(|part| css_number_in_range(part, false, 255.0, 100.0));
    channels_valid && (!alpha || css_number_in_range(parts[3], true, 1.0, 100.0))
}

fn css_number_in_range(
    value: &str,
    allow_leading_dot: bool,
    maximum: f64,
    percent_maximum: f64,
) -> bool {
    let number = value.strip_suffix('%').unwrap_or(value);
    if number.is_empty()
        || (!allow_leading_dot && number.starts_with('.'))
        || number.ends_with('.')
        || number
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && byte != b'.')
        || number.bytes().filter(|byte| *byte == b'.').count() > 1
    {
        return false;
    }
    let Ok(amount) = number.parse::<f64>() else {
        return false;
    };
    amount.is_finite()
        && amount >= 0.0
        && amount
            <= if value.ends_with('%') {
                percent_maximum
            } else {
                maximum
            }
}

const RESERVED_HOST_APP_IDS: [&str; 5] = [
    "chat",
    "llm-provider",
    "com.ma-zierl.kestral-artifacts",
    "com.ma-zierl.host.file-broker",
    "com.ma-zierl.host.permissions",
];

/// Reverse-DNS id that cannot impersonate a bundled app or a bridged MCP
/// server (`mcp-*`). Mirrors the app.json JSON Schema.
pub fn id_is_valid(id: &str) -> bool {
    if id.starts_with("mcp-") || !id.contains('.') || RESERVED_HOST_APP_IDS.contains(&id) {
        return false;
    }
    id.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
        && !id.starts_with('.')
        && !id.ends_with('.')
}

fn host_compatible(min_host_version: &str) -> Result<bool, String> {
    let min = Version::parse(min_host_version).map_err(|error| {
        format!(
            "min_host_version '{}' is not strict semver: {error}",
            min_host_version
        )
    })?;
    let host = Version::parse(HOST_VERSION)
        .map_err(|error| format!("host version '{HOST_VERSION}' is not strict semver: {error}"))?;
    Ok(host >= min)
}

// -- Integrity ----------------------------------------------------------------

/// Verify every listed asset hashes to its pinned sha256, and that no
/// unlisted files hide under `ui/` or `backend/`. Reads bytes only.
pub fn verify_integrity(package_dir: &Path, integrity: &Integrity) -> Result<Vec<String>, String> {
    for (rel, expected) in &integrity.assets {
        if !is_safe_relative(rel) {
            return Err(format!("integrity lists an unsafe path: {rel}"));
        }
        let path = package_dir.join(rel);
        let mut file = open_regular_nofollow(&path, rel)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| format!("missing or unreadable asset '{rel}': {error}"))?;
        let actual = format!("sha256-{:x}", Sha256::digest(&bytes));
        if &actual != expected {
            return Err(format!("checksum mismatch for '{rel}'"));
        }
    }
    Ok(Vec::new())
}

fn is_safe_relative(rel: &str) -> bool {
    normalize_relative(rel).is_ok()
}

fn normalize_relative(rel: &str) -> Result<String, String> {
    if rel.is_empty() || rel.contains('\\') || rel.contains(':') || rel.starts_with('/') {
        return Err(format!("unsafe package path '{rel}'"));
    }
    let mut parts = Vec::new();
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(format!("unsafe package path '{rel}'"));
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

// -- Inspection ---------------------------------------------------------------

/// Produce the full pre-install inspection. Executes no package code.
pub fn inspect(package_dir: &Path) -> Result<PackageInspection, String> {
    inspect_with_trust(package_dir, &PublisherTrustStore::in_memory())
}

pub fn inspect_with_trust(
    package_dir: &Path,
    trust_store: &PublisherTrustStore,
) -> Result<PackageInspection, String> {
    let digest = package_digest(package_dir)?;
    let document = read_document(package_dir)?;
    let signature = read_signature_document(package_dir)?;
    inspect_document(
        package_dir,
        String::new(),
        digest,
        &document,
        signature.as_ref(),
        trust_store,
    )
}

fn inspect_document(
    package_dir: &Path,
    staged_id: String,
    package_digest: String,
    document: &PackageDocument,
    signature: Option<&PackageSignatureDocument>,
    trust_store: &PublisherTrustStore,
) -> Result<PackageInspection, String> {
    let mut warnings = Vec::new();
    let structural_error = structural_error(document);
    let (integrity_ok, integrity_error) = match verify_integrity(package_dir, &document.integrity) {
        Ok(asset_warnings) => {
            warnings.extend(asset_warnings);
            (true, None)
        }
        Err(error) => (false, Some(error)),
    };
    if integrity_ok {
        app_icon_view(package_dir, document.icon.as_ref())?;
    }

    let compatible = host_compatible(&document.min_host_version)?;
    let _version = Version::parse(&document.version).map_err(|error| {
        format!(
            "version '{}' is not strict semver: {error}",
            document.version
        )
    })?;
    if !compatible {
        warnings.push(format!(
            "requires host {} or newer (this host is {HOST_VERSION})",
            document.min_host_version
        ));
    }
    let signature_document = signature;
    let signature = match signature_document {
        None => SignatureState::Unsigned,
        Some(signature) => {
            match trust_store.verify_signature(&package_digest, signature, document.id.as_str()) {
                Ok(state) => state,
                Err(error) => SignatureState::Invalid { reason: error },
            }
        }
    };
    let signature = match document
        .publisher
        .as_ref()
        .and_then(|publisher| publisher.key_id.as_ref())
    {
        Some(expected) if signature.key_id() != Some(expected.as_str()) => {
            SignatureState::Invalid {
                reason: format!("publisher key id '{expected}' does not match the signed key"),
            }
        }
        _ => signature,
    };
    let blocking_error = structural_error
        .clone()
        .or_else(|| signature.blocking_error());
    let installable = blocking_error.is_none() && integrity_ok && compatible;

    Ok(PackageInspection {
        staged_id,
        package_digest,
        id: document.id.clone(),
        version: document.version.clone(),
        display_name: document.display_name.clone(),
        description: document.description.clone(),
        publisher: document.publisher.as_ref().map(|p| PublisherView {
            name: p.name.clone(),
            homepage: p.homepage.clone(),
            key_id: p.key_id.clone(),
        }),
        license: document.license.clone(),
        signature,
        signature_public_key: signature_document.map(|signature| signature.public_key.clone()),
        backend_kind: document.backend.kind_label().to_string(),
        backend_detail: document.backend.detail(),
        backend_authority_mode: document.backend.authority_mode(),
        data: document.data.summary(),
        min_host_version: document.min_host_version.clone(),
        host_version: HOST_VERSION.to_string(),
        host_compatible: compatible,
        capabilities: document
            .manifest
            .capabilities
            .iter()
            .map(|c| CapabilitySummary {
                name: c.name.to_string(),
                description: c.description.clone(),
                effect: effect_label(c).to_string(),
            })
            .collect(),
        grant_requests: document
            .manifest
            .grant_requests
            .iter()
            .map(grant_summary)
            .chain(document.consumer_grant_requests.iter().map(|consumer| {
                let mut summary = grant_summary(&consumer.request);
                summary.scope_label = format!("{} -> {}", consumer.holder, summary.scope_label);
                summary
            }))
            .collect(),
        extension_contributions: document
            .manifest
            .extension_contributions
            .iter()
            .map(|contribution| ExtensionContributionSummary {
                target_app: contribution.target_app.to_string(),
                extension_point: contribution.extension_point.to_string(),
                contract_version: contribution.contract_version,
                surface: contribution.surface.to_string(),
            })
            .collect(),
        surfaces: document
            .manifest
            .surfaces
            .iter()
            .map(|s| SurfaceSummary {
                name: s.name.to_string(),
                kind: surface_kind_label(&s.kind).to_string(),
                title: s.title.clone(),
                has_custom_ui: s.ui.is_some(),
            })
            .collect(),
        config: document
            .manifest
            .config_declarations
            .iter()
            .map(|c| ConfigSummary {
                name: c.name.to_string(),
                title: c.title.clone(),
                description: c.description.clone(),
            })
            .collect(),
        secrets: document
            .manifest
            .connectors
            .iter()
            .flat_map(|connector| {
                connector
                    .secret_names
                    .iter()
                    .map(move |name| SecretSummary {
                        connector: connector.name.clone(),
                        name: name.to_string(),
                        description: connector.description.clone(),
                    })
            })
            .collect(),
        artifact_types: document
            .manifest
            .artifact_types
            .iter()
            .map(|t| t.name.to_string())
            .collect(),
        event_subscriptions: document
            .manifest
            .event_subscriptions
            .iter()
            .map(|t| t.to_string())
            .collect(),
        integrity_ok,
        integrity_error,
        warnings,
        installable,
        blocking_error,
    })
}

/// Copy a package into host-owned staging and inspect those immutable bytes.
/// Only `app.json` and files named by `integrity.assets` are accepted.
pub fn stage_and_inspect(source: &Path, staging_root: &Path) -> Result<PackageInspection, String> {
    stage_and_inspect_with_trust(source, staging_root, &PublisherTrustStore::in_memory())
}

pub fn stage_and_inspect_with_trust(
    source: &Path,
    staging_root: &Path,
    trust_store: &PublisherTrustStore,
) -> Result<PackageInspection, String> {
    let document = read_document(source)?;
    if let Some(error) = structural_error(&document) {
        return Err(error);
    }
    let signature = read_signature_document(source)?;
    let declared = declared_paths(&document)?;
    validate_source_tree(source, &declared)?;

    fs::create_dir_all(staging_root)
        .map_err(|error| format!("create package staging root failed: {error}"))?;
    let staged_id = Uuid::new_v4().to_string();
    let staged_dir = staging_root.join(&staged_id);
    fs::create_dir(&staged_dir)
        .map_err(|error| format!("create package staging directory failed: {error}"))?;

    let result = (|| {
        for rel in &declared {
            copy_regular_file(source, &staged_dir, rel)?;
        }
        validate_source_tree(&staged_dir, &declared)?;
        verify_integrity(&staged_dir, &document.integrity)?;
        let digest = package_digest(&staged_dir)?;
        make_tree_read_only(&staged_dir)?;
        inspect_document(
            &staged_dir,
            staged_id.clone(),
            digest,
            &document,
            signature.as_ref(),
            trust_store,
        )
    })();
    if result.is_err() {
        let _ = remove_read_only_tree(&staged_dir);
    }
    result
}

pub fn staged_dir(staging_root: &Path, staged_id: &str) -> Result<PathBuf, String> {
    if Uuid::parse_str(staged_id).is_err() {
        return Err("invalid staged package identity".into());
    }
    Ok(staging_root.join(staged_id))
}

pub fn package_digest(package_dir: &Path) -> Result<String, String> {
    let document = read_document(package_dir)?;
    let declared = declared_paths(&document)?;
    validate_source_tree(package_dir, &declared)?;
    let mut hasher = Sha256::new();
    for rel in declared {
        let mut file = open_regular_nofollow(&package_dir.join(&rel), &rel)?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("inspect package file '{rel}' failed: {error}"))?;
        if !metadata.is_file() {
            return Err(format!("package entry '{rel}' is not a regular file"));
        }
        hasher.update((rel.len() as u64).to_le_bytes());
        hasher.update(rel.as_bytes());
        hasher.update(metadata.len().to_le_bytes());
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("read package file '{rel}' failed: {error}"))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    }
    Ok(format!("sha256-{:x}", hasher.finalize()))
}

pub fn copy_verified_package(
    source: &Path,
    destination: &Path,
    expected_digest: &str,
) -> Result<(), String> {
    copy_verified_package_with_hook(source, destination, expected_digest, |_| Ok(()))
}

fn copy_verified_package_with_hook(
    source: &Path,
    destination: &Path,
    expected_digest: &str,
    mut after_copy: impl FnMut(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let document = read_document(source)?;
    let declared = declared_paths(&document)?;
    validate_source_tree(source, &declared)?;
    if package_digest(source)? != expected_digest {
        return Err("staged package digest no longer matches the approved digest".into());
    }
    fs::create_dir(destination)
        .map_err(|error| format!("create install directory failed: {error}"))?;
    let copy_result = (|| {
        for rel in &declared {
            copy_regular_file(source, destination, rel)?;
            after_copy(destination)?;
        }
        let actual = package_digest(destination)?;
        if actual != expected_digest {
            return Err("post-copy package verification failed".into());
        }
        Ok(())
    })();
    if copy_result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    copy_result
}

fn declared_paths(document: &PackageDocument) -> Result<BTreeSet<String>, String> {
    let mut paths = BTreeSet::from([APP_JSON.to_string()]);
    let mut case_folded = BTreeSet::from([APP_JSON.to_string()]);
    for path in document.integrity.assets.keys() {
        let normalized = normalize_relative(path)?;
        if normalized != *path || !paths.insert(normalized.clone()) {
            return Err(format!("duplicate normalized package path '{path}'"));
        }
        if !case_folded.insert(normalized.to_ascii_lowercase()) {
            return Err(format!("case-colliding package path '{path}'"));
        }
    }
    let mut referenced: Vec<&String> = document
        .icon
        .iter()
        .filter_map(|icon| match icon {
            PackageIcon::Asset(path) => Some(path),
            PackageIcon::Kestral { .. } => None,
        })
        .collect();
    referenced.extend(
        document
            .manifest
            .surfaces
            .iter()
            .filter_map(|surface| surface.ui.as_ref().map(|ui| &ui.entry)),
    );
    if let Backend::Executable { platforms, .. } = &document.backend {
        referenced.extend(platforms.values());
    }
    if let Backend::AgentWorker { entry, .. } = &document.backend {
        referenced.push(entry);
    }
    if let AppData::Versioned { migration, .. } = &document.data {
        referenced.push(&migration.entry);
    }
    for referenced in referenced {
        if !paths.contains(referenced) {
            return Err(format!(
                "referenced package file '{referenced}' is not declared in integrity.assets"
            ));
        }
    }
    Ok(paths)
}

const MAX_ICON_BYTES: usize = 256 * 1024;

pub fn app_icon_view(
    package_dir: &Path,
    icon: Option<&PackageIcon>,
) -> Result<Option<AppIconView>, String> {
    let Some(icon) = icon else {
        return Ok(None);
    };
    match icon {
        PackageIcon::Kestral { name, .. } => Ok(Some(AppIconView::Kestral { name: *name })),
        PackageIcon::Asset(path) => {
            let bytes = fs::read(package_dir.join(path))
                .map_err(|error| format!("read app icon '{path}' failed: {error}"))?;
            if bytes.len() > MAX_ICON_BYTES {
                return Err(format!(
                    "app icon '{path}' is {} bytes; the maximum is {MAX_ICON_BYTES}",
                    bytes.len()
                ));
            }
            let media_type = icon_media_type(path, &bytes)?;
            Ok(Some(AppIconView::Asset {
                media_type: media_type.to_string(),
                data_base64: STANDARD.encode(bytes),
            }))
        }
    }
}

fn icon_media_type(path: &str, bytes: &[u8]) -> Result<&'static str, String> {
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("png") if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => Ok("image/png"),
        Some("webp") if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" => {
            Ok("image/webp")
        }
        Some("jpg" | "jpeg") if bytes.starts_with(&[0xff, 0xd8, 0xff]) => Ok("image/jpeg"),
        Some("svg") => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| format!("app icon '{path}' is not UTF-8 SVG"))?;
            let normalized = text.to_ascii_lowercase();
            if !normalized.contains("<svg") {
                return Err(format!("app icon '{path}' does not contain an SVG root"));
            }
            for forbidden in [
                "<script",
                "<foreignobject",
                "<style",
                "href",
                "url",
                "@import",
            ] {
                if normalized.contains(forbidden) {
                    return Err(format!(
                        "app icon '{path}' contains unsupported active or external SVG content"
                    ));
                }
            }
            if has_svg_event_handler(&normalized) {
                return Err(format!(
                    "app icon '{path}' contains unsupported active or external SVG content"
                ));
            }
            Ok("image/svg+xml")
        }
        _ => Err(format!(
            "app icon '{path}' must be a valid SVG, PNG, WebP, or JPEG image"
        )),
    }
}

fn has_svg_event_handler(svg: &str) -> bool {
    let bytes = svg.as_bytes();
    for start in 0..bytes.len().saturating_sub(3) {
        if !bytes[start].is_ascii_whitespace()
            || bytes[start + 1] != b'o'
            || bytes[start + 2] != b'n'
            || !bytes[start + 3].is_ascii_alphabetic()
        {
            continue;
        }
        let mut cursor = start + 4;
        while cursor < bytes.len() && bytes[cursor].is_ascii_alphabetic() {
            cursor += 1;
        }
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) == Some(&b'=') {
            return true;
        }
    }
    false
}

fn validate_source_tree(root: &Path, declared: &BTreeSet<String>) -> Result<(), String> {
    let mut actual = BTreeSet::new();
    walk_package_tree(root, root, &mut actual)?;
    actual.remove("app.signature.json");
    if &actual != declared {
        let extra: Vec<_> = actual.difference(declared).cloned().collect();
        let missing: Vec<_> = declared.difference(&actual).cloned().collect();
        return Err(format!(
            "package file declaration mismatch; extra={extra:?}, missing={missing:?}"
        ));
    }
    Ok(())
}

fn walk_package_tree(
    root: &Path,
    current: &Path,
    files: &mut BTreeSet<String>,
) -> Result<(), String> {
    for entry in
        fs::read_dir(current).map_err(|error| format!("read package directory failed: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read package entry failed: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect package entry failed: {error}"))?;
        let path = entry.path();
        if file_type.is_symlink() {
            return Err(format!(
                "package symlinks are unsupported: {}",
                path.display()
            ));
        }
        if file_type.is_dir() {
            walk_package_tree(root, &path, files)?;
        } else if file_type.is_file() {
            let rel = path
                .strip_prefix(root)
                .map_err(|_| "package path escaped its root".to_string())?;
            let rel = rel
                .to_str()
                .ok_or_else(|| "non-UTF-8 package paths are unsupported".to_string())?
                .replace('\\', "/");
            let normalized = normalize_relative(&rel)?;
            if !files.insert(normalized.clone()) {
                return Err(format!("duplicate normalized package path '{normalized}'"));
            }
        } else {
            return Err(format!("unsupported package file type: {}", path.display()));
        }
    }
    Ok(())
}

fn copy_regular_file(source_root: &Path, destination_root: &Path, rel: &str) -> Result<(), String> {
    let source = source_root.join(rel);
    let mut source_file = open_regular_nofollow(&source, rel)?;
    let destination = destination_root.join(rel);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create destination for '{rel}' failed: {error}"))?;
    }
    let mut destination_file = File::create(&destination)
        .map_err(|error| format!("create package file '{rel}' failed: {error}"))?;
    std::io::copy(&mut source_file, &mut destination_file)
        .map_err(|error| format!("copy package file '{rel}' failed: {error}"))?;
    Ok(())
}

fn open_regular_nofollow(path: &Path, label: &str) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("open package file '{label}' failed: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect package file '{label}' failed: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("package entry '{label}' is not a regular file"));
    }
    Ok(file)
}

fn make_tree_read_only(root: &Path) -> Result<(), String> {
    for entry in
        fs::read_dir(root).map_err(|error| format!("read staging directory failed: {error}"))?
    {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() {
            make_tree_read_only(&path)?;
        } else {
            let mut permissions = fs::metadata(&path)
                .map_err(|error| error.to_string())?
                .permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&path, permissions).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

// Clearing the Windows read-only attribute does not change Unix mode bits;
// Unix can remove read-only files from a writable directory without this.
#[cfg_attr(windows, allow(clippy::permissions_set_readonly_false))]
pub fn remove_read_only_tree(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() {
            remove_read_only_tree(&path)?;
        } else {
            #[cfg(windows)]
            {
                let mut permissions = fs::metadata(&path)
                    .map_err(|error| error.to_string())?
                    .permissions();
                permissions.set_readonly(false);
                fs::set_permissions(&path, permissions).map_err(|error| error.to_string())?;
            }
            fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
    }
    fs::remove_dir(root).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;

fn effect_label(capability: &CapabilityDeclaration) -> &'static str {
    use app_host_kernel::primitives::capability::CapabilityEffect::*;
    match capability.effect {
        Unspecified => "unspecified",
        ReadOnly => "read-only",
        LocalWrite => "local-write",
        ExternalWrite => "external-write",
        Destructive => "destructive",
    }
}

fn surface_kind_label(kind: &SurfaceKind) -> &'static str {
    match kind {
        SurfaceKind::Panel => "panel",
        SurfaceKind::Card => "card",
        SurfaceKind::Form => "form",
        SurfaceKind::Picker => "picker",
        SurfaceKind::Dashboard => "dashboard",
    }
}

fn grant_summary(request: &GrantRequest) -> GrantRequestSummary {
    let scope_label = match &request.scope {
        GrantScope::ExactCapability {
            provider,
            capability,
        } => format!("{provider}/{capability}"),
        GrantScope::AllProviderCapabilities { provider } => format!("{provider}/* (all actions)"),
    };
    let data_scope_label = match &request.data_scope {
        DataScope::None => "all data".to_string(),
        DataScope::AllResources => "all current and future resources".to_string(),
        DataScope::Resources { resource_ids } => format!(
            "resources: {}",
            resource_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let condition = match request.condition {
        GrantCondition::Silent => "silent",
        GrantCondition::Notify => "notify",
        GrantCondition::RequiresApproval => "requires approval",
    };
    let duration_label = match request.duration {
        GrantDuration::NonExpiring => "does not expire".to_string(),
        GrantDuration::ExpiresAfter { seconds } => format!("expires after {seconds}s"),
    };
    GrantRequestSummary {
        scope_label,
        data_scope_label,
        condition: condition.to_string(),
        reason: request.reason.clone(),
        duration_label,
    }
}

// -- Translation to the kernel manifest ---------------------------------------

/// The output of translating an approved package: a sealed kernel manifest and
/// the sandboxed UI bundles the host must register for the app's surfaces.
pub struct TranslatedPackage {
    pub sealed: SealedManifest,
    pub app_id: AppId,
    pub ui_bundles: Vec<(SurfaceName, SurfaceUiBundle)>,
    pub backend: Backend,
    pub data: AppData,
}

/// Translate a package into kernel + host artifacts. Reads the UI entry files
/// (to bundle them) but runs nothing. Fails if the package is not installable.
pub fn translate(
    package_dir: &Path,
    document: &PackageDocument,
) -> Result<TranslatedPackage, String> {
    if let Some(error) = structural_error(document) {
        return Err(error);
    }
    let app_id = AppId::new(&document.id);

    let mut surfaces = Vec::new();
    let mut ui_bundles = Vec::new();
    for surface in &document.manifest.surfaces {
        surfaces.push(SurfaceDeclaration {
            name: surface.name.clone(),
            kind: surface.kind,
            title: surface.title.clone(),
            description: surface.description.clone(),
            intents: surface.intents.clone(),
        });
        if let Some(ui) = &surface.ui {
            if !is_safe_relative(&ui.entry) {
                return Err(format!("surface '{}' has an unsafe ui.entry", surface.name));
            }
            let html = read_bounded_utf8(
                &package_dir.join(&ui.entry),
                &format!("ui entry '{}'", ui.entry),
                MAX_SURFACE_UI_BYTES,
            )?;
            let connect: Vec<&str> = ui.connect_src.iter().map(String::as_str).collect();
            // The CSP is ALWAYS host-authored (deny-by-default). A package may
            // widen only `connect-src`, via the structured `connect_src` field —
            // never by supplying a raw policy. Honoring `ui.csp` verbatim (as
            // this once did) let any package drop the host's
            // default-src/base-uri/form-action locks and exfiltrate from its
            // sandboxed frame. A raw override is refused rather than silently
            // ignored, so a package author is never misled about its effect.
            if ui.csp.is_some() {
                return Err(format!(
                    "surface '{}' sets ui.csp, which is no longer accepted; declare allowed \
                     network hosts in ui.connect_src instead (the host authors the policy)",
                    surface.name
                ));
            }
            // `connect_src` is the one field a package contributes to the
            // policy, so it is also the one place the same escape could
            // reopen: an entry carrying `;` would close `connect-src` and
            // append directives of its own, including overrides of the
            // `base-uri`/`form-action` locks the host appends afterwards.
            if let Some(bad) = connect
                .iter()
                .find(|source| !crate::surface_ui::is_valid_connect_src(source))
            {
                return Err(format!(
                    "surface '{}' declares an invalid ui.connect_src entry '{bad}'; each entry \
                     must be a bare source expression such as 'https://example.com' (no spaces, \
                     quotes, ';' or ',')",
                    surface.name
                ));
            }
            let bundle = SurfaceUiBundle::new(html, &connect);
            ui_bundles.push((surface.name.clone(), bundle));
        }
    }

    let manifest = AppManifest {
        app_id: app_id.clone(),
        version: document.version.clone(),
        display_name: document.display_name.clone(),
        description: document.description.clone(),
        capabilities: document.manifest.capabilities.clone(),
        surfaces,
        agents: document.manifest.agents.clone(),
        skills: document.manifest.skills.clone(),
        assistant_profiles: document.manifest.assistant_profiles.clone(),
        automations: document.manifest.automations.clone(),
        connectors: document.manifest.connectors.clone(),
        config_declarations: document.manifest.config_declarations.clone(),
        artifact_types: document.manifest.artifact_types.clone(),
        extension_points: document.manifest.extension_points.clone(),
        extension_contributions: document.manifest.extension_contributions.clone(),
        grant_requests: document.manifest.grant_requests.clone(),
        event_subscriptions: document.manifest.event_subscriptions.clone(),
    };
    manifest
        .require_consistent()
        .map_err(|error| format!("manifest is inconsistent: {error}"))?;

    Ok(TranslatedPackage {
        sealed: seal(manifest),
        app_id,
        ui_bundles,
        backend: document.backend.clone(),
        data: document.data.clone(),
    })
}

/// Every secret name the package's connectors declare, so uninstall can offer
/// to purge exactly those from the host secret store.
pub fn owned_secret_names(document: &PackageDocument) -> Vec<String> {
    document
        .manifest
        .connectors
        .iter()
        .flat_map(|connector| connector.secret_names.iter().map(|name| name.to_string()))
        .collect()
}

/// Capability names the package declares — used to bind backend handlers.
pub fn capability_names(document: &PackageDocument) -> Vec<CapabilityName> {
    document
        .manifest
        .capabilities
        .iter()
        .map(|c| c.name.clone())
        .collect()
}
