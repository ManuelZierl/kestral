//! Degraded-mode bridge for plain MCP servers.
//!
//! Any bare MCP server becomes an installable app with zero author effort:
//!
//! - each tool schema becomes a capability plus a generic input **form**
//!   surface; tool output schemas are imported when the server provides them
//! - each tool result becomes a generic **result card** artifact
//! - each capability gets a requires-approval grant request (the safe
//!   default), confirmed through trusted chrome at install
//!
//! The kernel sees none of this as MCP: it receives an ordinary sealed
//! manifest and ordinary capability handlers. Nothing here installs or
//! grants anything by itself — installation still runs through
//! phased kernel installation and its trusted-chrome prompts.
//!
//! Functional, safe, unremarkable-looking. Servers that ship a full manifest
//! or MCP Apps surfaces upgrade progressively past this bridge.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{json, Value};

use app_host_kernel::ids::{AppId, ArtifactTypeName, CapabilityName, SurfaceName};
use app_host_kernel::invocation::{
    CapabilityHandler, CapabilityOutcome, HandlerFailure, InvocationContext,
};
use app_host_kernel::manifest::{
    seal, AppManifest, ArtifactTypeDeclaration, GrantRequest, SealedManifest,
};
use app_host_kernel::primitives::artifact::ArtifactDraft;
use app_host_kernel::primitives::capability::{
    CapabilityDeclaration, CapabilityEffect, CapabilityRef,
};
use app_host_kernel::primitives::grant::{DataScope, GrantCondition, GrantDuration, GrantScope};
use app_host_kernel::primitives::surface::{SurfaceDeclaration, SurfaceKind};
use app_host_kernel::JsonObject;

use crate::protocol::McpToolDefinition;

pub const RESULT_CARD_ARTIFACT_TYPE: &str = "mcp-result-card";

/// The bridge's seam to the wire: given a tool name and validated arguments,
/// call the server and return the JSON result (or an app-level failure).
pub type McpToolCall =
    Arc<dyn Fn(&str, &JsonObject, &InvocationContext) -> Result<Value, String> + Send + Sync>;

/// Degraded mode by definition cannot know a tool's result shape, so the
/// result-card schema only pins the card structure, not the payload.
fn result_card_schema() -> JsonObject {
    let schema = json!({
        "type": "object",
        "properties": {
            "tool": {"type": "string"},
            "result": {},
        },
        "required": ["tool", "result"],
        "additionalProperties": false,
    });
    match schema {
        Value::Object(object) => object,
        _ => unreachable!("literal above is an object"),
    }
}

/// Derive a full, sealed app manifest from bare MCP tool declarations.
/// Callers must hand in tools that passed [`crate::protocol::validate_tools`]
/// (the client's `list_tools` already guarantees this).
pub fn manifest_for_mcp_server(
    server_id: &AppId,
    display_name: &str,
    version: &str,
    description: &str,
    tools: &[McpToolDefinition],
) -> SealedManifest {
    let capabilities = tools
        .iter()
        .map(|tool| CapabilityDeclaration {
            name: CapabilityName::new(&tool.name),
            description: tool.description.clone(),
            input_schema: tool.input_schema.clone(),
            // MCP tool metadata carries no reliable effect semantics, so the
            // bridge stays conservative. With the default requires-approval
            // grant, Unspecified also keeps generated-form submissions behind
            // trusted chrome.
            effect: CapabilityEffect::Unspecified,
            output_schema: tool.output_schema.clone(),
        })
        .collect();
    let mut surfaces: Vec<SurfaceDeclaration> = tools
        .iter()
        .map(|tool| SurfaceDeclaration {
            name: SurfaceName::new(format!("{}-form", tool.name)),
            kind: SurfaceKind::Form,
            title: format!("{} input", tool.name),
            description: format!("Auto-rendered form for the '{}' tool schema", tool.name),
            intents: vec![CapabilityRef {
                provider: server_id.clone(),
                capability: CapabilityName::new(&tool.name),
            }],
        })
        .collect();
    surfaces.push(SurfaceDeclaration {
        name: SurfaceName::new("result-cards"),
        kind: SurfaceKind::Card,
        title: format!("{display_name} results"),
        description: "Auto-rendered cards for tool results".to_string(),
        intents: Vec::new(),
    });
    let grant_requests = tools
        .iter()
        .map(|tool| GrantRequest {
            scope: GrantScope::ExactCapability {
                provider: server_id.clone(),
                capability: CapabilityName::new(&tool.name),
            },
            data_scope: DataScope::None,
            condition: GrantCondition::RequiresApproval,
            reason: format!("Invoke MCP tool '{}': {}", tool.name, tool.description),
            // Non-expiring is safe here only because every use is approved
            // interactively; the grant itself confers no silent authority.
            duration: GrantDuration::NonExpiring,
        })
        .collect();
    seal(AppManifest {
        app_id: server_id.clone(),
        version: version.to_string(),
        display_name: display_name.to_string(),
        description: description.to_string(),
        capabilities,
        surfaces,
        agents: Vec::new(),
        skills: Vec::new(),
        assistant_profiles: Vec::new(),
        automations: Vec::new(),
        connectors: Vec::new(),
        config_declarations: Vec::new(),
        artifact_types: vec![ArtifactTypeDeclaration {
            name: ArtifactTypeName::new(RESULT_CARD_ARTIFACT_TYPE),
            description: "Generic card wrapping one MCP tool result".to_string(),
            json_schema: result_card_schema(),
        }],
        extension_points: Vec::new(),
        extension_contributions: Vec::new(),
        grant_requests,
        event_subscriptions: Vec::new(),
    })
}

/// Bind every bridged capability to a handler that calls the server and
/// wraps the result as a generic result-card artifact. Remote failures of
/// any kind surface as `HandlerFailure` — a contained invocation failure.
pub fn handlers_for_mcp_server(
    tools: &[McpToolDefinition],
    call_tool: McpToolCall,
) -> BTreeMap<CapabilityName, CapabilityHandler> {
    tools
        .iter()
        .map(|tool| {
            let tool_name = tool.name.clone();
            let call_tool = call_tool.clone();
            let handler: CapabilityHandler = Box::new(move |input, context| {
                if context.cancellation.is_cancelled() {
                    return Err(HandlerFailure("MCP tool call cancelled".into()));
                }
                let result = call_tool(&tool_name, input, context).map_err(HandlerFailure)?;
                Ok(CapabilityOutcome {
                    result: result.clone(),
                    artifacts: vec![ArtifactDraft {
                        artifact_type: ArtifactTypeName::new(RESULT_CARD_ARTIFACT_TYPE),
                        title: format!("{tool_name} result"),
                        content: json!({"tool": tool_name, "result": result}),
                    }],
                })
            });
            (CapabilityName::new(&tool.name), handler)
        })
        .collect()
}
