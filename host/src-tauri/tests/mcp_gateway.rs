use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use app_host_kernel::kernel::Kernel;
use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, TrustedChrome,
};
use host_lib::config::{
    HostConfigService, McpExportInteraction, McpExportProfile, McpExportProfileView,
    McpExportedCapability,
};
use host_lib::mcp_export::principal_install_parts;
use host_lib::mcp_gateway::{start_gateway, AuditLog, BearerProfileAuth, GatewayContext};
use host_lib::test_app::{test_app_install_parts, TestAppStore, TEST_APP_ID};
use serde_json::{json, Value};

struct AllowChrome {
    capability_approvals: AtomicUsize,
}

impl TrustedChrome for AllowChrome {
    fn confirm_grant(&self, _: GrantIssuancePrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }
    fn approve_capability(&self, _: CapabilityApprovalPrompt) -> ApprovalDecision {
        self.capability_approvals.fetch_add(1, Ordering::Relaxed);
        ApprovalDecision::Approved
    }
    fn confirm_event_subscriptions(&self, _: EventSubscriptionPrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }
    fn show_notice(&self, _: ChromeNotice) -> Result<(), ChromeNoticeError> {
        Ok(())
    }
}

fn post(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    token: &str,
    session: Option<&str>,
    body: Value,
) -> reqwest::blocking::Response {
    let mut request = client.post(endpoint).bearer_auth(token).json(&body);
    if let Some(session) = session {
        request = request.header("Mcp-Session-Id", session);
    }
    request.send().unwrap()
}

#[test]
fn authenticated_client_sees_only_exported_tools_and_writes_require_local_approval() {
    let directory = std::env::temp_dir().join(format!("mcp-gateway-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let config = Arc::new(Mutex::new(HostConfigService::default()));
    let profile = McpExportProfileView {
        id: "workspace-remote".into(),
        profile: McpExportProfile {
            display_name: "Workspace remote".into(),
            enabled: true,
            capabilities: vec![
                McpExportedCapability {
                    provider: TEST_APP_ID.into(),
                    capability: "search".into(),
                },
                McpExportedCapability {
                    provider: TEST_APP_ID.into(),
                    capability: "create".into(),
                },
            ],
            interaction: McpExportInteraction::RequiresApproval,
            expires_after_seconds: None,
            rate_limit_per_minute: 10,
            expose_results: true,
            expose_artifacts: false,
        },
    };
    let mut token = {
        let mut config = config.lock().unwrap();
        config.upsert_mcp_export_profile(profile.clone()).unwrap();
        config.rotate_mcp_export_token(&profile.id).unwrap()
    };
    let chrome = Arc::new(AllowChrome {
        capability_approvals: AtomicUsize::new(0),
    });
    let kernel = Arc::new(Mutex::new(Kernel::new(chrome.clone())));
    {
        let mut kernel = kernel.lock().unwrap();
        let (manifest, handlers) =
            test_app_install_parts(Arc::new(Mutex::new(TestAppStore::default())));
        install_parts(
            &mut kernel,
            manifest,
            handlers,
            app_host_kernel::primitives::grant::GrantOrigin::SystemBundled,
        )
        .unwrap();
        let (manifest, handlers) = principal_install_parts(&profile.id, &profile.profile);
        install_parts(
            &mut kernel,
            manifest,
            handlers,
            app_host_kernel::primitives::grant::GrantOrigin::McpExport,
        )
        .unwrap();
    }
    let audit_failure = start_gateway(
        "127.0.0.1:0",
        GatewayContext {
            kernel: kernel.clone(),
            config: config.clone(),
            auth: Arc::new(BearerProfileAuth::new(config.clone())),
            audit: Arc::new(AuditLog::new(Some(directory.clone()))),
            cancel_pending_approvals: Arc::new(|| {}),
        },
    )
    .err()
    .expect("an unwritable audit destination must prevent gateway startup");
    assert!(audit_failure.contains("audit log"));
    let gateway = start_gateway(
        "127.0.0.1:0",
        GatewayContext {
            kernel,
            config: config.clone(),
            auth: Arc::new(BearerProfileAuth::new(config.clone())),
            audit: Arc::new(AuditLog::new(None)),
            cancel_pending_approvals: Arc::new(|| {}),
        },
    )
    .unwrap();
    let endpoint = format!("http://{}/mcp", gateway.local_addr());
    let client = reqwest::blocking::Client::new();

    assert_eq!(post(&client, &endpoint, "wrong", None, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).status(), 401);
    let unsupported_version: Value = post(
        &client,
        &endpoint,
        &token,
        None,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}),
    )
    .json()
    .unwrap();
    assert_eq!(unsupported_version["error"]["code"], -32602);
    assert_eq!(
        unsupported_version["error"]["message"],
        "unsupported MCP protocol version; expected '2025-06-18'"
    );
    let rejected_origin = client
        .post(&endpoint)
        .bearer_auth(&token)
        .header("Origin", "https://localhost.evil.example")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}))
        .send()
        .unwrap();
    assert_eq!(rejected_origin.status(), 403);
    let old_token = token.clone();
    token = config
        .lock()
        .unwrap()
        .rotate_mcp_export_token(&profile.id)
        .unwrap();
    assert_eq!(post(&client, &endpoint, &old_token, None, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).status(), 401);
    let initialize = post(
        &client,
        &endpoint,
        &token,
        None,
        json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
    );
    let session = initialize
        .headers()
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let tools: Value = post(
        &client,
        &endpoint,
        &token,
        Some(&session),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}),
    )
    .json()
    .unwrap();
    let tools = tools["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
    assert!(tools.iter().all(|tool| tool["name"]
        .as_str()
        .unwrap()
        .starts_with("com_example_workspace__")));
    assert!(tools.iter().all(|tool| tool["inputSchema"].is_object()));
    let search = tools
        .iter()
        .find(|tool| tool["name"].as_str().unwrap().contains("search"))
        .unwrap()["name"]
        .as_str()
        .unwrap();
    let create = tools
        .iter()
        .find(|tool| tool["name"].as_str().unwrap().contains("create"))
        .unwrap()["name"]
        .as_str()
        .unwrap();
    let search_result: Value = post(&client, &endpoint, &token, Some(&session), json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":search,"arguments":{"query":"missing"}}})).json().unwrap();
    assert!(search_result["result"]["content"].is_array());
    let approvals_before_write = chrome.capability_approvals.load(Ordering::Relaxed);
    let create_result: Value = post(&client, &endpoint, &token, Some(&session), json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":create,"arguments":{"title":"Remote","body":"Created"}}})).json().unwrap();
    assert!(create_result["result"]["content"].is_array());
    assert!(chrome.capability_approvals.load(Ordering::Relaxed) > approvals_before_write);
    let unexported: Value = post(&client, &endpoint, &token, Some(&session), json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"com_example_workspace__delete_not_exported","arguments":{}}})).json().unwrap();
    assert_eq!(unexported["error"]["code"], -32602);
    config
        .lock()
        .unwrap()
        .set_mcp_export_enabled(&profile.id, false)
        .unwrap();
    assert_eq!(
        post(
            &client,
            &endpoint,
            &token,
            Some(&session),
            json!({"jsonrpc":"2.0","id":7,"method":"tools/list","params":{}})
        )
        .status(),
        401
    );
    gateway.stop();
    let _ = std::fs::remove_dir_all(directory);
}
mod support;
use support::*;
