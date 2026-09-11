//! Bridge behavior against a real kernel — including acceptance criterion 5,
//! which lives here because the kernel is protocol-agnostic and the MCP
//! adapter owns the degraded-mode story.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use app_host_kernel::ids::{AppId, ArtifactTypeName, CapabilityName, SurfaceName};
use app_host_kernel::invocation::{CapabilityHandler, InvocationRequest, InvocationResult};
use app_host_kernel::kernel::Kernel;
use app_host_kernel::primitives::capability::{CapabilityEffect, CapabilityRef};
use app_host_kernel::primitives::grant::DataScope;
use app_host_kernel::primitives::grant::GrantCondition;
use app_host_kernel::primitives::run::{Initiator, RunTerminalState};
use app_host_kernel::primitives::surface::{ActionIntent, SurfaceKind};
use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, TrustedChrome,
};
use app_host_kernel::JsonObject;

use mcp_adapter::{
    handlers_for_mcp_server, manifest_for_mcp_server, McpToolDefinition, RESULT_CARD_ARTIFACT_TYPE,
};

fn install(
    kernel: &mut Kernel,
    manifest: app_host_kernel::manifest::SealedManifest,
    handlers: std::collections::BTreeMap<CapabilityName, CapabilityHandler>,
) {
    let prepared = kernel.prepare_install(manifest, handlers).unwrap();
    kernel.commit_install(prepared.await_approval()).unwrap();
}

fn invoke(
    kernel: &mut Kernel,
    run_id: &app_host_kernel::ids::RunId,
    capability: &CapabilityRef,
    input: JsonObject,
) -> InvocationResult {
    let prepared = match kernel
        .prepare_invocation(
            run_id,
            capability,
            InvocationRequest {
                input,
                data_scope: DataScope::None,
            },
        )
        .unwrap()
    {
        app_host_kernel::kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::kernel::PrepareInvocation::Refused(result) => return result,
    };
    match kernel
        .authorize_invocation(prepared.await_approval())
        .unwrap()
    {
        app_host_kernel::kernel::AuthorizeInvocation::Authorized(authorized) => {
            kernel.finalize_invocation(authorized.execute()).unwrap()
        }
        app_host_kernel::kernel::AuthorizeInvocation::Refused(result) => result,
    }
}

/// Approves everything and counts capability-approval prompts.
struct ApprovingChrome {
    approval_prompts: Mutex<Vec<CapabilityApprovalPrompt>>,
}

impl ApprovingChrome {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            approval_prompts: Mutex::new(Vec::new()),
        })
    }
}

impl TrustedChrome for ApprovingChrome {
    fn confirm_grant(&self, _prompt: GrantIssuancePrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn approve_capability(&self, prompt: CapabilityApprovalPrompt) -> ApprovalDecision {
        self.approval_prompts.lock().unwrap().push(prompt);
        ApprovalDecision::Approved
    }

    fn confirm_event_subscriptions(&self, _prompt: EventSubscriptionPrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn show_notice(&self, _notice: ChromeNotice) -> Result<(), ChromeNoticeError> {
        Ok(())
    }
}

fn obj(value: Value) -> JsonObject {
    match value {
        Value::Object(object) => object,
        other => panic!("expected JSON object, got {other}"),
    }
}

fn forecast_tools() -> Vec<McpToolDefinition> {
    vec![McpToolDefinition {
        name: "get_forecast".into(),
        description: "Get the weather forecast for a city".into(),
        input_schema: obj(json!({
            "type": "object",
            "properties": {"city": {"type": "string"}},
            "required": ["city"],
            "additionalProperties": false,
        })),
        output_schema: None,
    }]
}

/// Criterion 5: a bare MCP server, bridged with zero author effort,
/// does real work safely.
#[test]
fn criterion_5_degraded_mode_does_real_work_safely() {
    let chrome = ApprovingChrome::new();
    let mut kernel = Kernel::new(chrome.clone());
    let server_id = AppId::new("mcp-weather");
    let tools = forecast_tools();
    let manifest = manifest_for_mcp_server(
        &server_id,
        "Weather (MCP)",
        "0.1.0",
        "A plain MCP server, bridged",
        &tools,
    );
    // Safe defaults: every bridged capability is requires-approval, and
    // the bridge derives a form per tool plus a result-card surface.
    assert!(manifest
        .manifest
        .grant_requests
        .iter()
        .all(|g| g.condition == GrantCondition::RequiresApproval));
    assert!(manifest
        .manifest
        .capabilities
        .iter()
        .all(|capability| capability.effect == CapabilityEffect::Unspecified));
    let surface_kinds: Vec<(&str, SurfaceKind)> = manifest
        .manifest
        .surfaces
        .iter()
        .map(|s| (s.name.as_str(), s.kind))
        .collect();
    assert!(surface_kinds.contains(&("get_forecast-form", SurfaceKind::Form)));
    assert!(surface_kinds.contains(&("result-cards", SurfaceKind::Card)));
    let form_surface = manifest
        .manifest
        .surfaces
        .iter()
        .find(|surface| surface.name.as_str() == "get_forecast-form")
        .expect("bridge declares a form surface per tool");
    assert_eq!(
        form_surface.intents,
        vec![CapabilityRef {
            provider: AppId::new("mcp-weather"),
            capability: CapabilityName::new("get_forecast"),
        }]
    );
    assert!(manifest
        .manifest
        .artifact_types
        .iter()
        .any(|t| t.name.as_str() == RESULT_CARD_ARTIFACT_TYPE));

    let handlers = handlers_for_mcp_server(
        &tools,
        Arc::new(|tool_name: &str, arguments: &JsonObject, _context| {
            assert_eq!(tool_name, "get_forecast");
            Ok(json!({"city": arguments["city"], "forecast": "sunny"}))
        }),
    );
    install(&mut kernel, manifest, handlers);

    // The generated form is the provider's own surface, but a bare MCP tool
    // has an unspecified effect. Its default approval-required grant must
    // therefore still enter trusted chrome rather than treating the form
    // submit click as sufficient consent.
    let binding = kernel
        .open_surface(&server_id, &SurfaceName::new("get_forecast-form"))
        .unwrap();
    let (run_id, prepared) = kernel
        .prepare_surface_action(
            &binding,
            ActionIntent {
                capability: CapabilityRef {
                    provider: server_id,
                    capability: CapabilityName::new("get_forecast"),
                },
                input: obj(json!({"city": "Berlin"})),
                data_scope: DataScope::None,
                goal: "check the weather in Berlin".into(),
            },
        )
        .unwrap();
    let prepared = match prepared {
        app_host_kernel::kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::kernel::PrepareInvocation::Refused(result) => {
            panic!("expected prepared MCP form action, got {result:?}")
        }
    };
    let result = match kernel
        .authorize_invocation(prepared.await_approval())
        .unwrap()
    {
        app_host_kernel::kernel::AuthorizeInvocation::Authorized(authorized) => {
            kernel.finalize_invocation(authorized.execute()).unwrap()
        }
        app_host_kernel::kernel::AuthorizeInvocation::Refused(result) => result,
    };
    kernel
        .end_run(&run_id, RunTerminalState::Completed)
        .unwrap();

    let InvocationResult::Completed { result, artifacts } = result else {
        panic!("expected completion");
    };
    assert_eq!(result, json!({"city": "Berlin", "forecast": "sunny"}));
    assert_eq!(chrome.approval_prompts.lock().unwrap().len(), 1);
    let card = &artifacts[0];
    assert_eq!(
        card.artifact_type,
        ArtifactTypeName::new(RESULT_CARD_ARTIFACT_TYPE)
    );
    assert_eq!(
        card.content,
        json!({"tool": "get_forecast", "result": {"city": "Berlin", "forecast": "sunny"}})
    );
    assert_eq!(card.provenance.run_id, run_id);
}

#[test]
fn conservative_unspecified_effects_for_mcp_bridge() {
    let manifest = manifest_for_mcp_server(
        &AppId::new("mcp-weather"),
        "Weather (MCP)",
        "0.1.0",
        "A plain MCP server, bridged",
        &forecast_tools(),
    );
    for cap in &manifest.manifest.capabilities {
        assert_eq!(
            cap.effect,
            CapabilityEffect::Unspecified,
            "MCP bridge capability '{}' should have Unspecified effect",
            cap.name
        );
        assert!(
            cap.output_schema.is_none(),
            "without an advertised outputSchema, capability '{}' has none",
            cap.name
        );
    }
}

/// An advertised output schema is imported into the capability declaration,
/// and the kernel then enforces it: a result violating the schema becomes a
/// contained invocation failure.
#[test]
fn advertised_output_schemas_are_imported_and_enforced() {
    let chrome = ApprovingChrome::new();
    let mut kernel = Kernel::new(chrome);
    let server_id = AppId::new("mcp-typed");
    let tools = vec![McpToolDefinition {
        name: "typed_tool".into(),
        description: "Returns a typed result".into(),
        input_schema: obj(json!({"type": "object", "additionalProperties": false})),
        output_schema: Some(obj(json!({
            "type": "object",
            "properties": {"answer": {"type": "number"}},
            "required": ["answer"],
            "additionalProperties": false,
        }))),
    }];
    let manifest = manifest_for_mcp_server(&server_id, "Typed (MCP)", "0.1.0", "typed", &tools);
    assert!(manifest.manifest.capabilities[0].output_schema.is_some());

    // A server that violates its own advertised output schema fails the
    // invocation — contained, attributable, no artifact committed.
    let handlers = handlers_for_mcp_server(
        &tools,
        Arc::new(|_tool: &str, _arguments: &JsonObject, _context| {
            Ok(json!({"answer": "not a number"}))
        }),
    );
    install(&mut kernel, manifest, handlers);
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: server_id.clone(),
                reason: "typed call".into(),
            },
            "typed tool call",
        )
        .unwrap();
    let result = invoke(
        &mut kernel,
        &run_id,
        &CapabilityRef {
            provider: server_id,
            capability: CapabilityName::new("typed_tool"),
        },
        obj(json!({})),
    );
    assert!(matches!(result, InvocationResult::Failed { .. }));
}

/// A remote error (transport, server, or tool) surfaces as a contained
/// invocation failure — never a panic, never a poisoned kernel.
#[test]
fn remote_errors_become_contained_invocation_failures() {
    let chrome = ApprovingChrome::new();
    let mut kernel = Kernel::new(chrome);
    let server_id = AppId::new("mcp-flaky");
    let tools = forecast_tools();
    let manifest = manifest_for_mcp_server(&server_id, "Flaky (MCP)", "0.1.0", "flaky", &tools);
    let handlers = handlers_for_mcp_server(
        &tools,
        Arc::new(|_tool: &str, _arguments: &JsonObject, _context| {
            Err("MCP transport failure: connection reset".to_string())
        }),
    );
    install(&mut kernel, manifest, handlers);
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: server_id.clone(),
                reason: "flaky call".into(),
            },
            "flaky tool call",
        )
        .unwrap();
    let result = invoke(
        &mut kernel,
        &run_id,
        &CapabilityRef {
            provider: server_id,
            capability: CapabilityName::new("get_forecast"),
        },
        obj(json!({"city": "Berlin"})),
    );
    let InvocationResult::Failed { error } = result else {
        panic!("expected contained failure");
    };
    assert!(error.contains("transport failure"));
    // The kernel is still fully usable after the remote failure.
    kernel.end_run(&run_id, RunTerminalState::Failed).unwrap();
}
