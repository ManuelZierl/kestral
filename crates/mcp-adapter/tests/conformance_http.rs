//! Conformance of the Streamable HTTP transport against a local in-process
//! test server: initialization, session ids, protocol-version headers, JSON
//! and SSE response bodies, error mapping, timeouts, cancellation, and
//! clean shutdown via HTTP DELETE.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use app_host_kernel::JsonObject;
use mcp_adapter::transport::RequestOptions;
use mcp_adapter::{McpClient, McpError, StreamableHttpTransport};

const SESSION_ID: &str = "test-session-42";
const PROTOCOL_VERSION: &str = "2025-06-18";

/// What the test server observed, for conformance assertions.
#[derive(Default)]
struct Observed {
    initialized_notification: bool,
    delete_with_session: bool,
    delete_with_version: bool,
    /// Protocol-version headers on every request after initialize.
    post_init_version_headers: Vec<Option<String>>,
    /// Session headers on every request after initialize.
    post_init_session_headers: Vec<Option<String>>,
    auth_headers: Vec<Option<String>>,
}

struct TestServer {
    port: u16,
    observed: Arc<Mutex<Observed>>,
    shutdown: Arc<AtomicBool>,
}

impl TestServer {
    fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        // Unblock recv() so the server thread can exit.
        let _ = reqwest::blocking::Client::new()
            .post(self.endpoint())
            .timeout(Duration::from_millis(200))
            .body("{}")
            .send();
    }
}

fn header(request: &tiny_http::Request, name: &str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|header| header.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str().to_string())
}

fn json_response(body: &Value) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_string(body.to_string()).with_header(
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
    )
}

fn sse_response(message: &Value) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let body = format!("event: message\ndata: {message}\n\n");
    tiny_http::Response::from_string(body).with_header(
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream"[..]).unwrap(),
    )
}

/// A minimal Streamable HTTP MCP server: one endpoint, JSON or SSE answers,
/// a session id issued at initialize, one thread per request so a slow tool
/// cannot block the cancellation notification.
fn start_test_server() -> TestServer {
    let server =
        Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("test server binds a port"));
    let address = server
        .server_addr()
        .to_ip()
        .expect("TCP test server returns an IP listen address");
    let port = address.port();
    let observed = Arc::new(Mutex::new(Observed::default()));
    let shutdown = Arc::new(AtomicBool::new(false));

    let loop_server = server.clone();
    let loop_observed = observed.clone();
    let loop_shutdown = shutdown.clone();
    std::thread::spawn(move || {
        while let Ok(request) = loop_server.recv() {
            if loop_shutdown.load(Ordering::Relaxed) {
                break;
            }
            let observed = loop_observed.clone();
            std::thread::spawn(move || handle(request, observed));
        }
    });

    TestServer {
        port,
        observed,
        shutdown,
    }
}

fn handle(mut request: tiny_http::Request, observed: Arc<Mutex<Observed>>) {
    observed
        .lock()
        .unwrap()
        .auth_headers
        .push(header(&request, "Authorization"));
    if request.method() == &tiny_http::Method::Delete {
        let with_session = header(&request, "Mcp-Session-Id").as_deref() == Some(SESSION_ID);
        let with_version =
            header(&request, "MCP-Protocol-Version").as_deref() == Some(PROTOCOL_VERSION);
        let mut observed = observed.lock().unwrap();
        observed.delete_with_session = with_session;
        observed.delete_with_version = with_version;
        let _ = request.respond(tiny_http::Response::empty(200));
        return;
    }

    let mut body = String::new();
    let _ = request.as_reader().read_to_string(&mut body);
    let message: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let id = message.get("id").cloned().unwrap_or(Value::Null);

    if method != "initialize" && !method.is_empty() {
        let mut observed = observed.lock().unwrap();
        observed
            .post_init_version_headers
            .push(header(&request, "MCP-Protocol-Version"));
        observed
            .post_init_session_headers
            .push(header(&request, "Mcp-Session-Id"));
    }

    match method {
        "initialize" => {
            let response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "http-test-server", "version": "0.0.1"},
                },
            });
            let _ = request.respond(
                json_response(&response).with_header(
                    tiny_http::Header::from_bytes(&b"Mcp-Session-Id"[..], SESSION_ID.as_bytes())
                        .unwrap(),
                ),
            );
        }
        "notifications/initialized" => {
            observed.lock().unwrap().initialized_notification = true;
            let _ = request.respond(tiny_http::Response::empty(202));
        }
        "notifications/cancelled" => {
            let _ = request.respond(tiny_http::Response::empty(202));
        }
        // tools/list answers over SSE to exercise the optional stream path.
        "tools/list" => {
            let response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": [{
                        "name": "get_forecast",
                        "description": "Get the weather forecast for a city",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"city": {"type": "string"}},
                            "required": ["city"],
                            "additionalProperties": false,
                        },
                        "outputSchema": {
                            "type": "object",
                            "properties": {
                                "city": {"type": "string"},
                                "forecast": {"type": "string"},
                            },
                            "required": ["city", "forecast"],
                            "additionalProperties": false,
                        },
                    }],
                },
            });
            let _ = request.respond(sse_response(&response));
        }
        "tools/call" => {
            let tool = message
                .get("params")
                .and_then(|params| params.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            match tool {
                "get_forecast" => {
                    let city = message
                        .get("params")
                        .and_then(|params| params.get("arguments"))
                        .and_then(|arguments| arguments.get("city"))
                        .and_then(Value::as_str)
                        .unwrap_or("nowhere");
                    let data = json!({"city": city, "forecast": "sunny"});
                    let response = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{"type": "text", "text": data.to_string()}],
                            "structuredContent": data,
                        },
                    });
                    let _ = request.respond(json_response(&response));
                }
                "slow_tool" => {
                    std::thread::sleep(Duration::from_secs(5));
                    let response = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {"content": [], "structuredContent": {"late": true}},
                    });
                    let _ = request.respond(json_response(&response));
                }
                "malformed_response" => {
                    let response = json!({"jsonrpc": "2.0", "id": id});
                    let _ = request.respond(json_response(&response));
                }
                "oversized_response" => {
                    let body = "x".repeat(8 * 1024 * 1024 + 1);
                    let _ = request.respond(
                        tiny_http::Response::from_string(body).with_header(
                            tiny_http::Header::from_bytes(
                                &b"Content-Type"[..],
                                &b"application/json"[..],
                            )
                            .unwrap(),
                        ),
                    );
                }
                _ => {
                    let response = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32602, "message": format!("unknown tool '{tool}'")},
                    });
                    let _ = request.respond(json_response(&response));
                }
            }
        }
        _ => {
            let response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("method '{method}' not found")},
            });
            let _ = request.respond(json_response(&response));
        }
    }
}

fn obj(value: Value) -> JsonObject {
    match value {
        Value::Object(object) => object,
        other => panic!("expected object, got {other}"),
    }
}

#[test]
fn streamable_http_full_session_conforms() {
    let server = start_test_server();
    let transport = StreamableHttpTransport::with_headers(
        &server.endpoint(),
        vec![("Authorization".into(), "Bearer test-credential".into())],
    )
    .unwrap();
    let client = McpClient::connect(Box::new(transport)).expect("initialize over HTTP succeeds");
    assert_eq!(client.protocol_version(), PROTOCOL_VERSION);
    assert_eq!(client.server_name(), "http-test-server");

    // Discovery arrives over an SSE response body; output schema imported.
    let tools = client.list_tools().expect("tools/list over SSE answers");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "get_forecast");
    assert!(tools[0].output_schema.is_some());

    // A call answered as plain JSON.
    let options = RequestOptions::with_timeout(Duration::from_secs(10));
    let result = client
        .call_tool("get_forecast", &obj(json!({"city": "Berlin"})), &options)
        .expect("tools/call answers");
    assert_eq!(result, json!({"city": "Berlin", "forecast": "sunny"}));

    // A server-side error maps to a contained McpError::Server.
    let error = client
        .call_tool("no_such_tool", &obj(json!({})), &options)
        .unwrap_err();
    match error {
        McpError::Server { message, .. } => assert!(message.contains("unknown tool")),
        other => panic!("expected server error, got {other:?}"),
    }

    // Clean shutdown terminates the session with HTTP DELETE.
    client.shutdown();
    assert!(matches!(
        client.call_tool("get_forecast", &obj(json!({})), &options),
        Err(McpError::Transport(message)) if message.contains("closed")
    ));
    std::thread::sleep(Duration::from_millis(200));

    let observed = server.observed.lock().unwrap();
    assert!(
        observed.initialized_notification,
        "client sent notifications/initialized"
    );
    assert!(
        observed.delete_with_session,
        "shutdown sent DELETE with the session id"
    );
    assert!(
        observed.delete_with_version,
        "shutdown sent DELETE with the negotiated protocol version"
    );
    assert!(
        !observed.post_init_version_headers.is_empty()
            && observed
                .post_init_version_headers
                .iter()
                .all(|header| header.as_deref() == Some(PROTOCOL_VERSION)),
        "every post-initialize request carries the negotiated MCP-Protocol-Version, got {:?}",
        observed.post_init_version_headers
    );
    assert!(
        observed
            .post_init_session_headers
            .iter()
            .all(|header| header.as_deref() == Some(SESSION_ID)),
        "every post-initialize request carries the session id, got {:?}",
        observed.post_init_session_headers
    );
    assert!(
        !observed.auth_headers.is_empty()
            && observed
                .auth_headers
                .iter()
                .all(|header| header.as_deref() == Some("Bearer test-credential")),
        "every MCP HTTP request carries the configured authentication header"
    );
}

#[test]
fn streamable_http_rejects_malformed_and_oversized_json_responses() {
    let server = start_test_server();
    let transport = StreamableHttpTransport::new(&server.endpoint()).unwrap();
    let client = McpClient::connect(Box::new(transport)).unwrap();
    let options = RequestOptions::with_timeout(Duration::from_secs(10));

    assert!(matches!(
        client.call_tool("malformed_response", &obj(json!({})), &options),
        Err(McpError::Protocol(_))
    ));
    assert!(matches!(
        client.call_tool("oversized_response", &obj(json!({})), &options),
        Err(McpError::Protocol(message)) if message.contains("exceeded")
    ));
}

#[test]
fn streamable_http_timeout_is_contained() {
    let server = start_test_server();
    let transport = StreamableHttpTransport::new(&server.endpoint()).unwrap();
    let client = McpClient::connect(Box::new(transport)).unwrap();

    let options = RequestOptions::with_timeout(Duration::from_millis(400));
    let started = Instant::now();
    let error = client
        .call_tool("slow_tool", &obj(json!({})), &options)
        .unwrap_err();
    assert!(
        matches!(error, McpError::Timeout { .. }),
        "expected timeout, got {error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn streamable_http_cancellation_short_circuits() {
    let server = start_test_server();
    let transport = StreamableHttpTransport::new(&server.endpoint()).unwrap();
    let client = McpClient::connect(Box::new(transport)).unwrap();

    let options = RequestOptions {
        timeout: Duration::from_secs(30),
        cancel: Some(Arc::new(|| true)),
    };
    let started = Instant::now();
    let error = client
        .call_tool("slow_tool", &obj(json!({})), &options)
        .unwrap_err();
    assert!(matches!(error, McpError::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(3));
}
