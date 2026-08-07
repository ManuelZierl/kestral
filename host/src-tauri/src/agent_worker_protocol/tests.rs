use super::*;

struct FakeBridge;

impl AgentHostBridge for FakeBridge {
    fn generate(
        &self,
        _request: AgentLlmRequest,
        _timeout: Duration,
        _is_cancelled: &dyn Fn() -> bool,
    ) -> Result<LlmResponse, String> {
        Ok(LlmResponse {
            message: ChatMessage {
                role: "assistant".into(),
                content: "hello from the real worker".into(),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            reasoning: None,
            usage: None,
            provider_metrics: None,
            finish_reason: "stop".into(),
        })
    }

    fn invoke_tool(
        &self,
        _tool_name: &str,
        _arguments: JsonObject,
        _timeout: Duration,
        _is_cancelled: &dyn Fn() -> bool,
    ) -> Result<ToolInvocationOutcome, String> {
        Err("no tool expected".into())
    }
}

struct UsageBridge;

impl AgentHostBridge for UsageBridge {
    fn generate(
        &self,
        _request: AgentLlmRequest,
        _timeout: Duration,
        _is_cancelled: &dyn Fn() -> bool,
    ) -> Result<LlmResponse, String> {
        Ok(LlmResponse {
            message: ChatMessage {
                role: "assistant".into(),
                content: "usage stays host-side".into(),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            reasoning: None,
            usage: Some(crate::llm_client::Usage {
                prompt_tokens: Some(1),
                completion_tokens: Some(2),
                total_tokens: Some(3),
                cache_read_tokens: None,
                cache_write_tokens: None,
                cost: None,
            }),
            provider_metrics: None,
            finish_reason: "stop".into(),
        })
    }

    fn invoke_tool(
        &self,
        _tool_name: &str,
        _arguments: JsonObject,
        _timeout: Duration,
        _is_cancelled: &dyn Fn() -> bool,
    ) -> Result<ToolInvocationOutcome, String> {
        Err("no tool expected".into())
    }
}

#[test]
fn worker_events_reject_unknown_fields() {
    let error = serde_json::from_str::<AgentWorkerEvent>(
        r#"{"type":"ready","protocol_version":1,"credential":"no"}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("unknown field `credential`"));
}

#[test]
fn agent_apps_reuse_provider_runtime() {
    let root = env::temp_dir().join(format!("agent-worker-client-{}", Uuid::new_v4()));
    let manifest = root.join("host/src-tauri");
    let node = root
        .join("host/provider-worker/runtime")
        .join(if cfg!(windows) { "node.exe" } else { "node" });
    std::fs::create_dir_all(node.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&manifest).unwrap();
    std::fs::write(&node, b"node").unwrap();

    let resolved = resolve_agent_node(&manifest, root.join("target/host"), None).unwrap();

    assert_eq!(resolved, node);
    std::fs::remove_dir_all(root).unwrap();
}

struct WorkerFixture {
    paths: WorkerPaths,
    root: PathBuf,
}

impl WorkerFixture {
    fn new() -> Self {
        let root = env::temp_dir().join(format!("agent-worker-fixture-{}", Uuid::new_v4()));
        let worker = root.join("worker.mjs");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &worker,
            r#"import { createInterface } from "node:readline";

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

send({ type: "ready", protocol_version: 1 });
const lines = createInterface({ input: process.stdin });
lines.on("line", (line) => {
  const command = JSON.parse(line);
  if (command.command === "agent-run") {
    send({
      type: "llm-request",
      request_id: command.request_id,
      call_id: "fixture-call",
      model: command.model ?? "fixture-model",
      messages: command.messages,
      tools: command.tools,
      reasoning: command.reasoning,
    });
  } else if (command.command === "llm-completed") {
    if (Object.hasOwn(command.response, "usage")) {
      send({ type: "failed", request_id: command.request_id, code: "usage-leak", message: "host-only usage crossed the worker boundary" });
      return;
    }
    send({
      type: "completed",
      request_id: command.request_id,
      text: command.response.message.content,
      reasoning: command.response.reasoning,
      finish_reason: command.response.finish_reason,
      turns: 1,
      transcript: [command.response.message],
    });
  } else if (command.command === "shutdown") {
    send({ type: "acknowledged", request_id: command.request_id, command: "shutdown" });
    process.exit(0);
  }
});
"#,
        )
        .unwrap();
        let paths = resolve_package_agent_worker_paths(worker).unwrap();
        Self { paths, root }
    }
}

impl Drop for WorkerFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn agent_worker_protocol_round_trips_model_calls_without_credentials() {
    let fixture = WorkerFixture::new();
    let result = run_agent_job_with_paths(
        fixture.paths.clone(),
        AgentJob {
            system_prompt: Some("Reply briefly".into()),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hello".into(),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            tools: vec![],
            model: None,
            reasoning: None,
            max_turns: 2,
            max_duration: Duration::from_secs(60),
        },
        &FakeBridge,
        &ProgressReporter::default(),
        &|| false,
    )
    .unwrap();

    assert_eq!(result.text, "hello from the real worker");
    assert_eq!(result.turns, 1);
}

#[test]
fn agent_worker_protocol_does_not_receive_host_only_usage() {
    let fixture = WorkerFixture::new();
    let result = run_agent_job_with_paths(
        fixture.paths.clone(),
        AgentJob {
            system_prompt: None,
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hello".into(),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            tools: vec![],
            model: None,
            reasoning: None,
            max_turns: 2,
            max_duration: Duration::from_secs(60),
        },
        &UsageBridge,
        &ProgressReporter::default(),
        &|| false,
    )
    .unwrap();

    assert_eq!(result.text, "usage stays host-side");
}
