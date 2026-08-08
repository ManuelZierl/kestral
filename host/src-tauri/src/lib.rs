//! Agentic App Host — Tauri shell.
//!
//! The desktop shell contributes the two user-facing boundaries the kernel
//! cannot provide for itself:
//!
//! - **Trusted chrome**: approval prompts and notices rendered in
//!   shell-owned UI that apps cannot draw over ([`chrome::ShellChrome`]).
//! - **A window**: views over the public kernel API (apps, runs, artifacts)
//!   and surface forms that emit `ActionIntent`s.
//!
//! The host runtime also owns adapters, persistence, package and process
//! lifecycle, profiles, and transports outside the protocol-agnostic kernel.
//! Apps still enter kernel authority only through its public action path.

#[cfg(all(feature = "dev-mcp", not(debug_assertions)))]
compile_error!("the dev-mcp feature must not be enabled in production builds");

mod agent_worker;
mod agent_worker_protocol;
mod app_data;
pub mod app_manager;
mod artifacts_app;
mod atomic_json;
pub mod chat_app;
pub mod chat_model_profiles;
mod chat_runtime;
pub mod chat_store;
mod chrome;
pub mod config;
mod file_resources;
mod git_source;
pub(crate) mod host_paths;
mod kernel_state;
mod llm_client;
pub mod llm_provider;
mod managed_data;
pub mod mcp;
pub mod mcp_export;
pub mod mcp_gateway;
mod node_worker;
mod permissions_app;
mod profile_migration;
// Generic capability-provider fixture for in-crate and integration tests.
pub mod package;
pub(crate) mod profiles;
pub mod publisher_trust;
pub mod remote_api;
mod remote_auth;
mod surface_state;
pub mod surface_ui;
mod system_reset;
#[cfg(any(test, feature = "test-fixtures"))]
pub mod test_app;
mod tool_mapping;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chat_store::{ChatStore, ChatThread, ChatThreadSummary};
use config::{
    ConnectionTestResult, ConnectorConfigView, ConnectorProbe, HostConfig, HostConfigService,
    McpExportProfileView, McpServerConfigView, ModelListResult,
};
use file_resources::{
    file_broker_handlers, file_broker_manifest, file_resource_grant_request,
    file_resource_registry_path, FileResourceGrantOperation, FileResourceRegistryService,
    FileResourceView, TrustedFileResourceView,
};
use mcp::{McpConnections, McpServerStatusView};
use mcp_gateway::{AuditLog, BearerProfileAuth, GatewayContext, RunningGateway};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::ipc::{Channel, CommandArg, CommandItem, InvokeError};
use tauri::Runtime;
use tauri::{Emitter, Manager, State};

use app_host_kernel::ids::{
    AppId, ArtifactId, ExtensionPointName, GrantId, ResourceId, RunId, SecretName, SecretRef,
    SurfaceName,
};
use app_host_kernel::invocation::CapabilityHandler;
use app_host_kernel::kernel::{
    CapabilityUseView, Kernel, PrepareInvocation, PreparedGrant, PreparedInstall,
    SurfaceActionOutcome,
};
use app_host_kernel::manifest::GrantRequest;
use app_host_kernel::primitives::artifact::Artifact;
use app_host_kernel::primitives::grant::{
    DataScope, Grant, GrantCondition, GrantDuration, GrantOrigin, GrantScope, GrantStatus,
    GrantStatusView,
};
use app_host_kernel::primitives::surface::ActionIntent;
use app_host_kernel::schema::{validate_against_schema, SchemaViolation};
use app_host_kernel::services::broker::{GrantCheck, IssueResult};
use app_host_kernel::services::ledger::{LedgerEvent, LedgerRecord};
use app_host_kernel::services::registry::InstalledApp;
use app_host_kernel::services::router::{AppDataChangeKind, AppEventEnvelope};
use app_host_kernel::services::surfaces::SurfaceBinding;
use app_host_kernel::JsonObject;

use host_paths::HostPaths;
use profiles::{ProfileIdentity, ProfileRecord, ProfileRegistryService, ProfileView};
use surface_ui::{SurfaceUiBundle, SurfaceUiRegistry};

use app_manager::{
    AppManager, AppStatusView, ManagedAppOperation, ManagedAppTransitionPlan,
    ManagedAppTransitionRequest, UpdateJournal, UpdatePhase,
};
use package::PackageInspection;
use publisher_trust::{RevokeKeyRequest, TrustKeyRequest, TrustRecord};

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct InstalledAppView {
    #[serde(flatten)]
    app: InstalledApp,
    icon: Option<package::AppIconView>,
    theme_colors: Vec<package::AppThemeColor>,
}

use chat_app::ChatPromptPreview;
use chrome::{
    OAuthPublicEvent, PendingApprovals, PendingOAuthSessions, ShellChrome, TrustedNoticeRecord,
    TrustedNoticeStore, CHROME_OAUTH_EVENT,
};

pub(crate) struct Host {
    // Lock ordering matters: the llm-provider handler may acquire the config
    // lock while the kernel lock is already held, so command paths must never
    // block on the kernel while holding the config lock.
    kernel: Arc<Mutex<Kernel>>,
    kernel_invoker: agent_worker::KernelInvokerClient,
    config: Arc<Mutex<HostConfigService>>,
    chat_store: Arc<Mutex<ChatStore>>,
    active_chat_sends: Arc<Mutex<std::collections::HashMap<String, ActiveChatSend>>>,
    pending: Arc<PendingApprovals>,
    oauth: Arc<PendingOAuthSessions>,
    notices: Arc<Mutex<TrustedNoticeStore>>,
    profiles: Arc<Mutex<ProfileRegistryService>>,
    startup_apps_installed: Mutex<bool>,
    mcp_export_transition: tauri::async_runtime::Mutex<()>,
    mcp_connections: Arc<McpConnections>,
    mcp_gateway: Mutex<Option<RunningGateway>>,
    /// Shared MCP-gateway audit log. Owned by the Host (not the transient
    /// running gateway) so its in-memory recent-activity window survives
    /// gateway stop/start and can be read by the Settings UI.
    mcp_audit: Arc<AuditLog>,
    file_resources: Arc<Mutex<FileResourceRegistryService>>,
    file_resource_transition: tauri::async_runtime::Mutex<()>,
    /// Static custom app-surface UI bundles, served to sandboxed frames. Not
    /// kernel state — pure host presentation over the grant-checked action
    /// path (see `surface_ui`).
    surface_ui: Arc<Mutex<SurfaceUiRegistry>>,
    /// Bounded app-private state for sandboxed surfaces. This is presentation
    /// state, not a capability path; open bindings still scope every access.
    surface_state: Arc<Mutex<surface_state::SurfaceStateStore>>,
    /// Bounded app-owned domain data for backend-free packages. Access is
    /// scoped either by a live surface binding or a generated read capability.
    managed_data: Arc<Mutex<managed_data::ManagedDataStore>>,
    /// Third-party app lifecycle (install/enable/disable/uninstall). Bundled
    /// apps are not managed here.
    app_manager: Arc<Mutex<AppManager>>,
    managed_app_transition: tauri::async_runtime::Mutex<()>,
    paths: HostPaths,
}

struct ActiveChatSend {
    cancelled: Arc<AtomicBool>,
    run_ids: Vec<RunId>,
    request_id: String,
    message: String,
}

struct ActiveChatSendGuard {
    sends: Arc<Mutex<std::collections::HashMap<String, ActiveChatSend>>>,
    thread_id: String,
}

impl Drop for ActiveChatSendGuard {
    fn drop(&mut self) {
        if let Ok(mut sends) = self.sends.lock() {
            sends.remove(&self.thread_id);
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ConfigStorageInfo {
    config_path: String,
    secrets_path: String,
    chat_store_path: String,
    file_resource_registry_path: String,
    profile_registry_path: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct SystemResetRequestResult {
    restart_required: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateKestralProfileRequest {
    display_name: String,
    slug: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct SendChatMessageResult {
    thread: ChatThread,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct AttachChatArtifactResult {
    thread: ChatThread,
    contribution: crate::chat_store::ChatContribution,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatStreamEvent {
    kind: String,
    content: String,
    reasoning: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct GrantView {
    #[serde(flatten)]
    grant: Grant,
    holder_display_name: String,
    status: GrantStatus,
    origin: GrantOrigin,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantEditorRequest {
    holder: AppId,
    scope: GrantScope,
    data_scope: DataScope,
    condition: GrantCondition,
    duration: GrantDuration,
    reason: String,
    allow_all_provider_scope: bool,
    #[serde(default)]
    acknowledge_less_interactive_mcp: bool,
}

impl GrantEditorRequest {
    fn grant_request(&self) -> Result<GrantRequest, String> {
        if matches!(&self.scope, GrantScope::AllProviderCapabilities { .. })
            && !self.allow_all_provider_scope
        {
            return Err("all-provider scope requires the advanced-scope acknowledgement".into());
        }
        if self.scope.provider().as_str().starts_with("mcp-")
            && self.condition != GrantCondition::RequiresApproval
            && !self.acknowledge_less_interactive_mcp
        {
            return Err(
                "notify or silent MCP access requires acknowledging that future tool calls may proceed without approval"
                    .into(),
            );
        }
        Ok(GrantRequest {
            scope: self.scope.clone(),
            data_scope: self.data_scope.clone(),
            condition: self.condition,
            duration: self.duration,
            reason: if self.reason.trim().is_empty() {
                "Added from the permissions page".into()
            } else {
                self.reason.trim().to_string()
            },
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case", deny_unknown_fields)]
enum PermissionProposalSubmission {
    Issued {
        grant_id: GrantId,
        effective_condition: GrantCondition,
    },
    AlreadyActive {
        grant_id: GrantId,
        effective_condition: GrantCondition,
    },
    Refused,
}

enum HostState<'a> {
    Tauri(State<'a, Arc<Host>>),
    Direct(&'a Arc<Host>),
}

impl<'a> HostState<'a> {
    fn direct(host: &'a Arc<Host>) -> Self {
        Self::Direct(host)
    }

    fn inner(&self) -> &'a Arc<Host> {
        match self {
            Self::Tauri(state) => state.inner(),
            Self::Direct(host) => host,
        }
    }
}

impl std::ops::Deref for HostState<'_> {
    type Target = Arc<Host>;

    fn deref(&self) -> &Self::Target {
        self.inner()
    }
}

impl<'a, 'de: 'a, R: Runtime> CommandArg<'de, R> for HostState<'a> {
    fn from_command(command: CommandItem<'de, R>) -> Result<Self, InvokeError> {
        State::<Arc<Host>>::from_command(command).map(Self::Tauri)
    }
}

/// Wall-clock timestamp for install records. The kernel's own clock is
/// internal; host-side records use system time.
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Kernel access for synchronous commands, which Tauri dispatches on the
/// webview's main thread. Must never wait: the kernel mutex is held for the
/// whole duration of a trusted-chrome prompt, and `resolve_approval` — the
/// only way the user's answer reaches the kernel — runs on this same thread.
/// A waiting lock here freezes the window and deadlocks the approval until
/// its timeout. Failing fast lets the frontend simply poll again.
fn with_kernel_now<T>(
    host: &Arc<Host>,
    operation: impl FnOnce(&mut Kernel) -> Result<T, String>,
) -> Result<T, String> {
    // This must stay a non-blocking acquire: some handler paths run under the
    // kernel lock and may reach back into host config, so waiting here while a
    // config lock is held would turn lock inversion into a deadlock.
    let mut kernel = match host.kernel.try_lock() {
        Ok(kernel) => kernel,
        Err(std::sync::TryLockError::WouldBlock) => {
            return Err("kernel busy: another host operation owns the kernel".into());
        }
        Err(std::sync::TryLockError::Poisoned(_)) => {
            return Err("kernel lock poisoned".into());
        }
    };
    operation(&mut kernel)
}

/// Kernel access for short state transitions. Operations that wait for trusted
/// chrome must use the prepare/approve/commit APIs and call this helper only
/// before and after the wait.
async fn with_kernel_blocking<T: Send + 'static>(
    host: Arc<Host>,
    operation: impl FnOnce(&mut Kernel) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut kernel = host.kernel.lock().map_err(|_| "kernel lock poisoned")?;
        operation(&mut kernel)
    })
    .await
    .map_err(|error| format!("kernel task failed: {error}"))?
}

async fn install_kernel_app_phased(
    host: Arc<Host>,
    manifest: app_host_kernel::manifest::SealedManifest,
    handlers: std::collections::BTreeMap<app_host_kernel::ids::CapabilityName, CapabilityHandler>,
    origin: GrantOrigin,
) -> Result<(), String> {
    let prepared = with_kernel_blocking(host.clone(), move |kernel| {
        let expected_manifest = manifest.clone();
        if let Ok(installed) = kernel.installed_app(&expected_manifest.manifest.app_id) {
            if installed.manifest != expected_manifest.manifest
                || installed.content_hash != expected_manifest.content_hash
            {
                kernel
                    .upgrade_app(expected_manifest, handlers)
                    .map_err(|error| error.to_string())?;
                return Ok(None);
            }
        }
        match kernel.prepare_install_with_grant_origin(manifest, handlers, origin) {
            Ok(prepared) => Ok(Some(prepared)),
            Err(app_host_kernel::KernelError::AppAlreadyInstalled(app_id)) => {
                let installed = kernel
                    .installed_app(&app_id)
                    .map_err(|error| error.to_string())?;
                if installed.manifest == expected_manifest.manifest
                    && installed.content_hash == expected_manifest.content_hash
                {
                    Ok(None)
                } else {
                    Err(format!(
                        "app '{app_id}' is already installed with different content"
                    ))
                }
            }
            Err(error) => Err(error.to_string()),
        }
    })
    .await?;
    let Some(prepared): Option<PreparedInstall> = prepared else {
        return Ok(());
    };
    let approval = tauri::async_runtime::spawn_blocking(move || prepared.await_approval())
        .await
        .map_err(|error| format!("install approval task failed: {error}"))?;
    with_kernel_blocking(host, move |kernel| {
        kernel
            .commit_install(approval)
            .map(|_| ())
            .map_err(|error| error.to_string())
    })
    .await
}

async fn install_bundled_apps_phased(
    host: Arc<Host>,
    config: Arc<Mutex<HostConfigService>>,
    file_resources: Arc<Mutex<FileResourceRegistryService>>,
) -> Result<(), String> {
    let llm_manifest = app_host_kernel::manifest::seal(llm_provider::llm_provider_manifest());
    install_kernel_app_phased(
        host.clone(),
        llm_manifest,
        llm_provider::llm_provider_handlers(config),
        GrantOrigin::SystemBundled,
    )
    .await?;

    let artifacts_manifest = app_host_kernel::manifest::seal(artifacts_app::artifacts_manifest());
    install_kernel_app_phased(
        host.clone(),
        artifacts_manifest,
        artifacts_app::artifacts_handlers(),
        GrantOrigin::SystemBundled,
    )
    .await?;

    let file_broker = file_broker_manifest();
    let file_broker_sealed = app_host_kernel::manifest::seal(file_broker);

    let permissions_manifest =
        app_host_kernel::manifest::seal(permissions_app::permissions_manifest());
    install_kernel_app_phased(
        host.clone(),
        permissions_manifest,
        permissions_app::permissions_handlers(),
        GrantOrigin::SystemBundled,
    )
    .await?;

    install_kernel_app_phased(
        host.clone(),
        file_broker_sealed,
        file_broker_handlers(file_resources.clone()),
        GrantOrigin::SystemBundled,
    )
    .await?;

    let chat_manifest = with_kernel_blocking(host.clone(), |kernel| {
        Ok(chat_app::chat_manifest_for_kernel(kernel))
    })
    .await?;
    let chat_handlers = chat_app::chat_handlers(host.chat_store.clone());
    install_kernel_app_phased(
        host,
        chat_manifest,
        chat_handlers,
        GrantOrigin::SystemBundled,
    )
    .await
}

async fn activate_managed_app(
    host: Arc<Host>,
    app_id: String,
    prepared: app_manager::PreparedActivation,
) -> Result<(), String> {
    let result = activate_managed_app_checked(host.clone(), app_id.clone(), prepared).await;
    if let Err(reason) = result {
        host.app_manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .record_failure(&app_id, reason);
    }
    Ok(())
}

async fn activate_managed_app_checked(
    host: Arc<Host>,
    app_id: String,
    prepared: app_manager::PreparedActivation,
) -> Result<(), String> {
    let prepared_client = prepared.client();
    let manager = host.app_manager.clone();
    let prepare_app_id = app_id.clone();
    let activation = with_kernel_blocking(host.clone(), move |kernel| {
        let manager = manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?;
        manager
            .prepare_kernel_activation(kernel, &prepare_app_id, prepared)
            .map_err(|error| error.reason)
    })
    .await;
    let activation = match activation {
        Ok(activation) => activation,
        Err(error) => {
            if let Some(client) = prepared_client {
                client.shutdown();
            }
            return Err(error);
        }
    };

    let approval =
        tauri::async_runtime::spawn_blocking(move || activation.install.await_approval())
            .await
            .map_err(|error| format!("app approval task failed: {error}"))?;
    let continuation = {
        let manager = host.app_manager.clone();
        let activation = activation.continuation;
        let app_id = app_id.clone();
        with_kernel_blocking(host.clone(), move |kernel| {
            let mut manager = manager
                .lock()
                .map_err(|_| "app manager lock poisoned".to_string())?;
            manager
                .commit_kernel_activation(kernel, &app_id, approval, activation)
                .map_err(|error| error.reason)
        })
        .await
    };
    let continuation = match continuation {
        Ok(continuation) => continuation,
        Err(error) => {
            if let Some(client) = prepared_client.clone() {
                client.shutdown();
            }
            return Err(error);
        }
    };

    let activation_client = continuation.client.clone();
    let prepared_grants = {
        let manager = host.app_manager.clone();
        let requests = continuation.consumer_grant_requests.clone();
        with_kernel_blocking(host.clone(), move |kernel| {
            manager
                .lock()
                .map_err(|_| "app manager lock poisoned".to_string())?
                .prepare_consumer_grants(kernel, requests)
        })
        .await
    };
    let grant_plans = match prepared_grants {
        Ok(grant_plans) => grant_plans,
        Err(error) => {
            let rollback = with_kernel_blocking(host.clone(), {
                let app_id = continuation.app_id.clone();
                move |kernel| {
                    if kernel.installed_app(&app_id).is_ok() {
                        kernel
                            .uninstall(&app_id)
                            .map_err(|value| value.to_string())?;
                    }
                    Ok(())
                }
            })
            .await;
            if let Some(client) = activation_client {
                client.shutdown();
            }
            let reason = match rollback {
                Ok(()) => error,
                Err(rollback) => format!("{error}; activation rollback failed: {rollback}"),
            };
            return Err(reason);
        }
    };
    let grant_approvals = tauri::async_runtime::spawn_blocking(move || {
        grant_plans
            .into_iter()
            .map(|grant| grant.await_approval())
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|error| format!("consumer grant approval task failed: {error}"))?;

    let surface_ui = host.surface_ui.clone();
    let manager = host.app_manager.clone();
    let finish_app_id = app_id.clone();
    let result = with_kernel_blocking(host.clone(), move |kernel| {
        let mut manager = manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?;
        let mut surface_ui = surface_ui
            .lock()
            .map_err(|_| "surface UI registry lock poisoned".to_string())?;
        manager
            .finish_kernel_activation(
                kernel,
                &mut surface_ui,
                &finish_app_id,
                continuation,
                grant_approvals,
            )
            .map_err(|error| error.reason)
    })
    .await;
    if let Err(error) = result {
        if let Some(client) = activation_client {
            client.shutdown();
        }
        return Err(error);
    }
    Ok(())
}

async fn prepare_journaled_activation(
    host: &Arc<Host>,
    journal: &UpdateJournal,
    revision_id: &str,
) -> Result<app_manager::PreparedActivation, String> {
    let preparation = host
        .app_manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?
        .transition_activation_preparation(journal, revision_id, host.kernel_invoker.clone())?;
    tauri::async_runtime::spawn_blocking(move || preparation.prepare())
        .await
        .map_err(|error| format!("app activation preparation failed: {error}"))?
}

async fn rollback_journaled_transition(
    host: &Arc<Host>,
    journal: &mut UpdateJournal,
    activation_error: String,
) -> Result<(), String> {
    let previous_revision_id = if journal.phase == UpdatePhase::DataRollbackCommitted {
        journal
            .current_revision_id
            .clone()
            .ok_or_else(|| "rollback journal is missing the previous revision".to_string())?
    } else {
        host.app_manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .begin_journaled_rollback(journal)?
    };
    if !journal.enabled {
        host.app_manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .finish_journaled_rollback(journal, &previous_revision_id)?;
        return Err(format!(
            "update preparation failed and was rolled back to the previous revision: {activation_error}"
        ));
    }
    let rollback_result =
        match prepare_journaled_activation(host, journal, &previous_revision_id).await {
            Ok(prepared) => {
                activate_managed_app_checked(host.clone(), journal.app_id.clone(), prepared).await
            }
            Err(error) => Err(error),
        };
    match rollback_result {
        Ok(()) => {
            host.app_manager
                .lock()
                .map_err(|_| "app manager lock poisoned".to_string())?
                .finish_journaled_rollback(journal, &previous_revision_id)?;
            Err(format!(
                "update activation failed and was rolled back to the previous revision: {activation_error}"
            ))
        }
        Err(rollback_error) => Err(format!(
            "update activation failed ({activation_error}); rollback also failed ({rollback_error})"
        )),
    }
}

async fn continue_journaled_transition(
    host: &Arc<Host>,
    mut journal: UpdateJournal,
    deactivate: bool,
) -> Result<(), String> {
    if deactivate && journal.phase == UpdatePhase::Prepared {
        let manager = host.app_manager.clone();
        let surface_ui = host.surface_ui.clone();
        let (client, next_journal) = with_kernel_blocking(host.clone(), move |kernel| {
            let mut manager = manager
                .lock()
                .map_err(|_| "app manager lock poisoned".to_string())?;
            let mut surface_ui = surface_ui
                .lock()
                .map_err(|_| "surface UI registry lock poisoned".to_string())?;
            let client =
                manager.deactivate_journaled_transition(kernel, &mut surface_ui, &mut journal)?;
            Ok((client, journal))
        })
        .await?;
        journal = next_journal;
        if let Some(client) = client {
            client.shutdown();
        }
    }

    if journal.phase == UpdatePhase::Deactivated && journal.data_transition.is_some() {
        let preparation = host
            .app_manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .data_migration_preparation(&journal)?
            .ok_or_else(|| "app-data migration preparation is missing".to_string())?;
        let migration = tauri::async_runtime::spawn_blocking(move || preparation.execute())
            .await
            .map_err(|error| format!("app-data migration task failed: {error}"))?;
        let (source_digest, candidate_digest) = match migration {
            Ok(digests) => digests,
            Err(error) => return rollback_journaled_transition(host, &mut journal, error).await,
        };
        host.app_manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .mark_data_candidate_validated(&mut journal, source_digest, candidate_digest)?;
    }
    if journal.phase == UpdatePhase::DataCandidateValidated {
        host.app_manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .commit_data_candidate(&mut journal)?;
    }

    let backup_retention = host
        .config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .get_host_config()
        .host
        .app_data_backup_retention;
    if !journal.enabled {
        return host
            .app_manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .commit_journaled_transition(&mut journal, backup_retention);
    }

    let target_revision_id = journal.target_revision.revision_id.clone();
    let target_result =
        match prepare_journaled_activation(host, &journal, &target_revision_id).await {
            Ok(prepared) => {
                activate_managed_app_checked(host.clone(), journal.app_id.clone(), prepared).await
            }
            Err(error) => Err(error),
        };
    match target_result {
        Ok(()) => host
            .app_manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .commit_journaled_transition(&mut journal, backup_retention),
        Err(error) => rollback_journaled_transition(host, &mut journal, error).await,
    }
}

async fn recover_managed_app_transition_phased(host: &Arc<Host>) -> Result<(), String> {
    let Some(mut journal) = host
        .app_manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?
        .pending_update_journal()
    else {
        return Ok(());
    };
    match journal.phase {
        UpdatePhase::Prepared => continue_journaled_transition(host, journal, true).await,
        UpdatePhase::Deactivated => continue_journaled_transition(host, journal, false).await,
        UpdatePhase::DataCandidateValidated | UpdatePhase::DataCommitted => {
            continue_journaled_transition(host, journal, false).await
        }
        UpdatePhase::Activated => {
            let backup_retention = host
                .config
                .lock()
                .map_err(|_| "config lock poisoned".to_string())?
                .get_host_config()
                .host
                .app_data_backup_retention;
            host.app_manager
                .lock()
                .map_err(|_| "app manager lock poisoned".to_string())?
                .commit_journaled_transition(&mut journal, backup_retention)
        }
        UpdatePhase::RollingBack | UpdatePhase::DataRollbackCommitted => {
            rollback_journaled_transition(
                host,
                &mut journal,
                "interrupted target activation".into(),
            )
            .await
        }
        UpdatePhase::RolledBack | UpdatePhase::Committed => host
            .app_manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .clear_completed_journal(&journal),
    }
}

async fn issue_grant_phased(
    host: Arc<Host>,
    holder: AppId,
    request: GrantRequest,
) -> Result<(), String> {
    issue_grant_phased_result(host, holder, request)
        .await
        .map(|_| ())
}

async fn issue_grant_phased_result(
    host: Arc<Host>,
    holder: AppId,
    request: GrantRequest,
) -> Result<IssueResult, String> {
    let prepared = with_kernel_blocking(host.clone(), move |kernel| {
        kernel
            .prepare_grant(&holder, request)
            .map_err(|error| error.to_string())
    })
    .await?;
    let approval = tauri::async_runtime::spawn_blocking(move || prepared.await_approval())
        .await
        .map_err(|error| format!("grant approval task failed: {error}"))?;
    with_kernel_blocking(host, move |kernel| {
        kernel
            .commit_grant(approval)
            .map_err(|error| error.to_string())
    })
    .await
}

struct StartupClaim<'a> {
    installed: &'a Mutex<bool>,
    completed: bool,
}

impl<'a> StartupClaim<'a> {
    fn acquire(installed: &'a Mutex<bool>) -> Result<Option<Self>, String> {
        let mut claimed = installed
            .lock()
            .map_err(|_| "bootstrap lock poisoned".to_string())?;
        if *claimed {
            return Ok(None);
        }
        *claimed = true;
        Ok(Some(Self {
            installed,
            completed: false,
        }))
    }

    fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for StartupClaim<'_> {
    fn drop(&mut self) {
        if !self.completed {
            if let Ok(mut installed) = self.installed.lock() {
                *installed = false;
            }
        }
    }
}

/// Install the bundled startup apps. Called by the frontend once its trusted
/// chrome listeners are ready, because installation asks the user to confirm
/// each requested grant.
#[tauri::command]
async fn bootstrap_startup_apps(host: HostState<'_>) -> Result<(), String> {
    // Claim before installing so a concurrent call cannot double-install. The
    // guard releases every failure path, including early `?` returns.
    let Some(mut startup_claim) = StartupClaim::acquire(&host.startup_apps_installed)? else {
        return Ok(());
    };
    let _transition_guard = host.managed_app_transition.lock().await;
    if let Err(error) = recover_managed_app_transition_phased(host.inner()).await {
        return Err(format!("managed app transition recovery failed: {error}"));
    }
    let config_state = host.config.clone();
    let manager_state = host.app_manager.clone();
    let surface_ui_state = host.surface_ui.clone();
    let activation_preparations = manager_state
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?
        .enabled_activation_preparations_with_invoker(&host.kernel_invoker);
    let prepared_activations = match tauri::async_runtime::spawn_blocking(move || {
        activation_preparations
            .into_iter()
            .map(|(id, preparation)| (id, preparation.and_then(|value| value.prepare())))
            .collect::<Vec<_>>()
    })
    .await
    {
        Ok(activations) => activations,
        Err(error) => {
            return Err(format!("managed app preparation failed: {error}"));
        }
    };
    let install_result = install_bundled_apps_phased(
        host.inner().clone(),
        config_state.clone(),
        host.inner().file_resources.clone(),
    )
    .await;
    let host_state = host.inner().clone();
    let bootstrap_host = host_state.clone();
    let result = async move {
        // Hydrate secrets first, then surface any bundled-install error.
        with_kernel_blocking(bootstrap_host.clone(), {
            let config_state = config_state.clone();
            move |kernel| {
                config_state
                    .lock()
                    .map_err(|_| "config lock poisoned".to_string())?
                    .bootstrap_secrets(kernel)
            }
        })
        .await?;
        install_result?;

        let (export_profiles, export_transitions) = {
            let config = config_state
                .lock()
                .map_err(|_| "config lock poisoned".to_string())?;
            let host_config = config.get_host_config();
            (host_config.mcp_exports, host_config.mcp_export_transitions)
        };
        let pending_export_profiles = export_profiles.clone();
        let pending_export_transitions = export_transitions.clone();
        let pending_exports = with_kernel_blocking(bootstrap_host.clone(), move |kernel| {
            for profile_id in mcp_export::stale_principal_ids(
                kernel,
                &pending_export_profiles,
                &pending_export_transitions,
            ) {
                mcp_export::uninstall_principal(kernel, &profile_id)?;
            }
            Ok::<_, String>(mcp_export::pending_principal_installs(
                kernel,
                &pending_export_profiles,
                &pending_export_transitions,
            ))
        })
        .await?;
        for (profile_id, profile) in pending_exports {
            let (manifest, handlers) = mcp_export::principal_install_parts(&profile_id, &profile);
            install_kernel_app_phased(
                bootstrap_host.clone(),
                manifest,
                handlers,
                GrantOrigin::McpExport,
            )
            .await?;
        }
        let completed_transitions: Vec<String> = export_transitions.keys().cloned().collect();
        if !completed_transitions.is_empty() {
            let mut config = config_state
                .lock()
                .map_err(|_| "config lock poisoned".to_string())?;
            for profile_id in completed_transitions {
                config.complete_mcp_export_transition(&profile_id)?;
            }
        }

        let cleanup_clients = {
            let manager = manager_state.clone();
            let surface_ui = surface_ui_state.clone();
            with_kernel_blocking(bootstrap_host.clone(), move |kernel| {
                let mut manager = manager
                    .lock()
                    .map_err(|_| "app manager lock poisoned".to_string())?;
                let mut surface_ui = surface_ui
                    .lock()
                    .map_err(|_| "surface UI registry lock poisoned".to_string())?;
                manager.reconcile_disabled(kernel, &mut surface_ui)
            })
            .await?
        };
        for client in cleanup_clients {
            client.shutdown();
        }
        let pending_uninstalls = manager_state
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .pending_uninstall_ids();
        for id in pending_uninstalls {
            let secret_names = manager_state
                .lock()
                .map_err(|_| "app manager lock poisoned".to_string())?
                .pending_uninstall_secret_names(&id);
            if !secret_names.is_empty() {
                let owner = AppId::new(&id);
                with_kernel_blocking(bootstrap_host.clone(), move |kernel| {
                    for name in secret_names {
                        kernel.clear_secret(&SecretRef {
                            owner: owner.clone(),
                            name: SecretName::new(name),
                        });
                    }
                    Ok(())
                })
                .await?;
            }
            let mut config = config_state
                .lock()
                .map_err(|_| "config lock poisoned".to_string())?;
            manager_state
                .lock()
                .map_err(|_| "app manager lock poisoned".to_string())?
                .finish_uninstall(&mut config, &id)?;
        }
        for (id, prepared) in prepared_activations {
            match prepared {
                Ok(prepared) => {
                    activate_managed_app(bootstrap_host.clone(), id.clone(), prepared).await?;
                }
                Err(reason) => {
                    manager_state
                        .lock()
                        .map_err(|_| "app manager lock poisoned".to_string())?
                        .record_failure(&id, reason);
                }
            }
        }
        Ok::<(), String>(())
    }
    .await;
    if result.is_ok()
        && host_state
            .config
            .lock()
            .map(|config| config.mcp_gateway_settings().enabled)
            .unwrap_or(false)
    {
        if let Err(error) = start_gateway_for_host(&host_state) {
            eprintln!("MCP gateway failed to start at bootstrap: {error}");
        }
    }
    if result.is_ok() {
        startup_claim.complete();
    }
    result.map(|_| ())
}

#[tauri::command]
async fn attach_chat_artifact(
    host: HostState<'_>,
    thread_id: String,
    artifact_id: app_host_kernel::ids::ArtifactId,
    title: String,
) -> Result<AttachChatArtifactResult, String> {
    let (artifact, source_app) = with_kernel_blocking(host.inner().clone(), move |kernel| {
        let artifact = kernel
            .artifacts()
            .find(|artifact| artifact.artifact_id == artifact_id)
            .cloned()
            .ok_or_else(|| format!("unknown artifact: {artifact_id}"))?;
        let source_app = kernel
            .installed_app(&artifact.provenance.produced_by)
            .map(|app| (app.manifest.version.clone(), app.content_hash.clone()))
            .map_err(|error| error.to_string())?;
        Ok((artifact, source_app))
    })
    .await?;
    let content_digest = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&artifact.content)
                .map_err(|error| error.to_string())?
                .as_slice()
        )
    );
    let contribution = crate::chat_store::ChatContribution {
        source_app_id: artifact.provenance.produced_by.to_string(),
        source_app_version: source_app.0,
        source_contract: 1,
        item_id: artifact.artifact_id.to_string(),
        revision: 1,
        digest: content_digest,
        completeness: crate::chat_store::ChatContributionCompleteness::Complete,
        lifecycle: crate::chat_store::ChatContributionLifecycle::Accepted,
        kind: crate::chat_store::ChatContributionKind::ArtifactRef,
        title,
        body: serde_json::json!({"artifact_id": artifact.artifact_id, "provenance": artifact.provenance, "source_app_digest": source_app.1}),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    let thread = {
        let mut store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        store.upsert_contribution(&thread_id, contribution.clone())?
    };
    publish_chat_thread_change(
        host.inner(),
        thread.resource_id.clone(),
        thread.revision,
        AppDataChangeKind::Updated,
    )
    .await;
    Ok(AttachChatArtifactResult {
        thread,
        contribution,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct McpGatewayStatus {
    running: bool,
    local_address: Option<String>,
}

fn gateway_context(host: &Arc<Host>) -> GatewayContext {
    let pending = host.pending.clone();
    GatewayContext {
        kernel: host.kernel.clone(),
        config: host.config.clone(),
        auth: Arc::new(BearerProfileAuth::new(host.config.clone())),
        // Share the Host-owned audit log so recent activity outlives one
        // gateway session and is visible to the Settings UI.
        audit: host.mcp_audit.clone(),
        cancel_pending_approvals: Arc::new(move || {
            pending.deny_app_id_prefix("mcp-export/");
        }),
    }
}

fn start_gateway_for_host(host: &Arc<Host>) -> Result<McpGatewayStatus, String> {
    let settings = host
        .config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .mcp_gateway_settings();
    let mut gateway = host
        .mcp_gateway
        .lock()
        .map_err(|_| "MCP gateway lock poisoned".to_string())?;
    if gateway.is_none() {
        *gateway = Some(mcp_gateway::start_gateway(
            &settings.bind_address,
            gateway_context(host),
        )?);
    }
    Ok(McpGatewayStatus {
        running: true,
        local_address: gateway
            .as_ref()
            .map(|gateway| gateway.local_addr().to_string()),
    })
}

#[tauri::command]
fn list_mcp_export_profiles(host: HostState<'_>) -> Result<Vec<McpExportProfileView>, String> {
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())
        .map(|config| config.list_mcp_export_profiles())
}

#[tauri::command]
async fn upsert_mcp_export_profile(
    host: HostState<'_>,
    profile: McpExportProfileView,
) -> Result<McpExportProfileView, String> {
    // Serialize against enable/disable/delete. Those hold this guard across a
    // trusted-chrome approval — up to five minutes — while acting on a profile
    // snapshot taken before the wait. An edit slipping in during that window
    // let the pending install commit the stale snapshot, leaving the installed
    // principal's grants permanently disagreeing with the saved profile.
    let _transition_guard = host.mcp_export_transition.lock().await;
    let profile_id = profile.id.clone();
    let installed = with_kernel_blocking(host.inner().clone(), move |kernel| {
        Ok(mcp_export::is_principal_installed(kernel, &profile_id))
    })
    .await?;
    if installed {
        return Err("disable an MCP export profile before editing it".into());
    }
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .upsert_mcp_export_profile(profile)
}

#[tauri::command]
async fn set_mcp_export_enabled(
    host: HostState<'_>,
    profile_id: String,
    enabled: bool,
) -> Result<(), String> {
    let _transition_guard = host.mcp_export_transition.lock().await;
    let profile = host
        .config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .mcp_export_profile(&profile_id)
        .ok_or_else(|| format!("unknown MCP export profile: {profile_id}"))?;
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .begin_mcp_export_transition(&profile_id, enabled)?;

    let transition_result = if enabled {
        let installed_profile_id = profile_id.clone();
        let already_installed = with_kernel_blocking(host.inner().clone(), move |kernel| {
            Ok(mcp_export::is_principal_installed(
                kernel,
                &installed_profile_id,
            ))
        })
        .await?;
        if already_installed {
            Ok(())
        } else {
            let (manifest, handlers) = mcp_export::principal_install_parts(&profile_id, &profile);
            install_kernel_app_phased(
                host.inner().clone(),
                manifest,
                handlers,
                GrantOrigin::McpExport,
            )
            .await
        }
    } else {
        let uninstall_id = profile_id.clone();
        with_kernel_blocking(host.inner().clone(), move |kernel| {
            if mcp_export::is_principal_installed(kernel, &uninstall_id) {
                mcp_export::uninstall_principal(kernel, &uninstall_id)?;
            }
            Ok(())
        })
        .await
    };
    transition_result?;
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .complete_mcp_export_transition(&profile_id)
}

#[tauri::command]
async fn delete_mcp_export_profile(host: HostState<'_>, profile_id: String) -> Result<(), String> {
    let _transition_guard = host.mcp_export_transition.lock().await;
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .begin_mcp_export_transition(&profile_id, false)?;
    let uninstall_id = profile_id.clone();
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        if mcp_export::is_principal_installed(kernel, &uninstall_id) {
            mcp_export::uninstall_principal(kernel, &uninstall_id)?;
        }
        Ok(())
    })
    .await?;
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .delete_mcp_export_profile(&profile_id)
}

#[tauri::command]
fn rotate_mcp_export_token(host: HostState<'_>, profile_id: String) -> Result<String, String> {
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .rotate_mcp_export_token(&profile_id)
}

#[tauri::command]
fn has_mcp_export_token(host: HostState<'_>, profile_id: String) -> Result<bool, String> {
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())
        .map(|config| config.has_mcp_export_token(&profile_id))
}

#[tauri::command]
fn revoke_mcp_export_token(host: HostState<'_>, profile_id: String) -> Result<(), String> {
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .revoke_mcp_export_token(&profile_id)
}

#[tauri::command]
fn start_mcp_gateway(host: HostState<'_>) -> Result<McpGatewayStatus, String> {
    start_gateway_for_host(host.inner())
}

#[tauri::command]
fn stop_mcp_gateway(host: HostState<'_>) -> Result<(), String> {
    if let Some(gateway) = host
        .mcp_gateway
        .lock()
        .map_err(|_| "MCP gateway lock poisoned".to_string())?
        .take()
    {
        gateway.stop();
    }
    Ok(())
}

#[tauri::command]
fn mcp_gateway_status(host: HostState<'_>) -> Result<McpGatewayStatus, String> {
    let gateway = host
        .mcp_gateway
        .lock()
        .map_err(|_| "MCP gateway lock poisoned".to_string())?;
    Ok(McpGatewayStatus {
        running: gateway.is_some(),
        local_address: gateway
            .as_ref()
            .map(|gateway| gateway.local_addr().to_string()),
    })
}

/// Recent MCP-gateway audit events (newest last): remote calls and their
/// outcomes, auth failures, session and origin rejections. Session-scoped and
/// in-memory — enough to answer "what did a remote client just do?".
#[tauri::command]
fn mcp_export_recent_activity(host: HostState<'_>) -> Vec<serde_json::Value> {
    host.mcp_audit.recent()
}

#[tauri::command]
fn get_host_config(host: HostState<'_>) -> Result<HostConfig, String> {
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())
        .map(|config| config.get_host_config())
}

#[tauri::command]
fn get_config_storage_info(host: HostState<'_>) -> Result<ConfigStorageInfo, String> {
    Ok(ConfigStorageInfo {
        config_path: host.paths.config_path().display().to_string(),
        secrets_path: host.paths.secrets_index_path().display().to_string(),
        chat_store_path: host.paths.chat_store_path().display().to_string(),
        file_resource_registry_path: host
            .paths
            .file_resource_registry_path()
            .display()
            .to_string(),
        profile_registry_path: host.paths.profile_registry_path().display().to_string(),
    })
}

#[tauri::command]
fn request_system_reset<R: Runtime>(
    host: HostState<'_>,
    app: tauri::AppHandle<R>,
    confirmation: String,
) -> Result<SystemResetRequestResult, String> {
    system_reset::stage(&host.paths, &confirmation)?;
    std::thread::spawn(move || {
        // Let the successful IPC response reach the webview before shutdown.
        std::thread::sleep(std::time::Duration::from_millis(250));
        app.request_restart();
    });
    Ok(SystemResetRequestResult {
        restart_required: false,
    })
}

#[tauri::command]
fn get_active_kestral_profile(host: HostState<'_>) -> Result<ProfileView, String> {
    let ProfileIdentity {
        profile_id,
        display_name,
        slug,
        root,
        created_at,
        source,
    } = host.paths.profile_identity().clone();
    let launch = host.paths.launch_instructions();
    let selected_for_next_launch = host
        .profiles
        .lock()
        .map_err(|_| "profile registry lock poisoned".to_string())?
        .selected_next_launch_profile_id()
        == profile_id;
    Ok(ProfileView {
        profile: ProfileRecord {
            profile_id,
            display_name,
            slug,
            root,
            created_at,
        },
        current_runtime: true,
        selected_for_next_launch,
        source,
        launch_args: launch.launch_args,
        restart_instructions: launch.restart_instructions,
    })
}

#[tauri::command]
fn list_kestral_profiles(host: HostState<'_>) -> Result<Vec<ProfileView>, String> {
    host.profiles
        .lock()
        .map_err(|_| "profile registry lock poisoned".to_string())?
        .list_profiles(host.paths.profile_id())
}

#[tauri::command]
fn create_kestral_profile(
    host: HostState<'_>,
    request: CreateKestralProfileRequest,
) -> Result<ProfileView, String> {
    host.profiles
        .lock()
        .map_err(|_| "profile registry lock poisoned".to_string())?
        .create_clean_profile(request.display_name, request.slug)
}

#[tauri::command]
fn delete_kestral_profile(host: HostState<'_>, profile_id: String) -> Result<(), String> {
    host.profiles
        .lock()
        .map_err(|_| "profile registry lock poisoned".to_string())?
        .delete_profile(&profile_id, host.paths.profile_id())
}

#[tauri::command]
async fn update_host_config(
    host: HostState<'_>,
    patch: app_host_kernel::JsonObject,
) -> Result<HostConfig, String> {
    let has_active_sends = !host
        .active_chat_sends
        .lock()
        .map_err(|_| "chat execution lock poisoned".to_string())?
        .is_empty();
    if has_active_sends {
        return Err(
            "cancel or finish running Chat messages before changing provider settings".into(),
        );
    }
    let config = host.config.clone();
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        let mut config = config
            .lock()
            .map_err(|_| "config lock poisoned".to_string())?;
        let updated = config.update_host_config(patch)?;
        config.refresh_active_llm_secret(kernel);
        Ok(updated)
    })
    .await
}

#[tauri::command]
fn get_app_config(
    host: HostState<'_>,
    app_id: AppId,
) -> Result<app_host_kernel::JsonObject, String> {
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())
        .map(|config| config.get_app_config(app_id.as_str()))
}

#[tauri::command]
async fn update_app_config(
    host: HostState<'_>,
    app_id: AppId,
    config: app_host_kernel::JsonObject,
) -> Result<app_host_kernel::JsonObject, String> {
    let _transition_guard = host.managed_app_transition.lock().await;
    let app_id_for_lookup = app_id.clone();
    let manifest = with_kernel_blocking(host.inner().clone(), move |kernel| {
        kernel
            .installed_apps()
            .find(|app| app.manifest.app_id == app_id_for_lookup)
            .map(|app| app.manifest.clone())
            .ok_or_else(|| format!("unknown app: {app_id_for_lookup}"))
    })
    .await?;
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .update_app_config(app_id.as_str(), &manifest, config)
}

async fn require_secret_owner(host: &Arc<Host>, owner: &AppId) -> Result<(), String> {
    let owner_for_kernel = owner.clone();
    let installed = with_kernel_blocking(host.clone(), move |kernel| {
        Ok(kernel.installed_app(&owner_for_kernel).is_ok())
    })
    .await?;
    if installed {
        return Ok(());
    }
    let managed = host
        .app_manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?
        .records()
        .any(|record| record.id == owner.as_str() && !record.uninstalling);
    if managed {
        Ok(())
    } else {
        Err(format!("unknown app: {owner}"))
    }
}

#[tauri::command]
async fn get_chat_prompt_preview(
    host: HostState<'_>,
    candidate_config: Option<app_host_kernel::JsonObject>,
    thread_id: Option<String>,
) -> Result<ChatPromptPreview, String> {
    let model_profile_receipt = thread_id
        .as_deref()
        .map(|thread_id| {
            host.chat_store
                .lock()
                .map_err(|_| "chat store lock poisoned".to_string())?
                .get_thread(thread_id)
                .map(|thread| thread.model_profile_receipt)
        })
        .transpose()?
        .flatten();
    let source_app_version = if let Some(receipt) = &model_profile_receipt {
        let source_app_id = receipt.source_app_id.clone();
        with_kernel_blocking(host.inner().clone(), move |kernel| {
            Ok(
                crate::chat_model_profiles::model_profile_source(kernel, &source_app_id)
                    .ok()
                    .map(|app| app.manifest.version.clone()),
            )
        })
        .await?
    } else {
        None
    };
    let (llm_profile, model_id, selected_prompt) = {
        let config = host
            .config
            .lock()
            .map_err(|_| "config lock poisoned".to_string())?;
        let selected = match (&model_profile_receipt, source_app_version.as_deref()) {
            (Some(receipt), Some(version))
                if crate::chat_model_profiles::profile_is_current(
                    &config.get_app_config(&receipt.source_app_id),
                    &receipt.source_app_id,
                    version,
                    receipt,
                )? =>
            {
                config
                    .selectable_chat_llm_profile(&receipt.connector_id)
                    .ok()
                    .map(|profile| (Some(profile), receipt.model.clone(), receipt.prompt.clone()))
            }
            _ => None,
        };
        if let Some(selected) = selected {
            selected
        } else {
            let profile = config.current_llm_profile()?;
            let model = profile
                .as_ref()
                .map(|profile| profile.default_model.clone())
                .unwrap_or_default();
            (profile, model, None)
        }
    };
    let config = if let Some(candidate_config) = candidate_config {
        let manifest = with_kernel_blocking(host.inner().clone(), |kernel| {
            kernel
                .installed_app(&chat_app::chat_app_id())
                .map(|installed| installed.manifest.clone())
                .map_err(|error| error.to_string())
        })
        .await?;
        let parsed = serde_json::from_value::<serde_json::Value>(serde_json::Value::Object(
            candidate_config,
        ))
        .map_err(|error| format!("invalid candidate chat config: {error}"))?;
        let object = parsed
            .as_object()
            .cloned()
            .ok_or_else(|| "candidate config must be object".to_string())?;
        let validated = host
            .config
            .lock()
            .map_err(|_| "config lock poisoned".to_string())?
            .validate_candidate_app_config("chat", &manifest, object)?;
        validated
    } else {
        host.config
            .lock()
            .map_err(|_| "config lock poisoned".to_string())?
            .get_app_config("chat")
    };
    let runtime = chat_app::ChatPromptRuntimeInput {
        host_version: crate::package::HOST_VERSION.into(),
        mode: String::new(),
        model_id,
        connector_kind: llm_profile
            .as_ref()
            .map(|profile| profile.kind.as_str().to_string())
            .unwrap_or_default(),
        connector_id: llm_profile
            .as_ref()
            .map(|profile| profile.connector_id.clone())
            .unwrap_or_default(),
        profile_id: llm_profile
            .as_ref()
            .map(|profile| {
                profile
                    .connector_id
                    .split_once('/')
                    .map(|(_, profile)| profile)
                    .unwrap_or(&profile.connector_id)
                    .to_string()
            })
            .unwrap_or_default(),
    };
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        chat_app::current_prompt_preview_with_model_profile(
            kernel,
            &config,
            &runtime,
            selected_prompt.as_ref(),
        )
    })
    .await
}

#[tauri::command]
async fn list_chat_profiles(
    host: HostState<'_>,
) -> Result<Vec<crate::chat_store::ChatProfileView>, String> {
    with_kernel_blocking(host.inner().clone(), |kernel| {
        Ok(chat_app::list_available_profiles(kernel))
    })
    .await
}

#[tauri::command]
async fn list_chat_model_profiles(
    host: HostState<'_>,
) -> Result<Vec<crate::chat_model_profiles::ChatModelProfileView>, String> {
    let (sources, granted_tools) = with_kernel_blocking(host.inner().clone(), |kernel| {
        let sources = crate::chat_model_profiles::model_profile_sources(kernel)
            .into_iter()
            .map(|app| crate::chat_model_profiles::ModelProfileSource {
                app_id: app.manifest.app_id.to_string(),
                display_name: app.manifest.display_name.clone(),
                version: app.manifest.version.clone(),
            })
            .collect::<Vec<_>>();
        let granted_tools = kernel
            .available_capabilities_for(&chat_app::chat_app_id())
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|view| {
                view.capability.as_str() != "llm.generate"
                    && view.capability.as_str() != chat_app::CHAT_AGENT_ENGINE_CONTRACT
            })
            .map(|view| format!("{}/{}", view.provider_app_id, view.capability))
            .collect::<std::collections::BTreeSet<_>>();
        Ok((sources, granted_tools))
    })
    .await?;
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let (source_configs, configured_connectors, selectable_connectors, current_chat_config) = {
        let config = host
            .config
            .lock()
            .map_err(|_| "config lock poisoned".to_string())?;
        let source_configs = sources
            .iter()
            .map(|source| (source.app_id.clone(), config.get_app_config(&source.app_id)))
            .collect::<std::collections::BTreeMap<_, _>>();
        let configured_connectors = config
            .get_host_config()
            .connectors
            .into_keys()
            .collect::<std::collections::BTreeSet<_>>();
        let selectable_connectors = configured_connectors
            .iter()
            .filter(|connector_id| config.selectable_chat_llm_profile(connector_id).is_ok())
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        (
            source_configs,
            configured_connectors,
            selectable_connectors,
            config.get_app_config("chat"),
        )
    };
    let runtime = chat_app::ChatPromptRuntimeInput {
        host_version: crate::package::HOST_VERSION.into(),
        mode: String::new(),
        model_id: String::new(),
        connector_kind: String::new(),
        connector_id: String::new(),
        profile_id: String::new(),
    };
    let current_prompt_layers = with_kernel_blocking(host.inner().clone(), move |kernel| {
        chat_app::current_prompt_preview(kernel, &current_chat_config, &runtime).map(|preview| {
            preview
                .layers
                .into_iter()
                .filter(|layer| layer.included)
                .map(|layer| layer.id)
                .collect::<std::collections::BTreeSet<_>>()
        })
    })
    .await?;
    let mut views = Vec::new();
    for source in sources {
        let source_config = source_configs
            .get(&source.app_id)
            .expect("source config was collected for every model profile app");
        views.extend(crate::chat_model_profiles::profile_views(
            source_config,
            &source,
            &granted_tools,
            &configured_connectors,
            &selectable_connectors,
            &current_prompt_layers,
        )?);
    }
    Ok(views)
}

#[tauri::command]
async fn list_chat_agent_engines(
    host: HostState<'_>,
) -> Result<Vec<crate::chat_store::ChatAgentEngineView>, String> {
    with_kernel_blocking(host.inner().clone(), |kernel| {
        chat_app::list_chat_agent_engines(kernel)
    })
    .await
}

#[tauri::command]
async fn set_chat_agent_engine(
    host: HostState<'_>,
    thread_id: String,
    app_id: Option<String>,
) -> Result<crate::chat_store::ChatThread, String> {
    let receipt = if let Some(app_id) = app_id.as_deref() {
        let selected_app_id = app_id.to_string();
        Some(
            with_kernel_blocking(host.inner().clone(), move |kernel| {
                chat_app::resolve_chat_agent_engine_selection(kernel, &selected_app_id)
            })
            .await?,
        )
    } else {
        None
    };
    let updated = {
        let mut store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        if let Some(app_id) = app_id {
            store.set_chat_agent_engine(&thread_id, Some(app_id), receipt, None)?
        } else {
            store.set_chat_agent_engine(&thread_id, None, None, None)?
        }
    };
    let resource_id = updated.resource_id.clone();
    publish_chat_thread_change(
        host.inner(),
        resource_id,
        updated.revision,
        AppDataChangeKind::Updated,
    )
    .await;
    Ok(updated)
}

#[tauri::command]
async fn set_chat_model_profile(
    host: HostState<'_>,
    thread_id: String,
    profile_ref: Option<String>,
) -> Result<crate::chat_store::ChatThread, String> {
    let receipt = if let Some(profile_ref) = profile_ref.as_deref() {
        let (source_app_id, profile_id) = profile_ref
            .split_once('/')
            .ok_or_else(|| format!("invalid model profile reference: {profile_ref}"))?;
        let source_app_id_for_lookup = source_app_id.to_string();
        let source_app_version = with_kernel_blocking(host.inner().clone(), move |kernel| {
            crate::chat_model_profiles::model_profile_source(kernel, &source_app_id_for_lookup)
                .map(|app| app.manifest.version.clone())
        })
        .await?;
        let config = host
            .config
            .lock()
            .map_err(|_| "config lock poisoned".to_string())?;
        let app_config = config.get_app_config(source_app_id);
        let receipt = crate::chat_model_profiles::resolve_profile(
            &app_config,
            profile_id,
            source_app_id,
            &source_app_version,
        )?;
        config.selectable_chat_llm_profile(&receipt.connector_id)?;
        Some(receipt)
    } else {
        None
    };
    let thread = {
        let mut store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        store.set_model_profile(&thread_id, profile_ref, receipt)?
    };
    let resource_id = thread.resource_id.clone();
    publish_chat_thread_change(
        host.inner(),
        resource_id,
        thread.revision,
        AppDataChangeKind::Updated,
    )
    .await;
    Ok(thread)
}

pub(crate) async fn publish_chat_thread_change(
    host: &Arc<Host>,
    resource_id: String,
    revision: u64,
    change_kind: AppDataChangeKind,
) {
    let result = with_kernel_blocking(host.clone(), move |kernel| {
        kernel
            .publish_app_data_change(
                &chat_app::chat_app_id(),
                &resource_id,
                revision,
                change_kind,
            )
            .map_err(|error| error.to_string())
    })
    .await;
    if let Err(error) = result {
        eprintln!("failed to publish chat data change: {error}");
    }
}

#[tauri::command]
async fn set_chat_thread_profile(
    host: HostState<'_>,
    thread_id: String,
    app_id: String,
    profile_name: String,
) -> Result<crate::chat_store::ChatThread, String> {
    let receipt_app_id = app_id.clone();
    let receipt_profile_name = profile_name.clone();
    let receipt = with_kernel_blocking(host.inner().clone(), move |kernel| {
        let (app, profile) =
            chat_app::resolve_live_profile(kernel, &receipt_app_id, &receipt_profile_name)?;
        let reviewed_skill_digests = profile
            .instruction_skill_refs
            .iter()
            .map(|skill| {
                let instructions = app
                    .manifest
                    .skills
                    .iter()
                    .find(|decl| decl.name == *skill)
                    .ok_or_else(|| {
                        format!("unknown assistant profile skill: {receipt_app_id}/{skill}")
                    })?
                    .instructions
                    .clone();
                Ok(chat_app::hash_skill(&instructions))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let digest = chat_app::profile_digest_for(&app.manifest, profile, &reviewed_skill_digests)?;
        Ok(crate::chat_store::ChatProfileReceipt {
            app_id: receipt_app_id.clone(),
            profile_name: profile.profile_name.clone(),
            version: app.manifest.version.clone(),
            digest,
            reviewed_skill_digests,
            capability_refs: profile
                .suggested_capability_refs
                .iter()
                .map(|capability| format!("{}/{}", capability.provider, capability.capability))
                .collect(),
            engine_contract: profile.suggested_agent_engine_contract.clone(),
            status: "available".into(),
        })
    })
    .await?;
    let thread = {
        let mut store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        store.set_assistant_profile(
            &thread_id,
            Some(format!("{app_id}/{profile_name}")),
            Some(receipt),
        )?
    };
    let resource_id = thread.resource_id.clone();
    publish_chat_thread_change(
        host.inner(),
        resource_id,
        thread.revision,
        AppDataChangeKind::Updated,
    )
    .await;
    Ok(thread)
}

#[tauri::command]
async fn remove_chat_contribution(
    host: HostState<'_>,
    thread_id: String,
    source_app_id: String,
    kind: crate::chat_store::ChatContributionKind,
    item_id: String,
) -> Result<crate::chat_store::ChatThread, String> {
    let thread = {
        let mut store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        store.remove_contribution(
            &thread_id,
            &crate::chat_store::ContributionIdentity {
                source_app_id,
                kind,
                item_id,
            },
        )?
    };
    publish_chat_thread_change(
        host.inner(),
        thread.resource_id.clone(),
        thread.revision,
        AppDataChangeKind::Updated,
    )
    .await;
    Ok(thread)
}

#[tauri::command]
fn list_connector_configs(host: HostState<'_>) -> Result<Vec<ConnectorConfigView>, String> {
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())
        .map(|config| config.list_connector_configs())
}

#[tauri::command]
async fn upsert_connector_config(
    host: HostState<'_>,
    connector: ConnectorConfigView,
    acknowledge_data_egress: bool,
) -> Result<ConnectorConfigView, String> {
    let has_active_sends = !host
        .active_chat_sends
        .lock()
        .map_err(|_| "chat execution lock poisoned".to_string())?
        .is_empty();
    if has_active_sends {
        return Err(
            "cancel or finish running Chat messages before changing provider settings".into(),
        );
    }
    let config = host.config.clone();
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        let mut config = config
            .lock()
            .map_err(|_| "config lock poisoned".to_string())?;
        let connector = if acknowledge_data_egress {
            config.upsert_connector_config_with_egress_acknowledgement(connector)?
        } else {
            config.upsert_connector_config(connector)?
        };
        config.refresh_active_llm_secret(kernel);
        Ok(connector)
    })
    .await
}

#[tauri::command]
fn delete_connector_config(host: HostState<'_>, connector_id: String) -> Result<(), String> {
    let active_sends = host
        .active_chat_sends
        .lock()
        .map_err(|_| "chat execution lock poisoned".to_string())?;
    if !active_sends.is_empty() {
        return Err(
            "cancel or finish running Chat messages before changing provider settings".into(),
        );
    }
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .delete_connector_config(&connector_id)
}

#[tauri::command]
async fn put_secret(
    host: HostState<'_>,
    owner: AppId,
    secret_name: String,
    value: String,
) -> Result<(), String> {
    let _transition_guard = host.managed_app_transition.lock().await;
    require_secret_owner(host.inner(), &owner).await?;
    let provider_send_active = owner == AppId::new("llm-provider")
        && !host
            .active_chat_sends
            .lock()
            .map_err(|_| "chat execution lock poisoned".to_string())?
            .is_empty();
    if provider_send_active {
        return Err(
            "cancel or finish running Chat messages before changing provider credentials".into(),
        );
    }
    let config = host.config.clone();
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        config
            .lock()
            .map_err(|_| "config lock poisoned".to_string())?
            .put_secret(kernel, &owner, &secret_name, value)
    })
    .await
}

#[tauri::command]
fn has_secret(host: HostState<'_>, owner: AppId, secret_name: String) -> Result<bool, String> {
    let config = host
        .config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?;
    config.has_secret(&owner, &secret_name)
}

#[tauri::command]
fn list_file_resources(host: HostState<'_>) -> Result<Vec<FileResourceView>, String> {
    host.file_resources
        .lock()
        .map_err(|_| "file resource registry lock poisoned".to_string())
        .map(|registry| registry.list_resources())
}

#[tauri::command]
fn list_trusted_file_resources(
    host: HostState<'_>,
) -> Result<Vec<TrustedFileResourceView>, String> {
    host.file_resources
        .lock()
        .map_err(|_| "file resource registry lock poisoned".to_string())
        .map(|registry| registry.list_trusted_resources())
}

#[tauri::command]
async fn register_file_resource(
    host: HostState<'_>,
    path: String,
) -> Result<TrustedFileResourceView, String> {
    let _transition_guard = host.file_resource_transition.lock().await;
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("file resource path is required".into());
    }
    host.file_resources
        .lock()
        .map_err(|_| "file resource registry lock poisoned".to_string())?
        .register_resource(Path::new(&path))
}

#[tauri::command]
async fn remove_file_resource(host: HostState<'_>, resource_id: ResourceId) -> Result<(), String> {
    let _transition_guard = host.file_resource_transition.lock().await;
    host.file_resources
        .lock()
        .map_err(|_| "file resource registry lock poisoned".to_string())?
        .begin_removal(&resource_id)?;
    let resource_id_for_kernel = resource_id.clone();
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        kernel
            .revoke_grants_for_resource(&resource_id_for_kernel)
            .map_err(|error| error.to_string())
    })
    .await?;
    host.file_resources
        .lock()
        .map_err(|_| "file resource registry lock poisoned".to_string())?
        .finalize_removal(&resource_id)
}

#[tauri::command]
async fn grant_file_resource_access(
    host: HostState<'_>,
    holder: AppId,
    resource_id: ResourceId,
    operations: Vec<FileResourceGrantOperation>,
) -> Result<(), String> {
    let _transition_guard = host.file_resource_transition.lock().await;
    if operations.is_empty() {
        return Err("at least one file resource operation is required".into());
    }
    {
        let registry = host
            .file_resources
            .lock()
            .map_err(|_| "file resource registry lock poisoned".to_string())?;
        registry.active_resource(&resource_id)?;
    }
    for operation in operations {
        issue_grant_phased(
            host.inner().clone(),
            holder.clone(),
            file_resource_grant_request(holder.clone(), resource_id.clone(), operation),
        )
        .await?;
    }
    Ok(())
}

#[tauri::command]
async fn grant_artifact_access(
    host: HostState<'_>,
    holder: AppId,
    target: artifacts_app::ArtifactAccessTarget,
) -> Result<(), String> {
    let target_for_prepare = target.clone();
    let holder_for_prepare = holder.clone();
    let prepared = with_kernel_blocking(host.inner().clone(), move |kernel| {
        artifacts_app::validate_access_target(kernel, &target_for_prepare)?;
        artifacts_app::artifact_access_grant_requests(&holder_for_prepare, &target_for_prepare)
            .into_iter()
            .filter(|request| {
                let GrantScope::ExactCapability {
                    provider,
                    capability,
                } = &request.scope
                else {
                    return true;
                };
                let capability = app_host_kernel::primitives::capability::CapabilityRef {
                    provider: provider.clone(),
                    capability: capability.clone(),
                };
                !kernel
                    .grants_for(&holder_for_prepare)
                    .into_iter()
                    .any(|grant| {
                        grant.scope.covers(&capability)
                            && grant.data_scope.covers(&request.data_scope)
                    })
            })
            .map(|request| {
                kernel
                    .prepare_grant(&holder_for_prepare, request)
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()
    })
    .await?;
    if prepared.is_empty() {
        return Ok(());
    }
    let approvals = tauri::async_runtime::spawn_blocking(move || {
        PreparedGrant::await_grouped_approvals(prepared).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("grant approval task failed: {error}"))??;
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        // Approval may outlive the selected resource. Recheck before any grant
        // commits so a future artifact-removal path cannot leave stale access.
        artifacts_app::validate_access_target(kernel, &target)?;
        for approval in approvals {
            kernel
                .commit_grant(approval)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
async fn clear_secret(
    host: HostState<'_>,
    owner: AppId,
    secret_name: String,
) -> Result<(), String> {
    let _transition_guard = host.managed_app_transition.lock().await;
    require_secret_owner(host.inner(), &owner).await?;
    let provider_send_active = owner == AppId::new("llm-provider")
        && !host
            .active_chat_sends
            .lock()
            .map_err(|_| "chat execution lock poisoned".to_string())?
            .is_empty();
    if provider_send_active {
        return Err(
            "cancel or finish running Chat messages before changing provider credentials".into(),
        );
    }
    let config = host.config.clone();
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        config
            .lock()
            .map_err(|_| "config lock poisoned".to_string())?
            .clear_secret(kernel, &owner, &secret_name)
    })
    .await
}

/// Capture the probe under a brief config lock, then run the HTTP round-trip
/// without it. Holding the config lock across network I/O would stall every
/// config command for up to the 15s timeout.
fn connector_probe_now(host: &Arc<Host>, connector_id: &str) -> Result<ConnectorProbe, String> {
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .connector_probe(connector_id)
}

#[tauri::command]
async fn test_connector_config(
    host: HostState<'_>,
    connector_id: String,
) -> Result<ConnectionTestResult, String> {
    let probe = connector_probe_now(host.inner(), &connector_id)?;
    tauri::async_runtime::spawn_blocking(move || config::run_connector_test(&probe))
        .await
        .map_err(|error| format!("connector test task failed: {error}"))?
}

/// Model discovery probes the draft endpoint directly so the user can
/// discover a model BEFORE the profile is complete enough to save (a saved
/// profile's fields are just a draft that equals the persisted state).
/// Receives only the secret NAME — the value is resolved host-side.
#[tauri::command]
async fn discover_connector_models_draft(
    host: HostState<'_>,
    kind: config::ConnectorKind,
    base_url: String,
    default_model: Option<String>,
    api_key_secret_name: Option<String>,
) -> Result<ModelListResult, String> {
    if base_url.trim().is_empty() {
        return Err("base URL is required for model discovery".into());
    }
    let probe = host
        .config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .draft_probe(kind, &base_url, api_key_secret_name.as_deref());
    tauri::async_runtime::spawn_blocking(move || {
        let models = llm_client::models_from_config(
            kind,
            config::runtime_base_url(kind, &base_url),
            default_model.filter(|model| !model.trim().is_empty()),
            probe.api_key,
            true,
        )?;
        let count = models.len();
        Ok(ModelListResult {
            models: models
                .into_iter()
                .map(|model| config::ModelInfo {
                    id: model.id,
                    display_name: Some(model.display_name),
                    variants: model.variants,
                    text_verbosity: model.text_verbosity,
                })
                .collect(),
            message: match count {
                0 => "No models found. You can still type a model manually.".into(),
                1 => "Discovered 1 model".into(),
                count => format!("Discovered {count} models"),
            },
        })
    })
    .await
    .map_err(|error| format!("connector model discovery task failed: {error}"))?
}

#[tauri::command]
fn list_apps(host: HostState<'_>) -> Result<Vec<InstalledAppView>, String> {
    let presentations = host
        .app_manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?
        .app_presentation_views()?;
    with_kernel_now(host.inner(), |kernel| {
        Ok(kernel
            .installed_apps()
            .cloned()
            .map(|app| InstalledAppView {
                icon: presentations
                    .get(app.manifest.app_id.as_str())
                    .and_then(|presentation| presentation.icon.clone()),
                theme_colors: presentations
                    .get(app.manifest.app_id.as_str())
                    .map(|presentation| presentation.theme_colors.clone())
                    .unwrap_or_default(),
                app,
            })
            .collect())
    })
}

/// Grant-aware capability introspection for a consuming app. The consumer
/// sees only capabilities it holds a covering grant for, annotated with the
/// effective grant condition.
#[tauri::command]
async fn available_capabilities_for(
    host: HostState<'_>,
    app_id: AppId,
) -> Result<Vec<CapabilityUseView>, String> {
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        let mut available = kernel
            .available_capabilities_for(&app_id)
            .map_err(|error| error.to_string())?;
        permissions_app::contextualize_tools(kernel, &app_id, &mut available)?;
        artifacts_app::contextualize_tools(kernel, &mut available);
        Ok(available)
    })
    .await
}

/// Validates host-supplied extension-slot context against the target app's
/// sealed contract before it crosses into a sandboxed extension surface.
#[tauri::command]
async fn validate_extension_context(
    host: HostState<'_>,
    target_app: AppId,
    extension_point: ExtensionPointName,
    context: JsonObject,
) -> Result<(), String> {
    // This gates whether an extension slot (e.g. Chat's per-message actions)
    // renders its surface at all, and it runs once per slot. A non-blocking
    // acquire would fail whenever the kernel is briefly busy with a run, and
    // the caller fails closed with no retry — so the slot would silently never
    // appear. Wait for the lock (this is a cheap read) so a transient run does
    // not permanently hide the surface, matching `get_surface_ui`'s intent.
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        let app = kernel
            .installed_app(&target_app)
            .map_err(|error| error.to_string())?;
        let point = app
            .manifest
            .extension_points
            .iter()
            .find(|point| point.name == extension_point)
            .ok_or_else(|| {
                format!("app '{target_app}' does not declare extension point '{extension_point}'")
            })?;
        validate_against_schema(
            &serde_json::Value::Object(context),
            &point.context_schema,
            SchemaViolation::CapabilityInput,
            &format!("extension context for '{target_app}/{extension_point}'"),
        )
        .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
fn list_grants(host: HostState<'_>) -> Result<Vec<GrantView>, String> {
    with_kernel_now(host.inner(), |kernel| {
        let apps: Vec<InstalledApp> = kernel.installed_apps().cloned().collect();
        let holders: Vec<AppId> = apps.iter().map(|app| app.manifest.app_id.clone()).collect();
        let display_names = apps
            .iter()
            .map(|app| {
                (
                    app.manifest.app_id.clone(),
                    app.manifest.display_name.clone(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut grants = holders
            .iter()
            .flat_map(|holder| kernel.grant_statuses_for(holder))
            .map(|GrantStatusView { grant, status }| {
                let origin = grant.origin;
                GrantView {
                    holder_display_name: display_names
                        .get(&grant.holder)
                        .cloned()
                        .unwrap_or_else(|| grant.holder.as_str().to_string()),
                    grant,
                    status,
                    origin,
                }
            })
            .collect::<Vec<_>>();
        grants.sort_by(|left, right| {
            right
                .grant
                .issued_at
                .cmp(&left.grant.issued_at)
                .then_with(|| left.grant.grant_id.cmp(&right.grant.grant_id))
        });
        Ok(grants)
    })
}

#[tauri::command]
fn ledger_records(host: HostState<'_>) -> Result<Vec<LedgerRecord>, String> {
    with_kernel_now(host.inner(), |kernel| Ok(kernel.records().to_vec()))
}

#[tauri::command]
fn list_artifacts(host: HostState<'_>) -> Result<Vec<Artifact>, String> {
    with_kernel_now(host.inner(), |kernel| {
        Ok(kernel.artifacts().cloned().collect())
    })
}

/// A sandboxed app surface reads only ITS OWN artifacts. Scoped by
/// kernel-written provenance (`produced_by`), which apps cannot forge — the
/// bridge never hands a surface another app's artifacts.
#[tauri::command]
fn list_app_artifacts(host: HostState<'_>, app_id: AppId) -> Result<Vec<Artifact>, String> {
    with_kernel_now(host.inner(), |kernel| {
        Ok(kernel
            .artifacts()
            .filter(|artifact| artifact.provenance.produced_by == app_id)
            .cloned()
            .collect())
    })
}

/// Minimized event views for an app's own runs: topic,
/// attribution, and stable ids only — never raw ledger records. Computed
/// read-only from the ledger and scoped to runs the app initiated, so a
/// sandboxed surface observes only its own work.
#[tauri::command]
fn app_surface_events(host: HostState<'_>, app_id: AppId) -> Result<Vec<AppEventEnvelope>, String> {
    with_kernel_now(host.inner(), |kernel| {
        // Map each run to its initiating app from the RunStarted event, then
        // emit minimized views for events belonging to this app's runs.
        let mut own_runs = std::collections::BTreeSet::new();
        for record in kernel.records() {
            if let LedgerEvent::RunStarted {
                run_id, initiator, ..
            } = &record.event
            {
                if initiator.app_id() == &app_id {
                    own_runs.insert(run_id.clone());
                }
            }
        }
        Ok(kernel
            .records()
            .iter()
            .filter(|record| own_runs.contains(record.event.run_id()))
            .map(|record| AppEventEnvelope::from_event(&record.event, app_id.clone()))
            .collect())
    })
}

/// The static custom UI bundle for a surface, if the app registered one.
/// `None` means the surface uses a bundled Svelte screen or a generic
/// renderer. The bundle is served into a sandboxed frame (see
/// `host/src/lib/surfaces/AppSurfaceFrame.svelte`).
#[tauri::command]
fn get_surface_ui(
    host: HostState<'_>,
    app_id: AppId,
    surface: SurfaceName,
) -> Result<Option<SurfaceUiBundle>, String> {
    // The registry is populated only after kernel installation and is cleared
    // during disable/uninstall. Reading it directly keeps surface startup from
    // degrading permanently when the kernel is briefly busy with another run.
    host.surface_ui
        .lock()
        .map_err(|_| "surface UI registry lock poisoned".to_string())
        .map(|registry| registry.get(&app_id, &surface).cloned())
}

#[tauri::command]
async fn request_app_grants(host: HostState<'_>, app_id: AppId) -> Result<(), String> {
    let lookup_app_id = app_id.clone();
    let requests = with_kernel_blocking(host.inner().clone(), move |kernel| {
        let manifest = kernel
            .installed_app(&lookup_app_id)
            .map_err(|error| error.to_string())?
            .manifest
            .clone();
        Ok(manifest.grant_requests)
    })
    .await?;
    for request in requests {
        issue_grant_phased(host.inner().clone(), app_id.clone(), request).await?;
    }
    Ok(())
}

#[tauri::command]
async fn request_manifest_grant(
    host: HostState<'_>,
    app_id: AppId,
    request: GrantRequest,
) -> Result<(), String> {
    let lookup_app_id = app_id.clone();
    let expected_request = request.clone();
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        let manifest = &kernel
            .installed_app(&lookup_app_id)
            .map_err(|error| error.to_string())?
            .manifest;
        if !manifest.grant_requests.contains(&expected_request) {
            return Err("permission is not declared by this app's manifest".into());
        }
        Ok(())
    })
    .await?;
    issue_grant_phased(host.inner().clone(), app_id, request).await
}

#[tauri::command]
async fn issue_editor_grant(
    host: HostState<'_>,
    request: GrantEditorRequest,
) -> Result<(), String> {
    host.file_resources
        .lock()
        .map_err(|_| "file resource registry lock poisoned".to_string())?
        .validate_grant_data_scope(&request.scope, &request.data_scope)?;
    let grant_request = request.grant_request()?;
    let duplicate_request = grant_request.clone();
    let holder = request.holder.clone();
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        artifacts_app::validate_grant_data_scope(
            kernel,
            &duplicate_request.scope,
            &duplicate_request.data_scope,
        )?;
        let duplicate = duplicate_request.duration == GrantDuration::NonExpiring
            && kernel.grant_statuses_for(&holder).into_iter().any(|view| {
                view.status == GrantStatus::Active
                    && view.grant.scope == duplicate_request.scope
                    && view.grant.data_scope == duplicate_request.data_scope
                    && view.grant.condition == duplicate_request.condition
                    && view.grant.expires_at.is_none()
            });
        if duplicate {
            return Err("an equivalent active permission already exists".into());
        }
        Ok(())
    })
    .await?;
    issue_grant_phased(host.inner().clone(), request.holder, grant_request).await
}

#[tauri::command]
async fn replace_grant(
    host: HostState<'_>,
    grant_id: GrantId,
    request: GrantEditorRequest,
) -> Result<(), String> {
    host.file_resources
        .lock()
        .map_err(|_| "file resource registry lock poisoned".to_string())?
        .validate_grant_data_scope(&request.scope, &request.data_scope)?;
    let grant_request = request.grant_request()?;
    let validated_request = grant_request.clone();
    let holder = request.holder.clone();
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        artifacts_app::validate_grant_data_scope(
            kernel,
            &validated_request.scope,
            &validated_request.data_scope,
        )?;
        let exists = kernel
            .grant_statuses_for(&holder)
            .into_iter()
            .any(|view| view.grant.grant_id == grant_id);
        if !exists {
            return Err("grant does not belong to the selected holder".into());
        }
        // Keep both immutable facts: a denied replacement leaves the revocation visible.
        kernel
            .revoke_grant(&grant_id)
            .map_err(|error| error.to_string())?;
        Ok(())
    })
    .await?;
    issue_grant_phased(host.inner().clone(), request.holder, grant_request).await
}

#[tauri::command]
async fn submit_permission_proposal(
    host: HostState<'_>,
    artifact_id: ArtifactId,
) -> Result<PermissionProposalSubmission, String> {
    let proposal = with_kernel_blocking(host.inner().clone(), move |kernel| {
        let artifact = kernel
            .artifact(&artifact_id)
            .map_err(|error| error.to_string())?;
        let proposal = permissions_app::proposal_from_artifact(artifact)?;
        let run = kernel
            .run_view(&artifact.provenance.run_id)
            .map_err(|error| error.to_string())?;
        if run.initiating_app() != &proposal.holder {
            return Err("permission proposal holder does not match its initiating app".into());
        }
        kernel
            .installed_app(&proposal.holder)
            .map_err(|error| error.to_string())?;
        let GrantScope::ExactCapability {
            provider,
            capability,
        } = &proposal.scope
        else {
            return Err("permission proposal must name one exact capability".into());
        };
        kernel
            .capability_declaration(&app_host_kernel::primitives::capability::CapabilityRef {
                provider: provider.clone(),
                capability: capability.clone(),
            })
            .map_err(|error| error.to_string())?;
        Ok(proposal)
    })
    .await?;
    let capability = match &proposal.scope {
        GrantScope::ExactCapability {
            provider,
            capability,
        } => app_host_kernel::primitives::capability::CapabilityRef {
            provider: provider.clone(),
            capability: capability.clone(),
        },
        GrantScope::AllProviderCapabilities { .. } => {
            return Err("permission proposal must name one exact capability".into())
        }
    };
    let holder = proposal.holder.clone();
    let existing = with_kernel_blocking(host.inner().clone(), {
        let holder = holder.clone();
        let capability = capability.clone();
        move |kernel| Ok(kernel.check_grant(&holder, &capability))
    })
    .await?;
    if let GrantCheck::Allowed(grant) | GrantCheck::ApprovalRequired(grant) = existing {
        return Ok(PermissionProposalSubmission::AlreadyActive {
            grant_id: grant.grant_id,
            effective_condition: grant.condition,
        });
    }

    match issue_grant_phased_result(
        host.inner().clone(),
        holder.clone(),
        proposal.grant_request(),
    )
    .await?
    {
        IssueResult::Refused => Ok(PermissionProposalSubmission::Refused),
        IssueResult::Issued(grant) => {
            let effective_condition = with_kernel_blocking(host.inner().clone(), move |kernel| {
                match kernel.check_grant(&holder, &capability) {
                    GrantCheck::Allowed(effective) | GrantCheck::ApprovalRequired(effective) => {
                        Ok(effective.condition)
                    }
                    GrantCheck::Denied(reason) => Err(format!(
                        "new permission is not effective after issuance: {reason:?}"
                    )),
                }
            })
            .await?;
            Ok(PermissionProposalSubmission::Issued {
                grant_id: grant.grant_id,
                effective_condition,
            })
        }
    }
}

// -- MCP servers ---------------------------------------------------------------
//
// Configured servers live in host-owned config; nothing dials, installs, or
// grants until the user explicitly connects. Connecting dials and discovers
// tools OFF the kernel lock (a dead server must not stall the shell), then
// installs under it, where trusted chrome confirms every requires-approval
// grant.

#[tauri::command]
fn list_mcp_servers(host: HostState<'_>) -> Result<Vec<McpServerStatusView>, String> {
    let config = host
        .config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?;
    config
        .list_mcp_servers()
        .into_iter()
        .map(|server| {
            Ok(McpServerStatusView {
                connected: host.mcp_connections.is_active(&server.id)?,
                id: server.id,
                display_name: server.display_name,
                transport: server.transport,
            })
        })
        .collect()
}

#[tauri::command]
fn upsert_mcp_server(
    host: HostState<'_>,
    server: McpServerConfigView,
) -> Result<McpServerConfigView, String> {
    if host.mcp_connections.is_active(&server.id)? {
        return Err(format!(
            "MCP server '{}' is connected; disconnect it before editing",
            server.id
        ));
    }
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .upsert_mcp_server(server)
}

#[tauri::command]
fn delete_mcp_server(host: HostState<'_>, server_id: String) -> Result<(), String> {
    if host.mcp_connections.is_active(&server_id)? {
        return Err(format!(
            "MCP server '{server_id}' is connected; disconnect it before deleting"
        ));
    }
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .delete_mcp_server(&server_id)
}

#[tauri::command]
fn put_mcp_http_auth_secret(
    host: HostState<'_>,
    server_id: String,
    value: String,
) -> Result<(), String> {
    if host.mcp_connections.is_active(&server_id)? {
        return Err(format!(
            "MCP server '{server_id}' is active; disconnect it before changing authentication"
        ));
    }
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .put_mcp_http_auth_secret(&server_id, value)
}

#[tauri::command]
fn clear_mcp_http_auth_secret(host: HostState<'_>, server_id: String) -> Result<(), String> {
    if host.mcp_connections.is_active(&server_id)? {
        return Err(format!(
            "MCP server '{server_id}' is active; disconnect it before changing authentication"
        ));
    }
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .clear_mcp_http_auth_secret(&server_id)
}

#[tauri::command]
fn has_mcp_http_auth_secret(host: HostState<'_>, server_id: String) -> Result<bool, String> {
    host.config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .has_mcp_http_auth_secret(&server_id)
}

#[tauri::command]
async fn connect_mcp_server(host: HostState<'_>, server_id: String) -> Result<(), String> {
    let connections = host.mcp_connections.clone();
    connections.begin(&server_id)?;
    let config_snapshot = {
        let config = host
            .config
            .lock()
            .map_err(|_| "config lock poisoned".to_string())?;
        match config.mcp_server(&server_id) {
            Some(server) => config
                .mcp_http_auth_header(&server_id)
                .map(|header| (server, header)),
            None => Err(format!("unknown MCP server: {server_id}")),
        }
    };
    let (server_config, http_auth_header) = match config_snapshot {
        Ok(snapshot) => snapshot,
        Err(error) => {
            connections.abort(&server_id).map_err(|state_error| {
                format!("{error}; connect rollback failed: {state_error}")
            })?;
            return Err(error);
        }
    };

    // Dial + handshake + tool discovery with no host lock held.
    let dialed = {
        let server_config = server_config.clone();
        tauri::async_runtime::spawn_blocking(move || {
            mcp::dial_server(&server_config, http_auth_header)
        })
        .await
        .map_err(|error| format!("MCP dial task failed: {error}"))
    };
    let (client, tools) = match dialed {
        Ok(Ok(dialed)) => dialed,
        Ok(Err(error)) | Err(error) => {
            connections.abort(&server_id).map_err(|state_error| {
                format!("{error}; connect rollback failed: {state_error}")
            })?;
            if error.contains("HTTP 401") {
                return Err(format!(
                    "MCP HTTP authentication failed (401 Unauthorized). Add or update this server's authentication credential, then retry. Technical detail: {error}"
                ));
            }
            return Err(error);
        }
    };

    // Prepare, prompt, and commit in separate phases so the kernel mutex is
    // not held while trusted chrome waits for the user.
    let (manifest, handlers) =
        mcp::dialed_server_install_parts(&server_id, &server_config, &client, &tools);
    let install = install_kernel_app_phased(
        host.inner().clone(),
        manifest,
        handlers,
        GrantOrigin::ManifestRequested,
    )
    .await;
    match install {
        Ok(()) => connections.complete(&server_id, client),
        Err(error) => {
            client.shutdown();
            connections.abort(&server_id).map_err(|state_error| {
                format!("{error}; connect rollback failed: {state_error}")
            })?;
            Err(error)
        }
    }
}

#[tauri::command]
async fn disconnect_mcp_server(host: HostState<'_>, server_id: String) -> Result<(), String> {
    let connections = host.mcp_connections.clone();
    let surface_ui = host.surface_ui.clone();
    let removed_app = mcp::app_id_for_server(&server_id);
    let transition = mcp::begin_server_disconnect(&connections, &server_id)?;
    let uninstall_id = server_id.clone();
    let result = with_kernel_blocking(host.inner().clone(), move |kernel| {
        mcp::uninstall_server(kernel, &uninstall_id)
    })
    .await;
    // Uninstalling an app drops any custom surface UI it registered, so a
    // removed app leaves no bundle behind for a later app to inherit.
    if let Err(error) = result {
        connections
            .rollback_disconnect(&transition)
            .map_err(|state_error| format!("{error}; disconnect rollback failed: {state_error}"))?;
        return Err(error);
    }

    let client = connections.complete_disconnect(&transition)?;
    client.shutdown();
    {
        let mut registry = surface_ui
            .lock()
            .map_err(|_| "surface UI registry lock poisoned".to_string())?;
        registry.remove_app(&removed_app);
    }
    Ok(())
}

#[tauri::command]
async fn open_surface(
    host: HostState<'_>,
    app_id: AppId,
    surface: SurfaceName,
) -> Result<SurfaceBinding, String> {
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        kernel
            .open_surface(&app_id, &surface)
            .map_err(|error| error.to_string())
    })
    .await
}

/// Release a surface binding when its frame unmounts. Idempotent: closing an
/// already-closed or unknown binding is a no-op, so teardown never fails.
#[tauri::command]
async fn close_surface(host: HostState<'_>, binding: SurfaceBinding) -> Result<(), String> {
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        kernel.close_surface(&binding);
        Ok(())
    })
    .await
}

// -- App manager (install / inspect / lifecycle) ------------------------------

fn app_status_views_now(host: &Arc<Host>) -> Result<Vec<AppStatusView>, String> {
    // Read presentation state first, then try the kernel. The reverse order can
    // hold the entire kernel unavailable while backend startup owns the manager.
    let manager = host
        .app_manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?;
    let surface_ui = host
        .surface_ui
        .lock()
        .map_err(|_| "surface UI registry lock poisoned".to_string())?;
    with_kernel_now(host, |kernel| Ok(manager.status_views(kernel, &surface_ui)))
}

/// The full manager list: bundled apps (read-only) plus every managed app.
#[tauri::command]
fn list_installed_apps(host: HostState<'_>) -> Result<Vec<AppStatusView>, String> {
    app_status_views_now(host.inner())
}

/// Inspect a package directory before install. Runs no package code.
#[tauri::command]
fn inspect_package(host: HostState<'_>, package_dir: String) -> Result<PackageInspection, String> {
    let mut manager = host
        .app_manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?;
    manager.inspect(std::path::Path::new(&package_dir))
}

/// Fetch and inspect an app package from a public HTTPS Git repository.
/// Repository code is never checked out or executed.
#[tauri::command]
async fn inspect_git_package(
    host: HostState<'_>,
    git_url: String,
) -> Result<PackageInspection, String> {
    let manager = host.app_manager.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let staging_root = manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .staging_root()
            .to_path_buf();
        let exported = git_source::export_public_repository(&git_url, &staging_root)?;
        let mut manager = manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?;
        manager.inspect(exported.path())
    })
    .await
    .map_err(|error| format!("Git package inspection task failed: {error}"))?
}

/// Install a package. May block on trusted-chrome grant prompts, so it runs on
/// a blocking thread. Returns the refreshed manager list.
#[tauri::command]
async fn install_app(
    host: HostState<'_>,
    staged_id: String,
    package_digest: String,
) -> Result<Vec<AppStatusView>, String> {
    let _transition_guard = host.managed_app_transition.lock().await;
    let manager = host.app_manager.clone();
    let installed_at = now_rfc3339();
    let record = tauri::async_runtime::spawn_blocking({
        let manager = manager.clone();
        move || {
            let mut manager = manager
                .lock()
                .map_err(|_| "app manager lock poisoned".to_string())?;
            manager.install_record(&staged_id, &package_digest, &installed_at)
        }
    })
    .await
    .map_err(|error| format!("app install task failed: {error}"))??;

    let preparation = manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?
        .activation_preparation_with_invoker(&record.id, host.kernel_invoker.clone())?;
    let prepared = tauri::async_runtime::spawn_blocking(move || preparation.prepare())
        .await
        .map_err(|error| format!("app activation preparation failed: {error}"))?;

    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(reason) => {
            manager
                .lock()
                .map_err(|_| "app manager lock poisoned".to_string())?
                .record_failure(&record.id, reason);
            return app_status_views_now(host.inner());
        }
    };

    let record_id = record.id.clone();
    activate_managed_app(host.inner().clone(), record_id, prepared).await?;
    app_status_views_now(host.inner())
}

/// Enable or disable a managed app. Enabling may prompt for grants.
#[tauri::command]
async fn set_app_enabled(
    host: HostState<'_>,
    app_id: String,
    enabled: bool,
) -> Result<Vec<AppStatusView>, String> {
    let _transition_guard = host.managed_app_transition.lock().await;
    let manager = host.app_manager.clone();
    let surface_ui = host.surface_ui.clone();
    if enabled {
        manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .set_enabled_state(&app_id, true)?;
        let installed_app_id = AppId::new(&app_id);
        if with_kernel_blocking(host.inner().clone(), move |kernel| {
            Ok(kernel.installed_app(&installed_app_id).is_ok())
        })
        .await?
        {
            return app_status_views_now(host.inner());
        }
        let preparation = manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .activation_preparation_with_invoker(&app_id, host.kernel_invoker.clone())?;
        let prepared = tauri::async_runtime::spawn_blocking(move || preparation.prepare())
            .await
            .map_err(|error| format!("app activation preparation failed: {error}"))?;
        match prepared {
            Err(reason) => {
                manager
                    .lock()
                    .map_err(|_| "app manager lock poisoned".to_string())?
                    .record_failure(&app_id, reason);
            }
            Ok(prepared) => {
                activate_managed_app(host.inner().clone(), app_id.clone(), prepared).await?;
            }
        }
    } else {
        let removal_manager = manager.clone();
        let removal_surface_ui = surface_ui.clone();
        let removal_app_id = app_id.clone();
        let client = with_kernel_blocking(host.inner().clone(), move |kernel| {
            let mut manager = removal_manager
                .lock()
                .map_err(|_| "app manager lock poisoned".to_string())?;
            let mut surface_ui = removal_surface_ui
                .lock()
                .map_err(|_| "surface UI registry lock poisoned".to_string())?;
            manager.remove_runtime(kernel, &mut surface_ui, &removal_app_id)
        })
        .await?;
        if let Some(client) = client {
            client.shutdown();
        }
        manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .set_enabled_state(&app_id, false)?;
    }

    app_status_views_now(host.inner())
}

/// Uninstall a managed app, purging secrets / app data per the user's choice.
#[tauri::command]
async fn uninstall_app(
    host: HostState<'_>,
    app_id: String,
    purge_secrets: bool,
    purge_data: bool,
) -> Result<Vec<AppStatusView>, String> {
    let _transition_guard = host.managed_app_transition.lock().await;
    let manager = host.app_manager.clone();
    let surface_ui = host.surface_ui.clone();
    let config = host.config.clone();
    manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?
        .begin_uninstall(&app_id, purge_secrets, purge_data)?;
    let runtime_app_id = app_id.clone();
    let runtime_manager = manager.clone();
    let runtime_surface_ui = surface_ui.clone();
    let client = with_kernel_blocking(host.inner().clone(), move |kernel| {
        let mut manager = runtime_manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?;
        let mut surface_ui = runtime_surface_ui
            .lock()
            .map_err(|_| "surface UI registry lock poisoned".to_string())?;
        manager.remove_runtime(kernel, &mut surface_ui, &runtime_app_id)
    })
    .await?;
    if let Some(client) = client {
        client.shutdown();
    }
    let mut config = config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?;
    manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?
        .finish_uninstall(&mut config, &app_id)?;
    app_status_views_now(host.inner())
}

#[tauri::command]
fn plan_managed_app_transition(
    host: HostState<'_>,
    request: ManagedAppTransitionRequest,
) -> Result<ManagedAppTransitionPlan, String> {
    let mut manager = host
        .app_manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?;
    with_kernel_now(host.inner(), |kernel| {
        let manifests = kernel
            .installed_apps()
            .map(|app| app.manifest.clone())
            .collect::<Vec<_>>();
        manager.plan_managed_app_transition_with_manifests(request, &manifests)
    })
}

#[tauri::command]
async fn apply_managed_app_transition(
    host: HostState<'_>,
    transition_id: String,
) -> Result<Vec<AppStatusView>, String> {
    let plan = {
        let _transition_guard = host.managed_app_transition.lock().await;
        host.app_manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?
            .take_managed_app_transition_plan(&transition_id)?
    };
    if matches!(plan.operation, ManagedAppOperation::Install) {
        let staged_id = plan
            .staged_id
            .ok_or_else(|| "install plan is missing staged_id".to_string())?;
        let package_digest = plan
            .package_digest
            .ok_or_else(|| "install plan is missing package_digest".to_string())?;
        return install_app(host, staged_id, package_digest).await;
    }
    let _transition_guard = host.managed_app_transition.lock().await;
    let journal = host
        .app_manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?
        .begin_journaled_transition(plan)?;
    let Some(journal) = journal else {
        return app_status_views_now(host.inner());
    };
    continue_journaled_transition(host.inner(), journal, true).await?;
    app_status_views_now(host.inner())
}

/// One chat message → one run through the public action path. May block on
/// an approval prompt (e.g. the weather grant), so it runs on a blocking
/// thread.
#[tauri::command]
fn list_chat_threads(host: HostState<'_>) -> Result<Vec<ChatThreadSummary>, String> {
    host.chat_store
        .lock()
        .map_err(|_| "chat store lock poisoned".to_string())
        .map(|store| store.list_threads())
}

#[tauri::command]
fn get_chat_thread(host: HostState<'_>, thread_id: String) -> Result<ChatThread, String> {
    host.chat_store
        .lock()
        .map_err(|_| "chat store lock poisoned".to_string())?
        .get_thread(&thread_id)
}

#[tauri::command]
async fn create_chat_thread(host: HostState<'_>) -> Result<ChatThread, String> {
    let selected_engine = with_kernel_blocking(host.inner().clone(), |kernel| {
        let engines = chat_app::list_chat_agent_engines(kernel)?;
        if engines.len() == 1 && engines[0].available {
            let engine_ref = engines[0].app_id.clone();
            let receipt = chat_app::resolve_chat_agent_engine_selection(kernel, &engine_ref)?;
            Ok(Some((engine_ref, receipt)))
        } else {
            Ok(None)
        }
    })
    .await?;
    let thread = {
        let mut store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        match selected_engine {
            Some((engine_ref, receipt)) => {
                store.create_thread_with_agent_engine(Some(engine_ref), Some(receipt))?
            }
            None => store.create_thread()?,
        }
    };
    publish_chat_thread_change(
        host.inner(),
        thread.resource_id.clone(),
        thread.revision,
        AppDataChangeKind::Created,
    )
    .await;
    Ok(thread)
}

#[tauri::command]
fn rename_chat_thread(
    host: HostState<'_>,
    thread_id: String,
    title: String,
) -> Result<ChatThread, String> {
    host.chat_store
        .lock()
        .map_err(|_| "chat store lock poisoned".to_string())?
        .rename_thread(&thread_id, &title)
}

#[tauri::command]
async fn delete_chat_thread(host: HostState<'_>, thread_id: String) -> Result<(), String> {
    if host
        .active_chat_sends
        .lock()
        .map_err(|_| "chat execution lock poisoned".to_string())?
        .contains_key(&thread_id)
    {
        return Err("cancel the running message before deleting this chat".into());
    }
    let (resource_id, revision) = {
        let store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        let thread = store.get_thread(&thread_id)?;
        (thread.resource_id, thread.revision)
    };
    let resource_id_for_kernel = resource_id.clone();
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        kernel
            .revoke_grants_for_resource(&ResourceId::new(resource_id_for_kernel.clone()))
            .map_err(|error| error.to_string())
    })
    .await?;
    {
        let mut store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        store.delete_thread(&thread_id)?;
    }
    publish_chat_thread_change(
        host.inner(),
        resource_id,
        revision,
        AppDataChangeKind::Deleted,
    )
    .await;
    Ok(())
}

#[tauri::command]
async fn cancel_chat_message(host: HostState<'_>, thread_id: String) -> Result<(), String> {
    let run_ids = {
        let mut sends = host
            .active_chat_sends
            .lock()
            .map_err(|_| "chat execution lock poisoned".to_string())?;
        let send = sends
            .get_mut(&thread_id)
            .ok_or_else(|| "no message is running for this chat".to_string())?;
        send.cancelled.store(true, Ordering::Release);
        send.run_ids.clone()
    };
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        for run_id in run_ids {
            kernel.cancel_pending_invocations_for_run(&run_id);
        }
        Ok(())
    })
    .await
}

#[tauri::command]
async fn send_chat_message(
    host: HostState<'_>,
    thread_id: String,
    message: String,
    request_id: String,
    on_event: Channel<ChatStreamEvent>,
) -> Result<SendChatMessageResult, String> {
    let progress = Arc::new(move |event| {
        let _ = on_event.send(event);
    });
    send_chat_message_with_progress(host, thread_id, message, request_id, progress).await
}

async fn send_chat_message_with_progress(
    host: HostState<'_>,
    thread_id: String,
    message: String,
    request_id: String,
    on_event: Arc<dyn Fn(ChatStreamEvent) + Send + Sync>,
) -> Result<SendChatMessageResult, String> {
    chat_runtime::send_chat_message_with_progress(
        host.inner().clone(),
        thread_id,
        message,
        request_id,
        on_event,
    )
    .await
}

/// Map a finished invocation to the terminal state its Run should end in.
/// Shared by the "prepared" and "refused" branches of `submit_action` so the
/// two paths can never diverge on how a result closes its Run.
fn terminal_state_for(
    result: &app_host_kernel::invocation::InvocationResult,
) -> app_host_kernel::primitives::run::RunTerminalState {
    use app_host_kernel::invocation::InvocationResult;
    use app_host_kernel::primitives::run::RunTerminalState;
    match result {
        InvocationResult::Completed { .. } => RunTerminalState::Completed,
        InvocationResult::Failed { .. } => RunTerminalState::Failed,
        InvocationResult::Refused { .. } => RunTerminalState::Cancelled,
    }
}

async fn fail_started_run(host: Arc<Host>, run_id: RunId, error: String) -> String {
    match with_kernel_blocking(host, move |kernel| {
        kernel
            .end_run(
                &run_id,
                app_host_kernel::primitives::run::RunTerminalState::Failed,
            )
            .map_err(|value| value.to_string())
    })
    .await
    {
        Ok(()) => error,
        Err(end_error) => format!("{error}; failed to close run: {end_error}"),
    }
}

async fn submit_action_inner(
    host_state: Arc<Host>,
    binding: SurfaceBinding,
    intent: ActionIntent,
    progress: app_host_kernel::ProgressReporter,
) -> Result<SurfaceActionOutcome, String> {
    let (run_id, prepared) = with_kernel_blocking(host_state.clone(), move |kernel| {
        kernel
            .prepare_surface_action(&binding, intent)
            .map_err(|error| error.to_string())
    })
    .await?;
    progress.report(serde_json::json!({
        "kind": "invocation-start",
        "run_id": run_id.to_string(),
    }));
    let was_refused = matches!(&prepared, PrepareInvocation::Refused(_));
    let result = match prepared {
        PrepareInvocation::Refused(result) => result,
        PrepareInvocation::Prepared(prepared) => {
            let approval =
                match tauri::async_runtime::spawn_blocking(move || prepared.await_approval()).await
                {
                    Ok(approval) => approval,
                    Err(error) => {
                        return Err(fail_started_run(
                            host_state.clone(),
                            run_id.clone(),
                            format!("approval task failed: {error}"),
                        )
                        .await);
                    }
                };
            let authorized = match with_kernel_blocking(host_state.clone(), move |kernel| {
                kernel
                    .authorize_invocation(approval)
                    .map_err(|error| error.to_string())
            })
            .await
            {
                Ok(authorized) => authorized,
                Err(error) => {
                    return Err(fail_started_run(host_state.clone(), run_id.clone(), error).await);
                }
            };
            let ending_run_id = run_id.clone();
            let result = match authorized {
                app_host_kernel::AuthorizeInvocation::Refused(result) => result,
                app_host_kernel::AuthorizeInvocation::Authorized(authorized) => {
                    let executed = match tauri::async_runtime::spawn_blocking(move || {
                        authorized.execute_with_progress(progress)
                    })
                    .await
                    {
                        Ok(executed) => executed,
                        Err(error) => {
                            return Err(fail_started_run(
                                host_state.clone(),
                                run_id.clone(),
                                format!("invocation task failed: {error}"),
                            )
                            .await);
                        }
                    };
                    match with_kernel_blocking(host_state.clone(), move |kernel| {
                        kernel
                            .finalize_invocation(executed)
                            .map_err(|error| error.to_string())
                    })
                    .await
                    {
                        Ok(result) => result,
                        Err(error) => {
                            return Err(fail_started_run(
                                host_state.clone(),
                                run_id.clone(),
                                error,
                            )
                            .await);
                        }
                    }
                }
            };
            let terminal_state = terminal_state_for(&result);
            with_kernel_blocking(host_state.clone(), move |kernel| {
                kernel
                    .end_run(&ending_run_id, terminal_state)
                    .map_err(|error| error.to_string())?;
                Ok(())
            })
            .await?;
            result
        }
    };
    if was_refused {
        let terminal_state = terminal_state_for(&result);
        let ending_run_id = run_id.clone();
        with_kernel_blocking(host_state.clone(), move |kernel| {
            kernel
                .end_run(&ending_run_id, terminal_state)
                .map_err(|error| error.to_string())
        })
        .await?;
    }
    Ok(SurfaceActionOutcome { run_id, result })
}

/// The surface -> kernel action path (spec section 3.4). May block on an
/// approval prompt, so it runs on a blocking thread.
#[tauri::command]
async fn submit_action(
    host: HostState<'_>,
    binding: SurfaceBinding,
    intent: ActionIntent,
) -> Result<SurfaceActionOutcome, String> {
    submit_action_inner(
        host.inner().clone(),
        binding,
        intent,
        app_host_kernel::ProgressReporter::default(),
    )
    .await
}

#[tauri::command]
async fn submit_action_with_progress(
    host: HostState<'_>,
    binding: SurfaceBinding,
    intent: ActionIntent,
    on_event: Channel<serde_json::Value>,
) -> Result<SurfaceActionOutcome, String> {
    submit_action_inner(
        host.inner().clone(),
        binding,
        intent,
        app_host_kernel::ProgressReporter::new_checked(move |value| {
            on_event.send(value).map_err(|_| ())
        }),
    )
    .await
}

#[tauri::command]
async fn cancel_surface_action(
    host: HostState<'_>,
    binding: SurfaceBinding,
    run_id: RunId,
) -> Result<(), String> {
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        let run = kernel
            .run_view(&run_id)
            .map_err(|error| error.to_string())?;
        match run.initiator {
            app_host_kernel::primitives::run::Initiator::SurfaceAction { app_id, surface }
                if app_id == binding.app_id && surface == binding.surface =>
            {
                kernel.cancel_pending_invocations_for_run(&run_id);
                Ok(())
            }
            _ => Err("surface may cancel only its own action run".into()),
        }
    })
    .await
}

#[tauri::command]
async fn get_surface_state(
    host: HostState<'_>,
    binding: SurfaceBinding,
    key: String,
) -> Result<surface_state::SurfaceStateEntry, String> {
    let binding_for_validation = binding.clone();
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        kernel
            .require_open_surface(&binding_for_validation)
            .map_err(|error| error.to_string())
    })
    .await?;
    host.surface_state
        .lock()
        .map_err(|_| "surface state lock poisoned".to_string())?
        .get(&binding.app_id, &binding.surface, &key)
}

#[tauri::command]
async fn put_surface_state(
    host: HostState<'_>,
    binding: SurfaceBinding,
    key: String,
    expected_revision: u64,
    value: Option<JsonObject>,
) -> Result<surface_state::SurfaceStateEntry, String> {
    let binding_for_validation = binding.clone();
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        kernel
            .require_open_surface(&binding_for_validation)
            .map_err(|error| error.to_string())
    })
    .await?;
    host.surface_state
        .lock()
        .map_err(|_| "surface state lock poisoned".to_string())?
        .put(
            &binding.app_id,
            &binding.surface,
            &key,
            expected_revision,
            value,
        )
}

#[tauri::command]
async fn managed_data_request(
    host: HostState<'_>,
    binding: SurfaceBinding,
    request: managed_data::ManagedDataCommand,
) -> Result<Value, String> {
    let host = host.inner().clone();
    tokio::task::spawn_blocking(move || {
        // Keep the active package generation and live binding stable while
        // filesystem and schema work runs off the UI thread.
        let manager = host
            .app_manager
            .lock()
            .map_err(|_| "app manager lock poisoned".to_string())?;
        let data = manager.active_host_managed_data(binding.app_id.as_str())?;
        let contract = data
            .host_managed()
            .ok_or_else(|| "app does not declare host-managed data".to_string())?;
        let kernel = host
            .kernel
            .try_lock()
            .map_err(|_| "kernel busy".to_string())?;
        kernel
            .require_open_surface(&binding)
            .map_err(|error| error.to_string())?;
        match request {
            managed_data::ManagedDataCommand::V1(request) => host
                .managed_data
                .lock()
                .map_err(|_| "managed data lock poisoned".to_string())?
                .request(&binding.app_id, contract, request),
            managed_data::ManagedDataCommand::V1Tagged {
                contract_version,
                request,
            } => {
                if contract_version != 1 {
                    return Err(format!(
                        "unsupported managed-data command contract version {contract_version}"
                    ));
                }
                host.managed_data
                    .lock()
                    .map_err(|_| "managed data lock poisoned".to_string())?
                    .request(&binding.app_id, contract, request)
            }
            managed_data::ManagedDataCommand::V2 {
                contract_version,
                request,
            } => {
                if contract_version != 2 {
                    return Err(format!(
                        "unsupported managed-data command contract version {contract_version}"
                    ));
                }
                host.managed_data
                    .lock()
                    .map_err(|_| "managed data lock poisoned".to_string())?
                    .request_v2(&binding.app_id, contract, request)
            }
        }
    })
    .await
    .map_err(|error| format!("managed-data worker failed: {error}"))?
}

#[tauri::command]
async fn revoke_grant(host: HostState<'_>, grant_id: GrantId) -> Result<(), String> {
    with_kernel_blocking(host.inner().clone(), move |kernel| {
        kernel
            .revoke_grant(&grant_id)
            .map_err(|error| error.to_string())
    })
    .await
}

/// The user's answer to a trusted-chrome prompt.
#[tauri::command]
fn resolve_approval(host: HostState<'_>, request_id: u64, approved: bool) -> Result<(), String> {
    host.pending.resolve(request_id, approved)
}

/// The user's answer to a batched install checklist: one decision per grant
/// (aligned with the prompt's grants) plus the optional event-feed decision.
#[tauri::command]
fn resolve_install_approval(
    host: HostState<'_>,
    request_id: u64,
    event_approved: Option<bool>,
    grant_approvals: Vec<bool>,
) -> Result<(), String> {
    host.pending
        .resolve_install(request_id, event_approved, grant_approvals)
}

#[tauri::command]
async fn start_llm_oauth(host: HostState<'_>, connector_id: String) -> Result<String, String> {
    let profile = host
        .config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .llm_profile(&connector_id)?;
    if !profile.kind.defaults().oauth_credential_required {
        return Err(format!(
            "LLM profile '{connector_id}' is not an OAuth profile"
        ));
    }
    if profile
        .oauth_secret_ref
        .as_deref()
        .is_none_or(|name| name.trim().is_empty())
    {
        return Err(format!(
            "LLM profile '{connector_id}' has no OAuth secret reference"
        ));
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let (sender, controls) = std::sync::mpsc::channel();
    host.oauth
        .register(session_id.clone(), connector_id.clone(), sender)?;
    let host = host.inner().clone();
    let task_session_id = session_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = llm_client::run_oauth_login(
            &task_session_id,
            profile.kind,
            profile.base_url,
            &host.oauth,
            controls,
        );
        match result {
            Ok(credential) => {
                let persist = (|| {
                    let serialized = credential.serialize()?;
                    let mut kernel = host
                        .kernel
                        .lock()
                        .map_err(|_| "kernel lock poisoned".to_string())?;
                    host.config
                        .lock()
                        .map_err(|_| "config lock poisoned".to_string())?
                        .write_llm_profile_oauth_credential(&mut kernel, &connector_id, serialized)
                })();
                match persist {
                    Ok(()) => {
                        let _ = host.oauth.publish(OAuthPublicEvent::Completed {
                            session_id: task_session_id.clone(),
                        });
                    }
                    Err(error) => {
                        let _ = host.oauth.publish(OAuthPublicEvent::Failed {
                            session_id: task_session_id.clone(),
                            message: bounded_public_error(error),
                        });
                    }
                }
            }
            Err(error) => {
                let _ = host.oauth.publish(OAuthPublicEvent::Failed {
                    session_id: task_session_id.clone(),
                    message: bounded_public_error(error.to_string()),
                });
            }
        }
        host.oauth.finish(&task_session_id);
    });
    Ok(session_id)
}

fn bounded_public_error(mut value: String) -> String {
    const MAX_BYTES: usize = 16 * 1024;
    if value.len() > MAX_BYTES {
        let mut end = MAX_BYTES;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
    }
    value
}

#[tauri::command]
fn resolve_llm_oauth_prompt(
    host: HostState<'_>,
    session_id: String,
    prompt_id: String,
    value: Option<String>,
    cancelled: bool,
) -> Result<(), String> {
    host.oauth
        .resolve_prompt(&session_id, prompt_id, value, cancelled)
}

#[tauri::command]
fn cancel_llm_oauth(host: HostState<'_>, session_id: String) -> Result<(), String> {
    host.oauth.cancel(&session_id)
}

#[tauri::command]
fn list_trusted_notices(host: HostState<'_>) -> Result<Vec<TrustedNoticeRecord>, String> {
    host.notices
        .lock()
        .map_err(|_| "trusted notice store lock poisoned".to_string())
        .map(|inbox| inbox.recent())
}

#[tauri::command]
fn list_publisher_trust(host: HostState<'_>) -> Result<Vec<TrustRecord>, String> {
    let manager = host
        .app_manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?;
    Ok(manager.list_trusted_publishers())
}

#[tauri::command]
fn list_managed_app_revisions(
    host: HostState<'_>,
    app_id: String,
) -> Result<Vec<app_manager::AppRevision>, String> {
    let manager = host
        .app_manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?;
    manager.managed_app_revisions(&app_id)
}

#[tauri::command]
fn trust_publisher_key(
    host: HostState<'_>,
    request: TrustKeyRequest,
) -> Result<Vec<TrustRecord>, String> {
    let mut manager = host
        .app_manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?;
    manager.trust_publisher_key(&request.key_id, &request.public_key, request.scope)
}

#[tauri::command]
fn revoke_publisher_key(
    host: HostState<'_>,
    request: RevokeKeyRequest,
) -> Result<Vec<TrustRecord>, String> {
    let mut manager = host
        .app_manager
        .lock()
        .map_err(|_| "app manager lock poisoned".to_string())?;
    manager.revoke_publisher_key(&request.key_id, &request.scope)
}

#[cfg(test)]
pub(crate) fn build_host(
    paths: impl Into<HostPaths>,
    chrome: Arc<dyn app_host_kernel::services::chrome::TrustedChrome>,
    pending: Arc<PendingApprovals>,
    notices: Arc<Mutex<TrustedNoticeStore>>,
) -> Result<Arc<Host>, String> {
    let paths = paths.into();
    let profile_lock = kernel_state::ProfileLock::acquire(paths.kernel_state_path())?;
    profile_migration::run(&paths)?;
    build_host_with_lock(paths, profile_lock, chrome, pending, notices)
}

fn build_host_with_lock(
    paths: HostPaths,
    profile_lock: Arc<kernel_state::ProfileLock>,
    chrome: Arc<dyn app_host_kernel::services::chrome::TrustedChrome>,
    pending: Arc<PendingApprovals>,
    notices: Arc<Mutex<TrustedNoticeStore>>,
) -> Result<Arc<Host>, String> {
    let profiles = Arc::new(Mutex::new(ProfileRegistryService::open(
        paths.default_root().to_path_buf(),
    )?));
    let config = HostConfigService::new_with_namespace(
        paths.config_path().to_path_buf(),
        paths.profile_id().to_string(),
    )?;
    let chat_store = ChatStore::new(paths.chat_store_path().to_path_buf())?;
    let file_resources = Arc::new(Mutex::new(FileResourceRegistryService::new(
        file_resource_registry_path(paths.root()).to_path_buf(),
    )?));
    let app_manager = AppManager::new(
        paths.app_store_path().to_path_buf(),
        paths.trust_store_path().to_path_buf(),
        paths.app_records_root().to_path_buf(),
        paths.update_journal_path().to_path_buf(),
        paths.allow_unsafe_native_backends(),
    )?;
    let state_store =
        kernel_state::FileKernelStateStore::open_with_lock(paths.kernel_state_path(), profile_lock);
    let mut kernel =
        Kernel::with_state_store(chrome, state_store).map_err(|error| error.to_string())?;
    // MCP connections are explicitly user-initiated and their handlers are
    // process-local. Remove durable bridged principals on restart instead of
    // exposing a stale app with grants but no live transport.
    let stale_mcp_apps: Vec<AppId> = kernel
        .installed_apps()
        .filter_map(|app| {
            let id = app.manifest.app_id.as_str();
            (id.starts_with("mcp-") && !id.starts_with("mcp-export/"))
                .then(|| app.manifest.app_id.clone())
        })
        .collect();
    for app_id in stale_mcp_apps {
        kernel
            .uninstall(&app_id)
            .map_err(|error| format!("remove stale MCP app '{}': {error}", app_id))?;
    }
    let kernel = Arc::new(Mutex::new(kernel));
    let kernel_invoker = agent_worker::KernelInvokerClient::spawn(kernel.clone());
    let surface_ui = SurfaceUiRegistry::new();
    {
        let mut kernel_guard = kernel
            .lock()
            .map_err(|_| "kernel lock poisoned".to_string())?;
        file_resources
            .lock()
            .map_err(|_| "file resource registry lock poisoned".to_string())?
            .reconcile_with_kernel(&mut kernel_guard)?;
    }
    Ok(Arc::new(Host {
        kernel,
        kernel_invoker,
        config: Arc::new(Mutex::new(config)),
        chat_store: Arc::new(Mutex::new(chat_store)),
        active_chat_sends: Arc::new(Mutex::new(std::collections::HashMap::new())),
        pending,
        oauth: Arc::new(PendingOAuthSessions::default()),
        notices,
        profiles,
        file_resources,
        startup_apps_installed: Mutex::new(false),
        mcp_export_transition: tauri::async_runtime::Mutex::new(()),
        mcp_connections: Arc::new(McpConnections::default()),
        mcp_gateway: Mutex::new(None),
        mcp_audit: Arc::new(AuditLog::new(Some(paths.mcp_audit_path().to_path_buf()))),
        surface_ui: Arc::new(Mutex::new(surface_ui)),
        surface_state: Arc::new(Mutex::new(surface_state::SurfaceStateStore::new(
            surface_state::data_root(paths.app_records_root()),
        ))),
        managed_data: Arc::new(Mutex::new(managed_data::ManagedDataStore::new(
            managed_data::data_root(paths.app_records_root()),
        ))),
        app_manager: Arc::new(Mutex::new(app_manager)),
        managed_app_transition: tauri::async_runtime::Mutex::new(()),
        paths,
        file_resource_transition: tauri::async_runtime::Mutex::new(()),
    }))
}

fn setup_host(app: &tauri::App) -> Result<(), String> {
    let pending = Arc::new(PendingApprovals::default());
    let app_config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?;
    std::env::set_var("KESTRAL_WORKER_RESOURCE_DIR", resource_dir);
    let registry_lock = kernel_state::ProfileRegistryLock::acquire(&app_config_dir)?;
    let host_paths = HostPaths::resolve_startup(app_config_dir)?;
    let profile_lock = kernel_state::ProfileLock::acquire_for_startup(
        host_paths.kernel_state_path(),
        registry_lock,
    )?;
    profile_migration::run(&host_paths)?;
    system_reset::apply_pending(&host_paths)?;
    let notices = Arc::new(Mutex::new(
        TrustedNoticeStore::new(host_paths.notices_path().to_path_buf())
            .map_err(|error| error.to_string())?,
    ));
    let shell_chrome = ShellChrome::new(app.handle().clone(), pending.clone(), notices.clone());
    let host = build_host_with_lock(
        host_paths,
        profile_lock,
        Arc::new(shell_chrome),
        pending,
        notices,
    )?;
    let app_handle = app.handle().clone();
    host.oauth.set_publisher(Arc::new(move |event| {
        app_handle
            .emit(CHROME_OAUTH_EVENT, event)
            .map_err(|error| format!("emit OAuth event failed: {error}"))
    }))?;
    app.manage(host);
    Ok(())
}

fn startup_failure_message(error: &str) -> String {
    format!(
        "Kestral could not start:\n\n{error}\n\nIf a persisted file failed to load, it is \
         persisted state that this version does not read. Delete the named file (or the profile \
         data directory) and start Kestral again."
    )
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();

    #[cfg(all(debug_assertions, feature = "dev-mcp"))]
    let builder = builder.plugin(
        tauri_plugin_mcp_bridge::Builder::new()
            .bind_address("127.0.0.1")
            .build(),
    );

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            if let Err(error) = setup_host(app) {
                use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

                let message = startup_failure_message(&error);
                eprintln!("Kestral startup failed: {error}");
                let app_handle = app.handle().clone();
                app.dialog()
                    .message(message)
                    .title("Kestral failed to start")
                    .kind(MessageDialogKind::Error)
                    // Tauri setup runs on the main thread; blocking_show freezes GTK here.
                    .show(move |_| app_handle.exit(1));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            available_capabilities_for,
            validate_extension_context,
            bootstrap_startup_apps,
            attach_chat_artifact,
            clear_secret,
            connect_mcp_server,
            create_chat_thread,
            create_kestral_profile,
            cancel_chat_message,
            cancel_llm_oauth,
            cancel_surface_action,
            delete_chat_thread,
            delete_connector_config,
            delete_kestral_profile,
            delete_mcp_server,
            clear_mcp_http_auth_secret,
            disconnect_mcp_server,
            discover_connector_models_draft,
            get_chat_thread,
            get_chat_prompt_preview,
            list_chat_profiles,
            list_chat_model_profiles,
            list_chat_agent_engines,
            set_chat_model_profile,
            set_chat_thread_profile,
            set_chat_agent_engine,
            remove_chat_contribution,
            get_config_storage_info,
            get_active_kestral_profile,
            get_app_config,
            get_host_config,
            has_secret,
            has_mcp_http_auth_secret,
            grant_artifact_access,
            grant_file_resource_access,
            list_file_resources,
            list_kestral_profiles,
            list_trusted_file_resources,
            list_apps,
            list_trusted_notices,
            list_chat_threads,
            list_connector_configs,
            list_grants,
            list_mcp_servers,
            list_mcp_export_profiles,
            ledger_records,
            list_artifacts,
            list_app_artifacts,
            app_surface_events,
            get_surface_ui,
            get_surface_state,
            managed_data_request,
            open_surface,
            close_surface,
            list_installed_apps,
            list_publisher_trust,
            list_managed_app_revisions,
            inspect_package,
            inspect_git_package,
            plan_managed_app_transition,
            apply_managed_app_transition,
            install_app,
            set_app_enabled,
            trust_publisher_key,
            uninstall_app,
            put_secret,
            put_mcp_http_auth_secret,
            put_surface_state,
            request_app_grants,
            request_manifest_grant,
            submit_permission_proposal,
            request_system_reset,
            rename_chat_thread,
            submit_action,
            submit_action_with_progress,
            revoke_grant,
            revoke_publisher_key,
            issue_editor_grant,
            replace_grant,
            resolve_approval,
            resolve_install_approval,
            resolve_llm_oauth_prompt,
            register_file_resource,
            send_chat_message,
            start_llm_oauth,
            test_connector_config,
            start_mcp_gateway,
            stop_mcp_gateway,
            mcp_gateway_status,
            mcp_export_recent_activity,
            remove_file_resource,
            update_app_config,
            update_host_config,
            upsert_connector_config,
            upsert_mcp_server,
            upsert_mcp_export_profile,
            set_mcp_export_enabled,
            delete_mcp_export_profile,
            rotate_mcp_export_token,
            revoke_mcp_export_token,
            has_mcp_export_token,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests;
