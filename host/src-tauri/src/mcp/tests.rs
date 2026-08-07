use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use mcp_adapter::{McpClient, McpError, McpTransport, RequestOptions};
use serde_json::{json, Value};

use super::{McpConnections, Slot};

struct TestTransport;

impl McpTransport for TestTransport {
    fn request(
        &self,
        method: &str,
        _params: Value,
        _options: &RequestOptions,
    ) -> Result<Value, McpError> {
        assert_eq!(method, "initialize");
        Ok(json!({
            "protocolVersion": "2025-06-18",
            "serverInfo": {"name": "test", "version": "1"},
            "capabilities": {"tools": {}}
        }))
    }

    fn notify(&self, _method: &str, _params: Value) -> Result<(), McpError> {
        Ok(())
    }

    fn shutdown(&self) {}
}

fn client() -> Arc<McpClient> {
    Arc::new(McpClient::connect(Box::new(TestTransport)).expect("test client connects"))
}

fn ready_connections(server_id: &str, client: Arc<McpClient>) -> McpConnections {
    let connections = McpConnections::default();
    connections.begin(server_id).expect("connect begins");
    connections
        .complete(server_id, client)
        .expect("connect completes");
    connections
}

#[test]
fn connect_is_blocked_while_disconnect_is_in_flight() {
    let connections = ready_connections("one", client());
    let _transition = connections
        .begin_disconnect("one")
        .expect("disconnect begins");

    let error = connections
        .begin("one")
        .expect_err("connect must be blocked");

    assert!(error.contains("already connected"));
}

#[test]
fn successful_disconnect_removes_exact_slot() {
    let original = client();
    let connections = ready_connections("one", original.clone());
    let transition = connections
        .begin_disconnect("one")
        .expect("disconnect begins");

    let removed = connections
        .complete_disconnect(&transition)
        .expect("disconnect completes");

    assert!(Arc::ptr_eq(&removed, &original));
    assert!(!connections.is_active("one").expect("state is readable"));
}

#[test]
fn failed_disconnect_restores_exact_client() {
    let original = client();
    let connections = ready_connections("one", original.clone());
    let transition = connections
        .begin_disconnect("one")
        .expect("disconnect begins");

    connections
        .rollback_disconnect(&transition)
        .expect("disconnect rolls back");

    let state = connections.state.lock().expect("state is readable");
    let Some(Slot::Ready(restored)) = state.clients.get("one") else {
        panic!("ready client was not restored");
    };
    assert!(Arc::ptr_eq(restored, &original));
}

#[test]
fn stale_disconnect_cannot_replace_newer_client() {
    let original = client();
    let connections = ready_connections("one", original);
    let stale = connections
        .begin_disconnect("one")
        .expect("first disconnect begins");
    connections
        .complete_disconnect(&stale)
        .expect("first disconnect completes");

    let newer = client();
    connections.begin("one").expect("reconnect begins");
    connections
        .complete("one", newer.clone())
        .expect("reconnect completes");

    connections
        .rollback_disconnect(&stale)
        .expect_err("stale rollback must fail");

    let state = connections.state.lock().expect("state is readable");
    let Some(Slot::Ready(current)) = state.clients.get("one") else {
        panic!("newer client was replaced");
    };
    assert!(Arc::ptr_eq(current, &newer));
}

#[test]
fn poisoned_connection_state_is_reported() {
    let connections = McpConnections::default();
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let _state = connections.state.lock().expect("state initially locks");
        panic!("poison test state");
    }));

    assert_eq!(
        connections
            .is_active("one")
            .expect_err("poison is surfaced"),
        "MCP state poisoned"
    );
    assert_eq!(
        connections.begin("one").expect_err("poison is surfaced"),
        "MCP state poisoned"
    );
}
