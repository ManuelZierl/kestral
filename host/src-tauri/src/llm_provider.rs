//! LLM Provider App — an ordinary app providing model capabilities.
//!
//! Capabilities:
//!   - `llm.generate`: chat completion with optional tool-call support
//!   - `llm.models.list`: list models known to the configured provider
//!   - `llm.models.refresh`: refresh and list the configured provider's models
//!
//! Connectors:
//!   - `openai_api_key`     → OpenAI API key
//!   - `anthropic_api_key`  → Anthropic API key (future)
//!   - `ollama_base_url`    → Custom Ollama base URL
//!   - `llm_base_url`       → Generic OpenAI-compatible base URL
//!   - `llm_api_key`        → API key for generic endpoint
//!
//! This is not the agent. This is a dumb capability provider. The agent loop,
//! tool-selection context, transcript handling, and stop conditions live in
//! the consuming app (Chat, Research, Code, etc.).
//!
//! Provider work is executed through the kernel's phased invocation adapter,
//! so the invocation-scoped worker never holds the host kernel mutex.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use app_host_kernel::ids::{AppId, ArtifactTypeName, CapabilityName, SurfaceName};
use app_host_kernel::invocation::{CapabilityHandler, CapabilityOutcome, HandlerFailure};
use app_host_kernel::kernel::Kernel;
use app_host_kernel::manifest::{seal, AppManifest, ArtifactTypeDeclaration, ConnectorDeclaration};
use app_host_kernel::primitives::artifact::ArtifactDraft;
use app_host_kernel::primitives::capability::{
    CapabilityDeclaration, CapabilityEffect, CapabilityRef,
};
use app_host_kernel::primitives::grant::GrantOrigin;
use app_host_kernel::primitives::surface::{SurfaceDeclaration, SurfaceKind};
use app_host_kernel::JsonObject;
use app_host_kernel::KernelResult;

use crate::config::{active_llm_api_key_secret, HostConfigService, LlmProfileRuntime};
use crate::llm_client::{
    backend_from_profile_secrets, models_from_profile_secrets, ChatMessage, CompletionRequest,
    LlmBackend, LlmProviderError, NormalizedModel, OAuthCredential, ToolDefinition,
};

fn parse_required_array<T: serde::de::DeserializeOwned>(
    input: &JsonObject,
    field: &str,
) -> Result<Vec<T>, HandlerFailure> {
    let values = input
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| HandlerFailure(format!("{field} must be an array")))?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            serde_json::from_value(value.clone())
                .map_err(|error| HandlerFailure(format!("invalid {field}[{index}]: {error}")))
        })
        .collect()
}

fn parse_optional_array<T: serde::de::DeserializeOwned>(
    input: &JsonObject,
    field: &str,
) -> Result<Option<Vec<T>>, HandlerFailure> {
    match input.get(field) {
        None => Ok(None),
        Some(Value::Array(_)) => {
            let parsed = parse_required_array(input, field)?;
            if parsed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(parsed))
            }
        }
        Some(_) => Err(HandlerFailure(format!("{field} must be an array"))),
    }
}

pub fn llm_provider_app_id() -> AppId {
    AppId::new("llm-provider")
}

pub const LLM_RESPONSE_ARTIFACT_TYPE: &str = "llm-response";
pub const NO_PROVIDER_CONFIGURED_ERROR: &str = "no LLM provider profile is configured";

fn object(value: Value) -> JsonObject {
    match value {
        Value::Object(object) => object,
        _ => unreachable!("literals below are objects"),
    }
}

fn generate_input_schema() -> JsonObject {
    object(json!({
        "type": "object",
        "properties": {
            "model": {
                "type": "string",
                "minLength": 1,
                "description": "Model identifier (e.g. gpt-4o, llama3.2)"
            },
            "profile": {
                "type": "string",
                "minLength": 1,
                "description": "Connector profile id to pin for this call \
                                (e.g. llm-provider/local-ollama). Defaults to \
                                the active default profile."
            },
            "messages": {
                "type": "array",
                "items": message_schema(),
                "minItems": 1
            },
            "tools": {
                "type": "array",
                "items": tool_definition_schema(),
                "description": "Tool definitions in OpenAI function-calling format"
            },
            "response_format": {
                "type": "object",
                "description": "JSON Schema for structured output"
            },
            "reasoning": {
                "enum": ["minimal", "low", "medium", "high", "xhigh", "max"],
                "description": "Provider-neutral reasoning effort"
            },
            "temperature": {
                "type": "number",
                "minimum": 0,
                "maximum": 2,
                "description": "Sampling temperature (0.0-2.0)"
            },
            "max_output_tokens": {
                "type": "integer",
                "minimum": 1,
                "description": "Maximum tokens in the response"
            }
        },
        "required": ["messages"],
        "additionalProperties": false
    }))
}

fn tool_call_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {"type": "string", "minLength": 1},
            "type": {"const": "function"},
            "function": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "arguments": {"type": "string"}
                },
                "required": ["name", "arguments"],
                "additionalProperties": false
            }
        },
        "required": ["id", "type", "function"],
        "additionalProperties": false
    })
}

fn message_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "role": {"const": "system"},
                    "content": {"type": "string"}
                },
                "required": ["role", "content"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "role": {"const": "user"},
                    "content": {"type": "string"}
                },
                "required": ["role", "content"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "role": {"const": "assistant"},
                    "content": {"type": "string"},
                    "tool_calls": {"type": "array", "items": tool_call_schema()}
                },
                "required": ["role", "content"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "role": {"const": "tool"},
                    "content": {"type": "string"},
                    "tool_call_id": {"type": "string", "minLength": 1},
                    "name": {"type": "string", "minLength": 1}
                },
                "required": ["role", "content", "tool_call_id"],
                "additionalProperties": false
            }
        ]
    })
}

fn tool_definition_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "type": {"const": "function"},
            "function": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "description": {"type": "string"},
                    "parameters": {"type": "object"}
                },
                "required": ["name", "description", "parameters"],
                "additionalProperties": false
            }
        },
        "required": ["type", "function"],
        "additionalProperties": false
    })
}

fn assistant_message_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "role": {"const": "assistant"},
            "content": {"type": "string"},
            "tool_calls": {"type": "array", "items": tool_call_schema()}
        },
        "required": ["role", "content"],
        "additionalProperties": false
    })
}

fn llm_response_schema() -> JsonObject {
    object(json!({
        "type": "object",
        "properties": {
            "message": assistant_message_schema(),
            "reasoning": {"type": "string"},
            "finish_reason": {"type": "string"},
            "usage": {
                "type": "object",
                "properties": {
                    "prompt_tokens": {"type": "integer", "minimum": 0},
                    "completion_tokens": {"type": "integer", "minimum": 0},
                    "total_tokens": {"type": "integer", "minimum": 0},
                    "cache_read_tokens": {"type": "integer", "minimum": 0},
                    "cache_write_tokens": {"type": "integer", "minimum": 0},
                    "cost": {"type": "number", "minimum": 0}
                },
                "additionalProperties": false
            },
            "provider_metrics": {
                "type": "object",
                "properties": {
                    "time_to_first_token_ms": {"type": "integer", "minimum": 0},
                    "total_latency_ms": {"type": "integer", "minimum": 0}
                },
                "required": ["total_latency_ms"],
                "additionalProperties": false
            }
        },
        "required": ["message", "finish_reason"],
        "additionalProperties": false
    }))
}

fn models_input_schema() -> JsonObject {
    object(json!({
        "type": "object",
        "properties": {
            "profile": {"type": "string", "minLength": 1}
        },
        "additionalProperties": false
    }))
}

fn models_output_schema() -> JsonObject {
    object(json!({
        "type": "object",
        "properties": {
            "models": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "display_name": {"type": "string"},
                        "reasoning": {"type": "boolean"},
                        "context_window": {"type": "integer", "minimum": 0},
                        "max_output_tokens": {"type": "integer", "minimum": 0}
                    },
                    "required": ["id", "display_name", "reasoning", "context_window", "max_output_tokens"],
                    "additionalProperties": false
                }
            },
            "refreshed": {"type": "boolean"}
        },
        "required": ["models", "refreshed"],
        "additionalProperties": false
    }))
}

fn connector_declaration(
    name: &str,
    description: &str,
    requires_secret: bool,
) -> ConnectorDeclaration {
    ConnectorDeclaration {
        name: name.into(),
        description: description.into(),
        secret_names: requires_secret
            .then(active_llm_api_key_secret)
            .into_iter()
            .collect(),
        config_schema: Some(object(json!({
            "type": "object",
            "properties": {
                "base_url": {"type": "string", "minLength": 1},
                "default_model": {"type": "string", "minLength": 1}
            },
            "required": ["base_url", "default_model"],
            "additionalProperties": false
        }))),
    }
}

pub fn llm_provider_manifest() -> AppManifest {
    AppManifest {
        app_id: llm_provider_app_id(),
        version: "0.4.0".into(),
        display_name: "LLM Provider".into(),
        description: "Provides generation and model discovery through broker-mediated secrets"
            .into(),
        capabilities: vec![
            CapabilityDeclaration {
                name: CapabilityName::new("llm.generate"),
                description: "Send a chat completion request to an LLM with optional tool calls."
                    .into(),
                input_schema: generate_input_schema(),
                effect: CapabilityEffect::ExternalWrite,
                output_schema: Some(llm_response_schema()),
            },
            CapabilityDeclaration {
                name: CapabilityName::new("llm.models.list"),
                description: "List models known to an LLM provider profile.".into(),
                input_schema: models_input_schema(),
                effect: CapabilityEffect::ReadOnly,
                output_schema: Some(models_output_schema()),
            },
            CapabilityDeclaration {
                name: CapabilityName::new("llm.models.refresh"),
                description: "Refresh and list models for an LLM provider profile.".into(),
                input_schema: models_input_schema(),
                effect: CapabilityEffect::ExternalWrite,
                output_schema: Some(models_output_schema()),
            },
        ],
        surfaces: vec![SurfaceDeclaration {
            name: SurfaceName::new("llm.generate-input"),
            kind: SurfaceKind::Form,
            title: "LLM Generate".into(),
            description: "Form for sending a prompt to the LLM".into(),
            intents: vec![CapabilityRef {
                provider: llm_provider_app_id(),
                capability: CapabilityName::new("llm.generate"),
            }],
        }],
        agents: vec![],
        skills: vec![],
        assistant_profiles: vec![],
        automations: vec![],
        connectors: vec![
            connector_declaration("ollama", "Local Ollama instance", false),
            connector_declaration(
                "open-ai-compatible",
                "Generic OpenAI-compatible endpoint",
                false,
            ),
            connector_declaration("openai", "OpenAI API access", true),
            connector_declaration("anthropic", "Anthropic API access", true),
            connector_declaration("anthropic-oauth", "Anthropic OAuth access", true),
            connector_declaration("openai-codex", "OpenAI Codex OAuth access", true),
            connector_declaration("github-copilot", "GitHub Copilot OAuth access", true),
            connector_declaration("openrouter", "OpenRouter API access", true),
            connector_declaration("google", "Google Gemini API access", true),
            connector_declaration("mistral", "Mistral API access", true),
            connector_declaration("amazon-bedrock", "Amazon Bedrock bearer access", true),
        ],
        config_declarations: vec![],
        artifact_types: vec![ArtifactTypeDeclaration {
            name: ArtifactTypeName::new(LLM_RESPONSE_ARTIFACT_TYPE),
            description:
                "An LLM response artifact: message, tool calls, usage, and provider metrics".into(),
            json_schema: llm_response_schema(),
        }],
        extension_points: vec![],
        extension_contributions: vec![],
        // The LLM provider itself needs no grants — it provides capabilities
        // rather than consuming them. Consuming apps hold grants over
        // llm-provider/llm.generate.
        grant_requests: vec![],
        event_subscriptions: vec![],
    }
}

/// Build handlers for the LLM provider app.
///
/// The default model is used when the input does not specify one.
pub fn llm_provider_handlers(
    host_config: Arc<Mutex<HostConfigService>>,
) -> BTreeMap<CapabilityName, CapabilityHandler> {
    let generate_config = Arc::clone(&host_config);
    let generate_handler: CapabilityHandler = Box::new(move |input, context| {
        if context.cancellation.is_cancelled() {
            return Err(HandlerFailure("LLM request cancelled".into()));
        }
        let Some(profile) = resolve_profile(input, &generate_config)? else {
            return Err(HandlerFailure(NO_PROVIDER_CONFIGURED_ERROR.into()));
        };
        let model = input
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(&profile.default_model)
            .to_string();

        let messages: Vec<ChatMessage> = parse_required_array(input, "messages")?;

        let tools: Option<Vec<ToolDefinition>> = parse_optional_array(input, "tools")?;

        let response_format: Option<JsonObject> = input
            .get("response_format")
            .and_then(|v| v.as_object().cloned());

        let reasoning = reasoning_for_request(input, profile.default_variant);
        let text_verbosity = profile.default_text_verbosity;

        let temperature = input.get("temperature").and_then(Value::as_f64);

        let max_tokens = input.get("max_output_tokens").and_then(Value::as_u64);

        let oauth_credential = read_profile_oauth_credential(&generate_config, &profile)?;
        let backend = backend_from_profile_secrets(
            &context.secrets,
            profile.kind,
            profile.base_url.clone(),
            profile.api_key_secret_ref.is_some(),
            oauth_credential,
        )
        .map_err(|e| HandlerFailure(format!("LLM backend init failed: {e}")))?;

        let request = CompletionRequest {
            model,
            messages,
            tools,
            response_format,
            reasoning,
            text_verbosity,
            temperature,
            max_tokens,
        };

        context.progress.report(json!({"kind": "llm-stream-start"}));
        let response = complete_provider_request_and_persist(
            backend.as_ref(),
            &request,
            &|content, reasoning| {
                context.progress.report(json!({
                    "kind": "llm-stream-delta",
                    "content": content,
                    "reasoning": reasoning,
                }));
            },
            &|| context.cancellation.is_cancelled(),
            &generate_config,
            &profile,
        );
        let response = response?;
        if context.cancellation.is_cancelled() {
            return Err(HandlerFailure("LLM request cancelled".into()));
        }

        let serialized = serde_json::to_value(&response).map_err(|error| {
            HandlerFailure(format!("failed to serialize LLM response: {error}"))
        })?;
        Ok(CapabilityOutcome {
            result: serialized.clone(),
            artifacts: vec![ArtifactDraft {
                artifact_type: ArtifactTypeName::new(LLM_RESPONSE_ARTIFACT_TYPE),
                title: format!("LLM response ({})", response.finish_reason),
                content: serialized,
            }],
        })
    });

    BTreeMap::from([
        (CapabilityName::new("llm.generate"), generate_handler),
        (
            CapabilityName::new("llm.models.list"),
            models_handler(Arc::clone(&host_config), false),
        ),
        (
            CapabilityName::new("llm.models.refresh"),
            models_handler(host_config, true),
        ),
    ])
}

fn reasoning_for_request(
    input: &JsonObject,
    default_variant: Option<crate::config::ModelVariant>,
) -> Option<String> {
    input
        .get("reasoning")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| default_variant.map(|variant| variant.as_str().to_string()))
}

fn resolve_profile(
    input: &JsonObject,
    host_config: &Arc<Mutex<HostConfigService>>,
) -> Result<Option<LlmProfileRuntime>, HandlerFailure> {
    let requested_profile = input.get("profile").and_then(Value::as_str);
    // Handlers execute outside the kernel lock, and config operations release
    // this mutex before network I/O. Wait for a brief config transition rather
    // than turning ordinary Settings activity into a failed model request.
    let guard = host_config
        .lock()
        .map_err(|_| HandlerFailure("host config lock poisoned".into()))?;
    let active = guard.current_llm_profile().map_err(HandlerFailure)?;
    let (profile, is_active_default) = match requested_profile {
        None => return Ok(active),
        Some(id) => {
            let is_active_default = active
                .as_ref()
                .is_some_and(|active| id == active.connector_id);
            (
                guard.llm_profile(id).map_err(HandlerFailure)?,
                is_active_default,
            )
        }
    };

    // The broker exposes only the active profile's synthetic credential.
    if !is_active_default
        && (profile.api_key_secret_ref.is_some() || profile.oauth_secret_ref.is_some())
    {
        return Err(HandlerFailure(format!(
            "LLM profile '{}' was pinned for this request but is no longer \
             the active default, so its credential is unavailable. Retry with \
             the current default profile.",
            profile.connector_id
        )));
    }
    Ok(Some(profile))
}

fn models_handler(host_config: Arc<Mutex<HostConfigService>>, refresh: bool) -> CapabilityHandler {
    Box::new(move |input, context| {
        if context.cancellation.is_cancelled() {
            return Err(HandlerFailure("LLM models request cancelled".into()));
        }
        let profile = resolve_profile(input, &host_config)?
            .ok_or_else(|| HandlerFailure(NO_PROVIDER_CONFIGURED_ERROR.into()))?;
        let needs_api_key = profile.api_key_secret_ref.is_some();
        let oauth_credential = read_profile_oauth_credential(&host_config, &profile)?;
        let result = models_from_profile_secrets(
            &context.secrets,
            profile.kind,
            profile.base_url.clone(),
            profile.default_model.clone(),
            needs_api_key,
            oauth_credential,
            refresh,
        );
        let (models, update) = match result {
            Ok(value) => value,
            Err(error) => return Err(HandlerFailure(format!("LLM models call failed: {error}"))),
        };
        let models =
            models.map_err(|error| HandlerFailure(format!("LLM models call failed: {error}")))?;
        // Persist any OAuth rotation best-effort: a transient host-config lock
        // must not discard a successful result (the write retries next call).
        if let Err(error) = persist_oauth_credential(&host_config, &profile, update) {
            eprintln!("[llm-provider] {}", error.0);
        }
        if context.cancellation.is_cancelled() {
            return Err(HandlerFailure("LLM models request cancelled".into()));
        }
        Ok(CapabilityOutcome {
            result: models_capability_result(models, refresh),
            artifacts: vec![],
        })
    })
}

// Model variants are host profile metadata. Keep the sealed 0.3 capability
// contract stable while Settings uses the richer direct discovery view.
fn models_capability_result(models: Vec<NormalizedModel>, refreshed: bool) -> Value {
    let models = models
        .into_iter()
        .map(|model| {
            json!({
                "id": model.id,
                "display_name": model.display_name,
                "reasoning": model.reasoning,
                "context_window": model.context_window,
                "max_output_tokens": model.max_output_tokens,
            })
        })
        .collect::<Vec<_>>();
    json!({"models": models, "refreshed": refreshed})
}

fn read_profile_oauth_credential(
    host_config: &Arc<Mutex<HostConfigService>>,
    profile: &LlmProfileRuntime,
) -> Result<Option<String>, HandlerFailure> {
    if profile.oauth_secret_ref.is_none() {
        return Ok(None);
    }
    host_config
        .lock()
        .map_err(|_| HandlerFailure("host config lock poisoned".into()))?
        .read_llm_profile_oauth_credential(&profile.connector_id)
        .map_err(HandlerFailure)
}

fn persist_credential_update(
    host_config: &Arc<Mutex<HostConfigService>>,
    profile: &LlmProfileRuntime,
    backend: &dyn LlmBackend,
) -> Result<(), HandlerFailure> {
    let update = backend
        .take_credential_update()
        .map_err(|error| HandlerFailure(format!("read OAuth rotation failed: {error}")))?;
    persist_oauth_credential(host_config, profile, update)
}

fn persist_oauth_credential(
    host_config: &Arc<Mutex<HostConfigService>>,
    profile: &LlmProfileRuntime,
    update: Option<OAuthCredential>,
) -> Result<(), HandlerFailure> {
    let Some(update) = update else {
        return Ok(());
    };
    let serialized = update.serialize().map_err(HandlerFailure)?;
    host_config
        .lock()
        .map_err(|_| HandlerFailure("host config lock poisoned".into()))?
        .write_llm_profile_oauth_credential_persisted(&profile.connector_id, serialized)
        .map_err(|error| HandlerFailure(format!("persist OAuth rotation failed: {error}")))
}

fn complete_provider_request(
    backend: &dyn LlmBackend,
    request: &CompletionRequest,
    on_delta: &dyn Fn(&str, &str),
    is_cancelled: &dyn Fn() -> bool,
) -> Result<crate::llm_client::LlmResponse, LlmProviderError> {
    backend.complete_stream_interruptible(request, on_delta, is_cancelled)
}

fn complete_provider_request_and_persist(
    backend: &dyn LlmBackend,
    request: &CompletionRequest,
    on_delta: &dyn Fn(&str, &str),
    is_cancelled: &dyn Fn() -> bool,
    host_config: &Arc<Mutex<HostConfigService>>,
    profile: &LlmProfileRuntime,
) -> Result<crate::llm_client::LlmResponse, HandlerFailure> {
    let result = complete_provider_request(backend, request, on_delta, is_cancelled)
        .map_err(|error| HandlerFailure(error.to_string()));
    // Persist any OAuth rotation best-effort: a transient host-config lock must
    // not discard a successful completion (the write retries next call).
    if let Err(error) = persist_credential_update(host_config, profile, backend) {
        eprintln!("[llm-provider] {}", error.0);
    }
    result
}

#[cfg(test)]
pub fn install_llm_provider(
    kernel: &mut Kernel,
    host_config: Arc<Mutex<HostConfigService>>,
) -> KernelResult<()> {
    let prepared = kernel.prepare_install_with_grant_origin(
        seal(llm_provider_manifest()),
        llm_provider_handlers(host_config),
        GrantOrigin::SystemBundled,
    )?;
    kernel.commit_install(prepared.await_approval()).map(|_| ())
}

/// Install a fake LLM provider app with deterministic responses.
///
/// Uses the same manifest as the real provider (same app_id, same
/// capabilities), so every grant and capability introspection path
/// works identically. Only the handler differs: it returns responses
/// from the provided queue instead of calling a live backend.
///
/// Panics if the responses list is empty.
pub fn install_fake_llm_provider(
    kernel: &mut Kernel,
    responses: Vec<serde_json::Value>,
) -> KernelResult<()> {
    install_fake_llm_provider_recording(kernel, responses, None)
}

/// Like [`install_fake_llm_provider`], but every `llm.generate` input is
/// also pushed into `recorded_inputs` so tests can assert what the model
/// actually received (e.g. conversation history).
pub fn install_fake_llm_provider_recording(
    kernel: &mut Kernel,
    responses: Vec<serde_json::Value>,
    recorded_inputs: Option<std::sync::Arc<std::sync::Mutex<Vec<JsonObject>>>>,
) -> KernelResult<()> {
    use app_host_kernel::invocation::{CapabilityOutcome, HandlerFailure};
    use std::sync::Arc;

    let manifest = llm_provider_manifest();
    let responses = Arc::new(std::sync::Mutex::new(responses));

    let generate_handler: app_host_kernel::invocation::CapabilityHandler = {
        let responses = responses.clone();
        Box::new(move |input, _context| {
            if let Some(recorder) = &recorded_inputs {
                recorder
                    .lock()
                    .map_err(|_| HandlerFailure("fake LLM input recorder poisoned".into()))?
                    .push(input.clone());
            }
            let mut locked = responses
                .lock()
                .map_err(|_| HandlerFailure("fake LLM response queue poisoned".into()))?;
            let response = locked.first().cloned().unwrap_or_else(|| {
                serde_json::json!({
                    "message": {"role": "assistant", "content": "fallback reply"},
                    "finish_reason": "stop"
                })
            });
            if !locked.is_empty() {
                locked.remove(0);
            }
            Ok(CapabilityOutcome {
                result: response,
                artifacts: vec![],
            })
        })
    };

    let list_handler: CapabilityHandler = Box::new(|_, _| {
        Ok(CapabilityOutcome {
            result: json!({"models": [], "refreshed": false}),
            artifacts: vec![],
        })
    });
    let refresh_handler: CapabilityHandler = Box::new(|_, _| {
        Ok(CapabilityOutcome {
            result: json!({"models": [], "refreshed": true}),
            artifacts: vec![],
        })
    });
    let handlers = BTreeMap::from([
        (CapabilityName::new("llm.generate"), generate_handler),
        (CapabilityName::new("llm.models.list"), list_handler),
        (CapabilityName::new("llm.models.refresh"), refresh_handler),
    ]);
    let prepared = kernel.prepare_install_with_grant_origin(
        seal(manifest),
        handlers,
        GrantOrigin::SystemBundled,
    )?;
    kernel.commit_install(prepared.await_approval())?;
    Ok(())
}

#[cfg(test)]
mod tests;
