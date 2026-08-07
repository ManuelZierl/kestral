use std::fs;
use std::path::Path;

use super::*;

#[test]
fn invocation_deadline_allows_worker_timeout_to_report_first() {
    assert!(INVOCATION_TIMEOUT > RESPONSE_TIMEOUT);
}

#[test]
fn llm_response_round_trip() {
    let response = sample_response();

    let json_val = serde_json::to_value(&response).unwrap();
    assert_eq!(json_val["message"]["role"], "assistant");
    assert_eq!(json_val["message"]["content"], "Hello");
    assert_eq!(json_val["message"]["tool_calls"][0]["id"], "call_1");
    assert_eq!(json_val["finish_reason"], "tool_calls");
    assert!(json_val["usage"]["total_tokens"].is_number());

    let deserialized: LlmResponse = serde_json::from_value(json_val).unwrap();
    assert_eq!(deserialized.finish_reason, "tool_calls");
    assert_eq!(deserialized.message.tool_calls.unwrap()[0].id, "call_1");
}

#[test]
fn llm_response_without_tool_calls() {
    let json_str = r#"{
        "message": {"role": "assistant", "content": "Just text"},
        "finish_reason": "stop"
    }"#;
    let deserialized: LlmResponse = serde_json::from_str(json_str).unwrap();
    assert_eq!(deserialized.finish_reason, "stop");
    assert!(deserialized.message.tool_calls.is_none());
    assert_eq!(deserialized.message.content, "Just text");
}

#[test]
fn response_and_event_reject_unknown_fields() {
    let response = r#"{
        "type":"completed",
        "request_id":"request-1",
        "response":{
            "message":{"role":"assistant","content":"ok","secret":"bad"},
            "finish_reason":"stop"
        }
    }"#;

    let error = serde_json::from_str::<WorkerEvent>(response).unwrap_err();

    assert!(error.to_string().contains("unknown field `secret`"));
}

#[test]
fn oauth_credential_is_strict_bounded_and_extensible() {
    let credential = OAuthCredential::parse_serialized(
        r#"{"type":"oauth","access":"access-token","refresh":"refresh-token","expires":1234,"account":{"id":"user-1"}}"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&credential).unwrap()["account"]["id"],
        "user-1"
    );

    for invalid in [
        r#"{"type":"oauth","access":"","refresh":"r","expires":1}"#,
        r#"{"type":"oauth","access":"a","refresh":"r","expires":-1}"#,
        r#"{"type":"oauth","access":"a","refresh":"r","expires":1,"constructor":{}}"#,
    ] {
        assert!(OAuthCredential::parse_serialized(invalid).is_err());
    }
    let oversized = format!(
        r#"{{"type":"oauth","access":"a","refresh":"r","expires":1,"extra":"{}"}}"#,
        "x".repeat(MAX_PUBLIC_STRING_BYTES + 1)
    );
    assert!(OAuthCredential::parse_serialized(&oversized).is_err());
}

#[test]
fn oauth_worker_events_and_prompts_reject_unknown_fields() {
    let event = serde_json::from_str::<WorkerEvent>(
        r#"{"type":"oauth-event","request_id":"r","event":{"type":"auth_url","url":"https://example.test","instructions":"Open it"}}"#,
    )
    .unwrap();
    assert!(matches!(event, WorkerEvent::OauthEvent { .. }));

    let unknown = r#"{"type":"oauth-prompt","request_id":"r","prompt_id":"p","prompt":{"type":"text","message":"Code","placeholder":null,"credential":"leak"}}"#;
    assert!(serde_json::from_str::<WorkerEvent>(unknown)
        .unwrap_err()
        .to_string()
        .contains("unknown field `credential`"));
}

#[test]
fn oauth_commands_match_worker_correlation_contract() {
    let login = serde_json::to_value(WorkerCommand::OauthLogin {
        request_id: "login-1".into(),
        provider: OAuthProviderConfig {
            kind: ProviderKind::OpenaiCodex,
            base_url: Some("https://chatgpt.com/backend-api".into()),
        },
    })
    .unwrap();
    let response = serde_json::to_value(WorkerCommand::OauthPromptResponse {
        request_id: "response-1".into(),
        target_request_id: "login-1".into(),
        prompt_id: "prompt-1".into(),
        value: Some("answer".into()),
        cancelled: false,
    })
    .unwrap();
    assert_eq!(login["command"], "oauth-login");
    assert_eq!(response["target_request_id"], "login-1");
    assert_eq!(response["prompt_id"], "prompt-1");
    assert!(response.get("cancelled").is_none());
}

#[test]
fn generate_command_matches_worker_protocol() {
    let command = WorkerCommand::Generate {
        request_id: "request-1".into(),
        provider: ProviderConfig {
            kind: ProviderKind::OpenAiCompatible,
            base_url: "https://provider.example/v1".into(),
            api_key: Some("credential".into()),
            oauth_credential: None,
        },
        model: "model-1".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: "hello".into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }],
        tools: None,
        response_format: Some(JsonObject::from_iter([(
            "type".into(),
            serde_json::Value::String("object".into()),
        )])),
        reasoning: Some("high".into()),
        text_verbosity: Some(TextVerbosity::High),
        temperature: Some(0.25),
        max_output_tokens: Some(100),
        timeout_ms: 120_000,
    };

    let encoded = serde_json::to_value(command).unwrap();

    assert_eq!(encoded["command"], "generate");
    assert_eq!(encoded["provider"]["kind"], "open-ai-compatible");
    assert_eq!(encoded["max_output_tokens"], 100);
    assert_eq!(encoded["response_format"]["type"], "object");
}

#[test]
fn model_commands_match_worker_protocol() {
    let provider = ProviderConfig {
        kind: ProviderKind::Ollama,
        base_url: "http://127.0.0.1:11434/v1".into(),
        api_key: None,
        oauth_credential: None,
    };
    let list = serde_json::to_value(WorkerCommand::ModelsList {
        request_id: "list-1".into(),
        provider: provider.clone(),
    })
    .unwrap();
    let refresh = serde_json::to_value(WorkerCommand::ModelsRefresh {
        request_id: "refresh-1".into(),
        provider,
    })
    .unwrap();

    assert_eq!(list["command"], "models-list");
    assert!(list.get("model").is_none());
    assert_eq!(refresh["command"], "models-refresh");
    assert_eq!(refresh["provider"]["kind"], "ollama");
}

#[test]
fn models_event_is_typed_and_strict() {
    let event = serde_json::from_str::<WorkerEvent>(
        r#"{"type":"models","request_id":"r","models":[{"id":"m","display_name":"Model","reasoning":true,"variants":["low","high"],"text_verbosity":["low","medium","high"],"context_window":128000,"max_output_tokens":8192}]}"#,
    )
    .unwrap();
    assert!(matches!(
        event,
        WorkerEvent::Models { models, .. }
            if models == vec![NormalizedModel {
                id: "m".into(),
                display_name: "Model".into(),
                reasoning: true,
                variants: vec![ModelVariant::Low, ModelVariant::High],
                text_verbosity: vec![
                    TextVerbosity::Low,
                    TextVerbosity::Medium,
                    TextVerbosity::High,
                ],
                context_window: 128_000,
                max_output_tokens: 8_192,
            }]
    ));

    let unknown = r#"{"type":"models","request_id":"r","models":[{"id":"m","display_name":"Model","reasoning":false,"variants":[],"context_window":1,"max_output_tokens":1,"cost":0}]}"#;
    assert!(serde_json::from_str::<WorkerEvent>(unknown)
        .unwrap_err()
        .to_string()
        .contains("unknown field `cost`"));
}

#[test]
fn event_parser_accepts_stream_delta_and_completed() {
    let delta = serde_json::from_str::<WorkerEvent>(
        r#"{"type":"stream-delta","request_id":"r","content":"a","reasoning":"b"}"#,
    )
    .unwrap();
    assert!(matches!(
        delta,
        WorkerEvent::StreamDelta { content, reasoning, .. }
            if content == "a" && reasoning == "b"
    ));

    let completed = r#"{
        "type":"completed",
        "request_id":"r",
        "response":{
            "message":{"role":"assistant","content":"ok"},
            "usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3,"cache_read_tokens":1,"cache_write_tokens":0,"cost":0.01},
            "provider_metrics":{"time_to_first_token_ms":12,"total_latency_ms":34},
            "finish_reason":"stop"
        }
    }"#;
    assert!(matches!(
        serde_json::from_str::<WorkerEvent>(completed).unwrap(),
        WorkerEvent::Completed { .. }
    ));
}

#[test]
fn explicit_worker_paths_must_be_set_together() {
    let error = resolve_worker_paths_from(
        Some(PathBuf::from("node")),
        None,
        Path::new("host/src-tauri"),
        PathBuf::from("target/host"),
    )
    .unwrap_err();

    assert_eq!(
        error,
        "KESTRAL_PROVIDER_NODE and KESTRAL_PROVIDER_WORKER must both be set"
    );
}

#[test]
fn explicit_worker_paths_report_the_exact_missing_path() {
    let root = temporary_directory("missing-path");
    let node = root.join(if cfg!(windows) { "node.exe" } else { "node" });
    let worker = root.join("worker.mjs");
    fs::write(&node, b"fake").unwrap();

    let error = validate_worker_paths(node, worker.clone()).unwrap_err();

    assert_eq!(
        error,
        format!("LLM provider worker script not found: {}", worker.display())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_tree_paths_precede_packaged_paths() {
    let root = temporary_directory("precedence");
    let manifest_dir = root.join("host/src-tauri");
    let source_root = root.join("host/provider-worker");
    let node = source_root
        .join("runtime")
        .join(if cfg!(windows) { "node.exe" } else { "node" });
    let worker = source_root.join("dist/worker.mjs");
    fs::create_dir_all(node.parent().unwrap()).unwrap();
    fs::create_dir_all(worker.parent().unwrap()).unwrap();
    fs::create_dir_all(&manifest_dir).unwrap();
    fs::write(&node, b"fake").unwrap();
    fs::write(&worker, b"fake").unwrap();

    let paths =
        resolve_worker_paths_from(None, None, &manifest_dir, root.join("packaged/host")).unwrap();

    assert_eq!(paths.node, node);
    assert_eq!(paths.worker, worker);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stdout_reader_rejects_oversized_lines() {
    let (sender, receiver) = mpsc::channel();
    let line = vec![b'x'; MAX_STDOUT_LINE_BYTES + 1];

    read_worker_events(line.as_slice(), sender);

    assert_eq!(
        receiver.recv().unwrap().unwrap_err(),
        "pi-ai worker output line exceeded 2 MiB"
    );
}

#[cfg(windows)]
#[test]
fn fake_worker_completes_interactive_oauth_without_public_credentials() {
    let node_output = std::process::Command::new("where.exe")
        .arg("node.exe")
        .output()
        .unwrap();
    assert!(
        node_output.status.success(),
        "node.exe is required for this test"
    );
    let node = String::from_utf8(node_output.stdout)
        .unwrap()
        .lines()
        .next()
        .map(PathBuf::from)
        .unwrap();
    let root = temporary_directory("oauth-worker");
    let worker = root.join("worker.mjs");
    fs::write(
        &worker,
        r#"import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin });
console.log(JSON.stringify({ type: "ready", protocol_version: 2 }));
lines.on("line", (line) => {
  const command = JSON.parse(line);
  if (command.command === "oauth-login") {
    console.log(JSON.stringify({ type: "oauth-event", request_id: command.request_id, event: { type: "auth_url", url: "https://example.test/login", instructions: "Sign in" } }));
    console.log(JSON.stringify({ type: "oauth-prompt", request_id: command.request_id, prompt_id: "code-1", prompt: { type: "manual_code", message: "Enter code", placeholder: "code" } }));
  } else if (command.command === "oauth-prompt-response") {
    console.log(JSON.stringify({ type: "oauth-completed", request_id: command.target_request_id, credential: { type: "oauth", access: "secret-access", refresh: "secret-refresh", expires: 123, accountId: "account-1" } }));
  } else if (command.command === "shutdown") {
    console.log(JSON.stringify({ type: "acknowledged", request_id: command.request_id, command: "shutdown" }));
    process.exit(0);
  }
});
"#,
    )
    .unwrap();

    let sessions = std::sync::Arc::new(PendingOAuthSessions::default());
    let (event_sender, event_receiver) = mpsc::channel();
    sessions
        .set_publisher(std::sync::Arc::new(move |event| {
            event_sender
                .send(event.clone())
                .map_err(|error| error.to_string())
        }))
        .unwrap();
    let (control_sender, controls) = mpsc::channel();
    sessions
        .register(
            "session-1".into(),
            "llm-provider/codex".into(),
            control_sender,
        )
        .unwrap();
    let login_sessions = sessions.clone();
    let login = thread::spawn(move || {
        run_oauth_login_with_paths(
            "session-1",
            ConnectorKind::OpenaiCodex,
            "https://chatgpt.com/backend-api".into(),
            &login_sessions,
            controls,
            WorkerPaths { node, worker },
        )
    });

    let auth_event = event_receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    let prompt_event = event_receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    assert!(matches!(auth_event, OAuthPublicEvent::AuthUrl { .. }));
    assert!(matches!(prompt_event, OAuthPublicEvent::Prompt { .. }));
    for event in [&auth_event, &prompt_event] {
        let public = serde_json::to_string(event).unwrap();
        assert!(!public.contains("secret-access"));
        assert!(!public.contains("secret-refresh"));
        assert!(!public.contains("credential"));
    }
    sessions
        .resolve_prompt("session-1", "code-1".into(), Some("1234".into()), false)
        .unwrap();
    let credential = login.join().unwrap().unwrap().serialize().unwrap();
    assert!(credential.contains("secret-access"));
    assert!(credential.contains("account-1"));
    sessions.finish("session-1");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn absent_usage_is_omitted_from_provider_output() {
    let mut response = sample_response();
    response.usage = None;

    let serialized = serde_json::to_value(response).unwrap();

    assert!(!serialized.as_object().unwrap().contains_key("usage"));
}

fn sample_response() -> LlmResponse {
    LlmResponse {
        message: ChatMessage {
            role: "assistant".into(),
            content: "Hello".into(),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".into(),
                type_: "function".into(),
                function: ToolCallFunction {
                    name: "notes__create_note".into(),
                    arguments: r#"{"text":"hello"}"#.into(),
                },
            }]),
            tool_call_id: None,
            name: None,
        },
        reasoning: None,
        usage: Some(Usage {
            prompt_tokens: Some(10),
            completion_tokens: Some(20),
            total_tokens: Some(30),
            cache_read_tokens: Some(4),
            cache_write_tokens: Some(2),
            cost: Some(0.001),
        }),
        provider_metrics: Some(ProviderMetrics {
            time_to_first_token_ms: Some(12),
            total_latency_ms: 34,
        }),
        finish_reason: "tool_calls".into(),
    }
}

fn temporary_directory(label: &str) -> PathBuf {
    let path = env::temp_dir().join(format!("kernel-llm-client-{label}-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}
