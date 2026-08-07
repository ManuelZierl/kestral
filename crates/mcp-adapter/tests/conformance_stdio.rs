//! Conformance of the stdio transport + client against the bundled Node
//! demo server. Requires `node` in PATH — already a prerequisite of the
//! host (the frontend builds with npm).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use app_host_kernel::JsonObject;
use mcp_adapter::transport::RequestOptions;
use mcp_adapter::{McpClient, McpError, McpTransport, StdioTransport};

fn demo_server_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("host")
        .join("demo-mcp-server")
        .join("server.mjs")
}

fn connect_demo_server() -> McpClient {
    let transport = mcp_adapter::stdio::spawn_node_server(&demo_server_script())
        .expect("node demo server launches");
    McpClient::connect(Box::new(transport)).expect("initialize handshake succeeds")
}

fn obj(value: Value) -> JsonObject {
    match value {
        Value::Object(object) => object,
        other => panic!("expected object, got {other}"),
    }
}

#[test]
fn stdio_handshake_discovery_and_calls_conform() {
    let client = connect_demo_server();
    assert_eq!(client.protocol_version(), "2025-06-18");
    assert_eq!(client.server_name(), "demo-weather");

    let tools = client.list_tools().expect("tools/list answers");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "get_forecast");
    assert_eq!(tools[0].input_schema["type"], json!("object"));

    let arguments = obj(json!({"city": "Berlin"}));
    let options = RequestOptions::with_timeout(Duration::from_secs(10));
    let result = client
        .call_tool("get_forecast", &arguments, &options)
        .expect("tools/call answers");
    assert_eq!(result["city"], json!("Berlin"));
    assert!(result["forecast"].is_string());

    // The demo server is deterministic: same city, same forecast.
    let again = client
        .call_tool("get_forecast", &arguments, &options)
        .unwrap();
    assert_eq!(again, result);

    // An unknown tool is a contained server error, not a hang or a crash.
    let error = client
        .call_tool("no_such_tool", &arguments, &options)
        .unwrap_err();
    match error {
        McpError::Server { message, .. } => assert!(message.contains("unknown tool")),
        other => panic!("expected server error, got {other:?}"),
    }
}

#[test]
fn stdio_cancellation_interrupts_a_silent_server() {
    // A process that speaks no MCP at all: every request would wait for the
    // full timeout if cancellation did not cut it short.
    let transport =
        StdioTransport::spawn("node", &["-e", "setInterval(() => {}, 1000)"]).expect("spawns");
    let cancelled = Arc::new(AtomicBool::new(true));
    let probe = cancelled.clone();
    let options = RequestOptions {
        timeout: Duration::from_secs(30),
        cancel: Some(Arc::new(move || probe.load(Ordering::Relaxed))),
    };
    let started = Instant::now();
    let error = transport
        .request("tools/list", json!({}), &options)
        .unwrap_err();
    assert!(matches!(error, McpError::Cancelled));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancellation should not wait for the 30s timeout"
    );
    transport.shutdown();
}

#[test]
fn stdio_timeout_bounds_a_silent_server() {
    let transport =
        StdioTransport::spawn("node", &["-e", "setInterval(() => {}, 1000)"]).expect("spawns");
    let options = RequestOptions::with_timeout(Duration::from_millis(300));
    let started = Instant::now();
    let error = transport
        .request("tools/list", json!({}), &options)
        .unwrap_err();
    assert!(matches!(error, McpError::Timeout { .. }));
    assert!(started.elapsed() < Duration::from_secs(5));
    transport.shutdown();
}

#[test]
fn stdio_rejects_malformed_server_output() {
    let transport = StdioTransport::spawn(
        "node",
        &[
            "-e",
            "process.stdin.once('data', () => { process.stdout.write('not-json\\n'); setInterval(() => {}, 1000); })",
        ],
    )
    .expect("spawns");
    let error = transport
        .request(
            "tools/list",
            json!({}),
            &RequestOptions::with_timeout(Duration::from_secs(2)),
        )
        .unwrap_err();
    assert!(matches!(error, McpError::Protocol(_)));
}

#[test]
fn stdio_write_obeys_the_request_timeout() {
    let transport =
        StdioTransport::spawn("node", &["-e", "setInterval(() => {}, 1000)"]).expect("spawns");
    let started = Instant::now();
    let error = transport
        .request(
            "tools/call",
            json!({"payload": "x".repeat(1024 * 1024)}),
            &RequestOptions::with_timeout(Duration::from_millis(300)),
        )
        .unwrap_err();
    assert!(matches!(error, McpError::Timeout { .. }));
    assert!(started.elapsed() < Duration::from_secs(5));
    transport.shutdown();
    transport.shutdown();
}
