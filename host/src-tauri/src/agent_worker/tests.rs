use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use app_host_kernel::manifest::{seal, AppManifest, GrantRequest};
use app_host_kernel::primitives::capability::{CapabilityDeclaration, CapabilityEffect};
use app_host_kernel::primitives::grant::{DataScope, GrantCondition, GrantDuration, GrantScope};
use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, TrustedChrome,
};
use serde_json::json;

fn obj(value: Value) -> JsonObject {
    value
        .as_object()
        .cloned()
        .expect("schema literal is object")
}

struct ApprovingChrome {
    capability_prompts: AtomicUsize,
}

impl TrustedChrome for ApprovingChrome {
    fn confirm_grant(&self, _prompt: GrantIssuancePrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn approve_capability(&self, _prompt: CapabilityApprovalPrompt) -> ApprovalDecision {
        self.capability_prompts.fetch_add(1, Ordering::Relaxed);
        ApprovalDecision::Approved
    }

    fn confirm_event_subscriptions(&self, _prompt: EventSubscriptionPrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn show_notice(&self, _notice: ChromeNotice) -> Result<(), ChromeNoticeError> {
        Ok(())
    }
}

struct ToolCallingEngine;

impl AgentEngine for ToolCallingEngine {
    fn run(
        &self,
        job: AgentJob,
        bridge: &dyn AgentHostBridge,
        _progress: &ProgressReporter,
        cancellation: &CancellationHandle,
    ) -> Result<AgentResult, String> {
        assert_eq!(job.tools.len(), 1);
        let outcome = bridge.invoke_tool(
            &job.tools[0].function.name,
            JsonObject::from_iter([("value".into(), json!("hello"))]),
            Duration::from_secs(5),
            &|| cancellation.is_cancelled(),
        )?;
        let ToolInvocationOutcome::Completed(content) = outcome else {
            return Err("tool did not complete".into());
        };
        assert!(content.starts_with("UNTRUSTED TOOL OUTPUT"));
        Ok(AgentResult {
            text: "finished".into(),
            reasoning: None,
            finish_reason: agent_worker_protocol::AgentFinishReason::Stop,
            turns: 2,
            transcript: vec![ChatMessage {
                role: "assistant".into(),
                content: "finished".into(),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
        })
    }
}

fn empty_manifest(app_id: &str, capabilities: Vec<CapabilityDeclaration>) -> AppManifest {
    AppManifest {
        app_id: AppId::new(app_id),
        version: "1.0.0".into(),
        display_name: app_id.into(),
        description: app_id.into(),
        capabilities,
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
        grant_requests: vec![],
        event_subscriptions: vec![],
    }
}

fn install(
    kernel: &mut Kernel,
    manifest: AppManifest,
    handlers: BTreeMap<CapabilityName, CapabilityHandler>,
) {
    let prepared = kernel.prepare_install(seal(manifest), handlers).unwrap();
    kernel.commit_install(prepared.await_approval()).unwrap();
}

#[test]
fn manifest_is_headless_and_requests_no_grants() {
    let manifest = test_agent_manifest();
    assert_eq!(manifest.app_id, test_agent_app_id());
    assert!(manifest.surfaces.is_empty());
    assert!(manifest.grant_requests.is_empty());
    assert_eq!(manifest.capabilities.len(), 1);
    assert_eq!(manifest.capabilities[0].name.as_str(), AGENT_RUN);
}

#[test]
fn agent_handler_enforces_payload_and_duration_limits() {
    let short_duration = obj(json!({
        "messages": [{}],
        "max_duration_secs": MIN_AGENT_DURATION_SECS - 1,
    }));
    assert!(validate_agent_run_limits(&short_duration)
        .unwrap_err()
        .0
        .contains("max_duration_secs"));

    let oversized = obj(json!({
        "messages": [{"content": "x".repeat(MAX_AGENT_PAYLOAD_BYTES as usize)}],
        "max_payload_bytes": MAX_AGENT_PAYLOAD_BYTES * 2,
    }));
    assert!(validate_agent_run_limits(&oversized)
        .unwrap_err()
        .0
        .contains("byte limit"));
}

#[test]
fn tool_output_is_bounded_and_marked_untrusted() {
    let result = bounded_tool_result("external", &Value::String("x".repeat(40_000)));
    assert!(result.starts_with("UNTRUSTED TOOL OUTPUT"));
    assert!(result.chars().count() < MAX_TOOL_RESULT_CHARS + 256);
}

#[test]
fn invoker_capacity_enforces_per_app_limit() {
    let capacity = Arc::new(InvokerCapacity::default());
    let app_id = AppId::new("caller");
    let permits: Vec<_> = (0..MAX_OUTSTANDING_INVOCATIONS_PER_APP)
        .map(|_| capacity.reserve(&app_id).unwrap())
        .collect();

    let error = capacity.reserve(&app_id).err().unwrap();
    assert!(error.contains("invocation limit reached"), "{error}");

    drop(permits);
    assert!(capacity.reserve(&app_id).is_ok());
}

#[test]
fn trampoline_runs_tools_as_caller_attributed_grandchildren() {
    let chrome = Arc::new(ApprovingChrome {
        capability_prompts: AtomicUsize::new(0),
    });
    let kernel = Arc::new(Mutex::new(Kernel::new(chrome.clone())));
    let invoker =
        install_test_agent_with_engine(kernel.clone(), Arc::new(ToolCallingEngine)).unwrap();
    {
        let mut kernel = kernel.lock().unwrap();
        install(
            &mut kernel,
            empty_manifest("caller", vec![]),
            BTreeMap::new(),
        );
        let echo: CapabilityHandler = Box::new(|input, _| {
            assert_eq!(input.get("thread_id"), Some(&json!("thread-agent")));
            Ok(CapabilityOutcome {
                result: Value::Object(input.clone()),
                artifacts: vec![],
            })
        });
        let hidden: CapabilityHandler = Box::new(|_, _| {
            Ok(CapabilityOutcome {
                result: json!({}),
                artifacts: vec![],
            })
        });
        install(
            &mut kernel,
            empty_manifest(
                "tools",
                vec![
                    CapabilityDeclaration {
                        name: CapabilityName::new("echo"),
                        description: "Echo input".into(),
                        input_schema: obj(json!({
                            "type": "object",
                            "properties": {
                                "value": {"type": "string"},
                                "thread_id": {
                                    "type": "string",
                                    "x-kestral-host-input": "current-chat-thread-id"
                                }
                            },
                            "required": ["value", "thread_id"],
                            "additionalProperties": false
                        })),
                        effect: CapabilityEffect::ReadOnly,
                        output_schema: Some(obj(json!({"type": "object"}))),
                    },
                    CapabilityDeclaration {
                        name: CapabilityName::new("hidden"),
                        description: "Must not be exposed".into(),
                        input_schema: obj(json!({"type": "object"})),
                        effect: CapabilityEffect::ReadOnly,
                        output_schema: Some(obj(json!({"type": "object"}))),
                    },
                ],
            ),
            BTreeMap::from([
                (CapabilityName::new("echo"), echo),
                (CapabilityName::new("hidden"), hidden),
            ]),
        );
        for (provider, capability, condition) in [
            (TEST_AGENT_APP_ID, AGENT_RUN, GrantCondition::Silent),
            ("tools", "echo", GrantCondition::RequiresApproval),
            ("tools", "hidden", GrantCondition::Silent),
        ] {
            kernel
                .issue_grant(
                    &AppId::new("caller"),
                    &GrantRequest {
                        scope: GrantScope::ExactCapability {
                            provider: AppId::new(provider),
                            capability: CapabilityName::new(capability),
                        },
                        data_scope: DataScope::None,
                        condition,
                        reason: "test".into(),
                        duration: GrantDuration::NonExpiring,
                    },
                )
                .unwrap();
        }
    }
    let (parent, agent_run, prepared) = {
        let mut kernel = kernel.lock().unwrap();
        let parent = kernel
            .start_run(
                Initiator::App {
                    app_id: AppId::new("caller"),
                    reason: "chat".into(),
                },
                "chat",
            )
            .unwrap();
        let agent_run = kernel
            .start_run(
                Initiator::Run {
                    app_id: AppId::new("caller"),
                    parent_run_id: parent.clone(),
                },
                "agent",
            )
            .unwrap();
        let PrepareInvocation::Prepared(prepared) = kernel
            .prepare_invocation(
                &agent_run,
                &CapabilityRef {
                    provider: test_agent_app_id(),
                    capability: CapabilityName::new(AGENT_RUN),
                },
                app_host_kernel::invocation::InvocationRequest {
                    input: JsonObject::from_iter([
                        (
                            "messages".into(),
                            json!([{"role": "user", "content": "use echo"}]),
                        ),
                        (
                            "tools".into(),
                            json!({"allow_capabilities": ["tools/echo"]}),
                        ),
                    ]),
                    data_scope: DataScope::None,
                },
            )
            .unwrap()
        else {
            panic!("agent invocation refused");
        };
        (parent, agent_run, prepared)
    };
    let _chat_context = invoker
        .bind_chat_thread(&agent_run, "thread-agent")
        .unwrap();
    let approval = prepared.await_approval();
    let authorized = {
        let mut kernel = kernel.lock().unwrap();
        let AuthorizeInvocation::Authorized(authorized) =
            kernel.authorize_invocation(approval).unwrap()
        else {
            panic!("agent invocation refused during authorization");
        };
        authorized
    };
    let executed = authorized.execute();
    let result = kernel
        .lock()
        .unwrap()
        .finalize_invocation(executed)
        .unwrap();
    assert!(matches!(result, InvocationResult::Completed { .. }));
    let mut kernel = kernel.lock().unwrap();
    kernel
        .end_run(&agent_run, RunTerminalState::Completed)
        .unwrap();
    kernel
        .end_run(&parent, RunTerminalState::Completed)
        .unwrap();
    assert_eq!(chrome.capability_prompts.load(Ordering::Relaxed), 1);
    let grandchildren = kernel
        .records()
        .iter()
        .filter(|record| {
            matches!(
                &record.event,
                app_host_kernel::services::ledger::LedgerEvent::RunStarted {
                    initiator: Initiator::Run { app_id, parent_run_id },
                    ..
                } if app_id == &AppId::new("caller") && parent_run_id == &agent_run
            )
        })
        .count();
    assert_eq!(grandchildren, 1);
}
