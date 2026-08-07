//! Invocation-scoped bridge to the bundled pi-ai provider worker.
//!
//! Provider credentials arrive through the broker's [`SecretResolver`] and
//! exist only in the NDJSON request written to the worker's stdin.

use std::env;
use std::fmt;
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(all(test, windows))]
use std::thread;
#[cfg(test)]
use std::{io::Read, path::PathBuf, sync::mpsc};

use app_host_kernel::JsonObject;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use uuid::Uuid;

use crate::chrome::{OAuthControl, OAuthPublicEvent, PendingOAuthSessions};
use crate::config::{active_llm_api_key_secret, ConnectorKind, ModelVariant, TextVerbosity};
use crate::node_worker::{NodeWorkerProcess, WorkerPathConfig, WorkerPaths};

const PROTOCOL_VERSION: u32 = 2;
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_TIMEOUT_SECS: u64 = 120;
pub(crate) const RESPONSE_TIMEOUT: Duration = Duration::from_secs(RESPONSE_TIMEOUT_SECS);
pub(crate) const INVOCATION_TIMEOUT: Duration = Duration::from_secs(RESPONSE_TIMEOUT_SECS + 5);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(1);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
#[cfg(test)]
const MAX_STDOUT_LINE_BYTES: usize = crate::node_worker::MAX_STDOUT_LINE_BYTES;
const MAX_PUBLIC_STRING_BYTES: usize = 16 * 1024;
const MAX_URL_BYTES: usize = 8 * 1024;
const MAX_CREDENTIAL_BYTES: usize = 1024 * 1024;
const MAX_CREDENTIAL_TOKEN_BYTES: usize = 256 * 1024;
const MAX_CREDENTIAL_DEPTH: usize = 12;
const MAX_CREDENTIAL_NODES: usize = 4096;
const MAX_CREDENTIAL_KEYS: usize = 256;
const MAX_CREDENTIAL_ARRAY: usize = 1024;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone)]
pub struct LlmProviderError(pub String);

impl fmt::Display for LlmProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for LlmProviderError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub type_: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: JsonObject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderMetrics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u64>,
    pub total_latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmResponse {
    pub message: ChatMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metrics: Option<ProviderMetrics>,
    pub finish_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedModel {
    pub id: String,
    pub display_name: String,
    pub reasoning: bool,
    #[serde(default)]
    pub variants: Vec<ModelVariant>,
    pub text_verbosity: Vec<TextVerbosity>,
    pub context_window: u64,
    pub max_output_tokens: u64,
}

pub trait LlmBackend: Send + Sync {
    fn complete(&self, request: &CompletionRequest) -> Result<LlmResponse, LlmProviderError>;

    fn complete_stream_interruptible(
        &self,
        request: &CompletionRequest,
        on_delta: &dyn Fn(&str, &str),
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<LlmResponse, LlmProviderError> {
        if is_cancelled() {
            return Err(LlmProviderError("request cancelled".into()));
        }
        let response = self.complete(request)?;
        on_delta(
            &response.message.content,
            response.reasoning.as_deref().unwrap_or(""),
        );
        Ok(response)
    }

    fn take_credential_update(&self) -> Result<Option<OAuthCredential>, LlmProviderError> {
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OAuthCredential(Value);

impl OAuthCredential {
    pub fn parse_serialized(serialized: &str) -> Result<Self, String> {
        if serialized.len() > MAX_CREDENTIAL_BYTES {
            return Err("OAuth credential is too large".into());
        }
        let value: Value = serde_json::from_str(serialized)
            .map_err(|error| format!("invalid OAuth credential JSON: {error}"))?;
        validate_oauth_credential(&value)?;
        Ok(Self(value))
    }

    pub fn serialize(&self) -> Result<String, String> {
        serde_json::to_string(&self.0)
            .map_err(|error| format!("serialize OAuth credential failed: {error}"))
    }
}

impl Serialize for OAuthCredential {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for OAuthCredential {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        validate_oauth_credential(&value).map_err(serde::de::Error::custom)?;
        Ok(Self(value))
    }
}

fn validate_oauth_credential(value: &Value) -> Result<(), String> {
    let root = value
        .as_object()
        .ok_or_else(|| "OAuth credential must be an object".to_string())?;
    if root.get("type").and_then(Value::as_str) != Some("oauth") {
        return Err("OAuth credential type must be oauth".into());
    }
    for name in ["access", "refresh"] {
        let token = root
            .get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("OAuth credential {name} must be a non-empty string"))?;
        if token.len() > MAX_CREDENTIAL_TOKEN_BYTES {
            return Err(format!("OAuth credential {name} is too long"));
        }
    }
    let expires = root
        .get("expires")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| "OAuth credential expires must be finite and non-negative".to_string())?;
    let _ = expires;

    fn visit(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), String> {
        *nodes += 1;
        if *nodes > MAX_CREDENTIAL_NODES || depth > MAX_CREDENTIAL_DEPTH {
            return Err("OAuth credential is too complex".into());
        }
        match value {
            Value::String(value) if value.len() > MAX_PUBLIC_STRING_BYTES => {
                Err("OAuth credential contains an oversized string".into())
            }
            Value::Array(values) => {
                if values.len() > MAX_CREDENTIAL_ARRAY {
                    return Err("OAuth credential array is too large".into());
                }
                for value in values {
                    visit(value, depth + 1, nodes)?;
                }
                Ok(())
            }
            Value::Object(values) => {
                if values.len() > MAX_CREDENTIAL_KEYS {
                    return Err("OAuth credential object has too many fields".into());
                }
                for (key, value) in values {
                    if key.is_empty()
                        || key.len() > 256
                        || matches!(key.as_str(), "__proto__" | "prototype" | "constructor")
                    {
                        return Err("OAuth credential contains an invalid field name".into());
                    }
                    if depth == 0
                        && matches!(key.as_str(), "type" | "access" | "refresh" | "expires")
                    {
                        continue;
                    }
                    visit(value, depth + 1, nodes)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    let mut nodes = 0;
    visit(value, 0, &mut nodes)?;
    if serde_json::to_vec(value)
        .map_err(|error| format!("serialize OAuth credential failed: {error}"))?
        .len()
        > MAX_CREDENTIAL_BYTES
    {
        return Err("OAuth credential is too large".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OAuthPrompt {
    Text {
        message: String,
        placeholder: Option<String>,
    },
    Secret {
        message: String,
        placeholder: Option<String>,
    },
    ManualCode {
        message: String,
        placeholder: Option<String>,
    },
    Select {
        message: String,
        options: Vec<OAuthPromptOption>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthPromptOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<JsonObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_verbosity: Option<TextVerbosity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ProviderKind {
    Ollama,
    OpenAiCompatible,
    Openai,
    OpenaiCodex,
    Anthropic,
    GithubCopilot,
    Openrouter,
    Google,
    Mistral,
    AmazonBedrock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    kind: ProviderKind,
    base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oauth_credential: Option<OAuthCredential>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OAuthProviderConfig {
    kind: ProviderKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case", deny_unknown_fields)]
enum WorkerCommand {
    Generate {
        request_id: String,
        provider: ProviderConfig,
        model: String,
        messages: Vec<ChatMessage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tools: Option<Vec<ToolDefinition>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        response_format: Option<JsonObject>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        text_verbosity: Option<TextVerbosity>,
        #[serde(skip_serializing_if = "Option::is_none")]
        temperature: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_output_tokens: Option<u64>,
        timeout_ms: u64,
    },
    ModelsList {
        request_id: String,
        provider: ProviderConfig,
    },
    ModelsRefresh {
        request_id: String,
        provider: ProviderConfig,
    },
    OauthLogin {
        request_id: String,
        provider: OAuthProviderConfig,
    },
    OauthPromptResponse {
        request_id: String,
        target_request_id: String,
        prompt_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        cancelled: bool,
    },
    Cancel {
        request_id: String,
        target_request_id: String,
    },
    Shutdown {
        request_id: String,
    },
}

impl WorkerCommand {
    fn validate(&self) -> Result<(), String> {
        let provider = match self {
            Self::Generate { provider, .. }
            | Self::ModelsList { provider, .. }
            | Self::ModelsRefresh { provider, .. } => Some(provider),
            _ => None,
        };
        if provider.is_some_and(|provider| {
            provider.api_key.is_some() && provider.oauth_credential.is_some()
        }) {
            return Err(
                "worker provider API key and OAuth credential are mutually exclusive".into(),
            );
        }
        if let Self::OauthPromptResponse {
            value, cancelled, ..
        } = self
        {
            if *cancelled == value.is_some() {
                return Err("OAuth prompt response requires exactly one response".into());
            }
            if value.as_ref().is_some_and(|value| value.len() > 16_384) {
                return Err("OAuth prompt response is too long".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum WorkerEvent {
    Ready {
        protocol_version: u32,
    },
    StreamDelta {
        request_id: String,
        content: String,
        reasoning: String,
    },
    Completed {
        request_id: String,
        response: WorkerResponse,
        #[serde(default)]
        credential: Option<OAuthCredential>,
    },
    Models {
        request_id: String,
        models: Vec<NormalizedModel>,
        #[serde(default)]
        credential: Option<OAuthCredential>,
    },
    Failed {
        request_id: String,
        code: String,
        message: String,
        #[serde(default)]
        credential: Option<OAuthCredential>,
    },
    OauthEvent {
        request_id: String,
        event: WorkerOAuthEvent,
    },
    OauthPrompt {
        request_id: String,
        prompt_id: String,
        prompt: OAuthPrompt,
    },
    OauthCompleted {
        request_id: String,
        credential: OAuthCredential,
    },
    Acknowledged {
        request_id: String,
        command: AcknowledgedCommand,
        #[serde(default)]
        target_request_id: Option<String>,
        #[serde(default)]
        accepted: Option<bool>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WorkerOAuthEvent {
    AuthUrl {
        url: String,
        instructions: Option<String>,
    },
    DeviceCode {
        user_code: String,
        verification_uri: String,
        interval_seconds: Option<u64>,
        expires_in_seconds: Option<u64>,
    },
    Progress {
        message: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerResponse {
    message: ChatMessage,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    usage: Option<WorkerUsage>,
    #[serde(default)]
    provider_metrics: Option<ProviderMetrics>,
    finish_reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    #[serde(default)]
    cost: Option<f64>,
}

impl From<WorkerResponse> for LlmResponse {
    fn from(response: WorkerResponse) -> Self {
        Self {
            message: response.message,
            reasoning: response.reasoning,
            usage: response.usage.map(|usage| Usage {
                prompt_tokens: Some(usage.prompt_tokens),
                completion_tokens: Some(usage.completion_tokens),
                total_tokens: Some(usage.total_tokens),
                cache_read_tokens: Some(usage.cache_read_tokens),
                cache_write_tokens: Some(usage.cache_write_tokens),
                cost: usage.cost,
            }),
            provider_metrics: response.provider_metrics,
            finish_reason: response.finish_reason,
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum AcknowledgedCommand {
    Cancel,
    Shutdown,
}

pub struct PiAiWorkerBackend {
    paths: WorkerPaths,
    provider: ProviderConfig,
    credential_update: Mutex<Option<OAuthCredential>>,
}

impl PiAiWorkerBackend {
    fn new(
        kind: ConnectorKind,
        base_url: String,
        api_key: Option<String>,
        oauth_credential: Option<OAuthCredential>,
    ) -> Result<Self, String> {
        let provider_kind = match kind {
            ConnectorKind::Ollama => ProviderKind::Ollama,
            ConnectorKind::OpenAiCompatible => ProviderKind::OpenAiCompatible,
            ConnectorKind::Openai => ProviderKind::Openai,
            ConnectorKind::Anthropic => ProviderKind::Anthropic,
            ConnectorKind::AnthropicOauth => ProviderKind::Anthropic,
            ConnectorKind::OpenaiCodex => ProviderKind::OpenaiCodex,
            ConnectorKind::GithubCopilot => ProviderKind::GithubCopilot,
            ConnectorKind::Openrouter => ProviderKind::Openrouter,
            ConnectorKind::Google => ProviderKind::Google,
            ConnectorKind::Mistral => ProviderKind::Mistral,
            ConnectorKind::AmazonBedrock => ProviderKind::AmazonBedrock,
        };
        Ok(Self {
            paths: resolve_worker_paths()?,
            provider: ProviderConfig {
                kind: provider_kind,
                base_url,
                api_key,
                oauth_credential,
            },
            credential_update: Mutex::new(None),
        })
    }

    fn run(
        &self,
        request: &CompletionRequest,
        on_delta: &dyn Fn(&str, &str),
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<LlmResponse, LlmProviderError> {
        self.with_worker(|worker| {
            let request_id = Uuid::new_v4().to_string();
            worker.send(&WorkerCommand::Generate {
                request_id: request_id.clone(),
                provider: self.provider.clone(),
                model: request.model.clone(),
                messages: request.messages.clone(),
                tools: request.tools.clone(),
                response_format: request.response_format.clone(),
                reasoning: request.reasoning.clone(),
                text_verbosity: request.text_verbosity,
                temperature: request.temperature,
                max_output_tokens: request.max_tokens,
                timeout_ms: RESPONSE_TIMEOUT.as_millis() as u64,
            })?;

            let deadline = Instant::now() + RESPONSE_TIMEOUT;
            let mut cancellation_sent = false;
            loop {
                if is_cancelled() && !cancellation_sent {
                    worker.send(&WorkerCommand::Cancel {
                        request_id: Uuid::new_v4().to_string(),
                        target_request_id: request_id.clone(),
                    })?;
                    cancellation_sent = true;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(LlmProviderError("pi-ai worker response timed out".into()));
                }
                let wait = remaining.min(POLL_INTERVAL);
                let Some(event) = worker.try_recv(wait)? else {
                    continue;
                };
                match event {
                    WorkerEvent::StreamDelta {
                        request_id: event_request_id,
                        content,
                        reasoning,
                    } if event_request_id == request_id => on_delta(&content, &reasoning),
                    WorkerEvent::Completed {
                        request_id: event_request_id,
                        response,
                        credential,
                    } if event_request_id == request_id => {
                        self.store_credential_update(credential)?;
                        return Ok(response.into());
                    }
                    WorkerEvent::Failed {
                        request_id: event_request_id,
                        code,
                        message,
                        credential,
                    } if event_request_id == request_id => {
                        self.store_credential_update(credential)?;
                        return Err(LlmProviderError(format!("{code}: {message}")));
                    }
                    WorkerEvent::Acknowledged {
                        command: AcknowledgedCommand::Cancel,
                        target_request_id: Some(target),
                        accepted: Some(true),
                        ..
                    } if cancellation_sent && target == request_id => {
                        return Err(LlmProviderError("LLM request cancelled".into()));
                    }
                    _ => {
                        return Err(LlmProviderError(
                            "pi-ai worker emitted an unexpected event".into(),
                        ));
                    }
                }
            }
        })
    }

    fn models(&self, refresh: bool) -> Result<Vec<NormalizedModel>, LlmProviderError> {
        self.with_worker(|worker| {
            let request_id = Uuid::new_v4().to_string();
            let command = if refresh {
                WorkerCommand::ModelsRefresh {
                    request_id: request_id.clone(),
                    provider: self.provider.clone(),
                }
            } else {
                WorkerCommand::ModelsList {
                    request_id: request_id.clone(),
                    provider: self.provider.clone(),
                }
            };
            worker.send(&command)?;

            match worker.recv(RESPONSE_TIMEOUT)? {
                WorkerEvent::Models {
                    request_id: event_request_id,
                    models,
                    credential,
                } if event_request_id == request_id => {
                    self.store_credential_update(credential)?;
                    Ok(models)
                }
                WorkerEvent::Failed {
                    request_id: event_request_id,
                    code,
                    message,
                    credential,
                } if event_request_id == request_id => {
                    self.store_credential_update(credential)?;
                    Err(LlmProviderError(format!("{code}: {message}")))
                }
                _ => Err(LlmProviderError(
                    "pi-ai worker emitted an unexpected models event".into(),
                )),
            }
        })
    }

    fn with_worker<T>(
        &self,
        operation: impl FnOnce(&mut WorkerProcess) -> Result<T, LlmProviderError>,
    ) -> Result<T, LlmProviderError> {
        let mut worker = WorkerProcess::spawn_ready(&self.paths)?;
        let result = operation(&mut worker);
        // Best-effort graceful shutdown. The worker is force-killed on drop, so
        // a failed shutdown handshake must not discard a successful result.
        let _ = worker.shutdown();
        result
    }

    fn store_credential_update(
        &self,
        credential: Option<OAuthCredential>,
    ) -> Result<(), LlmProviderError> {
        if let Some(credential) = credential {
            *self
                .credential_update
                .lock()
                .map_err(|_| LlmProviderError("credential update lock poisoned".into()))? =
                Some(credential);
        }
        Ok(())
    }
}

pub fn run_oauth_login(
    session_id: &str,
    kind: ConnectorKind,
    base_url: String,
    sessions: &PendingOAuthSessions,
    controls: Receiver<OAuthControl>,
) -> Result<OAuthCredential, LlmProviderError> {
    run_oauth_login_with_paths(
        session_id,
        kind,
        base_url,
        sessions,
        controls,
        resolve_worker_paths().map_err(LlmProviderError)?,
    )
}

fn run_oauth_login_with_paths(
    session_id: &str,
    kind: ConnectorKind,
    base_url: String,
    sessions: &PendingOAuthSessions,
    controls: Receiver<OAuthControl>,
    paths: WorkerPaths,
) -> Result<OAuthCredential, LlmProviderError> {
    if !kind.defaults().oauth_credential_required {
        return Err(LlmProviderError(
            "selected connector does not use OAuth".into(),
        ));
    }
    let provider_kind = match kind {
        ConnectorKind::AnthropicOauth => ProviderKind::Anthropic,
        ConnectorKind::OpenaiCodex => ProviderKind::OpenaiCodex,
        ConnectorKind::GithubCopilot => ProviderKind::GithubCopilot,
        _ => unreachable!("OAuth connector kinds checked above"),
    };
    let mut worker = WorkerProcess::spawn_ready(&paths)?;
    let request_id = Uuid::new_v4().to_string();
    worker.send(&WorkerCommand::OauthLogin {
        request_id: request_id.clone(),
        provider: OAuthProviderConfig {
            kind: provider_kind,
            base_url: Some(base_url),
        },
    })?;

    loop {
        match controls.try_recv() {
            Ok(OAuthControl::PromptResponse {
                prompt_id,
                value,
                cancelled,
            }) => worker.send(&WorkerCommand::OauthPromptResponse {
                request_id: Uuid::new_v4().to_string(),
                target_request_id: request_id.clone(),
                prompt_id,
                value,
                cancelled,
            })?,
            Ok(OAuthControl::Cancel) => worker.send(&WorkerCommand::Cancel {
                request_id: Uuid::new_v4().to_string(),
                target_request_id: request_id.clone(),
            })?,
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                return worker.cancel(&request_id);
            }
        }

        let Some(event) = worker.try_recv(POLL_INTERVAL)? else {
            continue;
        };
        match event {
            WorkerEvent::OauthEvent {
                request_id: event_request_id,
                event,
            } if event_request_id == request_id => {
                let public = public_oauth_event(session_id, event)?;
                sessions.publish(public).map_err(LlmProviderError)?;
            }
            WorkerEvent::OauthPrompt {
                request_id: event_request_id,
                prompt_id,
                prompt,
            } if event_request_id == request_id => {
                bounded(&prompt_id, 128, "OAuth prompt id")?;
                validate_oauth_prompt(&prompt)?;
                sessions
                    .set_prompt(session_id, prompt_id.clone())
                    .map_err(LlmProviderError)?;
                sessions
                    .publish(OAuthPublicEvent::Prompt {
                        session_id: session_id.to_string(),
                        prompt_id,
                        prompt,
                    })
                    .map_err(LlmProviderError)?;
            }
            WorkerEvent::OauthCompleted {
                request_id: event_request_id,
                credential,
            } if event_request_id == request_id => {
                worker.shutdown()?;
                return Ok(credential);
            }
            WorkerEvent::Failed {
                request_id: event_request_id,
                code,
                message,
                ..
            } if event_request_id == request_id => {
                bounded(&code, 128, "OAuth failure code")?;
                bounded(&message, MAX_PUBLIC_STRING_BYTES, "OAuth failure message")?;
                return Err(LlmProviderError(format!("{code}: {message}")));
            }
            WorkerEvent::Acknowledged { .. } => {}
            _ => {
                return Err(LlmProviderError(
                    "pi-ai worker emitted an unexpected OAuth event".into(),
                ));
            }
        }
    }
}

fn bounded(value: &str, maximum: usize, label: &str) -> Result<(), LlmProviderError> {
    if value.is_empty() || value.len() > maximum {
        return Err(LlmProviderError(format!("{label} has invalid length")));
    }
    Ok(())
}

fn optional_bounded(
    value: &Option<String>,
    maximum: usize,
    label: &str,
) -> Result<(), LlmProviderError> {
    if let Some(value) = value {
        bounded(value, maximum, label)?;
    }
    Ok(())
}

fn public_oauth_event(
    session_id: &str,
    event: WorkerOAuthEvent,
) -> Result<OAuthPublicEvent, LlmProviderError> {
    Ok(match event {
        WorkerOAuthEvent::AuthUrl { url, instructions } => {
            bounded(&url, MAX_URL_BYTES, "OAuth authorization URL")?;
            optional_bounded(&instructions, MAX_PUBLIC_STRING_BYTES, "OAuth instructions")?;
            OAuthPublicEvent::AuthUrl {
                session_id: session_id.to_string(),
                url,
                instructions,
            }
        }
        WorkerOAuthEvent::DeviceCode {
            user_code,
            verification_uri,
            interval_seconds,
            expires_in_seconds,
        } => {
            bounded(&user_code, 1024, "OAuth device code")?;
            bounded(&verification_uri, MAX_URL_BYTES, "OAuth verification URL")?;
            OAuthPublicEvent::DeviceCode {
                session_id: session_id.to_string(),
                user_code,
                verification_uri,
                interval_seconds,
                expires_in_seconds,
            }
        }
        WorkerOAuthEvent::Progress { message } => {
            bounded(&message, MAX_PUBLIC_STRING_BYTES, "OAuth progress message")?;
            OAuthPublicEvent::Progress {
                session_id: session_id.to_string(),
                message,
            }
        }
    })
}

fn validate_oauth_prompt(prompt: &OAuthPrompt) -> Result<(), LlmProviderError> {
    match prompt {
        OAuthPrompt::Text {
            message,
            placeholder,
        }
        | OAuthPrompt::Secret {
            message,
            placeholder,
        }
        | OAuthPrompt::ManualCode {
            message,
            placeholder,
        } => {
            bounded(message, MAX_PUBLIC_STRING_BYTES, "OAuth prompt message")?;
            optional_bounded(
                placeholder,
                MAX_PUBLIC_STRING_BYTES,
                "OAuth prompt placeholder",
            )?;
        }
        OAuthPrompt::Select { message, options } => {
            bounded(message, MAX_PUBLIC_STRING_BYTES, "OAuth prompt message")?;
            if options.is_empty() || options.len() > 128 {
                return Err(LlmProviderError(
                    "OAuth prompt options have invalid length".into(),
                ));
            }
            for option in options {
                bounded(&option.id, 1024, "OAuth option id")?;
                bounded(&option.label, MAX_PUBLIC_STRING_BYTES, "OAuth option label")?;
                optional_bounded(
                    &option.description,
                    MAX_PUBLIC_STRING_BYTES,
                    "OAuth option description",
                )?;
            }
        }
    }
    Ok(())
}

impl LlmBackend for PiAiWorkerBackend {
    fn complete(&self, request: &CompletionRequest) -> Result<LlmResponse, LlmProviderError> {
        self.run(request, &|_, _| {}, &|| false)
    }

    fn complete_stream_interruptible(
        &self,
        request: &CompletionRequest,
        on_delta: &dyn Fn(&str, &str),
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<LlmResponse, LlmProviderError> {
        self.run(request, on_delta, is_cancelled)
    }

    fn take_credential_update(&self) -> Result<Option<OAuthCredential>, LlmProviderError> {
        Ok(self
            .credential_update
            .lock()
            .map_err(|_| LlmProviderError("credential update lock poisoned".into()))?
            .take())
    }
}

struct WorkerProcess(NodeWorkerProcess<WorkerEvent>);

impl WorkerProcess {
    fn spawn(paths: &WorkerPaths) -> Result<Self, LlmProviderError> {
        NodeWorkerProcess::spawn(paths, "pi-ai worker")
            .map(Self)
            .map_err(LlmProviderError)
    }

    fn spawn_ready(paths: &WorkerPaths) -> Result<Self, LlmProviderError> {
        let worker = Self::spawn(paths)?;
        match worker.recv(READY_TIMEOUT)? {
            WorkerEvent::Ready { protocol_version } if protocol_version == PROTOCOL_VERSION => {
                Ok(worker)
            }
            WorkerEvent::Ready { protocol_version } => Err(LlmProviderError(format!(
                "unsupported pi-ai worker protocol version {protocol_version}"
            ))),
            _ => Err(LlmProviderError(
                "pi-ai worker did not emit ready first".into(),
            )),
        }
    }

    fn send(&mut self, command: &WorkerCommand) -> Result<(), LlmProviderError> {
        command.validate().map_err(LlmProviderError)?;
        let encoded = serde_json::to_vec(command).map_err(|error| {
            LlmProviderError(format!("worker request serialization failed: {error}"))
        })?;
        serde_json::from_slice::<WorkerCommand>(&encoded).map_err(|error| {
            LlmProviderError(format!("worker request validation failed: {error}"))
        })?;
        self.0.send(command).map_err(LlmProviderError)
    }

    fn recv(&self, timeout: Duration) -> Result<WorkerEvent, LlmProviderError> {
        self.0.recv(timeout).map_err(LlmProviderError)
    }

    fn try_recv(&self, timeout: Duration) -> Result<Option<WorkerEvent>, LlmProviderError> {
        self.0.try_recv(timeout).map_err(LlmProviderError)
    }

    fn cancel<T>(&mut self, target_request_id: &str) -> Result<T, LlmProviderError> {
        let cancel_request_id = Uuid::new_v4().to_string();
        self.send(&WorkerCommand::Cancel {
            request_id: cancel_request_id,
            target_request_id: target_request_id.to_string(),
        })?;
        let deadline = Instant::now() + CANCEL_TIMEOUT;
        while Instant::now() < deadline {
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .min(POLL_INTERVAL);
            let Some(event) = self.try_recv(wait)? else {
                continue;
            };
            if let WorkerEvent::Failed {
                request_id,
                code,
                message,
                ..
            } = event
            {
                if request_id == target_request_id && code == "cancelled" {
                    return Err(LlmProviderError(format!("{code}: {message}")));
                }
            }
        }
        Err(LlmProviderError("request cancelled".into()))
    }

    fn shutdown(&mut self) -> Result<(), LlmProviderError> {
        let request_id = Uuid::new_v4().to_string();
        self.send(&WorkerCommand::Shutdown {
            request_id: request_id.clone(),
        })?;
        match self.recv(SHUTDOWN_TIMEOUT)? {
            WorkerEvent::Acknowledged {
                request_id: event_request_id,
                command: AcknowledgedCommand::Shutdown,
                target_request_id: None,
                accepted: None,
            } if event_request_id == request_id => {}
            _ => {
                return Err(LlmProviderError(
                    "pi-ai worker emitted an unexpected shutdown event".into(),
                ));
            }
        }
        // Give the exit wait its own full budget: a slow shutdown ack must not
        // starve it into a spurious timeout (mirrors agent_worker_protocol::shutdown).
        self.0
            .wait_for_exit(SHUTDOWN_TIMEOUT)
            .map_err(LlmProviderError)
    }
}

#[cfg(test)]
fn read_worker_events(stdout: impl Read, sender: mpsc::Sender<Result<WorkerEvent, String>>) {
    crate::node_worker::read_worker_events(stdout, sender, "pi-ai worker")
}

fn resolve_worker_paths() -> Result<WorkerPaths, String> {
    crate::node_worker::resolve_worker_paths(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        std::env::current_exe()
            .map_err(|error| format!("failed to resolve current executable: {error}"))?,
        &provider_worker_path_config(),
    )
}

#[cfg(test)]
fn resolve_worker_paths_from(
    explicit_node: Option<PathBuf>,
    explicit_worker: Option<PathBuf>,
    manifest_dir: &Path,
    current_exe: PathBuf,
) -> Result<WorkerPaths, String> {
    crate::node_worker::resolve_worker_paths_from(
        explicit_node,
        explicit_worker,
        manifest_dir,
        current_exe,
        &provider_worker_path_config(),
    )
}

#[cfg(test)]
fn validate_worker_paths(node: PathBuf, worker: PathBuf) -> Result<WorkerPaths, String> {
    crate::node_worker::validate_worker_paths(node, worker, "LLM provider worker")
}

fn provider_worker_path_config() -> WorkerPathConfig<'static> {
    WorkerPathConfig {
        node_env: "KESTRAL_PROVIDER_NODE",
        worker_env: "KESTRAL_PROVIDER_WORKER",
        resource_env: "KESTRAL_WORKER_RESOURCE_DIR",
        source_directory: "provider-worker",
        packaged_directory: "provider-worker",
        packaged_worker: "worker.mjs",
        display_name: "LLM provider worker",
    }
}

pub fn backend_from_profile_secrets(
    resolver: &app_host_kernel::services::broker::SecretResolver,
    kind: ConnectorKind,
    base_url: String,
    needs_api_key: bool,
    oauth_credential: Option<String>,
) -> Result<Box<dyn LlmBackend>, String> {
    Ok(Box::new(backend_from_resolver(
        resolver,
        kind,
        base_url,
        needs_api_key,
        oauth_credential,
    )?))
}

/// Outcome of a broker-authorized model listing: the listing itself can fail
/// independently of an OAuth credential refresh that must still be persisted.
pub type ModelListingOutcome = (
    Result<Vec<NormalizedModel>, LlmProviderError>,
    Option<OAuthCredential>,
);

pub fn models_from_profile_secrets(
    resolver: &app_host_kernel::services::broker::SecretResolver,
    kind: ConnectorKind,
    base_url: String,
    _default_model: String,
    needs_api_key: bool,
    oauth_credential: Option<String>,
    refresh: bool,
) -> Result<ModelListingOutcome, LlmProviderError> {
    let backend = backend_from_resolver(resolver, kind, base_url, needs_api_key, oauth_credential)
        .map_err(LlmProviderError)?;
    let result = backend.models(refresh);
    let update = backend.take_credential_update()?;
    Ok((result, update))
}

pub fn models_from_config(
    kind: ConnectorKind,
    base_url: String,
    _configured_model: Option<String>,
    api_key: Option<String>,
    refresh: bool,
) -> Result<Vec<NormalizedModel>, String> {
    // OAuth providers ship a pi-ai model catalog that can be listed before
    // login. Never refresh that draft catalog: refresh would require a stored,
    // broker-authorized credential and belongs to llm.models.refresh.
    let refresh = refresh && !kind.defaults().oauth_credential_required;
    PiAiWorkerBackend::new(kind, base_url, api_key, None)?
        .models(refresh)
        .map_err(|error| error.to_string())
}

fn backend_from_resolver(
    resolver: &app_host_kernel::services::broker::SecretResolver,
    kind: ConnectorKind,
    base_url: String,
    needs_api_key: bool,
    oauth_credential: Option<String>,
) -> Result<PiAiWorkerBackend, String> {
    let api_key = if needs_api_key {
        Some(
            resolver
                .resolve(&active_llm_api_key_secret())
                .map_err(|error| format!("missing configured API key: {error}"))?
                .to_string(),
        )
    } else {
        None
    };
    let oauth_credential = if kind.defaults().oauth_credential_required {
        // Resolve the broker alias first. The latest persisted profile value
        // is then used so a worker rotation takes effect on the next call
        // without bypassing broker authorization.
        resolver
            .resolve(&active_llm_api_key_secret())
            .map_err(|error| format!("missing configured OAuth credential: {error}"))?;
        Some(OAuthCredential::parse_serialized(
            oauth_credential
                .as_deref()
                .ok_or_else(|| "missing persisted OAuth credential".to_string())?,
        )?)
    } else {
        None
    };
    PiAiWorkerBackend::new(kind, base_url, api_key, oauth_credential)
}

#[cfg(test)]
mod tests;
