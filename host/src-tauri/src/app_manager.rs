//! App lifecycle manager: install, enable/disable, uninstall, and status for
//! third-party packages (`docs/writing-apps.md`), layered over the public kernel API.
//!
//! The manager owns nothing the kernel owns. Every lifecycle step routes
//! through phased kernel install / `Kernel::uninstall` (which prompt trusted chrome
//! for grants and revoke authority in both directions), plus host-side cleanup
//! the kernel can't do: stopping backend processes, dropping sandboxed UI
//! bundles, and purging persisted secrets / app data per the user's explicit
//! choice.
//!
//! Bundled/startup apps (chat and llm-provider) are NOT managed here —
//! they are shown as read-only, bundled, and non-removable.
//! They hold no privileged capability access; they use the same kernel API a
//! third-party app does.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use semver::Version;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use app_host_kernel::ids::{AppId, ArtifactTypeName, CapabilityName, SurfaceName};
use app_host_kernel::invocation::{CapabilityHandler, CapabilityOutcome, HandlerFailure};
use app_host_kernel::kernel::{GrantApproval, Kernel, PreparedGrant, PreparedInstall};
use app_host_kernel::manifest::{AppManifest, ExtensionContribution};
use app_host_kernel::primitives::artifact::ArtifactDraft;
use app_host_kernel::primitives::grant::{DataScope, GrantScope};
use mcp_adapter::stdio::{
    app_container_moniker, delete_app_container_profile, native_backend_sandbox_support,
};
use mcp_adapter::{McpClient, McpToolCall, McpTransport, StdioTransport, StreamableHttpTransport};

use crate::agent_worker::{self, KernelInvokerClient, PackageWorkerAgentEngine};
use crate::atomic_json::{
    load_json_document, persist_json_document, standard_writer, AtomicJsonError,
};
use crate::config::HostConfigService;
use crate::package::{self, Backend};
use crate::publisher_trust::{
    PackageSignatureDocument, PublisherTrustStore, SignatureState, TrustRecord, TrustScope,
};
use crate::surface_ui::SurfaceUiBundle;
use crate::surface_ui::SurfaceUiRegistry;

mod update_journal;

pub(crate) use update_journal::{AppDataTransitionJournal, UpdateJournal, UpdatePhase};

const STORE_VERSION: u32 = 4;

const RETAINED_REVISIONS: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedAppOperation {
    Install,
    Update,
    Reinstall,
    VersionConflict,
    Downgrade,
    Revert,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedAppVersionRelation {
    Same,
    Higher,
    Lower,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedAppPublisherContinuity {
    Same,
    Changed,
    New,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ManagedAppPermissionDiff<T> {
    pub unchanged: Vec<T>,
    pub added: Vec<T>,
    pub widened: Vec<T>,
    pub removed: Vec<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ManagedAppUpdateDiff {
    pub version_relation: ManagedAppVersionRelation,
    pub display_name_changed: bool,
    pub description_changed: bool,
    pub backend_kind_changed: bool,
    pub current_backend_authority_mode: Option<package::BackendAuthorityMode>,
    pub target_backend_authority_mode: Option<package::BackendAuthorityMode>,
    pub current_data: Option<package::AppDataSummary>,
    pub target_data: package::AppDataSummary,
    pub publisher_key_continuity: ManagedAppPublisherContinuity,
    pub capabilities_added: Vec<String>,
    pub capabilities_removed: Vec<String>,
    pub surfaces_added: Vec<String>,
    pub surfaces_removed: Vec<String>,
    pub permissions: ManagedAppPermissionDiff<package::GrantRequestSummary>,
    pub consumer_permissions: ManagedAppPermissionDiff<package::GrantRequestSummary>,
    pub extension_warnings: Vec<ManagedAppExtensionWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedAppExtensionWarning {
    pub contributor_app_id: String,
    pub extension_point: String,
    pub surface: String,
    pub contribution_contract_version: u32,
    pub current_target_contract_version: u32,
    pub target_contract_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedAppTransitionRequest {
    pub operation: ManagedAppOperation,
    pub staged_id: Option<String>,
    pub package_digest: Option<String>,
    pub app_id: Option<String>,
    pub revision_id: Option<String>,
    pub acknowledge_downgrade: bool,
    pub acknowledge_revert_data_caveat: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ManagedAppTransitionPlan {
    pub transition_id: String,
    pub app_id: String,
    pub operation: ManagedAppOperation,
    pub current_revision_id: Option<String>,
    pub target_revision_id: String,
    pub target_version: String,
    pub diff: ManagedAppUpdateDiff,
    pub requires_explicit_approval: bool,
    pub data_rollback_supported: bool,
    pub data_rollback_caveat: Option<String>,
    pub data_transition: Option<ManagedAppDataTransition>,
    pub staged_id: Option<String>,
    pub package_digest: Option<String>,
    pub revision_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedAppDataTransition {
    pub source_format_version: Option<u32>,
    pub target_format_version: u32,
    pub destructive: bool,
    pub reverse_migration: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppRevision {
    pub revision_id: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub backend_kind: String,
    pub publisher: Option<String>,
    pub signature_verdict: String,
    pub signature_key_id: Option<String>,
    pub min_host_version: String,
    pub installed_at: String,
    pub payload_dir: String,
    pub package_digest: String,
}

/// One persisted installed-package record. Runtime status (active/failed) is
/// derived live, not stored — only durable facts live here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InstallRecord {
    pub id: String,
    pub enabled: bool,
    /// Durable uninstall tombstone. Startup retries cleanup before treating
    /// the record as removable; this prevents a failed file operation from
    /// becoming an unrecoverable half-uninstall.
    pub uninstalling: bool,
    pub lifecycle_generation: u64,
    pub purge_secrets: bool,
    pub purge_data: bool,
    pub purge_secret_names: Vec<String>,
    pub active_revision_id: String,
    pub revisions: Vec<AppRevision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreDocument {
    version: u32,
    apps: Vec<InstallRecord>,
}

/// One row of the manager list, combining bundled + third-party apps.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppStatusView {
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub description: String,
    pub bundled: bool,
    pub enabled: bool,
    /// active | disabled | failed | needs-permissions
    pub status: String,
    pub status_detail: Option<String>,
    pub backend_kind: String,
    /// bundled | unsigned | valid-unknown-key | trusted | invalid | revoked
    pub signature: String,
    pub publisher: Option<String>,
    pub missing_permissions: usize,
    pub surfaces: Vec<AppSurfaceInfo>,
    pub min_host_version: Option<String>,
    pub installed_at: Option<String>,
    pub revisions: Vec<AppRevision>,
    pub extension_contributions: Vec<AppExtensionContributionView>,
    /// False for bundled apps — they are not removable through the manager.
    pub removable: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AppExtensionCompatibility {
    Exact,
    TargetMissing,
    PointMissing,
    ContractMismatch,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppExtensionContributionView {
    pub target_app: String,
    pub extension_point: String,
    pub contract_version: u32,
    pub surface: String,
    pub compatibility: AppExtensionCompatibility,
    pub target_contract_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppSurfaceInfo {
    pub name: String,
    pub kind: String,
    pub title: String,
    pub has_custom_ui: bool,
}

/// Live app manager state: persisted records, copied payloads, and open MCP
/// sessions. Agent workers are invocation-scoped and leave no live session.
pub struct AppManager {
    store_path: Option<PathBuf>,
    journal_path: Option<PathBuf>,
    trust_store: PublisherTrustStore,
    apps_root: PathBuf,
    allow_unsafe_native_backends: bool,
    staging_root: PathBuf,
    staged_inspections: BTreeMap<String, StagedInspection>,
    /// Server-owned, one-time transition plans. The frontend receives a copy
    /// for review but may apply only the opaque transition id.
    pending_transition_plans: BTreeMap<String, ManagedAppTransitionPlan>,
    records: BTreeMap<String, InstallRecord>,
    update_journal: Option<UpdateJournal>,
    /// Transient activation failures (e.g. a backend that would not start).
    /// Not persisted: recomputed each session on enable.
    failures: BTreeMap<String, String>,
    /// Live sessions for MCP-backed apps, keyed by app id.
    clients: BTreeMap<String, Arc<McpClient>>,
}

/// Everything that can be prepared without touching kernel state. In
/// particular, MCP process startup and handshake happen before the kernel
/// mutex is acquired.
pub struct PreparedActivation {
    revision_id: String,
    translated: package::TranslatedPackage,
    consumer_grant_requests: Vec<package::ConsumerGrantRequest>,
    handlers: BTreeMap<CapabilityName, CapabilityHandler>,
    client: Option<Arc<McpClient>>,
    lifecycle_generation: u64,
    package_digest: String,
}

/// Manager-owned activation input captured before filesystem, process, or
/// protocol work begins. Callers prepare this value after releasing the
/// app-manager mutex.
pub(crate) struct ActivationPreparation {
    id: String,
    revision: AppRevision,
    apps_root: PathBuf,
    allow_unsafe_native_backends: bool,
    lifecycle_generation: u64,
    kernel_invoker: Option<KernelInvokerClient>,
    data_dir_override: Option<PathBuf>,
}

pub(crate) struct DataMigrationPreparation {
    app_id: String,
    apps_root: PathBuf,
    source_revision_id: Option<String>,
    source_format_version: Option<u32>,
    candidate: crate::app_data::AppDataRevision,
    migration_revision: Option<AppRevision>,
    target_revision: AppRevision,
    allow_unsafe_native_backends: bool,
    lifecycle_generation: u64,
}

#[derive(Debug, Clone)]
struct StagedInspection {
    inspection: package::PackageInspection,
    signature: Option<PackageSignatureDocument>,
}

impl PreparedActivation {
    pub fn client(&self) -> Option<Arc<McpClient>> {
        self.client.clone()
    }
}

impl ActivationPreparation {
    pub(crate) fn prepare(self) -> Result<PreparedActivation, String> {
        let payload_dir = PathBuf::from(&self.revision.payload_dir);
        let actual_digest = package::package_digest(&payload_dir)?;
        if actual_digest != self.revision.package_digest {
            return Err(format!(
                "installed package digest mismatch: expected {}, got {actual_digest}",
                self.revision.package_digest
            ));
        }
        let document = package::read_document(&payload_dir)?;
        let translated = package::translate(&payload_dir, &document)?;
        if let Some(error) =
            backend_policy_error(self.allow_unsafe_native_backends, &translated.backend)
        {
            return Err(error);
        }
        let (handlers, client) = match &translated.backend {
            Backend::None => {
                let mut handlers = crate::managed_data::handlers_for_exports(
                    &self.apps_root,
                    &translated.app_id,
                    &translated.data,
                )?;
                handlers.extend(crate::managed_data::handlers_for_proposals(
                    &self.apps_root,
                    &payload_dir,
                    &translated.app_id,
                    &translated.data,
                    &self.revision.package_digest,
                )?);
                (handlers, None)
            }
            Backend::AgentWorker { entry, .. } => {
                let kernel_invoker = self.kernel_invoker.ok_or_else(|| {
                    "agent-worker activation requires the host invocation dispatcher".to_string()
                })?;
                let engine = Arc::new(PackageWorkerAgentEngine::new(payload_dir.join(entry))?);
                (
                    agent_worker::agent_worker_handlers(
                        translated.app_id.clone(),
                        kernel_invoker,
                        engine,
                    ),
                    None,
                )
            }
            backend => {
                let data_dir = match self.data_dir_override {
                    Some(path) => path,
                    None => crate::app_data::active_dir(
                        &self.apps_root,
                        &self.id,
                        &self.revision.revision_id,
                        &document.data,
                        &self.revision.installed_at,
                    )?,
                };
                let client = dial_backend(
                    &payload_dir,
                    &data_dir,
                    &translated.app_id,
                    &document.display_name,
                    backend,
                )?;
                let names = package::capability_names(&document);
                let artifact_type = document
                    .manifest
                    .artifact_types
                    .first()
                    .map(|declaration| declaration.name.clone());
                let handlers =
                    handlers_for_capabilities(&names, artifact_type, client.as_tool_call());
                (handlers, Some(client))
            }
        };
        Ok(PreparedActivation {
            revision_id: self.revision.revision_id,
            translated,
            consumer_grant_requests: document.consumer_grant_requests,
            handlers,
            client,
            lifecycle_generation: self.lifecycle_generation,
            package_digest: self.revision.package_digest,
        })
    }
}

impl DataMigrationPreparation {
    pub(crate) fn execute(self) -> Result<(Option<String>, String), String> {
        let (candidate_dir, source_digest) = crate::app_data::stage_candidate(
            &self.apps_root,
            &self.app_id,
            &self.candidate,
            self.source_revision_id.as_deref(),
        )?;
        if let Some(migration_revision) = self.migration_revision {
            let payload_dir = PathBuf::from(&migration_revision.payload_dir);
            let document = package::read_document(&payload_dir)?;
            let migration = match &document.data {
                package::AppData::Versioned { migration, .. } => migration,
                package::AppData::None => {
                    return Err("selected migration revision declares no app-owned data".into())
                }
                package::AppData::HostManaged { .. } => {
                    return Err(
                        "host-managed data does not use publisher migration commands".into(),
                    )
                }
            };
            let from = self
                .source_format_version
                .ok_or_else(|| "app-data migration source format is missing".to_string())?;
            if !migration.transitions.iter().any(|transition| {
                transition.from == from && transition.to == self.candidate.format_version
            }) {
                return Err(format!(
                    "migration revision no longer declares app-data transition {from} -> {}",
                    self.candidate.format_version
                ));
            }
            crate::app_data::run_migration_command(
                &payload_dir,
                &candidate_dir,
                &self.app_id,
                &document.backend,
                migration,
                from,
                self.candidate.format_version,
            )?;
        }

        let digest_apps_root = self.apps_root.clone();
        let digest_app_id = self.app_id.clone();
        let digest_candidate_id = self.candidate.revision_id.clone();
        let prepared = ActivationPreparation {
            id: self.app_id,
            revision: self.target_revision,
            apps_root: self.apps_root,
            allow_unsafe_native_backends: self.allow_unsafe_native_backends,
            lifecycle_generation: self.lifecycle_generation,
            kernel_invoker: None,
            data_dir_override: Some(candidate_dir),
        }
        .prepare()?;
        if let Some(client) = prepared.client() {
            client.shutdown();
        }
        let candidate_digest = crate::app_data::revision_digest(
            &digest_apps_root,
            &digest_app_id,
            &digest_candidate_id,
        )?;
        Ok((source_digest, candidate_digest))
    }
}

fn backend_policy_error(
    allow_unsafe_native_backends: bool,
    backend: &package::Backend,
) -> Option<String> {
    use package::{Backend, BackendAuthorityMode};

    let native = match backend {
        Backend::McpStdio { authority_mode, .. }
        | Backend::Executable { authority_mode, .. }
        | Backend::AgentWorker { authority_mode, .. } => authority_mode,
        Backend::None | Backend::McpStreamableHttp { .. } => return None,
    };

    match native {
        BackendAuthorityMode::Sandboxed => {
            let support = native_backend_sandbox_support();
            if support.supports_sandboxed_execution() {
                None
            } else {
                Some(format!(
                    "sandboxed native backend launch is unsupported on this platform: filesystem={}, network={}",
                    support.filesystem.reason().unwrap_or("unsupported"),
                    support.network.reason().unwrap_or("unsupported"),
                ))
            }
        }
        BackendAuthorityMode::Unsandboxed if allow_unsafe_native_backends => None,
        BackendAuthorityMode::Unsandboxed => Some(
            "unsandboxed native backends require --allow-unsafe-native-backends, the user-level KESTRAL_ALLOW_UNSAFE_NATIVE_BACKENDS=true environment variable, or a debug build"
                .into(),
        ),
    }
}

pub struct ActivationFailure {
    pub reason: String,
    pub client: Option<Arc<McpClient>>,
}

pub struct PreparedKernelActivation {
    pub install: PreparedInstall,
    pub continuation: KernelActivation,
}

pub struct KernelActivation {
    pub app_id: AppId,
    pub revision_id: String,
    pub lifecycle_generation: u64,
    pub package_digest: String,
    pub consumer_grant_requests: Vec<package::ConsumerGrantRequest>,
    pub ui_bundles: Vec<(SurfaceName, SurfaceUiBundle)>,
    pub client: Option<Arc<McpClient>>,
}

struct PreparedRevisionRollback {
    app_id: String,
    previous_record: InstallRecord,
    added_record: InstallRecord,
    payload_dir: PathBuf,
}

impl AppManager {
    pub(crate) fn validate_persisted_profile(
        store_path: &Path,
        trust_store_path: &Path,
        apps_root: &Path,
        logical_apps_root: &Path,
        journal_path: &Path,
    ) -> Result<(), String> {
        let stored = load_json_document::<StoreDocument>(store_path, "installed apps")?;
        if let Some(document) = stored {
            if document.version != STORE_VERSION {
                return Err(format!(
                    "unsupported installed-apps store version {}",
                    document.version
                ));
            }
            let mut app_ids = BTreeSet::new();
            for record in document.apps {
                if record.id.is_empty() || !app_ids.insert(record.id.clone()) {
                    return Err("installed-apps store contains an empty or duplicate app id".into());
                }
                let mut revision_ids = BTreeSet::new();
                for revision in &record.revisions {
                    if revision.revision_id.is_empty()
                        || !revision_ids.insert(revision.revision_id.clone())
                    {
                        return Err(format!(
                            "installed app '{}' contains an empty or duplicate revision id",
                            record.id
                        ));
                    }
                    let payload = Path::new(&revision.payload_dir);
                    let materialized_payload = payload
                        .strip_prefix(logical_apps_root)
                        .ok()
                        .map(|relative| apps_root.join(relative));
                    if materialized_payload
                        .as_ref()
                        .is_none_or(|candidate| !candidate.exists())
                    {
                        return Err(format!(
                            "installed app '{}' revision '{}' references missing or out-of-profile payload '{}'",
                            record.id,
                            revision.revision_id,
                            payload.display()
                        ));
                    }
                    let materialized_payload =
                        materialized_payload.expect("validated materialized payload exists");
                    let digest = package::package_digest(&materialized_payload)?;
                    if digest != revision.package_digest {
                        return Err(format!(
                            "installed app '{}' revision '{}' package digest mismatch",
                            record.id, revision.revision_id
                        ));
                    }
                }
                if !revision_ids.contains(&record.active_revision_id) {
                    return Err(format!(
                        "installed app '{}' active revision '{}' is absent",
                        record.id, record.active_revision_id
                    ));
                }
            }
        }
        if let Some(journal) =
            load_json_document::<UpdateJournal>(journal_path, "app update journal")?
        {
            journal.validate()?;
        }
        crate::app_data::validate_all(&apps_root.join(".data"))?;
        crate::managed_data::ManagedDataStore::validate_all(&apps_root.join(".data"))?;
        PublisherTrustStore::new(trust_store_path.to_path_buf())?;
        Ok(())
    }

    /// Load persisted records from `store_path`; copy payloads live under
    /// `apps_root`.
    pub fn new(
        store_path: PathBuf,
        trust_store_path: PathBuf,
        apps_root: PathBuf,
        journal_path: PathBuf,
        allow_unsafe_native_backends: bool,
    ) -> Result<Self, String> {
        let stored = load_json_document::<StoreDocument>(&store_path, "installed apps")?;
        let journal = load_json_document::<UpdateJournal>(&journal_path, "app update journal")?;
        if let Some(journal) = journal.as_ref() {
            journal.validate()?;
        }
        let trust_store = PublisherTrustStore::new(trust_store_path)?;
        let records = match stored {
            None => BTreeMap::new(),
            Some(document) => {
                if document.version != STORE_VERSION {
                    return Err(format!(
                        "unsupported installed-apps store version {}",
                        document.version
                    ));
                }
                document
                    .apps
                    .into_iter()
                    .map(|record| (record.id.clone(), record))
                    .collect()
            }
        };
        let staging_root = apps_root.join(".staging");
        Ok(Self {
            store_path: Some(store_path),
            journal_path: Some(journal_path),
            trust_store,
            apps_root,
            allow_unsafe_native_backends,
            staging_root,
            staged_inspections: BTreeMap::new(),
            pending_transition_plans: BTreeMap::new(),
            records,
            update_journal: journal,
            failures: BTreeMap::new(),
            clients: BTreeMap::new(),
        })
    }

    /// In-memory manager for tests (no persistence).
    pub fn in_memory(apps_root: PathBuf) -> Self {
        let staging_root = apps_root.join(".staging");
        Self {
            store_path: None,
            journal_path: None,
            trust_store: PublisherTrustStore::in_memory(),
            apps_root,
            allow_unsafe_native_backends: false,
            staging_root,
            staged_inspections: BTreeMap::new(),
            pending_transition_plans: BTreeMap::new(),
            records: BTreeMap::new(),
            update_journal: None,
            failures: BTreeMap::new(),
            clients: BTreeMap::new(),
        }
    }

    pub fn records(&self) -> impl Iterator<Item = &InstallRecord> {
        self.records.values()
    }

    pub(crate) fn active_host_managed_data(&self, id: &str) -> Result<package::AppData, String> {
        let record = self.record(id)?;
        if !record.enabled || record.uninstalling {
            return Err(format!("app '{id}' is not active"));
        }
        let revision = self.active_revision(record)?;
        let actual_digest = package::package_digest(Path::new(&revision.payload_dir))?;
        if actual_digest != revision.package_digest {
            return Err(format!(
                "installed app '{id}' package digest mismatch; managed-data access refused"
            ));
        }
        let document = package::read_document(Path::new(&revision.payload_dir))?;
        match document.data {
            data @ package::AppData::HostManaged { .. } => Ok(data),
            package::AppData::None | package::AppData::Versioned { .. } => {
                Err(format!("app '{id}' does not declare host-managed data"))
            }
        }
    }

    pub fn app_presentation_views(
        &self,
    ) -> Result<BTreeMap<String, package::AppPresentationView>, String> {
        self.records
            .values()
            .map(|record| {
                let revision = self.active_revision(record)?;
                let package_dir = Path::new(&revision.payload_dir);
                let document = package::read_document(package_dir)?;
                Ok((
                    record.id.clone(),
                    package::AppPresentationView {
                        icon: package::app_icon_view(package_dir, document.icon.as_ref())?,
                        theme_colors: document.theme_colors,
                    },
                ))
            })
            .collect()
    }

    fn unsafe_native_backends_allowed(&self) -> bool {
        self.allow_unsafe_native_backends || cfg!(debug_assertions)
    }

    fn backend_policy_error(&self, backend: &package::Backend) -> Option<String> {
        backend_policy_error(self.unsafe_native_backends_allowed(), backend)
    }

    fn record(&self, id: &str) -> Result<&InstallRecord, String> {
        self.records
            .get(id)
            .ok_or_else(|| format!("'{id}' is not a managed app"))
    }

    fn record_mut(&mut self, id: &str) -> Result<&mut InstallRecord, String> {
        self.records
            .get_mut(id)
            .ok_or_else(|| format!("'{id}' is not a managed app"))
    }

    fn active_revision<'a>(&'a self, record: &'a InstallRecord) -> Result<&'a AppRevision, String> {
        record
            .revisions
            .iter()
            .find(|revision| revision.revision_id == record.active_revision_id)
            .ok_or_else(|| {
                format!(
                    "managed app '{}' has no active revision '{}': store is corrupt",
                    record.id, record.active_revision_id
                )
            })
    }

    fn revision<'a>(
        &'a self,
        record: &'a InstallRecord,
        revision_id: &str,
    ) -> Result<&'a AppRevision, String> {
        record
            .revisions
            .iter()
            .find(|revision| revision.revision_id == revision_id)
            .ok_or_else(|| {
                format!(
                    "managed app '{}' has no retained revision '{revision_id}'",
                    record.id
                )
            })
    }

    fn retain_recent_revisions(revisions: &mut Vec<AppRevision>, active_revision_id: &str) {
        revisions.sort_by(|left, right| left.installed_at.cmp(&right.installed_at));
        if revisions.len() <= RETAINED_REVISIONS {
            return;
        }
        let mut retained = Vec::with_capacity(RETAINED_REVISIONS);
        let mut active = None;
        for revision in revisions.iter().rev() {
            if revision.revision_id == active_revision_id {
                active = Some(revision.clone());
                break;
            }
        }
        if let Some(active) = active {
            retained.push(active);
        }
        for revision in revisions.iter().rev() {
            if retained.len() >= RETAINED_REVISIONS {
                break;
            }
            if retained
                .iter()
                .any(|candidate| candidate.revision_id == revision.revision_id)
            {
                continue;
            }
            retained.push(revision.clone());
        }
        retained.sort_by(|left, right| left.installed_at.cmp(&right.installed_at));
        *revisions = retained;
    }

    pub fn list_trusted_publishers(&self) -> Vec<TrustRecord> {
        self.trust_store.list()
    }

    pub fn trust_publisher_key(
        &mut self,
        key_id: &str,
        public_key: &str,
        scope: TrustScope,
    ) -> Result<Vec<TrustRecord>, String> {
        self.trust_store.trust_key(key_id, public_key, scope)?;
        Ok(self.trust_store.list())
    }

    pub fn revoke_publisher_key(
        &mut self,
        key_id: &str,
        scope: &TrustScope,
    ) -> Result<Vec<TrustRecord>, String> {
        self.trust_store.revoke_key(key_id, scope)?;
        Ok(self.trust_store.list())
    }

    pub fn managed_app_revisions(&self, app_id: &str) -> Result<Vec<AppRevision>, String> {
        Ok(self.record(app_id)?.revisions.clone())
    }

    pub fn record_failure(&mut self, id: &str, reason: String) {
        self.failures.insert(id.to_string(), reason);
    }

    // -- inspection ----------------------------------------------------------

    /// Inspect a package directory. Runs no package code (delegates to
    /// [`package::inspect`]).
    pub fn inspect(&mut self, package_dir: &Path) -> Result<package::PackageInspection, String> {
        let package_dir = package::resolve_package_directory(package_dir)?;
        let signature = package::read_signature_document(&package_dir)?;
        let inspection = package::stage_and_inspect_with_trust(
            &package_dir,
            &self.staging_root,
            &self.trust_store,
        )?;
        let document = package::read_document(&package_dir)?;
        let mut inspection = inspection;
        if let Some(error) = self.backend_policy_error(&document.backend) {
            inspection.installable = false;
            if inspection.blocking_error.is_none() {
                inspection.blocking_error = Some(error);
            }
        }
        self.staged_inspections.insert(
            inspection.staged_id.clone(),
            StagedInspection {
                inspection: inspection.clone(),
                signature,
            },
        );
        Ok(inspection)
    }

    pub(crate) fn staging_root(&self) -> &Path {
        &self.staging_root
    }

    // -- install -------------------------------------------------------------

    /// Install a package: verify + translate (no code), copy payload, persist
    /// a record, then activate (which prompts trusted chrome for grants). A
    /// backend that fails to start leaves the app installed but `failed`, not
    /// an install error — startup is separable from installation.
    pub fn install_record(
        &mut self,
        staged_id: &str,
        approved_digest: &str,
        installed_at: &str,
    ) -> Result<InstallRecord, String> {
        let staged_dir = package::staged_dir(&self.staging_root, staged_id)?;
        let StagedInspection {
            inspection,
            signature,
        } = self.staged_inspections.remove(staged_id).ok_or_else(|| {
            "staged inspection metadata is missing; inspect the package again before installing"
                .to_string()
        })?;
        if inspection.package_digest != approved_digest {
            return Err("staged package digest does not match the approved digest".into());
        }
        let document = package::read_document(&staged_dir)?;
        if let Some(error) = package::structural_error(&document) {
            return Err(error);
        }
        if !inspection.host_compatible {
            return Err(format!(
                "requires host {} or newer (this host is {})",
                document.min_host_version,
                package::HOST_VERSION
            ));
        }
        let (signature_verdict, signature_key_id) = if let Some(signature) = signature.as_ref() {
            match self.trust_store.verify_signature(
                approved_digest,
                signature,
                document.id.as_str(),
            ) {
                Ok(state @ SignatureState::ValidUnknownKey { .. })
                | Ok(state @ SignatureState::Trusted { .. }) => (
                    state.label().to_string(),
                    state.key_id().map(|key_id| key_id.to_string()),
                ),
                Ok(SignatureState::Unsigned) => ("unsigned".into(), None),
                Ok(SignatureState::Invalid { reason }) => {
                    return Err(format!("invalid package signature: {reason}"))
                }
                Ok(SignatureState::Revoked { key_id, .. }) => {
                    return Err(format!(
                        "package signature key '{key_id}' is revoked for this package"
                    ))
                }
                Err(error) => return Err(error),
            }
        } else {
            (
                inspection.signature.label().to_string(),
                inspection
                    .signature
                    .key_id()
                    .map(|key_id| key_id.to_string()),
            )
        };
        let id = document.id.clone();
        if self.records.contains_key(&id) {
            return Err(format!("app '{id}' is already installed"));
        }
        let app_root = self.apps_root.join(&id);
        fs::create_dir_all(&app_root)
            .map_err(|error| format!("create app payload root failed: {error}"))?;
        let revision_root = app_root.join("revisions");
        fs::create_dir_all(&revision_root)
            .map_err(|error| format!("create app revision root failed: {error}"))?;
        let revision_id = Uuid::new_v4().to_string();
        let payload_dir = revision_root.join(&revision_id);
        let temporary_dir = revision_root.join(format!(".installing-{}", Uuid::new_v4()));
        if payload_dir.exists() {
            return Err(format!(
                "verified payload destination already exists: {}",
                payload_dir.display()
            ));
        }
        package::copy_verified_package(&staged_dir, &temporary_dir, approved_digest)?;
        fs::rename(&temporary_dir, &payload_dir)
            .map_err(|error| format!("activate verified payload failed: {error}"))?;
        if package::package_digest(&payload_dir)? != approved_digest {
            let _ = fs::remove_dir_all(&payload_dir);
            return Err("post-install package verification failed".into());
        }

        let revision = AppRevision {
            revision_id: revision_id.clone(),
            version: document.version.clone(),
            display_name: document.display_name.clone(),
            description: document.description.clone(),
            backend_kind: document.backend.kind_label().to_string(),
            publisher: document.publisher.as_ref().map(|p| p.name.clone()),
            signature_verdict,
            signature_key_id,
            min_host_version: document.min_host_version.clone(),
            installed_at: installed_at.to_string(),
            payload_dir: payload_dir.to_string_lossy().to_string(),
            package_digest: approved_digest.to_string(),
        };

        let record = InstallRecord {
            id: id.clone(),
            enabled: true,
            uninstalling: false,
            lifecycle_generation: 0,
            purge_secrets: false,
            purge_data: false,
            purge_secret_names: Vec::new(),
            active_revision_id: revision.revision_id.clone(),
            revisions: vec![revision],
        };

        self.records.insert(id.clone(), record.clone());
        if let Err(error) = self.persist() {
            if !error.is_indeterminate() {
                // Roll back the on-disk payload and record only when the
                // registry candidate was not committed.
                self.records.remove(&id);
                let _ = fs::remove_dir_all(&payload_dir);
            }
            return Err(error.into_message());
        }

        let _ = package::remove_read_only_tree(&staged_dir);
        Ok(record)
    }

    // -- enable / disable ----------------------------------------------------

    /// Enable or disable a managed app. Disabling tears the app fully out of
    /// the kernel: no handlers, surfaces, subscriptions, broker secrets, or
    /// grants remain — only the persisted record and payload stay for re-enable.
    pub fn set_enabled_state(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        let record = self.record_mut(id)?;
        if record.uninstalling {
            return Err(format!("'{id}' is being uninstalled"));
        }
        if record.enabled == enabled {
            return Ok(());
        }
        // Keep the whole prior record so a pre-rename failure restores every
        // field (not just `enabled`), never leaving memory ahead of disk.
        let previous = record.clone();
        record.enabled = enabled;
        record.lifecycle_generation = record.lifecycle_generation.saturating_add(1);
        if let Err(error) = self.persist() {
            if !error.is_indeterminate() {
                self.records.insert(id.to_string(), previous);
            }
            return Err(error.into_message());
        }
        Ok(())
    }

    /// Remove kernel and UI runtime state, returning the external backend
    /// session for shutdown after the kernel mutex is released.
    pub fn remove_runtime(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        id: &str,
    ) -> Result<Option<Arc<McpClient>>, String> {
        self.deactivate(kernel, surface_ui, id)
    }

    // -- uninstall -----------------------------------------------------------

    pub fn begin_uninstall(
        &mut self,
        id: &str,
        purge_secrets: bool,
        purge_data: bool,
    ) -> Result<InstallRecord, String> {
        let existing = self.record(id)?.clone();
        // Already staged for uninstall on a prior (persisted) call: nothing new
        // to write, and the original request's purge choices stand.
        if existing.uninstalling {
            return Ok(existing);
        }
        let purge_secret_names = if purge_secrets {
            let payload_dir = PathBuf::from(&self.active_revision(&existing)?.payload_dir);
            let document = package::read_document(&payload_dir)?;
            package::owned_secret_names(&document)
        } else {
            existing.purge_secret_names.clone()
        };
        {
            let record = self.record_mut(id).expect("record was just loaded");
            record.uninstalling = true;
            record.purge_secrets = purge_secrets;
            record.purge_data = purge_data;
            record.purge_secret_names = purge_secret_names;
            record.lifecycle_generation = record.lifecycle_generation.saturating_add(1);
        }
        // Restore the pre-staging record if the registry write fails before
        // rename, so an uninstall is never half-applied in memory while disk
        // still shows the app active.
        if let Err(error) = self.persist() {
            if !error.is_indeterminate() {
                self.records.insert(id.to_string(), existing);
            }
            return Err(error.into_message());
        }
        Ok(self.records.get(id).expect("just persisted").clone())
    }

    pub fn finish_uninstall(
        &mut self,
        config: &mut HostConfigService,
        id: &str,
    ) -> Result<(), String> {
        let record = self.record(id)?.clone();
        if !record.uninstalling {
            return Err(format!("'{id}' is not marked for uninstall"));
        }
        if record.purge_secrets {
            let owner = AppId::new(id);
            for name in &record.purge_secret_names {
                config.clear_secret_persisted(&owner, name)?;
            }
        }
        let payload_dir = PathBuf::from(&self.active_revision(&record)?.payload_dir);
        if let Ok(document) = package::read_document(&payload_dir) {
            if matches!(
                document.backend,
                Backend::McpStdio { .. } | Backend::Executable { .. } | Backend::AgentWorker { .. }
            ) {
                let _ = delete_app_container_profile(&app_container_moniker(id));
            }
        }
        if record.purge_data {
            config.remove_app_config(id)?;
            let data_dir = self.apps_root.join(".data").join(id);
            if data_dir.exists() {
                fs::remove_dir_all(&data_dir)
                    .map_err(|error| format!("remove app data failed: {error}"))?;
            }
        }
        let app_root = self.apps_root.join(id);
        if app_root.exists() {
            fs::remove_dir_all(&app_root)
                .map_err(|error| format!("remove app payload root failed: {error}"))?;
        }
        let mut remaining = self.records.clone();
        remaining.remove(id);
        match self.persist_records(&remaining) {
            Ok(()) => self.records = remaining,
            Err(error) if error.is_indeterminate() => {
                self.records = remaining;
                return Err(error.into_message());
            }
            Err(error) => return Err(error.into_message()),
        }
        self.failures.remove(id);
        Ok(())
    }

    // -- bootstrap -----------------------------------------------------------

    /// Prepare all enabled app backends without touching kernel state.
    pub fn prepare_enabled_activations(&self) -> Vec<(String, Result<PreparedActivation, String>)> {
        self.records
            .values()
            .filter(|record| record.enabled && !record.uninstalling)
            .map(|record| (record.id.clone(), self.prepare_activation(&record.id)))
            .collect()
    }

    /// Remove runtimes that are durably disabled. Returns external sessions
    /// for shutdown after the caller releases the kernel mutex.
    pub fn reconcile_disabled(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
    ) -> Result<Vec<Arc<McpClient>>, String> {
        let disabled: Vec<String> = self
            .records
            .values()
            .filter(|record| !record.enabled || record.uninstalling)
            .map(|record| record.id.clone())
            .collect();
        let mut clients = Vec::new();
        for id in disabled {
            if let Some(client) = self.remove_runtime(kernel, surface_ui, &id)? {
                clients.push(client);
            }
        }
        Ok(clients)
    }

    pub fn pending_uninstall_ids(&self) -> Vec<String> {
        self.records
            .values()
            .filter(|record| record.uninstalling)
            .map(|record| record.id.clone())
            .collect()
    }

    pub fn pending_uninstall_secret_names(&self, id: &str) -> Vec<String> {
        self.records
            .get(id)
            .filter(|record| record.uninstalling && record.purge_secrets)
            .map(|record| record.purge_secret_names.clone())
            .unwrap_or_default()
    }

    // -- status --------------------------------------------------------------

    /// The full manager list: bundled apps (read-only) plus every managed app,
    /// active or not.
    pub fn status_views(
        &self,
        kernel: &Kernel,
        surface_ui: &SurfaceUiRegistry,
    ) -> Vec<AppStatusView> {
        let mut views: BTreeMap<String, AppStatusView> = BTreeMap::new();
        let target_contracts = active_extension_contracts(kernel);

        // Bundled / startup apps: present in the kernel, absent from records.
        for app in kernel.installed_apps() {
            let id = app.manifest.app_id.as_str().to_string();
            if self.records.contains_key(&id) {
                continue;
            }
            views.insert(
                id.clone(),
                AppStatusView {
                    id: id.clone(),
                    display_name: app.manifest.display_name.clone(),
                    version: app.manifest.version.clone(),
                    description: app.manifest.description.clone(),
                    bundled: true,
                    enabled: true,
                    status: "active".into(),
                    status_detail: Some("Bundled app".into()),
                    backend_kind: "bundled".into(),
                    signature: "bundled".into(),
                    publisher: None,
                    missing_permissions: missing_permissions(kernel, &app.manifest.app_id, app),
                    surfaces: surface_infos(surface_ui, &app.manifest),
                    min_host_version: None,
                    installed_at: None,
                    revisions: Vec::new(),
                    extension_contributions: extension_contribution_views(
                        &app.manifest.extension_contributions,
                        &target_contracts,
                    ),
                    removable: false,
                },
            );
        }

        // Managed apps (may be disabled / failed and absent from the kernel).
        for record in self.records.values() {
            let app_id = AppId::new(&record.id);
            let active = kernel.installed_app(&app_id).ok();
            let active_revision = self.active_revision(record).ok();
            let retained_document = active_revision
                .and_then(|revision| package::read_document(Path::new(&revision.payload_dir)).ok());
            let extension_contributions = active
                .map(|app| app.manifest.extension_contributions.as_slice())
                .or_else(|| {
                    retained_document
                        .as_ref()
                        .map(|document| document.manifest.extension_contributions.as_slice())
                })
                .map(|contributions| extension_contribution_views(contributions, &target_contracts))
                .unwrap_or_default();
            let (status, detail, missing, surfaces) = if !record.enabled {
                (
                    "disabled".to_string(),
                    None,
                    0,
                    self.record_surfaces(record, surface_ui),
                )
            } else if let Some(app) = active {
                let incoming_missing = active_revision
                    .and(retained_document.as_ref())
                    .map(|document| missing_consumer_permissions(kernel, document))
                    .unwrap_or(0);
                let missing = missing_permissions(kernel, &app_id, app) + incoming_missing;
                let status = if missing > 0 {
                    "needs-permissions"
                } else {
                    "active"
                };
                (
                    status.to_string(),
                    None,
                    missing,
                    surface_infos(surface_ui, &app.manifest),
                )
            } else {
                let reason = self
                    .failures
                    .get(&record.id)
                    .cloned()
                    .unwrap_or_else(|| "not running".into());
                (
                    "failed".to_string(),
                    Some(reason),
                    0,
                    self.record_surfaces(record, surface_ui),
                )
            };

            views.insert(
                record.id.clone(),
                AppStatusView {
                    id: record.id.clone(),
                    display_name: active_revision
                        .map(|revision| revision.display_name.clone())
                        .unwrap_or_else(|| record.id.clone()),
                    version: active_revision
                        .map(|revision| revision.version.clone())
                        .unwrap_or_else(|| "unknown".into()),
                    description: active_revision
                        .map(|revision| revision.description.clone())
                        .unwrap_or_default(),
                    bundled: false,
                    enabled: record.enabled,
                    status,
                    status_detail: detail,
                    backend_kind: active_revision
                        .map(|revision| revision.backend_kind.clone())
                        .unwrap_or_default(),
                    signature: active_revision
                        .map(|revision| revision.signature_verdict.clone())
                        .unwrap_or_else(|| "unsigned".into()),
                    publisher: active_revision.and_then(|revision| revision.publisher.clone()),
                    missing_permissions: missing,
                    surfaces,
                    min_host_version: active_revision
                        .map(|revision| revision.min_host_version.clone()),
                    installed_at: active_revision.map(|revision| revision.installed_at.clone()),
                    revisions: record.revisions.clone(),
                    extension_contributions,
                    removable: true,
                },
            );
        }

        views.into_values().collect()
    }

    /// Surfaces for a record that isn't currently in the kernel: re-read from
    /// the payload's `app.json` (best-effort).
    fn record_surfaces(
        &self,
        record: &InstallRecord,
        surface_ui: &SurfaceUiRegistry,
    ) -> Vec<AppSurfaceInfo> {
        let Ok(revision) = self.active_revision(record) else {
            return Vec::new();
        };
        let Ok(document) = package::read_document(Path::new(&revision.payload_dir)) else {
            return Vec::new();
        };
        let app_id = AppId::new(&record.id);
        document
            .manifest
            .surfaces
            .iter()
            .map(|surface| AppSurfaceInfo {
                name: surface.name.to_string(),
                kind: format!("{:?}", surface.kind).to_lowercase(),
                title: surface.title.clone(),
                has_custom_ui: surface.ui.is_some()
                    || surface_ui.get(&app_id, &surface.name).is_some(),
            })
            .collect()
    }

    // -- internal: activation ------------------------------------------------

    /// Prepare an activation without accessing kernel state. External backend
    /// startup and MCP discovery must not run while the kernel mutex is held.
    pub fn prepare_activation(&self, id: &str) -> Result<PreparedActivation, String> {
        let record = self.record(id)?;
        let revision_id = record.active_revision_id.clone();
        self.prepare_activation_for_revision(id, &revision_id, None)
    }

    #[cfg(test)]
    pub(crate) fn prepare_activation_with_invoker(
        &self,
        id: &str,
        kernel_invoker: KernelInvokerClient,
    ) -> Result<PreparedActivation, String> {
        let record = self.record(id)?;
        let revision_id = record.active_revision_id.clone();
        self.prepare_activation_for_revision(id, &revision_id, Some(kernel_invoker))
    }

    pub(crate) fn enabled_activation_preparations_with_invoker(
        &self,
        kernel_invoker: &KernelInvokerClient,
    ) -> Vec<(String, Result<ActivationPreparation, String>)> {
        self.records
            .values()
            .filter(|record| record.enabled && !record.uninstalling)
            .map(|record| {
                (
                    record.id.clone(),
                    self.activation_preparation_for_revision(
                        &record.id,
                        &record.active_revision_id,
                        Some(kernel_invoker.clone()),
                    ),
                )
            })
            .collect()
    }

    pub(crate) fn activation_preparation_with_invoker(
        &self,
        id: &str,
        kernel_invoker: KernelInvokerClient,
    ) -> Result<ActivationPreparation, String> {
        let record = self.record(id)?;
        self.activation_preparation_for_revision(
            id,
            &record.active_revision_id,
            Some(kernel_invoker),
        )
    }

    fn activation_preparation_for_revision(
        &self,
        id: &str,
        revision_id: &str,
        kernel_invoker: Option<KernelInvokerClient>,
    ) -> Result<ActivationPreparation, String> {
        let record = self.record(id)?;
        Ok(ActivationPreparation {
            id: id.to_string(),
            revision: self.revision(record, revision_id)?.clone(),
            apps_root: self.apps_root.clone(),
            allow_unsafe_native_backends: self.unsafe_native_backends_allowed(),
            lifecycle_generation: record.lifecycle_generation,
            kernel_invoker,
            data_dir_override: None,
        })
    }

    pub(crate) fn prepare_activation_for_revision(
        &self,
        id: &str,
        revision_id: &str,
        kernel_invoker: Option<KernelInvokerClient>,
    ) -> Result<PreparedActivation, String> {
        self.activation_preparation_for_revision(id, revision_id, kernel_invoker)?
            .prepare()
    }

    /// Validate the translated manifest and collect its install prompts. No
    /// prompt is shown and no kernel state changes in this phase.
    pub fn prepare_kernel_activation(
        &self,
        kernel: &Kernel,
        id: &str,
        prepared: PreparedActivation,
    ) -> Result<PreparedKernelActivation, ActivationFailure> {
        let record = self.records.get(id).ok_or_else(|| ActivationFailure {
            reason: format!("'{id}' is no longer a managed app"),
            client: prepared.client.clone(),
        })?;
        if !record.enabled || record.uninstalling {
            return Err(ActivationFailure {
                reason: format!("app '{id}' is no longer enabled"),
                client: prepared.client,
            });
        }
        if record.lifecycle_generation != prepared.lifecycle_generation
            || self.revision(record, &prepared.revision_id).is_err()
        {
            return Err(ActivationFailure {
                reason: format!("app '{id}' changed while activation was prepared"),
                client: prepared.client,
            });
        }
        let lifecycle_generation = prepared.lifecycle_generation;
        let package_digest = prepared.package_digest.clone();
        let revision_id = prepared.revision_id.clone();
        let PreparedActivation {
            translated,
            consumer_grant_requests,
            handlers,
            client,
            revision_id: _,
            lifecycle_generation: _,
            package_digest: _,
        } = prepared;
        let install = kernel
            .prepare_install(translated.sealed, handlers)
            .map_err(|error| ActivationFailure {
                reason: format!("prepare install failed: {error}"),
                client: client.clone(),
            })?;
        Ok(PreparedKernelActivation {
            install,
            continuation: KernelActivation {
                app_id: translated.app_id,
                revision_id,
                lifecycle_generation,
                package_digest,
                consumer_grant_requests,
                ui_bundles: translated.ui_bundles,
                client,
            },
        })
    }

    pub fn commit_kernel_activation(
        &mut self,
        kernel: &mut Kernel,
        id: &str,
        approval: app_host_kernel::kernel::InstallApproval,
        continuation: KernelActivation,
    ) -> Result<KernelActivation, ActivationFailure> {
        let record = self.records.get(id).ok_or_else(|| ActivationFailure {
            reason: format!("'{id}' is no longer a managed app"),
            client: continuation.client.clone(),
        })?;
        if !record.enabled || record.uninstalling {
            return Err(ActivationFailure {
                reason: format!("app '{id}' is no longer enabled"),
                client: continuation.client,
            });
        }
        if record.lifecycle_generation != continuation.lifecycle_generation
            || self
                .revision(record, &continuation.revision_id)
                .map(|revision| revision.package_digest != continuation.package_digest)
                .unwrap_or(true)
        {
            return Err(ActivationFailure {
                reason: format!("app '{id}' changed while activation was awaiting approval"),
                client: continuation.client,
            });
        }
        kernel
            .commit_install(approval)
            .map_err(|error| ActivationFailure {
                reason: format!("install into kernel failed: {error}"),
                client: continuation.client.clone(),
            })?;
        Ok(continuation)
    }

    pub fn prepare_consumer_grants(
        &self,
        kernel: &Kernel,
        requests: Vec<package::ConsumerGrantRequest>,
    ) -> Result<Vec<PreparedGrant>, String> {
        requests
            .iter()
            .filter(|consumer| {
                !kernel
                    .grants_for(&consumer.holder)
                    .into_iter()
                    .any(|grant| {
                        scope_covers(&grant.scope, &consumer.request.scope)
                            && grant.data_scope.covers(&consumer.request.data_scope)
                            && grant.condition == consumer.request.condition
                    })
            })
            .map(|consumer| {
                kernel
                    .prepare_grant(&consumer.holder, consumer.request.clone())
                    .map_err(|error| error.to_string())
            })
            .collect()
    }

    pub fn finish_kernel_activation(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        id: &str,
        mut activation: KernelActivation,
        approvals: Vec<GrantApproval>,
    ) -> Result<(), ActivationFailure> {
        let Some(record) = self.records.get(id) else {
            let reason = Self::rollback_activation(
                kernel,
                &activation.app_id,
                format!("'{id}' is no longer a managed app"),
            );
            return Err(ActivationFailure {
                reason,
                client: activation.client,
            });
        };
        if !record.enabled || record.uninstalling {
            let reason = Self::rollback_activation(
                kernel,
                &activation.app_id,
                format!("app '{id}' is no longer enabled"),
            );
            return Err(ActivationFailure {
                reason,
                client: activation.client,
            });
        }
        if record.lifecycle_generation != activation.lifecycle_generation
            || self
                .revision(record, &activation.revision_id)
                .map(|revision| revision.package_digest != activation.package_digest)
                .unwrap_or(true)
        {
            let reason = Self::rollback_activation(
                kernel,
                &activation.app_id,
                format!("app '{id}' changed while consumer grants were awaiting approval"),
            );
            return Err(ActivationFailure {
                reason,
                client: activation.client,
            });
        }
        for approval in approvals {
            if let Err(error) = kernel.commit_grant(approval) {
                let rollback = kernel.uninstall(&activation.app_id).err();
                let reason = match rollback {
                    Some(rollback) => {
                        format!("consumer grant failed: {error}; rollback failed: {rollback}")
                    }
                    None => format!("consumer grant failed: {error}"),
                };
                return Err(ActivationFailure {
                    reason,
                    client: activation.client.take(),
                });
            }
        }
        if let Some(client) = activation.client.take() {
            self.clients.insert(id.to_string(), client);
        }
        for (surface, bundle) in activation.ui_bundles {
            surface_ui.register(activation.app_id.clone(), surface, bundle);
        }
        self.failures.remove(id);
        Ok(())
    }

    fn rollback_activation(kernel: &mut Kernel, app_id: &AppId, reason: String) -> String {
        if let Err(error) = kernel.uninstall(app_id) {
            return format!("{reason}; activation rollback failed: {error}");
        }
        reason
    }

    /// Remove an app from the kernel and stop everything host-side that carried
    /// its authority: backend session, sandboxed UI bundles. The kernel handles
    /// grants (both directions), runs, surfaces, and broker secrets.
    fn deactivate(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        id: &str,
    ) -> Result<Option<Arc<McpClient>>, String> {
        let app_id = AppId::new(id);
        if kernel.installed_app(&app_id).is_ok() {
            kernel
                .uninstall(&app_id)
                .map_err(|error| format!("uninstall app from kernel failed: {error}"))?;
        }
        surface_ui.remove_app(&app_id);
        Ok(self.clients.remove(id))
    }

    fn active_revision_document(
        &self,
        record: &InstallRecord,
    ) -> Result<package::PackageDocument, String> {
        let revision = self.active_revision(record)?;
        package::read_document(Path::new(&revision.payload_dir))
    }

    fn package_document_for_revision(
        &self,
        record: &InstallRecord,
        revision_id: &str,
    ) -> Result<package::PackageDocument, String> {
        let revision = self.revision(record, revision_id)?;
        package::read_document(Path::new(&revision.payload_dir))
    }

    fn summarize_grant_request(
        request: &app_host_kernel::manifest::GrantRequest,
    ) -> package::GrantRequestSummary {
        package::GrantRequestSummary {
            scope_label: grant_scope_label(&request.scope),
            data_scope_label: data_scope_label(&request.data_scope),
            condition: format!("{:?}", request.condition).to_lowercase(),
            reason: request.reason.clone(),
            duration_label: grant_duration_label(&request.duration),
        }
    }

    fn summarize_permissions(
        document: &package::PackageDocument,
    ) -> (
        Vec<package::GrantRequestSummary>,
        Vec<package::GrantRequestSummary>,
    ) {
        let permissions = document
            .manifest
            .grant_requests
            .iter()
            .map(Self::summarize_grant_request)
            .collect();
        let consumer_permissions = document
            .consumer_grant_requests
            .iter()
            .map(|consumer| {
                let mut summary = Self::summarize_grant_request(&consumer.request);
                summary.scope_label = format!("{} -> {}", consumer.holder, summary.scope_label);
                summary
            })
            .collect();
        (permissions, consumer_permissions)
    }

    fn diff_permission_sets(
        current: &[package::GrantRequestSummary],
        target: &[package::GrantRequestSummary],
    ) -> ManagedAppPermissionDiff<package::GrantRequestSummary> {
        let current_set: BTreeSet<String> = current.iter().map(permission_key).collect();
        let target_set: BTreeSet<String> = target.iter().map(permission_key).collect();
        let unchanged = target
            .iter()
            .filter(|summary| current_set.contains(&permission_key(summary)))
            .cloned()
            .collect();
        let added = target
            .iter()
            .filter(|summary| !current_set.contains(&permission_key(summary)))
            .cloned()
            .collect();
        let removed = current
            .iter()
            .filter(|summary| !target_set.contains(&permission_key(summary)))
            .cloned()
            .collect();
        ManagedAppPermissionDiff {
            unchanged,
            added,
            widened: Vec::new(),
            removed,
        }
    }

    fn diff_documents(
        &self,
        current: Option<&package::PackageDocument>,
        target: &package::PackageDocument,
        active_manifests: &[AppManifest],
    ) -> ManagedAppUpdateDiff {
        let current_capabilities = current.map(capability_summaries).unwrap_or_default();
        let target_capabilities = capability_summaries(target);
        let current_surfaces = current.map(surface_summaries).unwrap_or_default();
        let target_surfaces = surface_summaries(target);
        let (current_permissions, current_consumer_permissions) =
            current.map(Self::summarize_permissions).unwrap_or_default();
        let (target_permissions, target_consumer_permissions) = Self::summarize_permissions(target);

        ManagedAppUpdateDiff {
            version_relation: current
                .map(|current| compare_versions(&current.version, &target.version))
                .unwrap_or(ManagedAppVersionRelation::Higher),
            display_name_changed: current.map(|current| current.display_name.as_str())
                != Some(target.display_name.as_str()),
            description_changed: current.map(|current| current.description.as_str())
                != Some(target.description.as_str()),
            backend_kind_changed: current.map(|current| current.backend.kind_label())
                != Some(target.backend.kind_label()),
            current_backend_authority_mode: current
                .and_then(|current| current.backend.authority_mode()),
            target_backend_authority_mode: target.backend.authority_mode(),
            current_data: current.map(|document| document.data.summary()),
            target_data: target.data.summary(),
            publisher_key_continuity: publisher_continuity(
                current.and_then(|document| {
                    document
                        .publisher
                        .as_ref()
                        .and_then(|publisher| publisher.key_id.as_deref())
                }),
                target
                    .publisher
                    .as_ref()
                    .and_then(|publisher| publisher.key_id.as_deref()),
            ),
            capabilities_added: set_diff_names(&current_capabilities, &target_capabilities, true),
            capabilities_removed: set_diff_names(
                &current_capabilities,
                &target_capabilities,
                false,
            ),
            surfaces_added: set_diff_surface_names(&current_surfaces, &target_surfaces, true),
            surfaces_removed: set_diff_surface_names(&current_surfaces, &target_surfaces, false),
            permissions: Self::diff_permission_sets(&current_permissions, &target_permissions),
            consumer_permissions: Self::diff_permission_sets(
                &current_consumer_permissions,
                &target_consumer_permissions,
            ),
            extension_warnings: self.extension_update_warnings(current, target, active_manifests),
        }
    }

    fn extension_update_warnings(
        &self,
        current: Option<&package::PackageDocument>,
        target: &package::PackageDocument,
        active_manifests: &[AppManifest],
    ) -> Vec<ManagedAppExtensionWarning> {
        let Some(current) = current else {
            return Vec::new();
        };
        let current_points: BTreeMap<_, _> = current
            .manifest
            .extension_points
            .iter()
            .map(|point| (point.name.as_str(), point.contract_version))
            .collect();
        let target_points: BTreeMap<_, _> = target
            .manifest
            .extension_points
            .iter()
            .map(|point| (point.name.as_str(), point.contract_version))
            .collect();
        let mut contributors: BTreeMap<String, Vec<ExtensionContribution>> = active_manifests
            .iter()
            .map(|manifest| {
                (
                    manifest.app_id.to_string(),
                    manifest.extension_contributions.clone(),
                )
            })
            .collect();
        for record in self.records.values().filter(|record| !record.uninstalling) {
            if contributors.contains_key(&record.id) {
                continue;
            }
            let Ok(document) = self.active_revision_document(record) else {
                continue;
            };
            contributors.insert(record.id.clone(), document.manifest.extension_contributions);
        }

        let mut warnings = Vec::new();
        for (contributor_app_id, contributions) in contributors {
            for contribution in contributions {
                if contribution.target_app.as_str() != target.id {
                    continue;
                }
                let point_name = contribution.extension_point.as_str();
                let Some(current_version) = current_points.get(point_name).copied() else {
                    continue;
                };
                if current_version != contribution.contract_version {
                    continue;
                }
                let target_version = target_points.get(point_name).copied();
                if target_version == Some(contribution.contract_version) {
                    continue;
                }
                warnings.push(ManagedAppExtensionWarning {
                    contributor_app_id: contributor_app_id.clone(),
                    extension_point: point_name.to_string(),
                    surface: contribution.surface.to_string(),
                    contribution_contract_version: contribution.contract_version,
                    current_target_contract_version: current_version,
                    target_contract_version: target_version,
                });
            }
        }
        warnings.sort_by(|left, right| {
            (
                left.contributor_app_id.as_str(),
                left.extension_point.as_str(),
                left.surface.as_str(),
            )
                .cmp(&(
                    right.contributor_app_id.as_str(),
                    right.extension_point.as_str(),
                    right.surface.as_str(),
                ))
        });
        warnings
    }

    fn plan_data_transition(
        &self,
        app_id: &str,
        current_document: Option<&package::PackageDocument>,
        target: &package::PackageDocument,
        reverse_migration: bool,
    ) -> Result<Option<ManagedAppDataTransition>, String> {
        let persisted = crate::app_data::current_revision(&self.apps_root, app_id)?;
        let retained_managed = crate::managed_data::ManagedDataStore::new(
            crate::managed_data::data_root(&self.apps_root),
        )
        .exists(&AppId::new(app_id));
        if let Some(contract) = target.data.host_managed() {
            crate::managed_data::ManagedDataStore::new(crate::managed_data::data_root(
                &self.apps_root,
            ))
            .validate_contract(&AppId::new(app_id), contract)
            .map_err(|error| {
                format!("target host-managed data contract is incompatible: {error}")
            })?;
        }
        if current_document.is_none() {
            if retained_managed && !matches!(target.data, package::AppData::HostManaged { .. }) {
                return Err(
                    "retained host-managed data exists, but this package declares a different data kind"
                        .into(),
                );
            }
            return match (persisted.as_ref(), target.data.format_version()) {
                (None, _) => Ok(None),
                (Some(persisted), None) => Err(format!(
                    "retained app data is format {}, but this package declares no app-owned data",
                    persisted.format_version
                )),
                (Some(persisted), Some(target)) if persisted.format_version == target => Ok(None),
                (Some(persisted), Some(target)) => Err(format!(
                    "retained app data is format {}, but this package expects format {target}; install a matching package and update through its declared migration",
                    persisted.format_version
                )),
            };
        }
        let current_data = &current_document.expect("handled above").data;
        match (current_data, &target.data) {
            (package::AppData::HostManaged { .. }, package::AppData::HostManaged { .. }) => {
                return Ok(None)
            }
            (package::AppData::HostManaged { .. }, _) => {
                return Err(
                    "a package cannot replace host-managed data with another data kind".into(),
                )
            }
            (package::AppData::Versioned { .. }, package::AppData::HostManaged { .. }) => {
                return Err(
                    "a package cannot replace versioned native data with host-managed data without an explicit import contract"
                        .into(),
                )
            }
            (package::AppData::None, package::AppData::HostManaged { .. }) => return Ok(None),
            _ => {}
        }
        let current_format = match current_document.map(|document| &document.data) {
            None => unreachable!("handled above"),
            Some(package::AppData::None) => {
                if let Some(persisted) = persisted.as_ref() {
                    return Err(format!(
                        "app '{app_id}' has retained versioned data format {} but its current package declares no app-owned data",
                        persisted.format_version
                    ));
                }
                None
            }
            Some(package::AppData::Versioned { format_version, .. }) => {
                let persisted = persisted.as_ref().ok_or_else(|| {
                    format!("app '{app_id}' is missing its versioned app-data state")
                })?;
                if persisted.format_version != *format_version {
                    return Err(format!(
                        "app '{app_id}' package declares data format {format_version}, but active data is format {}",
                        persisted.format_version
                    ));
                }
                Some(*format_version)
            }
            Some(package::AppData::HostManaged { .. }) => {
                unreachable!("host-managed transition handled above")
            }
        };
        let target_format = target.data.format_version();
        match (current_format, target_format) {
            (None, None) => Ok(None),
            (Some(_), None) => Err(
                "a package cannot drop versioned app data; publish an explicit versioned migration instead"
                    .into(),
            ),
            (Some(source), Some(target_format)) if source == target_format => Ok(None),
            (None, Some(target_format)) => Ok(Some(ManagedAppDataTransition {
                source_format_version: None,
                target_format_version: target_format,
                destructive: false,
                reverse_migration: false,
            })),
            (Some(source), Some(target_format)) => {
                let migration = if reverse_migration {
                    match current_document.map(|document| &document.data) {
                        Some(package::AppData::Versioned { migration, .. }) => migration,
                        _ => return Err("current package has no app-data migration command".into()),
                    }
                } else {
                    match &target.data {
                        package::AppData::Versioned { migration, .. } => migration,
                        package::AppData::None => unreachable!("target format is present"),
                        package::AppData::HostManaged { .. } => {
                            unreachable!("host-managed transition handled above")
                        }
                    }
                };
                let transition = migration
                    .transitions
                    .iter()
                    .find(|transition| {
                        transition.from == source && transition.to == target_format
                    })
                    .ok_or_else(|| {
                        format!(
                            "package declares data format {target_format} but no tested migration from active format {source}"
                        )
                    })?;
                Ok(Some(ManagedAppDataTransition {
                    source_format_version: Some(source),
                    target_format_version: target_format,
                    destructive: transition.destructive,
                    reverse_migration,
                }))
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn plan_managed_app_transition(
        &mut self,
        request: ManagedAppTransitionRequest,
    ) -> Result<ManagedAppTransitionPlan, String> {
        self.plan_managed_app_transition_with_manifests(request, &[])
    }

    pub(crate) fn plan_managed_app_transition_with_manifests(
        &mut self,
        request: ManagedAppTransitionRequest,
        active_manifests: &[AppManifest],
    ) -> Result<ManagedAppTransitionPlan, String> {
        let plan: Result<ManagedAppTransitionPlan, String> = match request.operation {
            ManagedAppOperation::Revert => {
                let app_id = request
                    .app_id
                    .ok_or_else(|| "revert requires app_id".to_string())?;
                let revision_id = request
                    .revision_id
                    .ok_or_else(|| "revert requires revision_id".to_string())?;
                if !request.acknowledge_revert_data_caveat {
                    return Err("revert requires explicit acknowledgement of app-data compatibility or migration".into());
                }
                let record = self.record(&app_id)?;
                let current = self.active_revision(record)?;
                let target = self.revision(record, &revision_id)?;
                let current_doc = self.active_revision_document(record).ok();
                let target_doc = self.package_document_for_revision(record, &revision_id)?;
                let data_transition =
                    self.plan_data_transition(&app_id, current_doc.as_ref(), &target_doc, true)?;
                let diff = self.diff_documents(current_doc.as_ref(), &target_doc, active_manifests);
                Ok(ManagedAppTransitionPlan {
                    transition_id: Uuid::new_v4().to_string(),
                    app_id,
                    operation: ManagedAppOperation::Revert,
                    current_revision_id: Some(current.revision_id.clone()),
                    target_revision_id: target.revision_id.clone(),
                    target_version: target.version.clone(),
                    diff,
                    requires_explicit_approval: true,
                    data_rollback_supported: data_transition.is_some(),
                    data_rollback_caveat: data_transition.as_ref().map(|_| {
                        "Kestral stages a declared reverse migration and restores the prior data revision if activation fails."
                            .into()
                    }),
                    data_transition,
                    staged_id: None,
                    package_digest: None,
                    revision_id: Some(revision_id),
                })
            }
            operation => {
                let staged_id = request
                    .staged_id
                    .ok_or_else(|| "package-based transitions require staged_id".to_string())?;
                let package_digest = request.package_digest.ok_or_else(|| {
                    "package-based transitions require package_digest".to_string()
                })?;
                let staged_dir = package::staged_dir(&self.staging_root, &staged_id)?;
                let inspection = self.staged_inspections.get(&staged_id).ok_or_else(|| {
                    "staged inspection metadata is missing; inspect the package again before transitioning"
                        .to_string()
                })?;
                if inspection.inspection.package_digest != package_digest {
                    return Err("staged package digest does not match the approved digest".into());
                }
                let target_doc = package::read_document(&staged_dir)?;
                if let Some(error) = package::structural_error(&target_doc) {
                    return Err(error);
                }
                // Refuse invalid/revoked signatures and host-incompatible
                // packages at plan time too (mirroring install_record: only a
                // package that ships a signature document is verified), so the
                // transition fails early with the same gate a fresh install has.
                if let Some(signature) = inspection.signature.as_ref() {
                    match self.trust_store.verify_signature(
                        &package_digest,
                        signature,
                        target_doc.id.as_str(),
                    ) {
                        Ok(SignatureState::Invalid { reason }) => {
                            return Err(format!("invalid package signature: {reason}"));
                        }
                        Ok(SignatureState::Revoked { key_id, .. }) => {
                            return Err(format!(
                                "package signature key '{key_id}' is revoked for this package"
                            ));
                        }
                        Ok(_) => {}
                        Err(error) => return Err(error),
                    }
                }
                if !inspection.inspection.host_compatible {
                    return Err(format!(
                        "requires host {} or newer (this host is {})",
                        target_doc.min_host_version,
                        package::HOST_VERSION
                    ));
                }
                let _target_version = Version::parse(&target_doc.version).map_err(|error| {
                    format!(
                        "version '{}' is not strict semver: {error}",
                        target_doc.version
                    )
                })?;
                let app_id = target_doc.id.clone();
                let current_record = self.records.get(&app_id);
                let current_doc =
                    current_record.and_then(|record| self.active_revision_document(record).ok());
                let current_digest = current_record
                    .and_then(|record| self.active_revision(record).ok())
                    .map(|revision| revision.package_digest.as_str());
                let version_relation = current_doc
                    .as_ref()
                    .map(|current| compare_versions(&current.version, &target_doc.version))
                    .unwrap_or(ManagedAppVersionRelation::Higher);
                let classification = classify_transition(
                    current_record.is_some(),
                    version_relation.clone(),
                    current_digest,
                    &package_digest,
                    operation.clone(),
                )?;
                let diff = self.diff_documents(current_doc.as_ref(), &target_doc, active_manifests);
                let reverse_migration = matches!(classification, ManagedAppOperation::Downgrade);
                let data_transition = self.plan_data_transition(
                    &app_id,
                    current_doc.as_ref(),
                    &target_doc,
                    reverse_migration,
                )?;
                let data_rollback_caveat = data_transition.as_ref().map(|_| {
                    "The previous app-data revision is retained as a recoverable backup; failed activation restores it automatically."
                        .into()
                });
                if matches!(classification, ManagedAppOperation::Downgrade)
                    && !request.acknowledge_downgrade
                {
                    return Err("downgrade requires explicit acknowledgement".into());
                }
                Ok(ManagedAppTransitionPlan {
                    transition_id: Uuid::new_v4().to_string(),
                    app_id,
                    operation: classification,
                    current_revision_id: current_record
                        .map(|record| record.active_revision_id.clone()),
                    target_revision_id: Uuid::new_v4().to_string(),
                    target_version: target_doc.version.clone(),
                    diff,
                    requires_explicit_approval: true,
                    data_rollback_supported: data_transition.is_some(),
                    data_rollback_caveat,
                    data_transition,
                    staged_id: Some(staged_id),
                    package_digest: Some(package_digest),
                    revision_id: None,
                })
            }
        };
        let plan = plan?;
        self.pending_transition_plans
            .retain(|_, pending| pending.app_id != plan.app_id);
        self.pending_transition_plans
            .insert(plan.transition_id.clone(), plan.clone());
        Ok(plan)
    }

    pub(crate) fn take_managed_app_transition_plan(
        &mut self,
        transition_id: &str,
    ) -> Result<ManagedAppTransitionPlan, String> {
        self.pending_transition_plans
            .remove(transition_id)
            .ok_or_else(|| {
                format!(
                    "managed-app transition '{transition_id}' is missing, stale, or already applied; review the package again"
                )
            })
    }

    fn write_update_journal(
        &mut self,
        journal: Option<UpdateJournal>,
    ) -> Result<(), AtomicJsonError> {
        // Adopt the journal after commit, including an indeterminate post-rename
        // result. Assigning first on a pre-rename failure would leave this
        // process believing in a transition that crash recovery cannot find.
        if let Some(path) = self.journal_path.as_ref() {
            match journal.as_ref() {
                Some(document) => match persist_json_document(
                    path,
                    document,
                    "app update journal",
                    standard_writer().as_ref(),
                ) {
                    Ok(()) => {}
                    Err(error) if error.is_indeterminate() => {
                        self.update_journal = journal;
                        return Err(error);
                    }
                    Err(error) => return Err(error),
                },
                None => match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(AtomicJsonError::NotCommitted(format!(
                            "remove app update journal failed: {error}"
                        )))
                    }
                },
            }
        }
        self.update_journal = journal;
        Ok(())
    }

    fn data_transition_journal(
        &self,
        plan: &ManagedAppTransitionPlan,
        current_record: &InstallRecord,
        target_revision: &AppRevision,
    ) -> Result<Option<AppDataTransitionJournal>, String> {
        let Some(summary) = plan.data_transition.as_ref() else {
            return Ok(None);
        };
        let source = crate::app_data::current_revision(&self.apps_root, &plan.app_id)?;
        if source.as_ref().map(|revision| revision.format_version) != summary.source_format_version
        {
            return Err(
                "app data changed after the transition review; review the package again".into(),
            );
        }
        let migration_revision_id = summary.source_format_version.map(|_| {
            if summary.reverse_migration {
                current_record.active_revision_id.clone()
            } else {
                target_revision.revision_id.clone()
            }
        });
        Ok(Some(AppDataTransitionJournal {
            source_revision_id: source.map(|revision| revision.revision_id),
            source_format_version: summary.source_format_version,
            source_digest: None,
            candidate: crate::app_data::AppDataRevision {
                revision_id: Uuid::new_v4().to_string(),
                format_version: summary.target_format_version,
                package_revision_id: target_revision.revision_id.clone(),
                created_at: Utc::now().to_rfc3339(),
            },
            candidate_digest: None,
            migration_revision_id,
            destructive: summary.destructive,
        }))
    }

    pub(crate) fn begin_journaled_transition(
        &mut self,
        plan: ManagedAppTransitionPlan,
    ) -> Result<Option<UpdateJournal>, String> {
        let mut prepared_revision_rollback: Option<PreparedRevisionRollback> = None;
        let mut journal = match plan.operation {
            ManagedAppOperation::Revert => {
                let record = self.record(&plan.app_id)?.clone();
                if plan.current_revision_id.as_deref() != Some(record.active_revision_id.as_str()) {
                    return Err(
                        "managed-app state changed after the transition review; review it again"
                            .into(),
                    );
                }
                let revision_id = plan
                    .revision_id
                    .as_deref()
                    .ok_or_else(|| "revert plan is missing revision_id".to_string())?;
                let target_revision = self.revision(&record, revision_id)?.clone();
                let data_transition =
                    self.data_transition_journal(&plan, &record, &target_revision)?;
                UpdateJournal::new(
                    plan.transition_id,
                    plan.app_id,
                    ManagedAppOperation::Revert,
                    plan.current_revision_id,
                    target_revision,
                    record.revisions.clone(),
                    record.enabled,
                )
                .with_data_transition(data_transition)
            }
            ManagedAppOperation::Update
            | ManagedAppOperation::Reinstall
            | ManagedAppOperation::Downgrade => {
                let staged_id = plan
                    .staged_id
                    .as_deref()
                    .ok_or_else(|| "transition plan is missing staged_id".to_string())?;
                let package_digest = plan
                    .package_digest
                    .as_deref()
                    .ok_or_else(|| "transition plan is missing package_digest".to_string())?;
                let staged_dir = package::staged_dir(&self.staging_root, staged_id)?;
                let target_doc = package::read_document(&staged_dir)?;
                let staged = self.staged_inspections.get(staged_id).ok_or_else(|| {
                    "staged inspection metadata is missing; inspect the package again before transitioning"
                        .to_string()
                })?;
                if staged.inspection.package_digest != package_digest {
                    return Err("staged package digest changed after the transition review".into());
                }
                if target_doc.id != plan.app_id {
                    return Err("staged package app id does not match the reviewed app".into());
                }
                if let Some(signature) = staged.signature.as_ref() {
                    match self.trust_store.verify_signature(
                        package_digest,
                        signature,
                        target_doc.id.as_str(),
                    ) {
                        Ok(SignatureState::Invalid { reason }) => {
                            return Err(format!("invalid package signature: {reason}"));
                        }
                        Ok(SignatureState::Revoked { key_id, .. }) => {
                            return Err(format!(
                                "package signature key '{key_id}' is revoked for this package"
                            ));
                        }
                        Ok(_) => {}
                        Err(error) => return Err(error),
                    }
                }
                if !staged.inspection.host_compatible {
                    return Err(format!(
                        "requires host {} or newer (this host is {})",
                        target_doc.min_host_version,
                        package::HOST_VERSION
                    ));
                }
                let current_record = self.record(&plan.app_id)?.clone();
                let current_revision = self.active_revision(&current_record)?.clone();
                if plan.current_revision_id.as_deref()
                    != Some(current_revision.revision_id.as_str())
                {
                    return Err(
                        "managed-app state changed after the transition review; review it again"
                            .into(),
                    );
                }
                Uuid::parse_str(&plan.target_revision_id)
                    .map_err(|_| "reviewed target revision id is invalid".to_string())?;
                let app_root = self.apps_root.join(&plan.app_id).join("revisions");
                fs::create_dir_all(&app_root)
                    .map_err(|error| format!("create app revision root failed: {error}"))?;
                let payload_dir = app_root.join(&plan.target_revision_id);
                if payload_dir.exists() {
                    return Err(format!(
                        "verified payload destination already exists: {}",
                        payload_dir.display()
                    ));
                }
                let temporary_dir = app_root.join(format!(".transitioning-{}", Uuid::new_v4()));
                if let Err(error) =
                    package::copy_verified_package(&staged_dir, &temporary_dir, package_digest)
                {
                    let _ = fs::remove_dir_all(&temporary_dir);
                    return Err(error);
                }
                if let Err(error) = fs::rename(&temporary_dir, &payload_dir) {
                    let _ = fs::remove_dir_all(&temporary_dir);
                    return Err(format!("activate verified payload failed: {error}"));
                }
                let installed_digest = match package::package_digest(&payload_dir) {
                    Ok(digest) => digest,
                    Err(error) => {
                        let _ = fs::remove_dir_all(&payload_dir);
                        return Err(error);
                    }
                };
                if installed_digest != package_digest {
                    let _ = fs::remove_dir_all(&payload_dir);
                    return Err("post-transition package verification failed".into());
                }
                let revision = AppRevision {
                    revision_id: plan.target_revision_id.clone(),
                    version: target_doc.version.clone(),
                    display_name: target_doc.display_name.clone(),
                    description: target_doc.description.clone(),
                    backend_kind: target_doc.backend.kind_label().to_string(),
                    publisher: target_doc
                        .publisher
                        .as_ref()
                        .map(|publisher| publisher.name.clone()),
                    signature_verdict: staged.inspection.signature.label().to_string(),
                    signature_key_id: staged.inspection.signature.key_id().map(str::to_string),
                    min_host_version: target_doc.min_host_version,
                    installed_at: Utc::now().to_rfc3339(),
                    payload_dir: payload_dir.to_string_lossy().to_string(),
                    package_digest: package_digest.to_string(),
                };
                let previous_record = self.record(&plan.app_id)?.clone();
                let mut record = previous_record.clone();
                if !record
                    .revisions
                    .iter()
                    .any(|existing| existing.revision_id == revision.revision_id)
                {
                    self.record_mut(&plan.app_id)?
                        .revisions
                        .push(revision.clone());
                    record = self.record(&plan.app_id)?.clone();
                    if let Err(error) = self.persist() {
                        if !error.is_indeterminate() {
                            self.records
                                .insert(plan.app_id.clone(), previous_record.clone());
                            let _ = fs::remove_dir_all(&payload_dir);
                        }
                        return Err(error.into_message());
                    }
                }
                prepared_revision_rollback = Some(PreparedRevisionRollback {
                    app_id: plan.app_id.clone(),
                    previous_record,
                    added_record: record.clone(),
                    payload_dir,
                });
                let data_transition =
                    self.data_transition_journal(&plan, &current_record, &revision)?;
                UpdateJournal::new(
                    plan.transition_id,
                    plan.app_id,
                    plan.operation,
                    Some(current_revision.revision_id),
                    revision,
                    record.revisions,
                    record.enabled,
                )
                .with_data_transition(data_transition)
            }
            ManagedAppOperation::Install | ManagedAppOperation::VersionConflict => {
                return Err("operation is not a journaled existing-app transition".into());
            }
        };
        if !journal.enabled {
            journal.phase = UpdatePhase::Deactivated;
        }
        if let Err(error) = self.write_update_journal(Some(journal.clone())) {
            if error.is_indeterminate() {
                return Err(error.into_message());
            }
            let error = error.into_message();
            if let Some(rollback) = prepared_revision_rollback {
                self.records
                    .insert(rollback.app_id.clone(), rollback.previous_record);
                if let Err(rollback_error) = self.persist() {
                    self.records.insert(rollback.app_id, rollback.added_record);
                    return Err(format!(
                        "{error}; installed-app rollback also failed: {}",
                        rollback_error.into_message()
                    ));
                }
                if let Err(rollback_error) = fs::remove_dir_all(&rollback.payload_dir) {
                    return Err(format!(
                        "{error}; copied-payload rollback also failed: {rollback_error}"
                    ));
                }
            }
            return Err(error);
        }
        if !journal.enabled && journal.data_transition.is_none() {
            self.commit_revision_transition(&journal)?;
            self.write_update_journal(None)
                .map_err(AtomicJsonError::into_message)?;
            return Ok(None);
        }
        Ok(Some(journal))
    }

    fn require_current_transition(&self, journal: &UpdateJournal) -> Result<(), String> {
        match self.update_journal.as_ref() {
            Some(current) if current.transition_id == journal.transition_id => Ok(()),
            _ => Err(format!(
                "managed-app transition '{}' is no longer current",
                journal.transition_id
            )),
        }
    }

    pub(crate) fn pending_update_journal(&self) -> Option<UpdateJournal> {
        self.update_journal.clone()
    }

    pub(crate) fn clear_completed_journal(
        &mut self,
        journal: &UpdateJournal,
    ) -> Result<(), String> {
        self.require_current_transition(journal)?;
        match journal.phase {
            UpdatePhase::RolledBack | UpdatePhase::Committed => self
                .write_update_journal(None)
                .map_err(AtomicJsonError::into_message),
            _ => Err(format!(
                "cannot clear managed-app transition '{}' in phase {:?}",
                journal.transition_id, journal.phase
            )),
        }
    }

    pub(crate) fn deactivate_journaled_transition(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        journal: &mut UpdateJournal,
    ) -> Result<Option<Arc<McpClient>>, String> {
        self.require_current_transition(journal)?;
        let client = self.deactivate(kernel, surface_ui, &journal.app_id)?;
        journal.phase = UpdatePhase::Deactivated;
        self.write_update_journal(Some(journal.clone()))
            .map_err(AtomicJsonError::into_message)?;
        Ok(client)
    }

    pub(crate) fn transition_activation_preparation(
        &self,
        journal: &UpdateJournal,
        revision_id: &str,
        kernel_invoker: KernelInvokerClient,
    ) -> Result<ActivationPreparation, String> {
        self.require_current_transition(journal)?;
        self.activation_preparation_for_revision(&journal.app_id, revision_id, Some(kernel_invoker))
    }

    pub(crate) fn data_migration_preparation(
        &self,
        journal: &UpdateJournal,
    ) -> Result<Option<DataMigrationPreparation>, String> {
        self.require_current_transition(journal)?;
        let Some(data) = journal.data_transition.as_ref() else {
            return Ok(None);
        };
        let record = self.record(&journal.app_id)?;
        let migration_revision = data
            .migration_revision_id
            .as_deref()
            .map(|revision_id| self.revision(record, revision_id).cloned())
            .transpose()?;
        Ok(Some(DataMigrationPreparation {
            app_id: journal.app_id.clone(),
            apps_root: self.apps_root.clone(),
            source_revision_id: data.source_revision_id.clone(),
            source_format_version: data.source_format_version,
            candidate: data.candidate.clone(),
            migration_revision,
            target_revision: journal.target_revision.clone(),
            allow_unsafe_native_backends: self.unsafe_native_backends_allowed(),
            lifecycle_generation: record.lifecycle_generation,
        }))
    }

    pub(crate) fn mark_data_candidate_validated(
        &mut self,
        journal: &mut UpdateJournal,
        source_digest: Option<String>,
        candidate_digest: String,
    ) -> Result<(), String> {
        self.require_current_transition(journal)?;
        if journal.data_transition.is_none() {
            return Err("managed-app transition has no app-data candidate".into());
        }
        let data = journal
            .data_transition
            .as_mut()
            .expect("validated data transition exists");
        if data.source_revision_id.is_some() != source_digest.is_some() {
            return Err("app-data source digest does not match the journal transition".into());
        }
        data.source_digest = source_digest;
        data.candidate_digest = Some(candidate_digest);
        journal.phase = UpdatePhase::DataCandidateValidated;
        self.write_update_journal(Some(journal.clone()))
            .map_err(AtomicJsonError::into_message)
    }

    pub(crate) fn commit_data_candidate(
        &mut self,
        journal: &mut UpdateJournal,
    ) -> Result<(), String> {
        self.require_current_transition(journal)?;
        let data = journal
            .data_transition
            .as_ref()
            .ok_or_else(|| "managed-app transition has no app-data candidate".to_string())?;
        if let Some(source_revision_id) = data.source_revision_id.as_deref() {
            let expected = data
                .source_digest
                .as_deref()
                .ok_or_else(|| "validated app-data source digest is missing".to_string())?;
            let actual = crate::app_data::revision_digest(
                &self.apps_root,
                &journal.app_id,
                source_revision_id,
            )?;
            if actual != expected {
                return Err(
                    "active app data changed after candidate validation; source was preserved"
                        .into(),
                );
            }
        }
        let expected_candidate = data
            .candidate_digest
            .as_deref()
            .ok_or_else(|| "validated app-data candidate digest is missing".to_string())?;
        let actual_candidate = crate::app_data::revision_digest(
            &self.apps_root,
            &journal.app_id,
            &data.candidate.revision_id,
        )?;
        if actual_candidate != expected_candidate {
            return Err("app-data candidate changed after validation; source was preserved".into());
        }
        crate::app_data::commit_candidate(
            &self.apps_root,
            &journal.app_id,
            data.source_revision_id.as_deref(),
            data.candidate.clone(),
        )?;
        journal.phase = UpdatePhase::DataCommitted;
        self.write_update_journal(Some(journal.clone()))
            .map_err(AtomicJsonError::into_message)
    }

    pub(crate) fn commit_journaled_transition(
        &mut self,
        journal: &mut UpdateJournal,
        backup_retention: u32,
    ) -> Result<(), String> {
        self.require_current_transition(journal)?;
        journal.phase = UpdatePhase::Activated;
        self.write_update_journal(Some(journal.clone()))
            .map_err(AtomicJsonError::into_message)?;
        self.commit_revision_transition(journal)?;
        if journal.data_transition.is_some() {
            crate::app_data::prune_backups(&self.apps_root, &journal.app_id, backup_retention)?;
        }
        journal.phase = UpdatePhase::Committed;
        self.write_update_journal(Some(journal.clone()))
            .map_err(AtomicJsonError::into_message)?;
        self.write_update_journal(None)
            .map_err(AtomicJsonError::into_message)
    }

    pub(crate) fn begin_journaled_rollback(
        &mut self,
        journal: &mut UpdateJournal,
    ) -> Result<String, String> {
        self.require_current_transition(journal)?;
        let revision_id = journal
            .current_revision_id
            .clone()
            .ok_or_else(|| "rollback journal is missing the previous revision".to_string())?;
        journal.phase = UpdatePhase::RollingBack;
        self.write_update_journal(Some(journal.clone()))
            .map_err(AtomicJsonError::into_message)?;
        if let Some(data) = journal.data_transition.as_ref() {
            crate::app_data::rollback_transition(
                &self.apps_root,
                &journal.app_id,
                data.source_revision_id.as_deref(),
                &data.candidate.revision_id,
            )?;
            journal.phase = UpdatePhase::DataRollbackCommitted;
            self.write_update_journal(Some(journal.clone()))
                .map_err(AtomicJsonError::into_message)?;
        }
        Ok(revision_id)
    }

    pub(crate) fn finish_journaled_rollback(
        &mut self,
        journal: &mut UpdateJournal,
        previous_revision_id: &str,
    ) -> Result<(), String> {
        self.require_current_transition(journal)?;
        let previous = self.record(&journal.app_id)?.clone();
        let record = self.record_mut(&journal.app_id)?;
        record.active_revision_id = previous_revision_id.to_string();
        Self::retain_recent_revisions(&mut record.revisions, &record.active_revision_id);
        if let Err(error) = self.persist() {
            if !error.is_indeterminate() {
                self.records.insert(journal.app_id.clone(), previous);
            }
            return Err(error.into_message());
        }
        journal.phase = UpdatePhase::RolledBack;
        self.write_update_journal(Some(journal.clone()))
            .map_err(AtomicJsonError::into_message)?;
        self.write_update_journal(None)
            .map_err(AtomicJsonError::into_message)
    }

    #[cfg(test)]
    pub(crate) fn recover_managed_app_transition(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        kernel_invoker: KernelInvokerClient,
    ) -> Result<(), String> {
        let Some(journal) = self.update_journal.clone() else {
            return Ok(());
        };
        match journal.phase {
            UpdatePhase::Prepared | UpdatePhase::Deactivated => {
                self.apply_update_journal(kernel, surface_ui, kernel_invoker, journal)
            }
            UpdatePhase::Activated | UpdatePhase::RollingBack => {
                self.complete_update_journal(kernel, surface_ui, kernel_invoker, journal)
            }
            UpdatePhase::DataCandidateValidated | UpdatePhase::DataCommitted => Err(
                "test-only recovery helper cannot drive an app-data transition; use phased recovery"
                    .into(),
            ),
            UpdatePhase::DataRollbackCommitted => {
                self.complete_update_journal(kernel, surface_ui, kernel_invoker, journal)
            }
            UpdatePhase::RolledBack | UpdatePhase::Committed => self
                .write_update_journal(None)
                .map_err(AtomicJsonError::into_message),
        }
    }

    #[cfg(test)]
    fn apply_update_journal(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        kernel_invoker: KernelInvokerClient,
        mut journal: UpdateJournal,
    ) -> Result<(), String> {
        if !journal.enabled {
            self.commit_revision_transition(&journal)?;
            return self
                .write_update_journal(None)
                .map_err(AtomicJsonError::into_message);
        }
        if journal.phase == UpdatePhase::Prepared {
            let _ = self.deactivate(kernel, surface_ui, &journal.app_id);
            journal.phase = UpdatePhase::Deactivated;
            self.write_update_journal(Some(journal.clone()))
                .map_err(AtomicJsonError::into_message)?;
        }
        // Activate the target revision. If it fails after the old revision was
        // already torn down, roll back to the previous revision instead of
        // leaving the app permanently deactivated with the journal wedged at
        // `Deactivated` (which `recover_managed_app_transition` would only ever
        // retry against the same broken target).
        match self.activate_journal_target(kernel, surface_ui, kernel_invoker.clone(), &journal) {
            Ok(()) => {
                journal.phase = UpdatePhase::Activated;
                self.write_update_journal(Some(journal.clone()))
                    .map_err(AtomicJsonError::into_message)?;
                self.commit_revision_transition(&journal)?;
                self.write_update_journal(None)
                    .map_err(AtomicJsonError::into_message)
            }
            Err(activation_error) => {
                // No recorded previous revision to restore (e.g. a first
                // install): surface the failure as-is.
                if journal.current_revision_id.is_none() {
                    return Err(activation_error);
                }
                journal.phase = UpdatePhase::RollingBack;
                self.write_update_journal(Some(journal.clone()))
                    .map_err(AtomicJsonError::into_message)?;
                match self.restore_previous_revision(kernel, surface_ui, kernel_invoker, &journal) {
                    Ok(()) => {
                        self.write_update_journal(None)
                            .map_err(AtomicJsonError::into_message)?;
                        Err(format!(
                            "update activation failed and was rolled back to the previous revision: {activation_error}"
                        ))
                    }
                    Err(rollback_error) => Err(format!(
                        "update activation failed ({activation_error}); rollback also failed ({rollback_error})"
                    )),
                }
            }
        }
    }

    /// Activate the journal's target revision through the phased action path.
    /// Extracted so `apply_update_journal` can attempt a rollback if any step
    /// fails after the previous revision has already been deactivated.
    #[cfg(test)]
    fn activate_journal_target(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        kernel_invoker: KernelInvokerClient,
        journal: &UpdateJournal,
    ) -> Result<(), String> {
        let target_revision_id = journal.target_revision.revision_id.clone();
        let prepared = self.prepare_activation_for_revision(
            &journal.app_id,
            &target_revision_id,
            Some(kernel_invoker),
        )?;
        let prepared_kernel = self
            .prepare_kernel_activation(kernel, &journal.app_id, prepared)
            .map_err(|failure| failure.reason)?;
        let approval = prepared_kernel.install.await_approval();
        let continuation = self
            .commit_kernel_activation(
                kernel,
                &journal.app_id,
                approval,
                prepared_kernel.continuation,
            )
            .map_err(|failure| failure.reason)?;
        let consumer_approvals =
            self.prepare_consumer_grants(kernel, continuation.consumer_grant_requests.clone())?;
        self.finish_kernel_activation(
            kernel,
            surface_ui,
            &journal.app_id,
            continuation,
            PreparedGrant::await_grouped_approvals(consumer_approvals)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|failure| failure.reason)
    }

    #[cfg(test)]
    fn complete_update_journal(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        kernel_invoker: KernelInvokerClient,
        journal: UpdateJournal,
    ) -> Result<(), String> {
        match journal.phase {
            UpdatePhase::Activated => self.commit_revision_transition(&journal),
            UpdatePhase::RollingBack => {
                self.restore_previous_revision(kernel, surface_ui, kernel_invoker, &journal)
            }
            _ => Ok(()),
        }?;
        self.write_update_journal(None)
            .map_err(AtomicJsonError::into_message)
    }

    fn commit_revision_transition(&mut self, journal: &UpdateJournal) -> Result<(), String> {
        let previous = self.record(&journal.app_id)?.clone();
        let new_revision = {
            let record = self.record_mut(&journal.app_id)?;
            let new_revision = record
                .revisions
                .iter()
                .find(|revision| revision.revision_id == journal.target_revision.revision_id)
                .cloned()
                .unwrap_or_else(|| journal.target_revision.clone());
            record.active_revision_id = new_revision.revision_id.clone();
            if !record
                .revisions
                .iter()
                .any(|revision| revision.revision_id == new_revision.revision_id)
            {
                record.revisions.push(new_revision.clone());
            }
            Self::retain_recent_revisions(&mut record.revisions, &record.active_revision_id);
            new_revision
        };
        let _ = new_revision;
        match self.persist() {
            Ok(()) => Ok(()),
            Err(error) if error.is_indeterminate() => Err(error.into_message()),
            Err(error) => {
                self.records.insert(journal.app_id.clone(), previous);
                Err(error.into_message())
            }
        }
    }

    #[cfg(test)]
    fn restore_previous_revision(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        kernel_invoker: KernelInvokerClient,
        journal: &UpdateJournal,
    ) -> Result<(), String> {
        let previous_revision_id = journal
            .current_revision_id
            .as_deref()
            .ok_or_else(|| "rollback journal is missing the previous revision".to_string())?;
        let prepared = self.prepare_activation_for_revision(
            &journal.app_id,
            previous_revision_id,
            Some(kernel_invoker),
        )?;
        let prepared_kernel = self
            .prepare_kernel_activation(kernel, &journal.app_id, prepared)
            .map_err(|failure| failure.reason)?;
        let approval = prepared_kernel.install.await_approval();
        let continuation = self
            .commit_kernel_activation(
                kernel,
                &journal.app_id,
                approval,
                prepared_kernel.continuation,
            )
            .map_err(|failure| failure.reason)?;
        let consumer_approvals =
            self.prepare_consumer_grants(kernel, continuation.consumer_grant_requests.clone())?;
        self.finish_kernel_activation(
            kernel,
            surface_ui,
            &journal.app_id,
            continuation,
            PreparedGrant::await_grouped_approvals(consumer_approvals)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|failure| failure.reason)?;
        let previous = self.record(&journal.app_id)?.clone();
        let record = self.record_mut(&journal.app_id)?;
        record.active_revision_id = previous_revision_id.to_string();
        Self::retain_recent_revisions(&mut record.revisions, &record.active_revision_id);
        match self.persist() {
            Ok(()) => Ok(()),
            Err(error) if error.is_indeterminate() => Err(error.into_message()),
            Err(error) => {
                self.records.insert(journal.app_id.clone(), previous);
                Err(error.into_message())
            }
        }
    }

    fn persist(&self) -> Result<(), AtomicJsonError> {
        self.persist_records(&self.records)
    }

    fn persist_records(
        &self,
        records: &BTreeMap<String, InstallRecord>,
    ) -> Result<(), AtomicJsonError> {
        let Some(path) = &self.store_path else {
            return Ok(());
        };
        let document = StoreDocument {
            version: STORE_VERSION,
            apps: records.values().cloned().collect(),
        };
        persist_json_document(
            path,
            &document,
            "installed apps",
            standard_writer().as_ref(),
        )
    }
}

fn active_extension_contracts(kernel: &Kernel) -> BTreeMap<String, BTreeMap<String, u32>> {
    kernel
        .installed_apps()
        .map(|app| {
            (
                app.manifest.app_id.to_string(),
                app.manifest
                    .extension_points
                    .iter()
                    .map(|point| (point.name.to_string(), point.contract_version))
                    .collect(),
            )
        })
        .collect()
}

fn extension_contribution_views(
    contributions: &[ExtensionContribution],
    target_contracts: &BTreeMap<String, BTreeMap<String, u32>>,
) -> Vec<AppExtensionContributionView> {
    let mut views: Vec<_> = contributions
        .iter()
        .map(|contribution| {
            let target_app = contribution.target_app.to_string();
            let extension_point = contribution.extension_point.to_string();
            let (compatibility, target_contract_version) = match target_contracts.get(&target_app) {
                None => (AppExtensionCompatibility::TargetMissing, None),
                Some(points) => match points.get(&extension_point).copied() {
                    None => (AppExtensionCompatibility::PointMissing, None),
                    Some(version) if version == contribution.contract_version => {
                        (AppExtensionCompatibility::Exact, Some(version))
                    }
                    Some(version) => (AppExtensionCompatibility::ContractMismatch, Some(version)),
                },
            };
            AppExtensionContributionView {
                target_app,
                extension_point,
                contract_version: contribution.contract_version,
                surface: contribution.surface.to_string(),
                compatibility,
                target_contract_version,
            }
        })
        .collect();
    views.sort_by(|left, right| {
        (
            left.target_app.as_str(),
            left.extension_point.as_str(),
            left.surface.as_str(),
        )
            .cmp(&(
                right.target_app.as_str(),
                right.extension_point.as_str(),
                right.surface.as_str(),
            ))
    });
    views
}

/// Count grant requests an app declares that have no covering active grant —
/// the "needs permissions" signal.
fn missing_permissions(
    kernel: &Kernel,
    app_id: &AppId,
    app: &app_host_kernel::services::registry::InstalledApp,
) -> usize {
    let active = kernel.grants_for(app_id);
    app.manifest
        .grant_requests
        .iter()
        .filter(|request| {
            !active.iter().any(|grant| {
                scope_covers(&grant.scope, &request.scope)
                    && grant.data_scope.covers(&request.data_scope)
            })
        })
        .count()
}

fn missing_consumer_permissions(kernel: &Kernel, document: &package::PackageDocument) -> usize {
    document
        .consumer_grant_requests
        .iter()
        .filter(|consumer| {
            !kernel.grants_for(&consumer.holder).iter().any(|grant| {
                scope_covers(&grant.scope, &consumer.request.scope)
                    && grant.data_scope.covers(&consumer.request.data_scope)
            })
        })
        .count()
}

/// Whether an active grant scope satisfies a requested scope.
fn scope_covers(active: &GrantScope, requested: &GrantScope) -> bool {
    match (active, requested) {
        (a, b) if a == b => true,
        (
            GrantScope::AllProviderCapabilities { provider },
            GrantScope::ExactCapability {
                provider: requested_provider,
                ..
            },
        ) => provider == requested_provider,
        _ => false,
    }
}

fn compare_versions(current: &str, target: &str) -> ManagedAppVersionRelation {
    let current = Version::parse(current).ok();
    let target = Version::parse(target).ok();
    match (current, target) {
        (Some(current), Some(target)) if target > current => ManagedAppVersionRelation::Higher,
        (Some(current), Some(target)) if target < current => ManagedAppVersionRelation::Lower,
        _ => ManagedAppVersionRelation::Same,
    }
}

fn classify_transition(
    installed: bool,
    version_relation: ManagedAppVersionRelation,
    current_digest: Option<&str>,
    target_digest: &str,
    requested: ManagedAppOperation,
) -> Result<ManagedAppOperation, String> {
    if !installed {
        return match requested {
            ManagedAppOperation::Install => Ok(ManagedAppOperation::Install),
            _ => Err("app is not installed; use install".into()),
        };
    }
    if let Some(current_digest) = current_digest {
        if current_digest == target_digest {
            return match requested {
                ManagedAppOperation::Reinstall => Ok(ManagedAppOperation::Reinstall),
                ManagedAppOperation::Install => {
                    Err("app is already installed; use reinstall".into())
                }
                ManagedAppOperation::Update => {
                    Err("same digest cannot be treated as an update".into())
                }
                ManagedAppOperation::Downgrade => {
                    Err("same digest cannot be treated as a downgrade".into())
                }
                ManagedAppOperation::Revert => {
                    Err("use revert to switch retained revisions".into())
                }
                ManagedAppOperation::VersionConflict => {
                    Err("version conflict must be resolved explicitly".into())
                }
            };
        }
        if matches!(version_relation, ManagedAppVersionRelation::Same) {
            return Err(
                "same app id and version with a different digest is a version conflict".into(),
            );
        }
    }
    match version_relation {
        ManagedAppVersionRelation::Higher => match requested {
            ManagedAppOperation::Update => Ok(ManagedAppOperation::Update),
            ManagedAppOperation::Install => {
                Err("higher version requires explicit update intent".into())
            }
            ManagedAppOperation::Reinstall => Err("higher version is not a reinstall".into()),
            ManagedAppOperation::Downgrade => Err("higher version cannot be downgraded".into()),
            ManagedAppOperation::Revert => Err("use revert only for retained revisions".into()),
            ManagedAppOperation::VersionConflict => Err("higher version is not a conflict".into()),
        },
        ManagedAppVersionRelation::Lower => match requested {
            ManagedAppOperation::Downgrade => Ok(ManagedAppOperation::Downgrade),
            ManagedAppOperation::Revert => Ok(ManagedAppOperation::Revert),
            ManagedAppOperation::Install => {
                Err("lower version requires explicit downgrade or revert intent".into())
            }
            ManagedAppOperation::Update => {
                Err("lower version requires explicit downgrade or revert intent".into())
            }
            ManagedAppOperation::Reinstall => Err("lower version is not a reinstall".into()),
            ManagedAppOperation::VersionConflict => Err("lower version is not a conflict".into()),
        },
        ManagedAppVersionRelation::Same => match requested {
            ManagedAppOperation::Reinstall => Ok(ManagedAppOperation::Reinstall),
            ManagedAppOperation::Install => {
                Err("same version requires reinstall or conflict handling".into())
            }
            ManagedAppOperation::Update => {
                Err("same version requires reinstall or conflict handling".into())
            }
            ManagedAppOperation::Downgrade => Err("same version is not a downgrade".into()),
            ManagedAppOperation::Revert => Err("same version is not a revert".into()),
            ManagedAppOperation::VersionConflict => Ok(ManagedAppOperation::VersionConflict),
        },
    }
}

fn grant_scope_label(scope: &GrantScope) -> String {
    match scope {
        GrantScope::ExactCapability {
            provider,
            capability,
        } => format!("{provider}/{capability}"),
        GrantScope::AllProviderCapabilities { provider } => format!("{provider}/*"),
    }
}

fn grant_duration_label(duration: &app_host_kernel::primitives::grant::GrantDuration) -> String {
    match duration {
        app_host_kernel::primitives::grant::GrantDuration::NonExpiring => "non-expiring".into(),
        app_host_kernel::primitives::grant::GrantDuration::ExpiresAfter { seconds } => {
            format!("expires-after:{seconds}s")
        }
    }
}

fn data_scope_label(data_scope: &DataScope) -> String {
    match data_scope {
        DataScope::None => "all data".into(),
        DataScope::AllResources => "all current and future resources".into(),
        DataScope::Resources { resource_ids } => format!(
            "resources: {}",
            resource_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn permission_key(summary: &package::GrantRequestSummary) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        summary.scope_label,
        summary.data_scope_label,
        summary.condition,
        summary.reason,
        summary.duration_label
    )
}

fn publisher_continuity(
    current: Option<&str>,
    target: Option<&str>,
) -> ManagedAppPublisherContinuity {
    match (current, target) {
        (Some(current), Some(target)) if current == target => ManagedAppPublisherContinuity::Same,
        (Some(_), Some(_)) => ManagedAppPublisherContinuity::Changed,
        (None, Some(_)) => ManagedAppPublisherContinuity::New,
        _ => ManagedAppPublisherContinuity::Unknown,
    }
}

fn capability_summaries(document: &package::PackageDocument) -> Vec<package::CapabilitySummary> {
    document
        .manifest
        .capabilities
        .iter()
        .map(|capability| package::CapabilitySummary {
            name: capability.name.to_string(),
            description: capability.description.clone(),
            effect: format!("{:?}", capability.effect).to_lowercase(),
        })
        .collect()
}

fn surface_summaries(document: &package::PackageDocument) -> Vec<package::SurfaceSummary> {
    document
        .manifest
        .surfaces
        .iter()
        .map(|surface| package::SurfaceSummary {
            name: surface.name.to_string(),
            kind: format!("{:?}", surface.kind).to_lowercase(),
            title: surface.title.clone(),
            has_custom_ui: surface.ui.is_some(),
        })
        .collect()
}

fn set_diff_names(
    current: &[package::CapabilitySummary],
    target: &[package::CapabilitySummary],
    added: bool,
) -> Vec<String> {
    let current: BTreeSet<String> = current.iter().map(|item| item.name.clone()).collect();
    let target: BTreeSet<String> = target.iter().map(|item| item.name.clone()).collect();
    if added {
        target.difference(&current).cloned().collect()
    } else {
        current.difference(&target).cloned().collect()
    }
}

fn set_diff_surface_names(
    current: &[package::SurfaceSummary],
    target: &[package::SurfaceSummary],
    added: bool,
) -> Vec<String> {
    let current: BTreeSet<String> = current.iter().map(|item| item.name.clone()).collect();
    let target: BTreeSet<String> = target.iter().map(|item| item.name.clone()).collect();
    if added {
        target.difference(&current).cloned().collect()
    } else {
        current.difference(&target).cloned().collect()
    }
}

fn surface_infos(
    surface_ui: &SurfaceUiRegistry,
    manifest: &app_host_kernel::manifest::AppManifest,
) -> Vec<AppSurfaceInfo> {
    manifest
        .surfaces
        .iter()
        .map(|surface| AppSurfaceInfo {
            name: surface.name.to_string(),
            kind: format!("{:?}", surface.kind).to_lowercase(),
            title: surface.title.clone(),
            has_custom_ui: surface_ui.get(&manifest.app_id, &surface.name).is_some(),
        })
        .collect()
}

/// Dial an MCP backend and complete the readiness handshake (`tools/list`).
/// This is the only place the manager runs package code, and only after the
/// user has confirmed the install.
fn dial_backend(
    payload_dir: &Path,
    data_dir: &Path,
    app_id: &AppId,
    display_name: &str,
    backend: &Backend,
) -> Result<Arc<McpClient>, String> {
    std::fs::create_dir_all(data_dir)
        .map_err(|error| format!("create app data directory failed: {error}"))?;
    let payload = payload_dir
        .to_str()
        .ok_or_else(|| "package payload path is not UTF-8".to_string())?;
    let data = data_dir
        .to_str()
        .ok_or_else(|| "app data path is not UTF-8".to_string())?;
    let environment = [
        ("APP_HOST_PAYLOAD_DIR", payload),
        ("APP_HOST_DATA_DIR", data),
    ];
    let app_container_name = app_container_moniker(app_id.as_str());
    let transport: Box<dyn McpTransport> = match backend {
        Backend::None => return Err("no backend to dial".into()),
        Backend::McpStdio {
            authority_mode,
            command,
            args,
        } => {
            let resolved = resolve_payload_args(payload_dir, args);
            let args: Vec<&str> = resolved.iter().map(String::as_str).collect();
            match authority_mode {
                package::BackendAuthorityMode::Sandboxed => {
                    let resolved_command = resolve_sandboxed_command(command, payload_dir)?;
                    Box::new(
                        StdioTransport::spawn_sandboxed(
                            &app_container_name,
                            &resolved_command,
                            &args,
                            payload_dir,
                            data_dir,
                            &environment,
                        )
                        .map_err(|error| error.to_string())?,
                    )
                }
                package::BackendAuthorityMode::Unsandboxed => Box::new(
                    StdioTransport::spawn_in_isolated(command, &args, data_dir, &environment)
                        .map_err(|error| error.to_string())?,
                ),
            }
        }
        Backend::McpStreamableHttp { url } => {
            Box::new(StreamableHttpTransport::new(url).map_err(|error| error.to_string())?)
        }
        Backend::Executable {
            authority_mode,
            args,
            platforms,
            ..
        } => {
            let triple = current_platform_triple();
            let binary = platforms.get(triple).ok_or_else(|| {
                format!("package ships no executable for this platform ({triple})")
            })?;
            let binary_path = payload_dir.join(binary);
            let path = binary_path.to_string_lossy().to_string();
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            match authority_mode {
                package::BackendAuthorityMode::Sandboxed => Box::new(
                    StdioTransport::spawn_sandboxed(
                        &app_container_name,
                        &path,
                        &args,
                        payload_dir,
                        data_dir,
                        &environment,
                    )
                    .map_err(|error| error.to_string())?,
                ),
                package::BackendAuthorityMode::Unsandboxed => Box::new(
                    StdioTransport::spawn_in_isolated(&path, &args, data_dir, &environment)
                        .map_err(|error| error.to_string())?,
                ),
            }
        }
        Backend::AgentWorker { authority_mode, .. } => {
            let _ = display_name;
            if matches!(authority_mode, package::BackendAuthorityMode::Unsandboxed)
                && !cfg!(debug_assertions)
            {
                return Err("unsandboxed agent-worker backends require --allow-unsafe-native-backends, the user-level KESTRAL_ALLOW_UNSAFE_NATIVE_BACKENDS=true environment variable, or a debug build".into());
            }
            return Err("agent-worker backends use the agent adapter".into());
        }
    };
    let client = Arc::new(McpClient::connect(transport).map_err(|error| error.to_string())?);
    // Readiness probe. Failure here is a failed startup, not a crash.
    client.list_tools().map_err(|error| error.to_string())?;
    Ok(client)
}

fn resolve_payload_args(payload_dir: &Path, args: &[String]) -> Vec<String> {
    args.iter()
        .map(|argument| {
            let candidate = payload_dir.join(argument);
            if !argument.contains("..") && !Path::new(argument).is_absolute() && candidate.is_file()
            {
                candidate.to_string_lossy().to_string()
            } else {
                argument.clone()
            }
        })
        .collect()
}

fn resolve_sandboxed_command(command: &str, payload_dir: &Path) -> Result<String, String> {
    let candidate = Path::new(command);
    if candidate.is_absolute() {
        if candidate.is_file() {
            return Ok(candidate.to_string_lossy().to_string());
        }
        return Err(format!("sandboxed command not found: {command}"));
    }

    let probe_names = vec![command.to_string()];
    #[cfg(windows)]
    let probe_names = {
        let mut probe_names = probe_names;
        if Path::new(command).extension().is_none() {
            probe_names.extend(
                [".exe", ".cmd", ".bat", ".com"]
                    .into_iter()
                    .map(|ext| format!("{command}{ext}")),
            );
        }
        probe_names
    };

    for name in &probe_names {
        let payload_candidate = payload_dir.join(name);
        if payload_candidate.is_file() {
            return Ok(payload_candidate.to_string_lossy().to_string());
        }
    }

    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        for name in &probe_names {
            let resolved = dir.join(name);
            if resolved.is_file() {
                return Ok(resolved.to_string_lossy().to_string());
            }
        }
    }

    Err(format!(
        "sandboxed command not found on PATH or under payload: {command}"
    ))
}

/// Bind each declared capability to a handler that forwards to the backend by
/// capability name. Authored packages declare their capabilities statically,
/// so this needs no live discovery.
fn handlers_for_capabilities(
    names: &[CapabilityName],
    artifact_type: Option<ArtifactTypeName>,
    call: McpToolCall,
) -> BTreeMap<CapabilityName, CapabilityHandler> {
    names
        .iter()
        .map(|name| {
            let tool = name.to_string();
            let call = call.clone();
            let artifact_type = artifact_type.clone();
            let handler: CapabilityHandler = Box::new(move |input, context| {
                if context.cancellation.is_cancelled() {
                    return Err(HandlerFailure("backend call cancelled".into()));
                }
                let result = call(&tool, input, context).map_err(HandlerFailure)?;
                let artifacts = match &artifact_type {
                    Some(kind) => vec![ArtifactDraft {
                        artifact_type: kind.clone(),
                        title: format!("{tool} result"),
                        content: result.clone(),
                    }],
                    None => Vec::new(),
                };
                Ok(CapabilityOutcome { result, artifacts })
            });
            (name.clone(), handler)
        })
        .collect()
}

fn current_platform_triple() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x86_64",
        ("windows", "aarch64") => "windows-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        ("macos", "aarch64") => "macos-aarch64",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests;
