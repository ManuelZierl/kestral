//! Unit tests for session-level behavior that needs a scripted transport
//! rather than a real server.

use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};

use super::McpClient;
use crate::errors::McpError;
use crate::protocol::LATEST_PROTOCOL_VERSION;
use crate::transport::{McpTransport, RequestOptions};

/// A transport that completes the handshake, then answers every `tools/list`
/// page with the *same* `nextCursor` — the classic non-terminating-pagination
/// attack an untrusted server can mount while answering each request in time.
struct LoopingCursorTransport {
    list_calls: AtomicUsize,
    protocol_version: &'static str,
    cursor: Value,
}

impl McpTransport for LoopingCursorTransport {
    fn request(
        &self,
        method: &str,
        _params: Value,
        _options: &RequestOptions,
    ) -> Result<Value, McpError> {
        match method {
            "initialize" => Ok(json!({
                "protocolVersion": self.protocol_version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "looping fake", "version": "1.0.0"},
            })),
            "tools/list" => {
                self.list_calls.fetch_add(1, Ordering::Relaxed);
                Ok(json!({"tools": [], "nextCursor": self.cursor.clone()}))
            }
            other => Err(McpError::Protocol(format!("unexpected method '{other}'"))),
        }
    }

    fn notify(&self, _method: &str, _params: Value) -> Result<(), McpError> {
        Ok(())
    }

    fn shutdown(&self) {}
}

#[test]
fn initialize_requires_complete_tool_server_metadata() {
    for result in [
        json!({
            "protocolVersion": LATEST_PROTOCOL_VERSION,
            "serverInfo": {"name": "missing capabilities", "version": "1"},
        }),
        json!({
            "protocolVersion": LATEST_PROTOCOL_VERSION,
            "capabilities": {},
            "serverInfo": {"name": "missing tools", "version": "1"},
        }),
        json!({
            "protocolVersion": LATEST_PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "missing version"},
        }),
    ] {
        assert!(matches!(
            super::validate_initialize_result(&result),
            Err(McpError::Protocol(_))
        ));
    }
}

#[test]
fn list_tools_rejects_a_repeated_pagination_cursor() {
    let transport = LoopingCursorTransport {
        list_calls: AtomicUsize::new(0),
        protocol_version: LATEST_PROTOCOL_VERSION,
        cursor: json!("same-cursor"),
    };
    let client = McpClient::connect(Box::new(transport)).expect("handshake succeeds");
    let error = client
        .list_tools()
        .expect_err("a repeated cursor must be rejected");
    match error {
        McpError::Protocol(message) => {
            assert!(
                message.contains("repeated pagination cursor"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

#[test]
fn connect_rejects_a_non_current_protocol_version() {
    let transport = LoopingCursorTransport {
        list_calls: AtomicUsize::new(0),
        protocol_version: "2025-03-26",
        cursor: json!("unused"),
    };
    let error = match McpClient::connect(Box::new(transport)) {
        Ok(_) => panic!("old revision must be rejected"),
        Err(error) => error,
    };
    match error {
        McpError::Protocol(message) => {
            assert!(message.contains("2025-03-26"));
            assert!(message.contains(LATEST_PROTOCOL_VERSION));
        }
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

#[test]
fn list_tools_rejects_a_non_string_pagination_cursor() {
    let transport = LoopingCursorTransport {
        list_calls: AtomicUsize::new(0),
        protocol_version: LATEST_PROTOCOL_VERSION,
        cursor: Value::Null,
    };
    let client = McpClient::connect(Box::new(transport)).expect("handshake succeeds");
    assert!(matches!(client.list_tools(), Err(McpError::Protocol(_))));
}
