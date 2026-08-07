//! Credential-free bridge to an invocation-scoped agent worker.

use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use app_host_kernel::invocation::ProgressReporter;
use app_host_kernel::JsonObject;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::llm_client::{ChatMessage, LlmResponse, ToolDefinition};
use crate::node_worker::{NodeWorkerProcess, WorkerPaths};

const PROTOCOL_VERSION: u32 = 1;
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const CANCEL_GRACE: Duration = Duration::from_secs(1);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub(crate) struct AgentJob {
    pub(crate) system_prompt: Option<String>,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) tools: Vec<ToolDefinition>,
    pub(crate) model: Option<String>,
    pub(crate) reasoning: Option<String>,
    pub(crate) max_turns: u8,
    pub(crate) max_duration: Duration,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AgentResult {
    pub(crate) text: String,
    pub(crate) reasoning: Option<String>,
    pub(crate) finish_reason: AgentFinishReason,
    pub(crate) turns: u32,
    #[serde(default)]
    pub(crate) transcript: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AgentFinishReason {
    Stop,
    MaxTurns,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentLlmRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) tools: Vec<ToolDefinition>,
    pub(crate) reasoning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolInvocationOutcome {
    Completed(String),
    Refused(String),
    Failed(String),
}

pub(crate) trait AgentHostBridge: Send + Sync {
    fn generate(
        &self,
        request: AgentLlmRequest,
        timeout: Duration,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<LlmResponse, String>;

    fn invoke_tool(
        &self,
        tool_name: &str,
        arguments: JsonObject,
        timeout: Duration,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<ToolInvocationOutcome, String>;
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case", deny_unknown_fields)]
enum AgentWorkerCommand {
    AgentRun {
        request_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        system_prompt: Option<String>,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
        max_turns: u8,
    },
    ToolResult {
        request_id: String,
        target_request_id: String,
        tool_call_id: String,
        outcome: ToolOutcomeKind,
        content: String,
    },
    LlmCompleted {
        request_id: String,
        call_id: String,
        response: AgentWorkerLlmResponse,
    },
    LlmFailed {
        request_id: String,
        call_id: String,
        message: String,
    },
    Cancel {
        request_id: String,
        target_request_id: String,
    },
    Shutdown {
        request_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ToolOutcomeKind {
    Completed,
    Refused,
    Failed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentWorkerLlmResponse {
    message: ChatMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning: Option<String>,
    finish_reason: String,
}

impl From<LlmResponse> for AgentWorkerLlmResponse {
    fn from(response: LlmResponse) -> Self {
        Self {
            message: response.message,
            reasoning: response.reasoning,
            finish_reason: response.finish_reason,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum AgentWorkerEvent {
    Ready {
        protocol_version: u32,
    },
    LlmRequest {
        request_id: String,
        call_id: String,
        model: String,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        #[serde(default)]
        reasoning: Option<String>,
    },
    ToolRequest {
        request_id: String,
        tool_call_id: String,
        tool_name: String,
        arguments: JsonObject,
    },
    AgentEvent {
        request_id: String,
        event: String,
        #[serde(default)]
        tool_call_id: Option<String>,
        #[serde(default)]
        tool_name: Option<String>,
    },
    Completed {
        request_id: String,
        text: String,
        #[serde(default)]
        reasoning: Option<String>,
        finish_reason: AgentFinishReason,
        turns: u32,
        transcript: Vec<ChatMessage>,
    },
    Failed {
        request_id: String,
        code: String,
        message: String,
    },
    Acknowledged {
        request_id: String,
        command: AcknowledgedCommand,
        #[serde(default)]
        target_request_id: Option<String>,
    },
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum AcknowledgedCommand {
    Cancel,
    Shutdown,
}

pub(crate) fn run_agent_job_with_paths(
    paths: WorkerPaths,
    job: AgentJob,
    bridge: &dyn AgentHostBridge,
    progress: &ProgressReporter,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<AgentResult, String> {
    let total_deadline = Instant::now() + job.max_duration;
    let mut worker = NodeWorkerProcess::<AgentWorkerEvent>::spawn(&paths, "agent worker")?;
    match worker.recv(READY_TIMEOUT)? {
        AgentWorkerEvent::Ready { protocol_version } if protocol_version == PROTOCOL_VERSION => {}
        AgentWorkerEvent::Ready { protocol_version } => {
            return Err(format!(
                "unsupported agent worker protocol version {protocol_version}"
            ));
        }
        _ => return Err("agent worker did not emit ready first".into()),
    }

    let request_id = Uuid::new_v4().to_string();
    send(
        &mut worker,
        &AgentWorkerCommand::AgentRun {
            request_id: request_id.clone(),
            system_prompt: job.system_prompt,
            messages: job.messages,
            tools: job.tools,
            model: job.model,
            reasoning: job.reasoning,
            max_turns: job.max_turns,
        },
    )?;
    let mut idle_deadline = Instant::now() + IDLE_TIMEOUT;
    let mut cancellation_sent = false;
    loop {
        if is_cancelled() && !cancellation_sent {
            send(
                &mut worker,
                &AgentWorkerCommand::Cancel {
                    request_id: Uuid::new_v4().to_string(),
                    target_request_id: request_id.clone(),
                },
            )?;
            cancellation_sent = true;
            idle_deadline = Instant::now() + CANCEL_GRACE;
        }
        let now = Instant::now();
        let total_remaining = total_deadline.saturating_duration_since(now);
        if total_remaining.is_zero() {
            return Err("agent request duration limit exceeded".into());
        }
        let idle_remaining = idle_deadline.saturating_duration_since(now);
        if idle_remaining.is_zero() {
            return Err(if cancellation_sent {
                "agent request cancelled".into()
            } else {
                "agent worker idle timeout".into()
            });
        }
        let Some(event) =
            worker.try_recv(idle_remaining.min(total_remaining).min(POLL_INTERVAL))?
        else {
            continue;
        };
        idle_deadline = Instant::now() + IDLE_TIMEOUT;
        match event {
            AgentWorkerEvent::LlmRequest {
                request_id: event_request_id,
                call_id,
                model,
                messages,
                tools,
                reasoning,
            } if event_request_id == request_id => {
                let response = bridge.generate(
                    AgentLlmRequest {
                        model,
                        messages,
                        tools,
                        reasoning,
                    },
                    crate::llm_client::INVOCATION_TIMEOUT
                        .min(total_deadline.saturating_duration_since(Instant::now())),
                    is_cancelled,
                );
                let command = match response {
                    Ok(response) => AgentWorkerCommand::LlmCompleted {
                        request_id: request_id.clone(),
                        call_id,
                        response: response.into(),
                    },
                    Err(message) => AgentWorkerCommand::LlmFailed {
                        request_id: request_id.clone(),
                        call_id,
                        message,
                    },
                };
                send(&mut worker, &command)?;
            }
            AgentWorkerEvent::ToolRequest {
                request_id: event_request_id,
                tool_call_id,
                tool_name,
                arguments,
            } if event_request_id == request_id => {
                let outcome = bridge.invoke_tool(
                    &tool_name,
                    arguments,
                    IDLE_TIMEOUT.min(total_deadline.saturating_duration_since(Instant::now())),
                    is_cancelled,
                )?;
                let (outcome, content) = match outcome {
                    ToolInvocationOutcome::Completed(content) => {
                        (ToolOutcomeKind::Completed, content)
                    }
                    ToolInvocationOutcome::Refused(content) => (ToolOutcomeKind::Refused, content),
                    ToolInvocationOutcome::Failed(content) => (ToolOutcomeKind::Failed, content),
                };
                send(
                    &mut worker,
                    &AgentWorkerCommand::ToolResult {
                        request_id: Uuid::new_v4().to_string(),
                        target_request_id: request_id.clone(),
                        tool_call_id,
                        outcome,
                        content,
                    },
                )?;
            }
            AgentWorkerEvent::AgentEvent {
                request_id: event_request_id,
                event,
                tool_call_id,
                tool_name,
            } if event_request_id == request_id => {
                progress.report(json!({
                    "kind": "agent-event",
                    "event": event,
                    "tool_call_id": tool_call_id,
                    "tool_name": tool_name,
                }));
            }
            AgentWorkerEvent::Completed {
                request_id: event_request_id,
                text,
                reasoning,
                finish_reason,
                turns,
                transcript,
            } if event_request_id == request_id => {
                let result = AgentResult {
                    text,
                    reasoning,
                    finish_reason,
                    turns,
                    transcript,
                };
                // The reply is already fully received. A slow or failed
                // graceful shutdown must not discard it — `NodeWorkerProcess`
                // force-kills any lingering child on drop.
                let _ = shutdown(&mut worker);
                return Ok(result);
            }
            AgentWorkerEvent::Failed {
                request_id: event_request_id,
                code,
                message,
            } if event_request_id == request_id => return Err(format!("{code}: {message}")),
            AgentWorkerEvent::Acknowledged {
                command: AcknowledgedCommand::Cancel,
                target_request_id: Some(target),
                ..
            } if target == request_id => {}
            _ => return Err("agent worker emitted an unexpected event".into()),
        }
    }
}

fn send(
    worker: &mut NodeWorkerProcess<AgentWorkerEvent>,
    command: &AgentWorkerCommand,
) -> Result<(), String> {
    let encoded = serde_json::to_vec(command)
        .map_err(|error| format!("agent worker request serialization failed: {error}"))?;
    serde_json::from_slice::<AgentWorkerCommand>(&encoded)
        .map_err(|error| format!("agent worker request validation failed: {error}"))?;
    worker.send(command)
}

fn shutdown(worker: &mut NodeWorkerProcess<AgentWorkerEvent>) -> Result<(), String> {
    let request_id = Uuid::new_v4().to_string();
    send(
        worker,
        &AgentWorkerCommand::Shutdown {
            request_id: request_id.clone(),
        },
    )?;
    match worker.recv(SHUTDOWN_TIMEOUT)? {
        AgentWorkerEvent::Acknowledged {
            request_id: event_request_id,
            command: AcknowledgedCommand::Shutdown,
            target_request_id: None,
        } if event_request_id == request_id => worker.wait_for_exit(SHUTDOWN_TIMEOUT),
        _ => Err("agent worker emitted an unexpected shutdown event".into()),
    }
}

pub(crate) fn resolve_package_agent_worker_paths(worker: PathBuf) -> Result<WorkerPaths, String> {
    let node = match env::var_os("KESTRAL_AGENT_NODE") {
        Some(node) => PathBuf::from(node),
        None => resolve_agent_node(
            Path::new(env!("CARGO_MANIFEST_DIR")),
            env::current_exe()
                .map_err(|error| format!("failed to resolve current executable: {error}"))?,
            env::var_os("KESTRAL_WORKER_RESOURCE_DIR").map(PathBuf::from),
        )?,
    };
    crate::node_worker::validate_worker_paths(node, worker, "agent app worker")
}

fn resolve_agent_node(
    manifest_dir: &Path,
    current_exe: PathBuf,
    resource_dir: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let node_name = if cfg!(windows) { "node.exe" } else { "node" };
    let repository_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "failed to resolve repository root".to_string())?;
    let source = repository_root
        .join("host/provider-worker/runtime")
        .join(node_name);
    if cfg!(debug_assertions) && source.is_file() {
        return Ok(source);
    }
    Ok(resource_dir
        .unwrap_or_else(|| {
            current_exe
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default()
        })
        .join("provider-worker/runtime")
        .join(node_name))
}

#[cfg(test)]
mod tests;
