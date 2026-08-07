//! The Kernel facade: the one authoritative action path.
//!
//! ```text
//! surface → kernel: action request
//! kernel  → permission broker: check grant
//! kernel  → (trusted chrome: user approval, if required)
//! kernel  → capability invocation
//! kernel  → run ledger: record
//! kernel  → surface: result
//! ```
//!
//! Every way of doing work — chat message, surface button, automation
//! firing, child run — converges on the phased invocation API. Grant checks,
//! approvals, notices, provenance stamping, and ledger records happen here
//! and nowhere else, so correctness is not opt-in for apps.
//!
//! This is the entire public API. Bundled apps (chat included) and
//! third-party apps use exactly these methods — no privileged side door
//! exists to give out (architecture acceptance criteria 1 and 3).

use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration as StdDuration, Instant};

use chrono::Duration;
use serde::{Deserialize, Serialize};

use crate::clock::{Clock, SystemClock};
use crate::durable::{CommitOutcome, DurableKernelState, KernelStateStore};
use crate::errors::{KernelError, KernelResult};
use crate::ids::{
    new_artifact_id, new_run_id, AppId, ArtifactId, CapabilityName, GrantId, ResourceId, RunId,
    SecretRef, SurfaceName,
};
use crate::invocation::{
    CancellationHandle, CapabilityHandler, CapabilityOutcome, HandlerFailure, InvocationContext,
    InvocationRequest, InvocationResult, ProgressReporter, RefusalReason,
};
use crate::manifest::{GrantRequest, SealedManifest};
use crate::primitives::artifact::{Artifact, ArtifactDraft, Provenance};
use crate::primitives::capability::{CapabilityDeclaration, CapabilityRef};
use crate::primitives::grant::{
    DataScope, DenialReason, Grant, GrantCondition, GrantDuration, GrantOrigin, GrantScope,
    GrantStatusView,
};
use crate::primitives::run::{Initiator, RunTerminalState, RunView};
use crate::primitives::surface::ActionIntent;
use crate::schema::{validate_against_schema, SchemaViolation};
use crate::services::artifacts::ArtifactStore;
use crate::services::broker::{GrantCheck, IssueResult, PermissionBroker, SecretResolver};
use crate::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, EventSubscriptionPrompt,
    GrantIssuancePrompt, InstallApprovalDecision, InstallApprovalPrompt, TrustedChrome,
};
use crate::services::ledger::{payload_sha256, LedgerEvent, LedgerRecord, RunLedger};
use crate::services::registry::{InstalledApp, Registry};
use crate::services::router::{
    AppDataChangeKind, AppEventEnvelope, EventInboxStatus, LeaseManager, LeaseOutcome, LeaseTarget,
    MessageRouter,
};
use crate::services::surfaces::{SurfaceBinding, SurfaceManager};
use crate::JsonObject;

/// A grant-aware view of one capability as seen by a consuming app.
///
/// Returned by [`Kernel::available_capabilities_for`]: the consumer sees only
/// capabilities it has a live covering grant for, annotated with the
/// effective grant condition and the exact live data scopes under which it is
/// usable right now.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityUseView {
    pub provider_app_id: AppId,
    pub provider_display_name: String,
    pub capability: CapabilityName,
    pub description: String,
    pub input_schema: JsonObject,
    pub authorizations: Vec<CapabilityAuthorizationView>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityAuthorizationView {
    pub data_scope: DataScope,
    pub condition: GrantCondition,
}

fn refusal_for(denial: DenialReason) -> RefusalReason {
    match denial {
        DenialReason::NoGrant => RefusalReason::NoGrant,
        DenialReason::Expired => RefusalReason::GrantExpired,
        DenialReason::Revoked => RefusalReason::GrantRevoked,
    }
}

/// Best-effort text of a caught handler panic; payloads are conventionally
/// `&str` or `String`, anything else is reported as opaque.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> &str {
    if let Some(message) = panic.downcast_ref::<&str>() {
        message
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message
    } else {
        "non-string panic payload"
    }
}

const DEFAULT_INVOCATION_TIMEOUT: StdDuration = StdDuration::from_secs(60);
static NEXT_INVOCATION_ID: AtomicU64 = AtomicU64::new(1);

fn condition_rank(condition: GrantCondition) -> u8 {
    match condition {
        GrantCondition::Silent => 0,
        GrantCondition::Notify => 1,
        GrantCondition::RequiresApproval => 2,
    }
}

fn grant_duration_matches(grant: &Grant, duration: GrantDuration) -> bool {
    match (duration, grant.expires_at) {
        (GrantDuration::NonExpiring, None) => true,
        (GrantDuration::ExpiresAfter { seconds }, Some(expires_at)) => {
            expires_at == grant.issued_at + chrono::Duration::seconds(i64::from(seconds.get()))
        }
        _ => false,
    }
}

/// ```compile_fail
/// use app_host_kernel::PreparedInvocation;
/// fn duplicate(token: PreparedInvocation) { let _ = token.clone(); }
/// ```
pub struct PreparedInvocation {
    id: u64,
    approval: Option<CapabilityApprovalPrompt>,
    chrome: Arc<dyn TrustedChrome>,
    cancelled: Arc<AtomicBool>,
    cancel_on_drop: bool,
}

/// A validated app install whose user prompts can be answered without the
/// kernel mutex. The commit phase revalidates the manifest before changing
/// kernel state.
pub struct PreparedInstall {
    sealed_manifest: SealedManifest,
    handlers: BTreeMap<CapabilityName, CapabilityHandler>,
    grant_origin: GrantOrigin,
    mode: InstallMode,
    event_prompt: Option<EventSubscriptionPrompt>,
    grant_prompts: Vec<GrantIssuancePrompt>,
    provider_content_hashes: BTreeMap<AppId, Option<String>>,
    chrome: Arc<dyn TrustedChrome>,
}

enum InstallMode {
    Fresh,
    Rebind,
    Replace { previous_content_hash: String },
}

/// How grant-request validation treats a request whose target provider is not
/// in the registry. Fresh installs reject it; durable recovery tolerates it as
/// a dormant request so an uninstalled provider cannot block boot.
#[derive(Clone, Copy)]
enum AbsentProvider {
    Reject,
    Tolerate,
}

impl AbsentProvider {
    /// Pass through a provider-lookup result, swallowing only "provider absent"
    /// when tolerated. A present-but-incompatible provider still fails.
    fn tolerate(self, lookup: KernelResult<()>) -> KernelResult<()> {
        match (self, lookup) {
            (AbsentProvider::Tolerate, Err(KernelError::UnknownApp(_))) => Ok(()),
            (_, result) => result,
        }
    }
}

/// Trusted-chrome decisions for one [`PreparedInstall`]. This is single-use
/// and can only be consumed by [`Kernel::commit_install`].
pub struct InstallApproval {
    prepared: PreparedInstall,
    event_decision: Option<ApprovalDecision>,
    grant_decisions: Vec<ApprovalDecision>,
}

/// A validated user-added grant whose approval can be collected without the
/// kernel mutex.
pub struct PreparedGrant {
    holder: AppId,
    holder_display_name: String,
    holder_content_hash: String,
    provider_content_hash: String,
    request: GrantRequest,
    chrome: Arc<dyn TrustedChrome>,
}

pub struct GrantApproval {
    prepared: PreparedGrant,
    decision: ApprovalDecision,
}

impl PreparedInstall {
    /// Wait for all install prompts without a kernel reference or mutex.
    pub fn await_approval(self) -> InstallApproval {
        let prompt = InstallApprovalPrompt {
            app_id: self.sealed_manifest.manifest.app_id.clone(),
            app_display_name: self.sealed_manifest.manifest.display_name.clone(),
            event: self.event_prompt.clone(),
            grants: self.grant_prompts.clone(),
        };
        let InstallApprovalDecision {
            event_decision,
            grant_decisions,
        } = self.chrome.confirm_install(prompt);
        InstallApproval {
            prepared: self,
            event_decision,
            grant_decisions,
        }
    }
}

impl PreparedGrant {
    pub fn await_approval(self) -> GrantApproval {
        let prompt = GrantIssuancePrompt {
            app_id: self.holder.clone(),
            app_display_name: self.holder_display_name.clone(),
            scope: self.request.scope.clone(),
            data_scope: self.request.data_scope.clone(),
            condition: self.request.condition,
            duration: self.request.duration,
            reason: self.request.reason.clone(),
        };
        let decision = self.chrome.confirm_grant(prompt);
        GrantApproval {
            prepared: self,
            decision,
        }
    }

    /// Collect several standing-permission decisions through one trusted
    /// checklist. Interactive shells can present the whole set in one modal;
    /// non-interactive chromes retain their per-grant behavior through
    /// `TrustedChrome::confirm_install`'s default implementation.
    pub fn await_grouped_approvals(prepared: Vec<Self>) -> KernelResult<Vec<GrantApproval>> {
        let Some(first) = prepared.first() else {
            return Ok(Vec::new());
        };
        if prepared.iter().any(|grant| {
            grant.holder != first.holder
                || grant.holder_content_hash != first.holder_content_hash
                || !Arc::ptr_eq(&grant.chrome, &first.chrome)
        }) {
            return Err(KernelError::PreparedGrantGroupMismatch);
        }
        let prompt = InstallApprovalPrompt {
            app_id: first.holder.clone(),
            app_display_name: first.holder_display_name.clone(),
            event: None,
            grants: prepared
                .iter()
                .map(|grant| GrantIssuancePrompt {
                    app_id: grant.holder.clone(),
                    app_display_name: grant.holder_display_name.clone(),
                    scope: grant.request.scope.clone(),
                    data_scope: grant.request.data_scope.clone(),
                    condition: grant.request.condition,
                    duration: grant.request.duration,
                    reason: grant.request.reason.clone(),
                })
                .collect(),
        };
        let decisions = first.chrome.confirm_install(prompt).grant_decisions;
        Ok(prepared
            .into_iter()
            .enumerate()
            .map(|(index, prepared)| GrantApproval {
                prepared,
                decision: decisions
                    .get(index)
                    .copied()
                    .unwrap_or(ApprovalDecision::Denied),
            })
            .collect())
    }
}

/// An opaque proof that trusted chrome has answered. It can only be consumed
/// by [`Kernel::authorize_invocation`], which revalidates authority before
/// handing provider code an execution token.
///
/// ```compile_fail
/// use app_host_kernel::ApprovalResult;
/// fn duplicate(token: ApprovalResult) { let _ = token.clone(); }
/// ```
pub struct ApprovalResult {
    id: u64,
    decision: Option<ApprovalDecision>,
    cancelled: Arc<AtomicBool>,
    cancel_on_drop: bool,
}

/// An opaque, single-use execution token. Its handler runs outside the host
/// kernel mutex only after the kernel atomically authorizes dispatch.
pub struct AuthorizedInvocation {
    id: u64,
    handler: Arc<CapabilityHandler>,
    input: JsonObject,
    context: Box<InvocationContext>,
    cancelled: Arc<AtomicBool>,
    cancel_on_drop: bool,
}

/// ```compile_fail
/// use app_host_kernel::ExecutedInvocation;
/// fn finalize_twice(token: ExecutedInvocation) {
///     consume(token);
///     consume(token);
/// }
/// fn consume(_: ExecutedInvocation) {}
/// ```
pub struct ExecutedInvocation {
    id: u64,
    outcome: Option<Result<CapabilityOutcome, HandlerFailure>>,
    panic: Option<String>,
    cancelled: Arc<AtomicBool>,
    cancel_on_drop: bool,
}

struct PendingInvocation {
    run_id: RunId,
    acting_app: AppId,
    capability: CapabilityRef,
    grant: Grant,
    requested_data_scope: DataScope,
    provider_content_hash: String,
    output_schema: Option<JsonObject>,
    input: JsonObject,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
    state: PendingInvocationState,
}

enum PendingInvocationState {
    PreparedForApproval,
    AuthorizedForExecution,
}

impl PreparedInvocation {
    /// Wait for trusted chrome without a kernel reference or mutex. Consuming
    /// this token prevents a caller from submitting the same approval twice.
    pub fn await_approval(mut self) -> ApprovalResult {
        let decision = self
            .approval
            .as_ref()
            .map(|prompt| self.chrome.approve_capability(prompt.clone()));
        self.cancel_on_drop = false;
        ApprovalResult {
            id: self.id,
            decision,
            cancelled: self.cancelled.clone(),
            cancel_on_drop: true,
        }
    }
}

impl Drop for PreparedInvocation {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.cancelled.store(true, Ordering::Release);
        }
    }
}

impl Drop for ApprovalResult {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.cancelled.store(true, Ordering::Release);
        }
    }
}

impl AuthorizedInvocation {
    /// Execute provider code after kernel authorization. Cancellation is
    /// cooperative: code that already began may still cause external effects,
    /// but its late result is never committed after cancellation.
    pub fn execute(self) -> ExecutedInvocation {
        self.execute_with_progress(ProgressReporter::default())
    }

    pub fn execute_with_progress(mut self, progress: ProgressReporter) -> ExecutedInvocation {
        self.context.progress = progress.with_cancellation(self.context.cancellation.clone());
        let executed = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (self.handler)(&self.input, &self.context)
        })) {
            Ok(outcome) => ExecutedInvocation {
                id: self.id,
                outcome: Some(outcome),
                panic: None,
                cancelled: self.cancelled.clone(),
                cancel_on_drop: true,
            },
            Err(panic) => ExecutedInvocation {
                id: self.id,
                outcome: None,
                panic: Some(format!("handler panicked: {}", panic_message(&*panic))),
                cancelled: self.cancelled.clone(),
                cancel_on_drop: true,
            },
        };
        self.cancel_on_drop = false;
        executed
    }
}

impl Drop for AuthorizedInvocation {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.context.cancellation.cancel();
        }
    }
}

impl Drop for ExecutedInvocation {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.cancelled.store(true, Ordering::Release);
        }
    }
}

/// What flows back to a surface: the run it caused and its result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceActionOutcome {
    pub run_id: RunId,
    pub result: InvocationResult,
}

/// The first phase either yields work that may execute outside the host mutex
/// or an already-recorded refusal.
pub enum PrepareInvocation {
    Prepared(PreparedInvocation),
    Refused(InvocationResult),
}

/// The authorization phase either yields the only token that can execute a
/// handler or records a refusal without dispatching provider code.
pub enum AuthorizeInvocation {
    Authorized(AuthorizedInvocation),
    Refused(InvocationResult),
}

/// Composition root wiring the five services; owns the action path.
///
/// Every service is private: kernel state is only reachable through the
/// methods below, so provenance stamping, attribution checks, and ledger
/// discipline cannot be bypassed by an embedder holding a `Kernel`.
///
/// Trust boundary: attribution (`Initiator`, run/app identity on
/// `start_run`/`end_run`/`invoke`) is caller-asserted. The isolation
/// invariants hold against capability *handlers* — which never receive a
/// kernel handle — not against in-process embedder code, which is trusted
/// by construction in the single-process host.
pub struct Kernel {
    clock: Arc<dyn Clock>,
    chrome: Arc<dyn TrustedChrome>,
    registry: Registry,
    ledger: RunLedger,
    broker: PermissionBroker,
    surfaces: SurfaceManager,
    router: MessageRouter,
    leases: LeaseManager,
    artifacts: ArtifactStore,
    handlers: BTreeMap<AppId, BTreeMap<CapabilityName, Arc<CapabilityHandler>>>,
    pending_invocations: BTreeMap<u64, PendingInvocation>,
    state_store: Option<Arc<dyn KernelStateStore>>,
    recovery_required: AtomicBool,
}

impl Kernel {
    pub fn new(chrome: Arc<dyn TrustedChrome>) -> Self {
        Self::with_clock(chrome, Arc::new(SystemClock))
    }

    pub fn with_clock(chrome: Arc<dyn TrustedChrome>, clock: Arc<dyn Clock>) -> Self {
        Self {
            clock: clock.clone(),
            chrome: chrome.clone(),
            registry: Registry::new(clock.clone()),
            ledger: RunLedger::new(clock.clone()),
            broker: PermissionBroker::new(clock.clone(), chrome),
            surfaces: SurfaceManager::new(),
            router: MessageRouter::new(),
            leases: LeaseManager::new(clock),
            artifacts: ArtifactStore::new(),
            handlers: BTreeMap::new(),
            pending_invocations: BTreeMap::new(),
            state_store: None,
            recovery_required: AtomicBool::new(false),
        }
    }

    pub fn with_state_store(
        chrome: Arc<dyn TrustedChrome>,
        store: Arc<dyn KernelStateStore>,
    ) -> KernelResult<Self> {
        Self::with_clock_and_state_store(chrome, Arc::new(SystemClock), store)
    }

    pub fn with_clock_and_state_store(
        chrome: Arc<dyn TrustedChrome>,
        clock: Arc<dyn Clock>,
        store: Arc<dyn KernelStateStore>,
    ) -> KernelResult<Self> {
        let state = store.load().map_err(KernelError::Durability)?;
        let mut kernel = Self::with_clock(chrome.clone(), clock.clone());
        kernel.state_store = Some(store);
        if let Some(state) = state {
            kernel.registry = Registry::restore(clock.clone(), state.installed_apps)?;
            kernel.ledger = RunLedger::restore(clock.clone(), state.ledger_records)?;
            kernel.broker =
                PermissionBroker::restore(clock, chrome, state.grants, state.revoked_grant_ids)?;
            kernel.artifacts = ArtifactStore::restore(state.artifacts)?;
            let recovered_manifests: Vec<_> = kernel
                .registry
                .installed_apps()
                .map(|app| app.manifest.clone())
                .collect();
            for manifest in &recovered_manifests {
                kernel.validate_grant_requests(manifest, AbsentProvider::Tolerate)?;
            }
            kernel.validate_recovered_state()?;

            let active = kernel.ledger.active_run_ids();
            if !active.is_empty() {
                let mut events = kernel.ledger.incomplete_invocation_cancellations()?;
                events.extend(active.into_iter().map(|run_id| LedgerEvent::RunEnded {
                    run_id,
                    terminal_state: RunTerminalState::Interrupted,
                }));
                kernel.record_batch(events)?;
            }
        }
        Ok(kernel)
    }

    // -- app lifecycle ------------------------------------------------------

    /// Validate an install and collect the prompts required to authorize it.
    /// No kernel state is changed, so the returned plan may wait on trusted
    /// chrome after the host releases its kernel mutex.
    pub fn prepare_install(
        &self,
        sealed_manifest: SealedManifest,
        handlers: BTreeMap<CapabilityName, CapabilityHandler>,
    ) -> KernelResult<PreparedInstall> {
        self.prepare_install_with_grant_origin(
            sealed_manifest,
            handlers,
            GrantOrigin::ManifestRequested,
        )
    }

    pub fn prepare_install_with_grant_origin(
        &self,
        sealed_manifest: SealedManifest,
        handlers: BTreeMap<CapabilityName, CapabilityHandler>,
        grant_origin: GrantOrigin,
    ) -> KernelResult<PreparedInstall> {
        let manifest = &sealed_manifest.manifest;
        let declared: Vec<String> = manifest
            .capabilities
            .iter()
            .map(|capability| capability.name.to_string())
            .collect();
        let offered: Vec<String> = handlers.keys().map(ToString::to_string).collect();
        if declared.iter().collect::<std::collections::BTreeSet<_>>()
            != offered.iter().collect::<std::collections::BTreeSet<_>>()
        {
            return Err(KernelError::HandlerBindingMismatch {
                app: manifest.app_id.clone(),
                declared,
                offered,
            });
        }

        let mode = match self.registry.app(&manifest.app_id) {
            Ok(_) if self.handlers.contains_key(&manifest.app_id) => {
                return Err(KernelError::AppAlreadyInstalled(manifest.app_id.clone()));
            }
            Ok(installed)
                if installed.content_hash == sealed_manifest.content_hash
                    && installed.manifest == sealed_manifest.manifest =>
            {
                InstallMode::Rebind
            }
            // The first arm already rejected live duplicates, so reaching here
            // means the app is registered but has no bound handlers (restored
            // dormant state) and the manifest changed: replace it.
            Ok(installed) => {
                let mut registry = self.registry.clone();
                registry.uninstall(&manifest.app_id)?;
                registry.validate_install(&sealed_manifest)?;
                self.validate_install_grant_requests(manifest)?;
                InstallMode::Replace {
                    previous_content_hash: installed.content_hash.clone(),
                }
            }
            Err(KernelError::UnknownApp(_)) => {
                self.registry.validate_install(&sealed_manifest)?;
                self.validate_install_grant_requests(manifest)?;
                InstallMode::Fresh
            }
            Err(error) => return Err(error),
        };

        let prompt_for = |request: &GrantRequest| GrantIssuancePrompt {
            app_id: manifest.app_id.clone(),
            app_display_name: manifest.display_name.clone(),
            scope: request.scope.clone(),
            data_scope: request.data_scope.clone(),
            condition: request.condition,
            duration: request.duration,
            reason: request.reason.clone(),
        };
        // A manifest may list the same authority twice. Those requests are
        // indistinguishable — issuing one grant satisfies both — so asking the
        // user twice can only produce an unanswerable outcome (approve one,
        // deny its twin) that the commit path then has to guess at. Collapse
        // them to one prompt per distinct authority, which also makes the
        // by-value prompt/decision lookup at commit time unambiguous.
        let dedup_prompts = |prompts: Vec<GrantIssuancePrompt>| -> Vec<GrantIssuancePrompt> {
            let mut unique: Vec<GrantIssuancePrompt> = Vec::with_capacity(prompts.len());
            for prompt in prompts {
                let already_asked = unique.iter().any(|seen| {
                    seen.scope == prompt.scope
                        && seen.data_scope == prompt.data_scope
                        && seen.condition == prompt.condition
                        && seen.duration == prompt.duration
                });
                if !already_asked {
                    unique.push(prompt);
                }
            }
            unique
        };
        let (event_prompt, grant_prompts) = match mode {
            InstallMode::Fresh | InstallMode::Replace { .. } => {
                let event_prompt =
                    (!manifest.event_subscriptions.is_empty()).then(|| EventSubscriptionPrompt {
                        app_id: manifest.app_id.clone(),
                        app_display_name: manifest.display_name.clone(),
                        topics: manifest.event_subscriptions.clone(),
                    });
                let grant_prompts =
                    dedup_prompts(manifest.grant_requests.iter().map(prompt_for).collect());
                (event_prompt, grant_prompts)
            }
            // Rebind restores a dormant app to its declared authority. A
            // manifest grant that has no active grant behind it (an install
            // prompt lost to a startup race, a revocation, an expiry) is
            // re-requested through trusted chrome instead of silently staying
            // absent until someone notices in Settings. With intact authority
            // this collects no prompts, so a normal restart stays silent.
            InstallMode::Rebind => {
                let grant_prompts = manifest
                    .grant_requests
                    .iter()
                    .filter(|request| {
                        request.scope.provider() == &manifest.app_id
                            || self.registry.app(request.scope.provider()).is_ok()
                    })
                    .filter(|request| {
                        self.manifest_grant_for(&manifest.app_id, request, grant_origin)
                            .is_none()
                    })
                    .map(prompt_for)
                    .collect();
                (None, dedup_prompts(grant_prompts))
            }
        };

        let provider_content_hashes = self.provider_content_hashes_for(
            manifest,
            if matches!(&mode, InstallMode::Rebind) {
                AbsentProvider::Tolerate
            } else {
                AbsentProvider::Reject
            },
        )?;
        Ok(PreparedInstall {
            sealed_manifest,
            handlers,
            grant_origin,
            mode,
            event_prompt,
            grant_prompts,
            provider_content_hashes,
            chrome: self.chrome.clone(),
        })
    }

    /// Commit decisions collected by [`Kernel::prepare_install`]. All
    /// authority and manifest state is checked again before the atomic commit.
    pub fn commit_install(&mut self, approval: InstallApproval) -> KernelResult<Vec<IssueResult>> {
        let InstallApproval {
            prepared:
                PreparedInstall {
                    sealed_manifest,
                    handlers,
                    grant_origin,
                    mode,
                    event_prompt,
                    grant_prompts,
                    provider_content_hashes,
                    chrome: _,
                },
            event_decision,
            grant_decisions,
        } = approval;
        let manifest = &sealed_manifest.manifest;
        self.require_provider_content_hashes(&provider_content_hashes)?;
        let app_id = manifest.app_id.clone();
        let grant_requests = manifest.grant_requests.clone();

        let declared: Vec<String> = manifest
            .capabilities
            .iter()
            .map(|capability| capability.name.to_string())
            .collect();
        let offered: Vec<String> = handlers.keys().map(ToString::to_string).collect();
        if declared.iter().collect::<std::collections::BTreeSet<_>>()
            != offered.iter().collect::<std::collections::BTreeSet<_>>()
        {
            return Err(KernelError::HandlerBindingMismatch {
                app: app_id,
                declared,
                offered,
            });
        }

        match mode {
            InstallMode::Rebind => {
                let installed = self.registry.app(&app_id)?;
                if installed.content_hash != sealed_manifest.content_hash
                    || installed.manifest != sealed_manifest.manifest
                    || self.handlers.contains_key(&app_id)
                {
                    return Err(KernelError::PreparedInstallStale);
                }
                self.validate_grant_requests(manifest, AbsentProvider::Tolerate)?;
                if grant_decisions.len() != grant_prompts.len() {
                    return Err(KernelError::PreparedInstallStale);
                }

                // Reconcile authority: keep every request's existing active
                // grant; for requests that prepared a re-request prompt, issue
                // per the collected decision. A request with neither (its
                // grant vanished between prepare and commit) stays refused —
                // authority is never issued without a chrome decision — and
                // the next rebind re-requests it.
                let mut broker = self.broker.clone();
                let (results, issued_any) = Self::reconcile_manifest_grants(
                    &mut broker,
                    &app_id,
                    &grant_requests,
                    grant_origin,
                    &grant_prompts,
                    &grant_decisions,
                );
                if issued_any {
                    self.commit_parts(&self.registry, &self.ledger, &broker, &self.artifacts)?;
                    self.broker = broker;
                }
                self.handlers.insert(
                    app_id.clone(),
                    handlers
                        .into_iter()
                        .map(|(name, handler)| (name, Arc::new(handler)))
                        .collect(),
                );
                Ok(results)
            }
            InstallMode::Fresh => {
                if self.registry.app(&app_id).is_ok() {
                    return Err(KernelError::PreparedInstallStale);
                }
                self.registry.validate_install(&sealed_manifest)?;
                self.validate_install_grant_requests(manifest)?;
                if event_prompt.is_some() && event_decision != Some(ApprovalDecision::Approved) {
                    return Err(KernelError::EventSubscriptionDenied(app_id));
                }
                if grant_decisions.len() != grant_prompts.len() {
                    return Err(KernelError::PreparedInstallStale);
                }

                let mut registry = self.registry.clone();
                let mut broker = self.broker.clone();
                registry.install(sealed_manifest)?;
                let (results, _) = Self::reconcile_manifest_grants(
                    &mut broker,
                    &app_id,
                    &grant_requests,
                    grant_origin,
                    &grant_prompts,
                    &grant_decisions,
                );
                self.commit_parts(&registry, &self.ledger, &broker, &self.artifacts)?;
                self.registry = registry;
                self.broker = broker;
                self.handlers.insert(
                    app_id,
                    handlers
                        .into_iter()
                        .map(|(name, handler)| (name, Arc::new(handler)))
                        .collect(),
                );
                Ok(results)
            }
            InstallMode::Replace {
                previous_content_hash,
            } => self.replace_installed_app(
                sealed_manifest,
                handlers,
                grant_origin,
                previous_content_hash,
                event_prompt,
                event_decision,
                grant_prompts,
                grant_decisions,
            ),
        }
    }

    /// Atomically replace an installed app's sealed declaration and handlers
    /// when its authority and behavioral contract are unchanged. Version and
    /// top-level presentation text are the only declaration differences
    /// allowed. Grants, history, runs, artifacts, surfaces, and subscriptions
    /// are preserved.
    pub fn upgrade_app(
        &mut self,
        sealed_manifest: SealedManifest,
        handlers: BTreeMap<CapabilityName, CapabilityHandler>,
    ) -> KernelResult<()> {
        let manifest = &sealed_manifest.manifest;
        let app_id = manifest.app_id.clone();
        let declared: Vec<String> = manifest
            .capabilities
            .iter()
            .map(|capability| capability.name.to_string())
            .collect();
        let offered: Vec<String> = handlers.keys().map(ToString::to_string).collect();
        if declared.iter().collect::<std::collections::BTreeSet<_>>()
            != offered.iter().collect::<std::collections::BTreeSet<_>>()
        {
            return Err(KernelError::HandlerBindingMismatch {
                app: app_id,
                declared,
                offered,
            });
        }

        let mut registry = self.registry.clone();
        self.validate_grant_requests(manifest, AbsentProvider::Tolerate)?;
        registry.upgrade(sealed_manifest)?;

        let mut bound_handlers = self.handlers.clone();
        bound_handlers.insert(
            app_id,
            handlers
                .into_iter()
                .map(|(name, handler)| (name, Arc::new(handler)))
                .collect(),
        );

        self.commit_parts(&registry, &self.ledger, &self.broker, &self.artifacts)?;
        self.registry = registry;
        self.handlers = bound_handlers;
        Ok(())
    }

    /// Replace an app recovered from durable state whose code (manifest) has
    /// changed, as one atomic transition.
    ///
    /// Called only from phased install when a recovered same-id registration
    /// is present but its content hash / manifest drifted. Every step that could
    /// fail — candidate validation, the event-subscription prompt, and the
    /// single durable commit — runs *before* any committed state changes, and
    /// the old registration is staged for teardown on clones. A failure at any
    /// point leaves the previously working app, its grants, and its active
    /// runs exactly as they were, so an update can never destroy the version
    /// it was replacing.
    #[allow(clippy::too_many_arguments)]
    fn replace_installed_app(
        &mut self,
        sealed_manifest: SealedManifest,
        handlers: BTreeMap<CapabilityName, CapabilityHandler>,
        grant_origin: GrantOrigin,
        previous_content_hash: String,
        event_prompt: Option<EventSubscriptionPrompt>,
        event_decision: Option<ApprovalDecision>,
        grant_prompts: Vec<GrantIssuancePrompt>,
        grant_decisions: Vec<ApprovalDecision>,
    ) -> KernelResult<Vec<IssueResult>> {
        let app_id = sealed_manifest.manifest.app_id.clone();
        let grant_requests = sealed_manifest.manifest.grant_requests.clone();

        let installed = self.registry.app(&app_id)?;
        if installed.content_hash != previous_content_hash || self.handlers.contains_key(&app_id) {
            return Err(KernelError::PreparedInstallStale);
        }

        // Validate the candidate against a catalog with the old registration
        // removed, so the same-id replacement is not rejected as a duplicate.
        let mut staged_registry = self.registry.clone();
        staged_registry.uninstall(&app_id)?;
        staged_registry.validate_install(&sealed_manifest)?;
        self.validate_install_grant_requests(&sealed_manifest.manifest)?;

        // Re-consent: the new code must earn all authority afresh.
        if event_prompt.is_some() && event_decision != Some(ApprovalDecision::Approved) {
            return Err(KernelError::EventSubscriptionDenied(app_id));
        }
        if grant_decisions.len() != grant_prompts.len() {
            return Err(KernelError::PreparedInstallStale);
        }

        // Stage the whole swap on clones — the old app's runs, grants, and
        // registration out; the new registration and its fresh grants in.
        staged_registry.install(sealed_manifest)?;
        let active_runs = self.ledger.active_runs_for_app(&app_id);
        let mut end_events = self.pending_cancellation_events_for_app(&app_id);
        end_events.extend(active_runs.iter().map(|run_id| LedgerEvent::RunEnded {
            run_id: run_id.clone(),
            terminal_state: RunTerminalState::Cancelled,
        }));
        let views = end_events
            .iter()
            .map(|event| self.event_view(event))
            .collect::<KernelResult<Vec<_>>>()?;
        let mut staged_ledger = self.ledger.clone();
        staged_ledger.append_batch(end_events)?;
        let mut staged_broker = self.broker.clone();
        staged_broker.revoke_all_for(&app_id);
        staged_broker.revoke_all_over(&app_id);
        staged_broker.clear_all_for(&app_id);
        let (results, _) = Self::reconcile_manifest_grants(
            &mut staged_broker,
            &app_id,
            &grant_requests,
            grant_origin,
            &grant_prompts,
            &grant_decisions,
        );

        // Commit exactly once. Until this returns Ok nothing is durable.
        self.commit_parts(
            &staged_registry,
            &staged_ledger,
            &staged_broker,
            &self.artifacts,
        )?;

        // Swap runtime state only after the durable commit succeeded, then
        // apply the in-memory teardown that mirrors uninstall.
        self.ledger = staged_ledger;
        self.broker = staged_broker;
        self.registry = staged_registry;
        self.remove_pending_for_app(&app_id);
        for run_id in &active_runs {
            self.leases.release_all_for_run(run_id);
        }
        self.router.publish_batch(views, &self.registry);
        self.surfaces.close_all_for(&app_id);
        self.router.discard_inbox(&app_id);
        self.handlers.insert(
            app_id,
            handlers
                .into_iter()
                .map(|(name, handler)| (name, Arc::new(handler)))
                .collect(),
        );
        Ok(results)
    }

    /// Remove an app and everything that carried its authority: grants it
    /// holds, grants other apps hold over it, open surfaces, queued events,
    /// handlers. A later install under the same AppId must start from
    /// nothing — in both directions, since consumers consented to the old
    /// provider's code, not whatever is installed under the id next.
    ///
    /// Active runs initiated by this app are cancelled and their leases
    /// released so uninstalling never leaks advisory locks.
    pub fn uninstall(&mut self, app_id: &AppId) -> KernelResult<()> {
        self.registry.app(app_id)?; // fail before touching any state
        let active_runs = self.ledger.active_runs_for_app(app_id);
        let mut ledger = self.ledger.clone();
        let mut events = self.pending_cancellation_events_for_app(app_id);
        events.extend(active_runs.iter().map(|run_id| LedgerEvent::RunEnded {
            run_id: run_id.clone(),
            terminal_state: RunTerminalState::Cancelled,
        }));
        let views = events
            .iter()
            .map(|event| self.event_view(event))
            .collect::<KernelResult<Vec<_>>>()?;
        ledger.append_batch(events)?;
        let mut broker = self.broker.clone();
        broker.revoke_all_for(app_id);
        broker.revoke_all_over(app_id);
        broker.clear_all_for(app_id);
        let mut registry = self.registry.clone();
        registry.uninstall(app_id)?;
        self.commit_parts(&registry, &ledger, &broker, &self.artifacts)?;
        self.remove_pending_for_app(app_id);
        for run_id in &active_runs {
            self.leases.release_all_for_run(run_id);
        }
        self.ledger = ledger;
        self.broker = broker;
        self.registry = registry;
        self.router.publish_batch(views, &self.registry);
        self.surfaces.close_all_for(app_id);
        self.router.discard_inbox(app_id);
        self.handlers.remove(app_id);
        Ok(())
    }

    // -- runs ----------------------------------------------------------------

    /// Start a run on behalf of an installed app. Runs are the unit of
    /// attribution; permissions are checked per capability, not here.
    pub fn start_run(&mut self, initiator: Initiator, goal: &str) -> KernelResult<RunId> {
        self.registry.app(initiator.app_id())?;
        if let Initiator::Run {
            app_id,
            parent_run_id,
        } = &initiator
        {
            let parent = self.ledger.run_view(parent_run_id)?;
            if !parent.is_active() {
                return Err(KernelError::RunAlreadyEnded(parent_run_id.clone()));
            }
            if parent.initiating_app() != app_id {
                return Err(KernelError::ChildRunAttributionMismatch {
                    expected: parent.initiating_app().clone(),
                    got: app_id.clone(),
                });
            }
        }
        let run_id = new_run_id();
        self.record(LedgerEvent::RunStarted {
            run_id: run_id.clone(),
            initiator,
            goal: goal.to_string(),
        })?;
        Ok(run_id)
    }

    pub fn end_run(
        &mut self,
        run_id: &RunId,
        terminal_state: RunTerminalState,
    ) -> KernelResult<()> {
        let mut events = self.pending_cancellation_events_for_run(run_id);
        events.push(LedgerEvent::RunEnded {
            run_id: run_id.clone(),
            terminal_state,
        });
        self.record_batch(events)?;
        self.remove_pending_for_run(run_id);
        self.leases.release_all_for_run(run_id);
        Ok(())
    }

    pub fn run_view(&self, run_id: &RunId) -> KernelResult<RunView> {
        self.ledger.run_view(run_id)
    }

    // -- the action path -----------------------------------------------------

    pub fn prepare_invocation(
        &mut self,
        run_id: &RunId,
        capability: &CapabilityRef,
        request: InvocationRequest,
    ) -> KernelResult<PrepareInvocation> {
        self.prepare_invocation_with_timeout(
            run_id,
            capability,
            request,
            DEFAULT_INVOCATION_TIMEOUT,
        )
    }

    /// Prepare an invocation with an explicit wall-clock budget. The default
    /// public path uses [`DEFAULT_INVOCATION_TIMEOUT`]; the explicit seam is
    /// useful to hosts with tighter budgets and deterministic tests.
    pub fn prepare_invocation_with_timeout(
        &mut self,
        run_id: &RunId,
        capability: &CapabilityRef,
        request: InvocationRequest,
        timeout: StdDuration,
    ) -> KernelResult<PrepareInvocation> {
        self.reap_cancelled_pending()?;
        request.data_scope.validate_invocation()?;
        let run = self.ledger.run_view(run_id)?;
        if !run.is_active() {
            return Err(KernelError::RunAlreadyEnded(run_id.clone()));
        }
        let acting_app = run.initiating_app().clone();
        let direct_provider_surface_action = matches!(
            &run.initiator,
            Initiator::SurfaceAction { app_id, .. }
                if app_id == &acting_app && app_id == &capability.provider
        );
        // A caller may still present a stale run handle after uninstall; an
        // uninstalled app must never keep acting through it.
        self.registry.app(&acting_app)?;
        let declaration = self.registry.capability(capability)?;
        validate_against_schema(
            &serde_json::Value::Object(request.input.clone()),
            &declaration.input_schema,
            SchemaViolation::CapabilityInput,
            &capability.qualified_name(),
        )?;
        let output_schema = declaration.output_schema.clone();

        let grant = match self
            .broker
            .check(&acting_app, capability, &request.data_scope)
        {
            GrantCheck::Denied(reason) => {
                self.record(LedgerEvent::InvocationRefused {
                    run_id: run_id.clone(),
                    capability: capability.clone(),
                    reason,
                    data_scope: request.data_scope.clone(),
                })?;
                return Ok(PrepareInvocation::Refused(InvocationResult::Refused {
                    reason: refusal_for(reason),
                }));
            }
            GrantCheck::ApprovalRequired(grant) => grant,
            GrantCheck::Allowed(grant) => grant,
        };

        // Establish that the work *can* run before the ledger says it began.
        // This lookup has no side effects, so it belongs with the other
        // validation: recording `ApprovalRequested`/`CapabilityInvoked` first
        // and only then failing on an unbound handler would commit an event
        // claiming consent was sought or work started, with no
        // `PendingInvocation` left to ever resolve it into a terminal state.
        if self
            .handlers
            .get(&capability.provider)
            .and_then(|bound| bound.get(&capability.capability))
            .is_none()
        {
            return Err(KernelError::HandlerNotBound {
                app: capability.provider.clone(),
                capability: capability.capability.clone(),
            });
        }

        if grant.condition == GrantCondition::Notify && !direct_provider_surface_action {
            self.chrome
                .show_notice(ChromeNotice::GrantUse {
                    app_id: acting_app.clone(),
                    capability: capability.clone(),
                    grant_id: grant.grant_id.clone(),
                    run_id: run_id.clone(),
                })
                .map_err(|error| KernelError::Durability(error.to_string()))?;
        }

        let approval = if grant.condition == GrantCondition::RequiresApproval
            && !direct_provider_surface_action
        {
            let app = self.registry.app(&acting_app)?;
            let prompt = CapabilityApprovalPrompt {
                app_id: acting_app.clone(),
                app_display_name: app.manifest.display_name.clone(),
                capability: capability.clone(),
                data_scope: request.data_scope.clone(),
                grant_id: grant.grant_id.clone(),
                run_id: run_id.clone(),
                goal: run.goal.clone(),
            };
            self.record(LedgerEvent::ApprovalRequested {
                run_id: run_id.clone(),
                capability: capability.clone(),
                grant_id: grant.grant_id.clone(),
                data_scope: request.data_scope.clone(),
            })?;
            Some(prompt)
        } else {
            self.record(LedgerEvent::CapabilityInvoked {
                run_id: run_id.clone(),
                capability: capability.clone(),
                grant_id: grant.grant_id.clone(),
                input_sha256: payload_sha256(&serde_json::Value::Object(request.input.clone()))?,
                data_scope: request.data_scope.clone(),
            })?;
            None
        };
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(KernelError::InvalidInvocationTimeout)?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let id = NEXT_INVOCATION_ID.fetch_add(1, Ordering::Relaxed);
        self.pending_invocations.insert(
            id,
            PendingInvocation {
                run_id: run_id.clone(),
                acting_app,
                capability: capability.clone(),
                grant,
                requested_data_scope: request.data_scope.clone(),
                provider_content_hash: self
                    .registry
                    .app(&capability.provider)?
                    .content_hash
                    .clone(),
                output_schema,
                input: request.input,
                cancelled,
                deadline,
                state: PendingInvocationState::PreparedForApproval,
            },
        );
        // The handler and context leave the trusted core only after approval
        // and authority revalidation in authorize_invocation.
        Ok(PrepareInvocation::Prepared(PreparedInvocation {
            id,
            approval,
            chrome: self.chrome.clone(),
            cancelled: self.pending_invocations[&id].cancelled.clone(),
            cancel_on_drop: true,
        }))
    }

    /// Consume an approval response and atomically revalidate the invocation
    /// before dispatching provider code. A revoked grant, cancelled run,
    /// expiry, provider replacement, or uninstall during approval therefore
    /// prevents handler execution entirely.
    pub fn authorize_invocation(
        &mut self,
        mut approval: ApprovalResult,
    ) -> KernelResult<AuthorizeInvocation> {
        // Peek, do not remove until the authorization attempt resolves. The
        // public approval token is move-only, so every error path below must
        // reclaim the corresponding pending entry rather than imply a retry.
        match self.pending_invocations.get(&approval.id) {
            Some(pending)
                if matches!(pending.state, PendingInvocationState::PreparedForApproval) => {}
            _ => return Err(KernelError::PreparedInvocationConsumed),
        }
        if let Some(decision) = approval.decision {
            let (run_id, capability, grant_id, data_scope) = {
                let pending = &self.pending_invocations[&approval.id];
                (
                    pending.run_id.clone(),
                    pending.capability.clone(),
                    pending.grant.grant_id.clone(),
                    pending.requested_data_scope.clone(),
                )
            };
            let record_result = self.record(if decision == ApprovalDecision::Approved {
                LedgerEvent::ApprovalGranted {
                    run_id,
                    capability,
                    grant_id,
                    data_scope,
                }
            } else {
                LedgerEvent::ApprovalDenied {
                    run_id,
                    capability,
                    grant_id,
                    data_scope,
                }
            });
            if let Err(error) = record_result {
                self.require_recovery_after_consumed_transition();
                self.pending_invocations.remove(&approval.id);
                return Err(error);
            }
            if decision == ApprovalDecision::Denied {
                self.pending_invocations.remove(&approval.id);
                return Ok(AuthorizeInvocation::Refused(InvocationResult::Refused {
                    reason: RefusalReason::ApprovalDenied,
                }));
            }
        }
        let revalidation = {
            let pending = &self.pending_invocations[&approval.id];
            self.revalidate_pending(pending)
        };
        if let Err(error) = revalidation {
            let (run_id, capability, data_scope) = {
                let pending = &self.pending_invocations[&approval.id];
                (
                    pending.run_id.clone(),
                    pending.capability.clone(),
                    pending.requested_data_scope.clone(),
                )
            };
            let result = self.finalize_cancelled(run_id, capability, data_scope, error);
            if result.is_err() {
                self.require_recovery_after_consumed_transition();
            }
            self.pending_invocations.remove(&approval.id);
            return Ok(AuthorizeInvocation::Refused(result?));
        }
        let requires_approval = self.pending_invocations[&approval.id].grant.condition
            == GrantCondition::RequiresApproval;
        if requires_approval {
            let (run_id, capability, grant_id, input, data_scope) = {
                let pending = &self.pending_invocations[&approval.id];
                (
                    pending.run_id.clone(),
                    pending.capability.clone(),
                    pending.grant.grant_id.clone(),
                    pending.input.clone(),
                    pending.requested_data_scope.clone(),
                )
            };
            let record_result = self.record(LedgerEvent::CapabilityInvoked {
                run_id,
                capability,
                grant_id,
                input_sha256: payload_sha256(&serde_json::Value::Object(input))?,
                data_scope,
            });
            if let Err(error) = record_result {
                self.require_recovery_after_consumed_transition();
                self.pending_invocations.remove(&approval.id);
                return Err(error);
            }
        }
        let handler = {
            let pending = &self.pending_invocations[&approval.id];
            self.handlers
                .get(&pending.capability.provider)
                .and_then(|bound| bound.get(&pending.capability.capability))
                .ok_or_else(|| KernelError::HandlerNotBound {
                    app: pending.capability.provider.clone(),
                    capability: pending.capability.capability.clone(),
                })?
                .clone()
        };
        let context = {
            let pending = &self.pending_invocations[&approval.id];
            let installed = self.registry.app(&pending.acting_app)?;
            InvocationContext {
                run_id: pending.run_id.clone(),
                invoked_by: pending.acting_app.clone(),
                invoked_by_version: installed.manifest.version.clone(),
                invoked_by_content_hash: installed.content_hash.clone(),
                authorized_data_scope: pending.requested_data_scope.clone(),
                secrets: self.broker.secret_resolver_for(
                    &pending.capability.provider,
                    self.registry
                        .app(&pending.capability.provider)?
                        .manifest
                        .declared_secret_names(),
                ),
                artifacts: self
                    .artifacts
                    .snapshot_resolver_for(&pending.requested_data_scope),
                cancellation: CancellationHandle::new(pending.cancelled.clone(), pending.deadline),
                progress: ProgressReporter::default(),
            }
        };
        let input = self.pending_invocations[&approval.id].input.clone();
        // The entry stays in the map, now marked authorized; it is removed
        // only when finalization commits (or an explicit abort reclaims it).
        self.pending_invocations
            .get_mut(&approval.id)
            .expect("pending entry present through authorization")
            .state = PendingInvocationState::AuthorizedForExecution;
        approval.cancel_on_drop = false;
        Ok(AuthorizeInvocation::Authorized(AuthorizedInvocation {
            id: approval.id,
            handler,
            input,
            context: Box::new(context),
            cancelled: approval.cancelled.clone(),
            cancel_on_drop: true,
        }))
    }

    /// Finalize one dispatched handler result. Revalidation prevents a late
    /// result from entering the ledger or artifact store; it cannot undo
    /// external effects a handler started before cooperative cancellation.
    pub fn finalize_invocation(
        &mut self,
        mut executed: ExecutedInvocation,
    ) -> KernelResult<InvocationResult> {
        executed.cancel_on_drop = false;
        // Peek, do not remove yet: the entry is consumed only once the durable
        // completion (or refusal) transition commits, so a persistence failure
        // after the handler already ran does not lose the pending record.
        match self.pending_invocations.get(&executed.id) {
            Some(pending)
                if matches!(
                    pending.state,
                    PendingInvocationState::AuthorizedForExecution
                ) => {}
            _ => return Err(KernelError::PreparedInvocationConsumed),
        }
        let revalidation = {
            let pending = &self.pending_invocations[&executed.id];
            self.revalidate_pending(pending)
        };
        if let Err(error) = revalidation {
            let (run_id, capability, data_scope) = {
                let pending = &self.pending_invocations[&executed.id];
                (
                    pending.run_id.clone(),
                    pending.capability.clone(),
                    pending.requested_data_scope.clone(),
                )
            };
            let result = self.finalize_cancelled(run_id, capability, data_scope, error);
            if result.is_err() {
                self.require_recovery_after_consumed_transition();
            }
            self.pending_invocations.remove(&executed.id);
            return result;
        }
        let (run_id, capability, grant, data_scope, output_schema) = {
            let pending = &self.pending_invocations[&executed.id];
            (
                pending.run_id.clone(),
                pending.capability.clone(),
                pending.grant.clone(),
                pending.requested_data_scope.clone(),
                pending.output_schema.clone(),
            )
        };
        let result = if let Some(error) = executed.panic.take() {
            self.fail_invocation(&run_id, &capability, &grant, &data_scope, error)
        } else {
            match executed
                .outcome
                .take()
                .expect("authorized execution always returns an outcome")
            {
                Ok(outcome) => self.complete_outcome(
                    &run_id,
                    &capability,
                    &grant,
                    &data_scope,
                    output_schema.as_ref(),
                    outcome,
                ),
                Err(HandlerFailure(error)) => {
                    self.fail_invocation(&run_id, &capability, &grant, &data_scope, error)
                }
            }
        };
        if result.is_err() {
            self.require_recovery_after_consumed_transition();
        }
        self.pending_invocations.remove(&executed.id);
        result
    }

    /// Deliberately abandon a prepared-but-not-yet-authorized invocation,
    /// reclaiming its reservation instead of leaking it, and recording an
    /// honest cancellation. This is the explicit counterpart to letting a
    /// token drop — which cannot clean up, since `Drop` has no kernel handle.
    pub fn abort_prepared_invocation(
        &mut self,
        token: PreparedInvocation,
    ) -> KernelResult<InvocationResult> {
        self.abort_pending(token.id)
    }

    /// Deliberately abandon an authorized invocation before executing it.
    pub fn abort_authorized_invocation(
        &mut self,
        token: AuthorizedInvocation,
    ) -> KernelResult<InvocationResult> {
        self.abort_pending(token.id)
    }

    fn abort_pending(&mut self, id: u64) -> KernelResult<InvocationResult> {
        let (run_id, capability, data_scope) = match self.pending_invocations.get(&id) {
            Some(pending) => (
                pending.run_id.clone(),
                pending.capability.clone(),
                pending.requested_data_scope.clone(),
            ),
            None => return Err(KernelError::PreparedInvocationConsumed),
        };
        self.record(LedgerEvent::InvocationCancelled {
            run_id,
            capability,
            data_scope,
        })?;
        self.pending_invocations.remove(&id);
        Ok(InvocationResult::Refused {
            reason: RefusalReason::Cancelled,
        })
    }

    fn revalidate_pending(&self, pending: &PendingInvocation) -> KernelResult<()> {
        if !self.ledger.run_view(&pending.run_id)?.is_active() {
            return Err(KernelError::RunAlreadyEnded(pending.run_id.clone()));
        }
        self.registry.app(&pending.acting_app)?;
        if self
            .registry
            .app(&pending.capability.provider)?
            .content_hash
            != pending.provider_content_hash
        {
            return Err(KernelError::PreparedInvocationStale);
        }
        match self.broker.check(
            &pending.acting_app,
            &pending.capability,
            &pending.requested_data_scope,
        ) {
            GrantCheck::Allowed(grant) | GrantCheck::ApprovalRequired(grant)
                if grant.grant_id == pending.grant.grant_id => {}
            GrantCheck::Allowed(_) | GrantCheck::ApprovalRequired(_) => {
                return Err(KernelError::PreparedInvocationStale)
            }
            GrantCheck::Denied(reason) => {
                return Err(KernelError::PreparedInvocationDenied(reason));
            }
        }
        if Instant::now() >= pending.deadline {
            pending.cancelled.store(true, Ordering::Release);
        }
        if pending.cancelled.load(Ordering::Acquire) {
            return Err(KernelError::PreparedInvocationCancelled);
        }
        Ok(())
    }

    fn finalize_cancelled(
        &mut self,
        run_id: RunId,
        capability: CapabilityRef,
        data_scope: DataScope,
        error: KernelError,
    ) -> KernelResult<InvocationResult> {
        match error {
            KernelError::PreparedInvocationDenied(reason) => {
                self.record(LedgerEvent::InvocationRefused {
                    run_id,
                    capability,
                    reason,
                    data_scope,
                })?;
                Ok(InvocationResult::Refused {
                    reason: refusal_for(reason),
                })
            }
            // Cancellation and deadline expiry are not grant denials: the
            // grant stayed valid, the work was stopped. Record it honestly so
            // audit and UI never claim a revocation that did not happen.
            KernelError::PreparedInvocationCancelled
            | KernelError::PreparedInvocationStale
            | KernelError::UnknownApp(_)
            | KernelError::HandlerNotBound { .. } => {
                self.record(LedgerEvent::InvocationCancelled {
                    run_id,
                    capability,
                    data_scope,
                })?;
                Ok(InvocationResult::Refused {
                    reason: RefusalReason::Cancelled,
                })
            }
            error => Err(error),
        }
    }

    /// Run the provider's handler with app failures contained: a failing
    /// handler or an invalid artifact fails the invocation, never the kernel.
    fn complete_outcome(
        &mut self,
        run_id: &RunId,
        capability: &CapabilityRef,
        grant: &Grant,
        data_scope: &DataScope,
        output_schema: Option<&JsonObject>,
        outcome: CapabilityOutcome,
    ) -> KernelResult<InvocationResult> {
        let CapabilityOutcome { result, artifacts } = outcome;
        // Validate the handler's result against the optional output schema
        // before any artifact processing.
        if let Some(schema) = output_schema {
            if let Err(error) = validate_against_schema(
                &result,
                schema,
                SchemaViolation::CapabilityOutput,
                &capability.qualified_name(),
            ) {
                return self.fail_invocation(
                    run_id,
                    capability,
                    grant,
                    data_scope,
                    error.to_string(),
                );
            }
        }
        // Two phases: validate every draft, then store all of them — a
        // failing draft must leave nothing half-stored (no artifact in the
        // store, no ArtifactProduced event in the ledger).
        for draft in &artifacts {
            if let Err(error) = self.validate_draft(&capability.provider, draft) {
                return self.fail_invocation(
                    run_id,
                    capability,
                    grant,
                    data_scope,
                    error.to_string(),
                );
            }
        }
        let stored: Vec<Artifact> = artifacts
            .into_iter()
            .map(|draft| self.stamp_artifact(run_id, capability, grant, draft))
            .collect();
        let mut completion_events: Vec<LedgerEvent> = stored
            .iter()
            .map(|artifact| LedgerEvent::ArtifactProduced {
                run_id: run_id.clone(),
                artifact_id: artifact.artifact_id.clone(),
                artifact_type: artifact.artifact_type.clone(),
            })
            .collect();
        completion_events.push(LedgerEvent::CapabilityCompleted {
            run_id: run_id.clone(),
            capability: capability.clone(),
            grant_id: grant.grant_id.clone(),
            result_sha256: payload_sha256(&result)?,
            data_scope: data_scope.clone(),
        });
        self.commit_completion(completion_events, stored.clone())?;
        Ok(InvocationResult::Completed {
            result,
            artifacts: stored,
        })
    }

    fn fail_invocation(
        &mut self,
        run_id: &RunId,
        capability: &CapabilityRef,
        grant: &Grant,
        data_scope: &DataScope,
        error: String,
    ) -> KernelResult<InvocationResult> {
        self.record(LedgerEvent::CapabilityFailed {
            run_id: run_id.clone(),
            capability: capability.clone(),
            grant_id: grant.grant_id.clone(),
            error: error.clone(),
            data_scope: data_scope.clone(),
        })?;
        Ok(InvocationResult::Failed { error })
    }

    /// Reject a draft whose type is undeclared or whose content violates the
    /// declared schema — before anything is stored.
    fn validate_draft(&self, provider: &AppId, draft: &ArtifactDraft) -> KernelResult<()> {
        let declared_type = self
            .registry
            .artifact_type(provider, &draft.artifact_type)?;
        validate_against_schema(
            &draft.content,
            &declared_type.json_schema,
            SchemaViolation::ArtifactContent,
            &format!("{} (type '{}')", draft.title, draft.artifact_type),
        )
    }

    /// Stamp kernel-written provenance on a validated draft: apps
    /// propose content, never provenance. Storage happens only after the full
    /// completion ledger batch has committed.
    fn stamp_artifact(
        &self,
        run_id: &RunId,
        capability: &CapabilityRef,
        grant: &Grant,
        draft: ArtifactDraft,
    ) -> Artifact {
        Artifact {
            artifact_id: new_artifact_id(),
            artifact_type: draft.artifact_type,
            title: draft.title,
            content: draft.content,
            provenance: Provenance {
                run_id: run_id.clone(),
                capability: capability.clone(),
                grant_id: grant.grant_id.clone(),
                produced_by: capability.provider.clone(),
                recorded_at: self.clock.now(),
            },
        }
    }

    pub fn artifact(&self, artifact_id: &ArtifactId) -> KernelResult<&Artifact> {
        self.artifacts.get(artifact_id)
    }

    // -- surfaces ------------------------------------------------------------

    pub fn open_surface(
        &mut self,
        app_id: &AppId,
        surface: &SurfaceName,
    ) -> KernelResult<SurfaceBinding> {
        self.surfaces.open(&self.registry, app_id, surface)
    }

    pub fn close_surface(&mut self, binding: &SurfaceBinding) {
        self.surfaces.close(binding);
    }

    /// Validate that a host-side operation still belongs to a live surface.
    /// Surface-scoped presentation state is not a capability invocation, but
    /// its host adapter must still reject stale, closed, or forged bindings.
    pub fn require_open_surface(&self, binding: &SurfaceBinding) -> KernelResult<()> {
        self.surfaces.require_open(binding)
    }

    /// Start and prepare a surface action without executing provider code.
    /// Hosts use this to release their kernel mutex for phase two.
    pub fn prepare_surface_action(
        &mut self,
        binding: &SurfaceBinding,
        intent: ActionIntent,
    ) -> KernelResult<(RunId, PrepareInvocation)> {
        self.surfaces.require_open(binding)?;
        // Retrieve the declared surface and verify the intent's capability
        // is among its declared intents.
        let declared_surface = self.registry.surface(&binding.app_id, &binding.surface)?;
        if !declared_surface.intents.contains(&intent.capability) {
            return Err(KernelError::UndeclaredSurfaceIntent {
                app: binding.app_id.clone(),
                surface: binding.surface.clone(),
                capability: intent.capability.clone(),
            });
        }
        let run_id = self.start_run(
            Initiator::SurfaceAction {
                app_id: binding.app_id.clone(),
                surface: binding.surface.clone(),
            },
            &intent.goal,
        )?;
        let prepared = match self.prepare_invocation(
            &run_id,
            &intent.capability,
            InvocationRequest {
                input: intent.input,
                data_scope: intent.data_scope,
            },
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                // The kernel started this run, so the kernel must not leak
                // it. Cleanup failure must not shadow the invocation error.
                let _ = self.end_run(&run_id, RunTerminalState::Failed);
                return Err(error);
            }
        };
        Ok((run_id, prepared))
    }

    // -- leases, secrets, events ----------------------------------------------

    pub fn acquire_lease(
        &mut self,
        run_id: &RunId,
        target: LeaseTarget,
        duration: Duration,
    ) -> KernelResult<LeaseOutcome> {
        let run = self.ledger.run_view(run_id)?;
        if !run.is_active() {
            return Err(KernelError::RunAlreadyEnded(run_id.clone()));
        }
        let resource = target.resource_name();
        let outcome = self.leases.acquire(run_id, target, duration)?;
        if let LeaseOutcome::Conflict { holder } = &outcome {
            self.chrome
                .show_notice(ChromeNotice::LeaseConflict {
                    resource,
                    holding_run: holder.run_id.clone(),
                    requesting_run: run_id.clone(),
                })
                .map_err(|error| KernelError::Durability(error.to_string()))?;
        }
        Ok(outcome)
    }

    pub fn put_secret(&mut self, secret_ref: SecretRef, value: String) {
        self.broker.put_secret(secret_ref, value);
    }

    /// Broker-scoped secret access for the app's own connector handlers,
    /// limited to the names its manifest declares.
    pub fn secret_resolver_for(&self, app_id: &AppId) -> KernelResult<SecretResolver> {
        let declared = self.registry.app(app_id)?.manifest.declared_secret_names();
        Ok(self.broker.secret_resolver_for(app_id, declared))
    }

    pub fn drain_inbox(&mut self, app_id: &AppId) -> KernelResult<Vec<AppEventEnvelope>> {
        self.router.drain_inbox(app_id, &self.registry)
    }

    pub fn publish_app_data_change(
        &mut self,
        provider_app_id: &AppId,
        resource_ref: &str,
        revision: u64,
        change_kind: AppDataChangeKind,
    ) -> KernelResult<()> {
        self.registry.app(provider_app_id)?;
        if resource_ref.trim().is_empty() {
            return Err(KernelError::Durability(
                "resource ref must not be empty".into(),
            ));
        }
        self.router.publish_data_change(
            provider_app_id,
            resource_ref.to_string(),
            revision,
            change_kind,
            &self.registry,
        );
        Ok(())
    }

    pub fn inbox_status(&self, app_id: &AppId) -> KernelResult<EventInboxStatus> {
        self.router.inbox_status(app_id, &self.registry)
    }

    /// Issue a grant to an installed app, confirmed through trusted chrome.
    /// The display name shown to the user comes from the verified manifest,
    /// never from the caller.
    pub fn prepare_grant(
        &self,
        holder: &AppId,
        request: GrantRequest,
    ) -> KernelResult<PreparedGrant> {
        self.validate_grant_request(holder, &request)?;
        let holder_display_name = self.registry.app(holder)?.manifest.display_name.clone();
        let holder_content_hash = self.registry.app(holder)?.content_hash.clone();
        let provider_content_hash = self
            .registry
            .app(request.scope.provider())?
            .content_hash
            .clone();
        Ok(PreparedGrant {
            holder: holder.clone(),
            holder_display_name,
            holder_content_hash,
            provider_content_hash,
            request,
            chrome: self.chrome.clone(),
        })
    }

    pub fn commit_grant(&mut self, approval: GrantApproval) -> KernelResult<IssueResult> {
        let GrantApproval {
            prepared:
                PreparedGrant {
                    holder,
                    request,
                    chrome: _,
                    holder_display_name: _,
                    holder_content_hash,
                    provider_content_hash,
                },
            decision,
        } = approval;
        let current_hash = self.registry.app(&holder)?.content_hash.clone();
        if current_hash != holder_content_hash {
            return Err(KernelError::PreparedInstallStale);
        }
        if self.registry.app(request.scope.provider())?.content_hash != provider_content_hash {
            return Err(KernelError::PreparedInstallStale);
        }
        self.validate_grant_request(&holder, &request)?;
        let mut broker = self.broker.clone();
        let result =
            broker.issue_with_decision(&holder, &request, GrantOrigin::UserAdded, decision);
        if matches!(result, IssueResult::Issued(_)) {
            self.commit_parts(&self.registry, &self.ledger, &broker, &self.artifacts)?;
            self.broker = broker;
        }
        Ok(result)
    }

    pub fn issue_grant(
        &mut self,
        holder: &AppId,
        request: &GrantRequest,
    ) -> KernelResult<IssueResult> {
        self.validate_grant_request(holder, request)?;
        let display_name = self.registry.app(holder)?.manifest.display_name.clone();
        let mut broker = self.broker.clone();
        let result = broker.issue(holder, &display_name, request, GrantOrigin::UserAdded);
        if matches!(result, IssueResult::Issued(_)) {
            self.commit_parts(&self.registry, &self.ledger, &broker, &self.artifacts)?;
            self.broker = broker;
        }
        Ok(result)
    }

    pub fn revoke_grant(&mut self, grant_id: &GrantId) -> KernelResult<()> {
        let mut broker = self.broker.clone();
        broker.revoke(grant_id)?;
        self.commit_parts(&self.registry, &self.ledger, &broker, &self.artifacts)?;
        self.broker = broker;
        for pending in self.pending_invocations.values() {
            if pending.grant.grant_id == *grant_id {
                pending.cancelled.store(true, Ordering::Release);
            }
        }
        Ok(())
    }

    pub fn revoke_grants_for_resource(&mut self, resource_id: &ResourceId) -> KernelResult<()> {
        let grant_ids = self.broker.grant_ids_over_resource(resource_id);
        if grant_ids.is_empty() {
            return Ok(());
        }
        let mut broker = self.broker.clone();
        for grant_id in &grant_ids {
            broker.revoke(grant_id)?;
        }
        self.commit_parts(&self.registry, &self.ledger, &broker, &self.artifacts)?;
        self.broker = broker;
        let grant_ids: std::collections::BTreeSet<_> = grant_ids.into_iter().collect();
        for pending in self.pending_invocations.values() {
            if grant_ids.contains(&pending.grant.grant_id) {
                pending.cancelled.store(true, Ordering::Release);
            }
        }
        Ok(())
    }

    /// Request cooperative cancellation for work currently executing in a
    /// run. The host remains responsible for choosing the run's terminal
    /// state; late handler output cannot commit after this flag is set.
    pub fn cancel_pending_invocations_for_run(&mut self, run_id: &RunId) {
        self.cancel_pending_for_run(run_id);
    }

    // -- read views ------------------------------------------------------------
    //
    // The only windows into kernel state: aggregations and lookups, no
    // mutable access. Everything a shell renders comes from here.

    pub fn installed_apps(&self) -> impl Iterator<Item = &InstalledApp> {
        self.registry.installed_apps()
    }

    pub fn installed_app(&self, app_id: &AppId) -> KernelResult<&InstalledApp> {
        self.registry.app(app_id)
    }

    pub fn capability_declaration(
        &self,
        capability: &CapabilityRef,
    ) -> KernelResult<&CapabilityDeclaration> {
        self.registry.capability(capability)
    }

    pub fn records(&self) -> &[LedgerRecord] {
        self.ledger.records()
    }

    pub fn records_for_run<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> impl Iterator<Item = &'a LedgerRecord> {
        self.ledger.records_for_run(run_id)
    }

    pub fn check_grant(&self, holder: &AppId, capability: &CapabilityRef) -> GrantCheck {
        self.broker.check(holder, capability, &DataScope::None)
    }

    pub fn grants_for(&self, holder: &AppId) -> Vec<&Grant> {
        self.broker.grants_for(holder)
    }

    pub fn grant_statuses_for(&self, holder: &AppId) -> Vec<GrantStatusView> {
        self.broker.grant_statuses_for(holder)
    }

    pub fn clear_secret(&mut self, secret_ref: &SecretRef) {
        self.broker.clear_secret(secret_ref);
    }

    /// Grant-aware capability introspection for a consuming app.
    ///
    /// Returns every capability the consumer may use right now with each live
    /// data scope paired to its own interaction condition.
    ///
    /// Capabilities the consumer has no covering grant for are not included.
    /// Expired and revoked grants are treated as absent.
    pub fn available_capabilities_for(
        &self,
        consumer: &AppId,
    ) -> KernelResult<Vec<CapabilityUseView>> {
        self.registry.app(consumer)?;
        let mut result: Vec<CapabilityUseView> = Vec::new();
        for provider in self.registry.installed_apps() {
            let provider_app_id = provider.manifest.app_id.clone();
            let provider_display_name = provider.manifest.display_name.clone();
            for cap in &provider.manifest.capabilities {
                let cap_ref = CapabilityRef {
                    provider: provider_app_id.clone(),
                    capability: cap.name.clone(),
                };
                let cap_grants: Vec<&Grant> = self
                    .broker
                    .grants_for(consumer)
                    .into_iter()
                    .filter(|g| g.scope.covers(&cap_ref))
                    .collect();
                if cap_grants.is_empty() {
                    continue;
                }
                let mut authorizations = cap_grants
                    .iter()
                    .map(|grant| CapabilityAuthorizationView {
                        data_scope: grant.data_scope.clone(),
                        condition: grant.condition,
                    })
                    .collect::<Vec<_>>();
                authorizations.sort_by(|left, right| {
                    (&left.data_scope, condition_rank(left.condition))
                        .cmp(&(&right.data_scope, condition_rank(right.condition)))
                });
                authorizations.dedup();
                result.push(CapabilityUseView {
                    provider_app_id: provider_app_id.clone(),
                    provider_display_name: provider_display_name.clone(),
                    capability: cap.name.clone(),
                    description: cap.description.clone(),
                    input_schema: cap.input_schema.clone(),
                    authorizations,
                });
            }
        }
        // Stable ordering by (provider, capability) so callers get deterministic
        // results regardless of container iteration order.
        result.sort_by(|a, b| {
            (&a.provider_app_id, &a.capability).cmp(&(&b.provider_app_id, &b.capability))
        });
        Ok(result)
    }

    pub fn artifacts(&self) -> impl Iterator<Item = &Artifact> {
        self.artifacts.all()
    }

    // -- internals -------------------------------------------------------------

    /// Prepare a total event projection before appending. Routing then has no
    /// failure path, so a valid ledger append can never turn into a failed
    /// action because subscriber delivery was unavailable.
    fn validate_grant_request(&self, holder: &AppId, request: &GrantRequest) -> KernelResult<()> {
        request.validate()?;
        self.registry.app(holder)?;
        match &request.scope {
            GrantScope::ExactCapability {
                provider,
                capability,
            } => {
                self.registry.capability(&CapabilityRef {
                    provider: provider.clone(),
                    capability: capability.clone(),
                })?;
            }
            GrantScope::AllProviderCapabilities { provider } => {
                self.registry.app(provider)?;
            }
        }
        if request.reason.trim().is_empty() {
            return Err(KernelError::GrantReasonRequired);
        }
        Ok(())
    }

    /// The active (unrevoked, unexpired) grant that satisfies this manifest
    /// grant request exactly — same scope, condition, and origin — if one
    /// exists. Used by rebind to tell intact authority from authority that
    /// went missing while the app was dormant.
    fn manifest_grant_for(
        &self,
        app_id: &AppId,
        request: &GrantRequest,
        origin: GrantOrigin,
    ) -> Option<Grant> {
        self.broker
            .grants_for(app_id)
            .into_iter()
            .find(|grant| {
                grant.scope == request.scope
                    && grant.data_scope == request.data_scope
                    && grant.condition == request.condition
                    && grant.origin == origin
                    && grant_duration_matches(grant, request.duration)
            })
            .cloned()
    }

    /// Reconcile one manifest's request list against one decision per distinct
    /// authority. Duplicate requests return the same issued grant instead of
    /// creating duplicate authority or shifting later decisions by position.
    fn reconcile_manifest_grants(
        broker: &mut PermissionBroker,
        app_id: &AppId,
        requests: &[GrantRequest],
        origin: GrantOrigin,
        prompts: &[GrantIssuancePrompt],
        decisions: &[ApprovalDecision],
    ) -> (Vec<IssueResult>, bool) {
        let mut issued_any = false;
        let mut results = Vec::with_capacity(requests.len());
        for request in requests {
            let existing = broker
                .grants_for(app_id)
                .into_iter()
                .find(|grant| {
                    grant.scope == request.scope
                        && grant.data_scope == request.data_scope
                        && grant.condition == request.condition
                        && grant.origin == origin
                        && grant_duration_matches(grant, request.duration)
                })
                .cloned();
            if let Some(grant) = existing {
                results.push(IssueResult::Issued(grant));
                continue;
            }
            let decision = prompts
                .iter()
                .zip(decisions.iter())
                .find(|(prompt, _)| {
                    prompt.scope == request.scope
                        && prompt.data_scope == request.data_scope
                        && prompt.condition == request.condition
                        && prompt.duration == request.duration
                })
                .map(|(_, decision)| *decision);
            let result = decision.map_or(IssueResult::Refused, |decision| {
                broker.issue_with_decision(app_id, request, origin, decision)
            });
            issued_any |= matches!(result, IssueResult::Issued(_));
            results.push(result);
        }
        (results, issued_any)
    }

    /// Validate the complete requested-authority set before installation mutates
    /// registry state or asks trusted chrome. Self-scoped requests resolve
    /// against the candidate manifest; every other provider must already exist.
    fn validate_install_grant_requests(
        &self,
        manifest: &crate::manifest::AppManifest,
    ) -> KernelResult<()> {
        self.validate_grant_requests(manifest, AbsentProvider::Reject)
    }

    /// Validate an app's declared grant requests. `absent_provider` decides how
    /// a request that targets another app treats a provider the registry does
    /// not currently hold: a fresh install rejects it (fail fast at the
    /// boundary), while durable recovery tolerates it as a dormant request —
    /// the same rule the registry applies to extension contributions whose
    /// target was legitimately uninstalled, so one gone provider cannot brick
    /// the host at boot. A request against a *present* provider that no longer
    /// declares the capability is still corrupt state and always fails.
    fn validate_grant_requests(
        &self,
        manifest: &crate::manifest::AppManifest,
        absent_provider: AbsentProvider,
    ) -> KernelResult<()> {
        for request in &manifest.grant_requests {
            if request.reason.trim().is_empty() {
                return Err(KernelError::GrantReasonRequired);
            }
            match &request.scope {
                GrantScope::ExactCapability {
                    provider,
                    capability,
                } if provider == &manifest.app_id => {
                    if !manifest
                        .capabilities
                        .iter()
                        .any(|declaration| declaration.name == *capability)
                    {
                        return Err(KernelError::UndeclaredCapability {
                            app: provider.clone(),
                            capability: capability.clone(),
                        });
                    }
                }
                GrantScope::ExactCapability {
                    provider,
                    capability,
                } => {
                    let lookup = self.registry.capability(&CapabilityRef {
                        provider: provider.clone(),
                        capability: capability.clone(),
                    });
                    absent_provider.tolerate(lookup.map(|_| ()))?;
                }
                GrantScope::AllProviderCapabilities { provider }
                    if provider == &manifest.app_id => {}
                GrantScope::AllProviderCapabilities { provider } => {
                    absent_provider.tolerate(self.registry.app(provider).map(|_| ()))?;
                }
            }
        }
        Ok(())
    }

    fn provider_content_hashes_for(
        &self,
        manifest: &crate::manifest::AppManifest,
        absent_provider: AbsentProvider,
    ) -> KernelResult<BTreeMap<AppId, Option<String>>> {
        manifest
            .grant_requests
            .iter()
            .filter(|request| request.scope.provider() != &manifest.app_id)
            .map(|request| {
                let provider = request.scope.provider().clone();
                match self.registry.app(&provider) {
                    Ok(installed) => Ok((provider, Some(installed.content_hash.clone()))),
                    Err(KernelError::UnknownApp(_))
                        if matches!(absent_provider, AbsentProvider::Tolerate) =>
                    {
                        Ok((provider, None))
                    }
                    Err(error) => Err(error),
                }
            })
            .collect()
    }

    fn require_provider_content_hashes(
        &self,
        expected: &BTreeMap<AppId, Option<String>>,
    ) -> KernelResult<()> {
        for (provider, content_hash) in expected {
            let current = match self.registry.app(provider) {
                Ok(installed) => Some(installed.content_hash.as_str()),
                Err(KernelError::UnknownApp(_)) => None,
                Err(error) => return Err(error),
            };
            if current != content_hash.as_deref() {
                return Err(KernelError::PreparedInstallStale);
            }
        }
        Ok(())
    }

    fn record(&mut self, event: LedgerEvent) -> KernelResult<LedgerRecord> {
        let view = self.event_view(&event)?;
        let mut ledger = self.ledger.clone();
        let record = ledger.append(event)?;
        self.commit_parts(&self.registry, &ledger, &self.broker, &self.artifacts)?;
        self.ledger = ledger;
        self.router.publish(view, &self.registry);
        Ok(record)
    }

    /// Commit related records as one ledger transition, then publish their
    /// already-prepared views. Used for artifacts plus completion.
    fn record_batch(&mut self, events: Vec<LedgerEvent>) -> KernelResult<()> {
        let views = events
            .iter()
            .map(|event| self.event_view(event))
            .collect::<KernelResult<Vec<_>>>()?;
        let mut ledger = self.ledger.clone();
        ledger.append_batch(events)?;
        self.commit_parts(&self.registry, &ledger, &self.broker, &self.artifacts)?;
        self.ledger = ledger;
        self.router.publish_batch(views, &self.registry);
        Ok(())
    }

    fn commit_completion(
        &mut self,
        events: Vec<LedgerEvent>,
        produced: Vec<Artifact>,
    ) -> KernelResult<()> {
        let views = events
            .iter()
            .map(|event| self.event_view(event))
            .collect::<KernelResult<Vec<_>>>()?;
        let mut ledger = self.ledger.clone();
        ledger.append_batch(events)?;
        let mut artifacts = self.artifacts.clone();
        artifacts.put_all(produced);
        self.commit_parts(&self.registry, &ledger, &self.broker, &artifacts)?;
        self.ledger = ledger;
        self.artifacts = artifacts;
        self.router.publish_batch(views, &self.registry);
        Ok(())
    }

    fn commit_parts(
        &self,
        registry: &Registry,
        ledger: &RunLedger,
        broker: &PermissionBroker,
        artifacts: &ArtifactStore,
    ) -> KernelResult<()> {
        if self.recovery_required.load(Ordering::Acquire) {
            return Err(KernelError::Durability(
                "kernel recovery required after an unresolved durable transition; restart the host"
                    .into(),
            ));
        }
        let Some(store) = &self.state_store else {
            return Ok(());
        };
        match store.commit(&DurableKernelState {
            installed_apps: registry.installed_apps().cloned().collect(),
            grants: broker.durable_grants(),
            revoked_grant_ids: broker.durable_revocations(),
            ledger_records: ledger.records().to_vec(),
            artifacts: artifacts.all().cloned().collect(),
        }) {
            CommitOutcome::Committed => Ok(()),
            CommitOutcome::NotCommitted(error) => Err(KernelError::Durability(error)),
            CommitOutcome::Indeterminate(error) => {
                self.recovery_required.store(true, Ordering::Release);
                Err(KernelError::Durability(format!(
                    "{error}; durable commit outcome is indeterminate and kernel recovery is required"
                )))
            }
        }
    }

    fn validate_recovered_state(&self) -> KernelResult<()> {
        let mut produced = BTreeMap::new();
        let durable_grants: BTreeMap<_, _> = self
            .broker
            .durable_grants()
            .into_iter()
            .map(|grant| (grant.grant_id.clone(), grant))
            .collect();
        for (index, record) in self.ledger.records().iter().enumerate() {
            if let LedgerEvent::ArtifactProduced {
                run_id,
                artifact_id,
                artifact_type,
            } = &record.event
            {
                if produced
                    .insert(
                        artifact_id.clone(),
                        (index, run_id.clone(), artifact_type.clone()),
                    )
                    .is_some()
                {
                    return Err(KernelError::Durability(format!(
                        "duplicate artifact production record '{}'",
                        artifact_id
                    )));
                }
            }
        }
        if produced.len() != self.artifacts.all().count() {
            return Err(KernelError::Durability(
                "artifact records and durable artifacts differ".into(),
            ));
        }
        for artifact in self.artifacts.all() {
            let Some((produced_at, run_id, artifact_type)) = produced.get(&artifact.artifact_id)
            else {
                return Err(KernelError::Durability(format!(
                    "artifact '{}' has no production record",
                    artifact.artifact_id
                )));
            };
            if run_id != &artifact.provenance.run_id || artifact_type != &artifact.artifact_type {
                return Err(KernelError::Durability(format!(
                    "artifact '{}' provenance disagrees with its ledger record",
                    artifact.artifact_id
                )));
            }
            if artifact.provenance.produced_by != artifact.provenance.capability.provider {
                return Err(KernelError::Durability(format!(
                    "artifact '{}' producer disagrees with its capability provider",
                    artifact.artifact_id
                )));
            }
            let grant = durable_grants
                .get(&artifact.provenance.grant_id)
                .ok_or_else(|| {
                    KernelError::Durability(format!(
                        "artifact '{}' references unknown grant '{}'",
                        artifact.artifact_id, artifact.provenance.grant_id
                    ))
                })?;
            let run = self.ledger.run_view(run_id)?;
            if &grant.holder != run.initiating_app()
                || !grant.scope.covers(&artifact.provenance.capability)
            {
                return Err(KernelError::Durability(format!(
                    "artifact '{}' grant does not cover its run and capability",
                    artifact.artifact_id
                )));
            }
            let completion = self.ledger.records()[produced_at + 1..]
                .iter()
                .find(|record| !matches!(record.event, LedgerEvent::ArtifactProduced { .. }));
            if !completion.is_some_and(|record| {
                matches!(
                    &record.event,
                    LedgerEvent::CapabilityCompleted {
                        run_id: completed_run,
                        capability,
                        grant_id,
                        data_scope,
                        ..
                    } if completed_run == run_id
                        && capability == &artifact.provenance.capability
                        && grant_id == &artifact.provenance.grant_id
                        && grant.data_scope.covers(data_scope)
                )
            }) {
                return Err(KernelError::Durability(format!(
                    "artifact '{}' has no matching completion record",
                    artifact.artifact_id
                )));
            }
        }
        let revoked: std::collections::BTreeSet<_> =
            self.broker.durable_revocations().into_iter().collect();
        for grant in self.broker.durable_grants() {
            if revoked.contains(&grant.grant_id) {
                continue;
            }
            self.registry.app(&grant.holder).map_err(|_| {
                KernelError::Durability(format!(
                    "live grant '{}' has no installed holder",
                    grant.grant_id
                ))
            })?;
            self.registry.app(grant.scope.provider()).map_err(|_| {
                KernelError::Durability(format!(
                    "live grant '{}' has no installed provider",
                    grant.grant_id
                ))
            })?;
        }
        Ok(())
    }

    /// Derive only metadata already present in `event` or in previously
    /// committed records. This intentionally runs before append, so it must
    /// never depend on run state the event itself would change.
    fn event_view(&self, event: &LedgerEvent) -> KernelResult<AppEventEnvelope> {
        let actor = match event {
            LedgerEvent::RunStarted { initiator, .. } => initiator.app_id().clone(),
            _ => self
                .ledger
                .run_view(event.run_id())?
                .initiating_app()
                .clone(),
        };
        Ok(AppEventEnvelope::from_event(event, actor))
    }

    fn cancel_pending_for_run(&self, run_id: &RunId) {
        for pending in self.pending_invocations.values() {
            if pending.run_id == *run_id {
                pending.cancelled.store(true, Ordering::Release);
            }
        }
    }

    fn pending_cancellation_events_for_run(&self, run_id: &RunId) -> Vec<LedgerEvent> {
        self.pending_invocations
            .values()
            .filter(|pending| pending.run_id == *run_id)
            .map(|pending| LedgerEvent::InvocationCancelled {
                run_id: pending.run_id.clone(),
                capability: pending.capability.clone(),
                data_scope: pending.requested_data_scope.clone(),
            })
            .collect()
    }

    fn reap_cancelled_pending(&mut self) -> KernelResult<()> {
        let cancelled: Vec<_> = self
            .pending_invocations
            .iter()
            .filter(|(_, pending)| {
                pending.cancelled.load(Ordering::Acquire) || Instant::now() >= pending.deadline
            })
            .map(|(id, pending)| {
                (
                    *id,
                    LedgerEvent::InvocationCancelled {
                        run_id: pending.run_id.clone(),
                        capability: pending.capability.clone(),
                        data_scope: pending.requested_data_scope.clone(),
                    },
                )
            })
            .collect();
        if cancelled.is_empty() {
            return Ok(());
        }
        self.record_batch(cancelled.iter().map(|(_, event)| event.clone()).collect())?;
        for (id, _) in cancelled {
            self.pending_invocations.remove(&id);
        }
        Ok(())
    }

    fn pending_cancellation_events_for_app(&self, app_id: &AppId) -> Vec<LedgerEvent> {
        self.pending_invocations
            .values()
            .filter(|pending| {
                pending.acting_app == *app_id || pending.capability.provider == *app_id
            })
            .map(|pending| LedgerEvent::InvocationCancelled {
                run_id: pending.run_id.clone(),
                capability: pending.capability.clone(),
                data_scope: pending.requested_data_scope.clone(),
            })
            .collect()
    }

    fn remove_pending_for_run(&mut self, run_id: &RunId) {
        self.pending_invocations.retain(|_, pending| {
            if pending.run_id == *run_id {
                pending.cancelled.store(true, Ordering::Release);
                false
            } else {
                true
            }
        });
    }

    fn remove_pending_for_app(&mut self, app_id: &AppId) {
        self.pending_invocations.retain(|_, pending| {
            if pending.acting_app == *app_id || pending.capability.provider == *app_id {
                pending.cancelled.store(true, Ordering::Release);
                false
            } else {
                true
            }
        });
    }

    /// A one-shot phase token cannot replay a durable transition after the
    /// handler ran or trusted chrome answered. Refuse further durable work and
    /// let restart resolve the authoritative snapshot instead of continuing
    /// with an audit record that has no terminal event.
    fn require_recovery_after_consumed_transition(&self) {
        if self.state_store.is_some() {
            self.recovery_required.store(true, Ordering::Release);
        }
    }
}
