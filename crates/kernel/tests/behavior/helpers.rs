//! Shared fixtures for the kernel behavioral suite.

pub use app_host_kernel::clock::FixedClock;
pub use app_host_kernel::durable::{DurableKernelState, KernelStateStore, MemoryKernelStateStore};
pub use app_host_kernel::ids::{
    new_run_id, AppId, ArtifactTypeName, CapabilityName, EventTopic, ExtensionPointName,
    ResourceId, SecretName, SecretRef, SurfaceInstanceId, SurfaceName,
};
pub use app_host_kernel::invocation::{
    CapabilityHandler, CapabilityOutcome, InvocationRequest, InvocationResult, RefusalReason,
};
pub use app_host_kernel::kernel::Kernel;
pub use app_host_kernel::manifest::{
    seal, AppManifest, ArtifactTypeDeclaration, AssistantProfileDeclaration, ConnectorDeclaration,
    ExtensionContribution, ExtensionPointDeclaration, GrantRequest, SkillDeclaration,
};
pub use app_host_kernel::primitives::artifact::ArtifactDraft;
pub use app_host_kernel::primitives::capability::{
    CapabilityDeclaration, CapabilityEffect, CapabilityRef,
};
pub use app_host_kernel::primitives::grant::{
    DataScope, DenialReason, GrantCondition, GrantDuration, GrantOrigin, GrantScope,
};
pub use app_host_kernel::primitives::run::{Initiator, InvocationRecord, RunTerminalState};
pub use app_host_kernel::primitives::surface::{ActionIntent, SurfaceDeclaration, SurfaceKind};
pub use app_host_kernel::services::broker::{GrantCheck, IssueResult};
pub use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, TrustedChrome,
    MAX_CAPABILITY_APPROVAL_INPUT_BYTES,
};
pub use app_host_kernel::services::ledger::{LedgerEvent, RunLedger};
pub use app_host_kernel::JsonObject;
pub use app_host_kernel::KernelError;
pub use chrono::{DateTime, Duration, TimeZone, Utc};
pub use serde_json::{json, Value};
pub use std::collections::BTreeMap;
pub use std::num::NonZeroU32;
pub use std::sync::{mpsc, Arc, Mutex};

pub trait KernelTestExt {
    fn install(
        &mut self,
        manifest: app_host_kernel::manifest::SealedManifest,
        handlers: BTreeMap<CapabilityName, CapabilityHandler>,
    ) -> app_host_kernel::KernelResult<Vec<IssueResult>>;
    fn invoke(
        &mut self,
        run_id: &app_host_kernel::ids::RunId,
        capability: &CapabilityRef,
        input: JsonObject,
    ) -> app_host_kernel::KernelResult<InvocationResult>;
    fn invoke_with_data_scope(
        &mut self,
        run_id: &app_host_kernel::ids::RunId,
        capability: &CapabilityRef,
        data_scope: DataScope,
        input: JsonObject,
    ) -> app_host_kernel::KernelResult<InvocationResult>;
    fn submit_action(
        &mut self,
        binding: &app_host_kernel::services::surfaces::SurfaceBinding,
        intent: ActionIntent,
    ) -> app_host_kernel::KernelResult<app_host_kernel::kernel::SurfaceActionOutcome>;
}

impl KernelTestExt for Kernel {
    fn install(
        &mut self,
        manifest: app_host_kernel::manifest::SealedManifest,
        handlers: BTreeMap<CapabilityName, CapabilityHandler>,
    ) -> app_host_kernel::KernelResult<Vec<IssueResult>> {
        let prepared = self.prepare_install(manifest, handlers)?;
        self.commit_install(prepared.await_approval())
    }

    fn invoke(
        &mut self,
        run_id: &app_host_kernel::ids::RunId,
        capability: &CapabilityRef,
        input: JsonObject,
    ) -> app_host_kernel::KernelResult<InvocationResult> {
        self.invoke_with_data_scope(run_id, capability, DataScope::None, input)
    }

    fn invoke_with_data_scope(
        &mut self,
        run_id: &app_host_kernel::ids::RunId,
        capability: &CapabilityRef,
        data_scope: DataScope,
        input: JsonObject,
    ) -> app_host_kernel::KernelResult<InvocationResult> {
        let prepared = match self.prepare_invocation(
            run_id,
            capability,
            app_host_kernel::invocation::InvocationRequest { input, data_scope },
        )? {
            app_host_kernel::kernel::PrepareInvocation::Prepared(prepared) => prepared,
            app_host_kernel::kernel::PrepareInvocation::Refused(result) => return Ok(result),
        };
        match self.authorize_invocation(prepared.await_approval())? {
            app_host_kernel::kernel::AuthorizeInvocation::Authorized(authorized) => {
                self.finalize_invocation(authorized.execute())
            }
            app_host_kernel::kernel::AuthorizeInvocation::Refused(result) => Ok(result),
        }
    }

    fn submit_action(
        &mut self,
        binding: &app_host_kernel::services::surfaces::SurfaceBinding,
        intent: ActionIntent,
    ) -> app_host_kernel::KernelResult<app_host_kernel::kernel::SurfaceActionOutcome> {
        let (run_id, prepared) = self.prepare_surface_action(binding, intent)?;
        let phases = (|| {
            let result = match prepared {
                app_host_kernel::kernel::PrepareInvocation::Prepared(prepared) => {
                    match self.authorize_invocation(prepared.await_approval())? {
                        app_host_kernel::kernel::AuthorizeInvocation::Authorized(authorized) => {
                            self.finalize_invocation(authorized.execute())?
                        }
                        app_host_kernel::kernel::AuthorizeInvocation::Refused(result) => result,
                    }
                }
                app_host_kernel::kernel::PrepareInvocation::Refused(result) => result,
            };
            Ok(result)
        })();
        match phases {
            Ok(result) => {
                let terminal = match result {
                    InvocationResult::Completed { .. } => RunTerminalState::Completed,
                    InvocationResult::Failed { .. } => RunTerminalState::Failed,
                    InvocationResult::Refused { .. } => RunTerminalState::Cancelled,
                };
                self.end_run(&run_id, terminal)?;
                Ok(app_host_kernel::kernel::SurfaceActionOutcome { run_id, result })
            }
            Err(error) => {
                let _ = self.end_run(&run_id, RunTerminalState::Failed);
                Err(error)
            }
        }
    }
}

// -- shared fixtures ---------------------------------------------------------

/// Scriptable trusted chrome that records everything it was asked. Can be
/// told to advance a clock while the user "thinks" about an approval.
pub struct FakeChrome {
    pub grant_decision: Mutex<ApprovalDecision>,
    pub capability_decision: Mutex<ApprovalDecision>,
    pub subscription_decision: Mutex<ApprovalDecision>,
    pub grant_prompts: Mutex<Vec<GrantIssuancePrompt>>,
    pub approval_prompts: Mutex<Vec<CapabilityApprovalPrompt>>,
    pub subscription_prompts: Mutex<Vec<EventSubscriptionPrompt>>,
    pub notices: Mutex<Vec<ChromeNotice>>,
    pub notice_error: Mutex<Option<ChromeNoticeError>>,
    pub advance_on_approval: Mutex<Option<(Arc<FixedClock>, DateTime<Utc>)>>,
}

impl FakeChrome {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            grant_decision: Mutex::new(ApprovalDecision::Approved),
            capability_decision: Mutex::new(ApprovalDecision::Approved),
            subscription_decision: Mutex::new(ApprovalDecision::Approved),
            grant_prompts: Mutex::new(Vec::new()),
            approval_prompts: Mutex::new(Vec::new()),
            subscription_prompts: Mutex::new(Vec::new()),
            notices: Mutex::new(Vec::new()),
            notice_error: Mutex::new(None),
            advance_on_approval: Mutex::new(None),
        })
    }

    pub fn set_grant_decision(&self, decision: ApprovalDecision) {
        *self.grant_decision.lock().unwrap() = decision;
    }

    pub fn set_capability_decision(&self, decision: ApprovalDecision) {
        *self.capability_decision.lock().unwrap() = decision;
    }

    pub fn set_subscription_decision(&self, decision: ApprovalDecision) {
        *self.subscription_decision.lock().unwrap() = decision;
    }

    pub fn set_notice_error(&self, error: ChromeNoticeError) {
        *self.notice_error.lock().unwrap() = Some(error);
    }

    /// Simulate a user who sits on the approval prompt until `at`.
    pub fn advance_clock_on_approval(&self, clock: Arc<FixedClock>, at: DateTime<Utc>) {
        *self.advance_on_approval.lock().unwrap() = Some((clock, at));
    }
}

impl TrustedChrome for FakeChrome {
    fn confirm_grant(&self, prompt: GrantIssuancePrompt) -> ApprovalDecision {
        self.grant_prompts.lock().unwrap().push(prompt);
        *self.grant_decision.lock().unwrap()
    }

    fn approve_capability(&self, prompt: CapabilityApprovalPrompt) -> ApprovalDecision {
        self.approval_prompts.lock().unwrap().push(prompt);
        if let Some((clock, at)) = self.advance_on_approval.lock().unwrap().take() {
            clock.advance_to(at);
        }
        *self.capability_decision.lock().unwrap()
    }

    fn confirm_event_subscriptions(&self, prompt: EventSubscriptionPrompt) -> ApprovalDecision {
        self.subscription_prompts.lock().unwrap().push(prompt);
        *self.subscription_decision.lock().unwrap()
    }

    fn show_notice(&self, notice: ChromeNotice) -> Result<(), ChromeNoticeError> {
        if let Some(error) = self.notice_error.lock().unwrap().clone() {
            return Err(error);
        }
        self.notices.lock().unwrap().push(notice);
        Ok(())
    }
}

pub fn start_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 7, 12, 0, 0).unwrap()
}

pub fn test_kernel() -> (Kernel, Arc<FakeChrome>, Arc<FixedClock>) {
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let kernel = Kernel::with_clock(chrome.clone(), clock.clone());
    (kernel, chrome, clock)
}

pub fn notes_app() -> AppId {
    AppId::new("notes")
}

pub fn chat_app() -> AppId {
    AppId::new("chat")
}

pub fn create_note() -> CapabilityName {
    CapabilityName::new("create_note")
}

pub fn create_note_ref() -> CapabilityRef {
    CapabilityRef {
        provider: notes_app(),
        capability: create_note(),
    }
}

pub fn composer_surface() -> SurfaceName {
    SurfaceName::new("composer")
}

pub fn api_token_secret() -> SecretName {
    SecretName::new("notes-api-token")
}

pub fn expires_after(seconds: u32) -> GrantDuration {
    GrantDuration::ExpiresAfter {
        seconds: NonZeroU32::new(seconds).expect("test durations are non-zero"),
    }
}

pub fn obj(value: Value) -> JsonObject {
    match value {
        Value::Object(object) => object,
        other => panic!("expected JSON object, got {other}"),
    }
}

pub fn create_note_request(condition: GrantCondition, duration: GrantDuration) -> GrantRequest {
    GrantRequest {
        scope: GrantScope::ExactCapability {
            provider: notes_app(),
            capability: create_note(),
        },
        data_scope: DataScope::None,
        condition,
        reason: "cover create_note".into(),
        duration,
    }
}

pub fn create_note_intent(text: &str) -> ActionIntent {
    ActionIntent {
        capability: create_note_ref(),
        input: obj(json!({"text": text})),
        data_scope: DataScope::None,
        goal: format!("note: {text}"),
    }
}

pub fn resource_scope(resource_ids: &[&str]) -> DataScope {
    DataScope::resources(resource_ids.iter().map(|id| ResourceId::new(*id)).collect())
        .expect("resource scope is valid")
}

/// A provider app: one capability, one artifact type, one form surface, and
/// a grant request over its own capability.
pub fn notes_manifest(grant_condition: GrantCondition) -> AppManifest {
    notes_manifest_with_duration(grant_condition, GrantDuration::NonExpiring)
}

pub fn notes_manifest_with_duration(
    grant_condition: GrantCondition,
    duration: GrantDuration,
) -> AppManifest {
    AppManifest {
        app_id: notes_app(),
        version: "1.0.0".into(),
        display_name: "Notes".into(),
        description: "Creates note artifacts".into(),
        capabilities: vec![CapabilityDeclaration {
            name: create_note(),
            description: "Create a note from text".into(),
            input_schema: obj(json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
                "additionalProperties": false,
            })),
            effect: CapabilityEffect::LocalWrite,
            output_schema: Some(obj(json!({"type": "object", "additionalProperties": true}))),
        }],
        surfaces: vec![SurfaceDeclaration {
            name: composer_surface(),
            kind: SurfaceKind::Form,
            title: "Compose note".into(),
            description: "Form for creating a note".into(),
            intents: vec![create_note_ref()],
        }],
        agents: vec![],
        skills: vec![],
        assistant_profiles: vec![],
        automations: vec![],
        connectors: vec![ConnectorDeclaration {
            name: "notes-backend".into(),
            description: "Imaginary remote notes backend".into(),
            secret_names: vec![api_token_secret()],
            config_schema: None,
        }],
        config_declarations: vec![],
        artifact_types: vec![ArtifactTypeDeclaration {
            name: ArtifactTypeName::new("note"),
            description: "A short text note".into(),
            json_schema: obj(json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
                "additionalProperties": false,
            })),
        }],
        extension_points: vec![],
        extension_contributions: vec![],
        grant_requests: vec![create_note_request(grant_condition, duration)],
        event_subscriptions: vec![],
    }
}

pub fn create_note_handler() -> CapabilityHandler {
    Box::new(|input, _context| {
        Ok(CapabilityOutcome {
            result: json!({"created": true}),
            artifacts: vec![ArtifactDraft {
                artifact_type: ArtifactTypeName::new("note"),
                title: "Note".into(),
                content: json!({"text": input["text"]}),
            }],
        })
    })
}

pub fn counting_note_handler(calls: Arc<Mutex<usize>>) -> CapabilityHandler {
    Box::new(move |input, _context| {
        *calls.lock().unwrap() += 1;
        Ok(CapabilityOutcome {
            result: json!({"created": true}),
            artifacts: vec![ArtifactDraft {
                artifact_type: ArtifactTypeName::new("note"),
                title: "Note".into(),
                content: json!({"text": input["text"]}),
            }],
        })
    })
}

pub fn notes_handlers() -> BTreeMap<CapabilityName, CapabilityHandler> {
    BTreeMap::from([(create_note(), create_note_handler())])
}

pub fn install_notes(kernel: &mut Kernel, grant_condition: GrantCondition) {
    kernel
        .install(seal(notes_manifest(grant_condition)), notes_handlers())
        .expect("notes installs");
}

pub fn install_notes_with(kernel: &mut Kernel, handler: CapabilityHandler) {
    kernel
        .install(
            seal(notes_manifest(GrantCondition::Silent)),
            BTreeMap::from([(create_note(), handler)]),
        )
        .expect("notes installs");
}

/// Chat contributes no capabilities and holds no special access: it only
/// requests a grant over the notes capability, like any third-party app.
pub fn chat_manifest(app_id: AppId, subscriptions: Vec<EventTopic>) -> AppManifest {
    AppManifest {
        app_id,
        version: "1.0.0".into(),
        display_name: "Chat".into(),
        description: "Translates natural language into runs".into(),
        capabilities: vec![],
        surfaces: vec![],
        agents: vec![],
        skills: vec![],
        assistant_profiles: vec![],
        automations: vec![],
        connectors: vec![],
        config_declarations: vec![],
        artifact_types: vec![],
        extension_points: vec![],
        extension_contributions: vec![],
        grant_requests: vec![GrantRequest {
            scope: GrantScope::ExactCapability {
                provider: notes_app(),
                capability: create_note(),
            },
            data_scope: DataScope::None,
            condition: GrantCondition::Silent,
            reason: "Create notes from chat messages".into(),
            duration: GrantDuration::NonExpiring,
        }],
        event_subscriptions: subscriptions,
    }
}

pub fn install_chat(kernel: &mut Kernel) {
    let mut manifest = chat_manifest(chat_app(), vec![]);
    if kernel.installed_app(&notes_app()).is_err() {
        manifest.grant_requests.clear();
    }
    kernel
        .install(seal(manifest), BTreeMap::new())
        .expect("chat installs");
}

pub fn install_chat_with_grant_condition(kernel: &mut Kernel, condition: GrantCondition) {
    let mut manifest = chat_manifest(chat_app(), vec![]);
    manifest.grant_requests[0].condition = condition;
    kernel
        .install(seal(manifest), BTreeMap::new())
        .expect("chat installs");
}

pub fn chat_message_run(kernel: &mut Kernel, goal: &str) -> app_host_kernel::ids::RunId {
    kernel
        .start_run(
            Initiator::App {
                app_id: chat_app(),
                reason: "chat message".into(),
            },
            goal,
        )
        .expect("run starts")
}
