//! Stable mapping between kernel `CapabilityRef` values and LLM-provider-safe
//! tool names.
//!
//! Provider naming constraints:
//! - OpenAI: `^[a-zA-Z0-9_]{1,64}$`
//! - Anthropic: `^[a-zA-Z0-9_]{1,64}$`
//! - Ollama (OpenAI-compatible enum): same constraints as OpenAI
//!
//! Strategy:
//!   `mcp-weather/get_forecast` → `mcp_weather__get_forecast`
//!
//! Mapping is deterministic and reversible.

use app_host_kernel::ids::{AppId, ResourceId};
use app_host_kernel::kernel::{CapabilityUseView, Kernel};
use app_host_kernel::primitives::capability::CapabilityRef;
use app_host_kernel::primitives::grant::DataScope;
use app_host_kernel::JsonObject;
use serde_json::Value;

use crate::llm_client::{ToolDefinition, ToolFunction};

/// Longest tool name the providers accept (OpenAI/Anthropic/Ollama: 64).
pub const MAX_TOOL_NAME_LEN: usize = 64;
pub const HOST_INPUT_ANNOTATION: &str = "x-kestral-host-input";
pub const CURRENT_CHAT_THREAD_ID: &str = "current-chat-thread-id";
pub const MANAGED_DATA_PROPOSAL_ANNOTATION: &str = "x-kestral-managed-data-proposal";
pub const MANAGED_DATA_SCOPE_ANNOTATION: &str = "x-kestral-managed-data-scope";

pub(crate) fn invocation_data_scope(
    kernel: &Kernel,
    holder: &AppId,
    capability: &CapabilityRef,
    input: &JsonObject,
) -> DataScope {
    managed_data_invocation_data_scope(kernel, capability, input)
        .or_else(|| crate::artifacts_app::invocation_data_scope(kernel, holder, capability, input))
        .unwrap_or_else(|| crate::file_resources::invocation_data_scope(capability, input))
}

fn managed_data_invocation_data_scope(
    kernel: &Kernel,
    capability: &CapabilityRef,
    input: &JsonObject,
) -> Option<DataScope> {
    let app = kernel.installed_app(&capability.provider).ok()?;
    let declaration = app
        .manifest
        .capabilities
        .iter()
        .find(|declaration| declaration.name == capability.capability)?;
    if declaration
        .input_schema
        .get(MANAGED_DATA_PROPOSAL_ANNOTATION)
        != Some(&Value::Bool(true))
    {
        return None;
    }
    let properties = declaration.input_schema.get("properties")?.as_object()?;
    let (field, value) = properties.iter().find_map(|(field, schema)| {
        let annotation = schema
            .as_object()?
            .get(MANAGED_DATA_SCOPE_ANNOTATION)?
            .as_object()?;
        Some((field.as_str(), annotation))
    })?;
    let kind = value.get("kind")?.as_str()?;
    let collection = value.get("collection")?.as_str()?;
    let resource = match kind {
        "collection" if field == "targetGeneration" => {
            crate::managed_data::resource_id(&capability.provider, collection)
        }
        "record" if field == "targetId" => crate::managed_data::record_resource_id(
            &capability.provider,
            collection,
            input.get(field)?.as_str()?,
        ),
        "document" if field == "targetId" => crate::managed_data::document_resource_id(
            &capability.provider,
            collection,
            input.get(field)?.as_str()?,
        ),
        _ => return None,
    };
    DataScope::resources(vec![ResourceId::new(resource)]).ok()
}

#[derive(Clone)]
pub(crate) struct ChatToolBinding {
    pub(crate) capability: CapabilityRef,
    injected_input: JsonObject,
}

impl ChatToolBinding {
    pub(crate) fn bind(&self, mut arguments: JsonObject) -> JsonObject {
        for (name, value) in &self.injected_input {
            arguments.insert(name.clone(), value.clone());
        }
        arguments
    }
}

pub(crate) struct ChatTool {
    pub(crate) definition: ToolDefinition,
    pub(crate) binding: ChatToolBinding,
}

/// Convert a `CapabilityUseView` into an LLM tool definition using `name` as the
/// tool's provider-facing name. The caller owns naming so the definition's name
/// always matches the reverse-lookup key it stores (see [`unique_tool_name`]).
pub fn capability_view_to_tool_def_named(view: &CapabilityUseView, name: String) -> ToolDefinition {
    let capability = CapabilityRef {
        provider: view.provider_app_id.clone(),
        capability: view.capability.clone(),
    };
    let data_scopes = view
        .authorizations
        .iter()
        .map(|authorization| authorization.data_scope.clone())
        .collect::<Vec<_>>();
    ToolDefinition {
        type_: "function".into(),
        function: ToolFunction {
            name,
            description: format!("[{}] {}", view.provider_display_name, view.description),
            parameters: crate::file_resources::constrain_tool_schema_to_grants(
                &capability,
                &view.input_schema,
                &data_scopes,
            ),
        },
    }
}

/// Build a model-visible Chat tool and its trusted invocation binding. A
/// host-bound property stays in the kernel schema but is removed from the
/// model schema and supplied from host state immediately before invocation.
/// If the required host context is unavailable, the tool is not offered.
pub(crate) fn capability_view_to_chat_tool(
    view: &CapabilityUseView,
    name: String,
    current_chat_thread_id: Option<&str>,
) -> Result<Option<ChatTool>, String> {
    let mut definition = capability_view_to_tool_def_named(view, name);
    let mut injected_input = JsonObject::new();
    let annotated = definition
        .function
        .parameters
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| {
            properties
                .iter()
                .filter_map(|(property, schema)| {
                    schema
                        .as_object()
                        .and_then(|schema| schema.get(HOST_INPUT_ANNOTATION))
                        .map(|source| (property.clone(), source.clone(), schema.clone()))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    for (property, source, schema) in &annotated {
        let required = definition
            .function
            .parameters
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| required.iter().any(|name| name.as_str() == Some(property)));
        if !required {
            return Err(format!(
                "capability {}/{} host-bound property '{property}' must be required",
                view.provider_app_id, view.capability
            ));
        }
        let value = match source.as_str() {
            Some(CURRENT_CHAT_THREAD_ID) => {
                if schema.get("type").and_then(Value::as_str) != Some("string") {
                    return Err(format!(
                        "capability {}/{} host-bound property '{property}' must have type string",
                        view.provider_app_id, view.capability
                    ));
                }
                let Some(thread_id) = current_chat_thread_id.filter(|value| !value.is_empty())
                else {
                    return Ok(None);
                };
                Value::String(thread_id.to_string())
            }
            Some(crate::permissions_app::ACTIVE_PERMISSIONS_HOST_INPUT) => {
                if view.provider_app_id != crate::permissions_app::permissions_app_id()
                    || view.capability
                        != app_host_kernel::ids::CapabilityName::new(
                            crate::permissions_app::LIST_ACTIVE,
                        )
                {
                    return Err(format!(
                        "capability {}/{} cannot use reserved host input '{source}'",
                        view.provider_app_id, view.capability
                    ));
                }
                schema.get("const").cloned().ok_or_else(|| {
                    format!(
                        "capability {}/{} host-bound property '{property}' must declare const",
                        view.provider_app_id, view.capability
                    )
                })?
            }
            Some(crate::permissions_app::REQUESTABLE_PERMISSIONS_HOST_INPUT) => {
                if view.provider_app_id != crate::permissions_app::permissions_app_id()
                    || (view.capability
                        != app_host_kernel::ids::CapabilityName::new(
                            crate::permissions_app::LIST_REQUESTABLE,
                        )
                        && view.capability
                            != app_host_kernel::ids::CapabilityName::new(
                                crate::permissions_app::PROPOSE_GRANT,
                            ))
                {
                    return Err(format!(
                        "capability {}/{} cannot use reserved host input '{source}'",
                        view.provider_app_id, view.capability
                    ));
                }
                schema.get("const").cloned().ok_or_else(|| {
                    format!(
                        "capability {}/{} host-bound property '{property}' must declare const",
                        view.provider_app_id, view.capability
                    )
                })?
            }
            _ => {
                return Err(format!(
                    "capability {}/{} property '{property}' has unsupported {HOST_INPUT_ANNOTATION}",
                    view.provider_app_id, view.capability
                ));
            }
        };
        injected_input.insert(property.clone(), value);
    }

    if !annotated.is_empty() {
        if let Some(properties) = definition
            .function
            .parameters
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        {
            for (property, _, _) in &annotated {
                properties.remove(property);
            }
        }
        if let Some(required) = definition
            .function
            .parameters
            .get_mut("required")
            .and_then(Value::as_array_mut)
        {
            required.retain(|name| {
                !annotated
                    .iter()
                    .any(|(property, _, _)| name.as_str() == Some(property))
            });
        }
    }

    Ok(Some(ChatTool {
        definition,
        binding: ChatToolBinding {
            capability: CapabilityRef {
                provider: view.provider_app_id.clone(),
                capability: view.capability.clone(),
            },
            injected_input,
        },
    }))
}

/// Convert a `CapabilityRef` to a provider-safe tool name.
///
/// Rules:
/// - `/` becomes `__` (double underscore)
/// - `-`, `.`, ` ` become `_` (single underscore)
/// - other non-ASCII-alphanumeric chars are stripped
/// - the result is capped at [`MAX_TOOL_NAME_LEN`]
///
/// Deterministic but NOT injective (several characters fold to `_`), so
/// distinct capabilities can yield the same name; callers building a name set
/// de-collide with [`unique_tool_name`].
pub fn cap_ref_to_tool_name(cap_ref: &CapabilityRef) -> String {
    let raw = format!("{}/{}", cap_ref.provider, cap_ref.capability);
    sanitize_tool_name(&raw)
}

/// A provider-safe tool name for `cap_ref` that does not collide with any name
/// for which `is_taken` returns true. On the rare fold-collision, appends a
/// numeric suffix (kept within [`MAX_TOOL_NAME_LEN`]) instead of forcing the
/// caller to drop the entire tool set.
pub fn unique_tool_name(cap_ref: &CapabilityRef, is_taken: impl Fn(&str) -> bool) -> String {
    let base = cap_ref_to_tool_name(cap_ref);
    if !is_taken(&base) {
        return base;
    }
    let mut suffix: u32 = 2;
    loop {
        let tag = format!("_{suffix}");
        let head = base.len().min(MAX_TOOL_NAME_LEN.saturating_sub(tag.len()));
        let candidate = format!("{}{tag}", &base[..head]);
        if !is_taken(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn sanitize_tool_name(raw: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch == '/' {
            result.push('_');
            result.push('_');
        } else if ch == '-' || ch == '.' || ch == ' ' {
            result.push('_');
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            result.push(ch);
        }
    }
    let trimmed = result.trim_matches('_');
    // Providers cap tool names at 64 chars. All retained characters are ASCII,
    // so truncating on a byte boundary is also a char boundary.
    let capped = if trimmed.len() > MAX_TOOL_NAME_LEN {
        trimmed[..MAX_TOOL_NAME_LEN].trim_matches('_')
    } else {
        trimmed
    };
    if capped.is_empty() {
        "unknown_tool".into()
    } else {
        capped.to_string()
    }
}

#[cfg(test)]
mod tests;
