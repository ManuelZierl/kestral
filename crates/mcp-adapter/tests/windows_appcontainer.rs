#![cfg(windows)]

use std::fs;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use app_host_kernel::JsonObject;
use mcp_adapter::stdio::delete_app_container_profile;
use mcp_adapter::transport::RequestOptions;
use mcp_adapter::{McpClient, McpError, StdioTransport};
use serde_json::{json, Value};

struct TestCleanup {
    profile: String,
    root: PathBuf,
    active: bool,
}

impl TestCleanup {
    fn finish(mut self) {
        delete_app_container_profile(&self.profile).expect("AppContainer profile cleanup");
        fs::remove_dir_all(&self.root).expect("test directory cleanup");
        self.active = false;
    }
}

impl Drop for TestCleanup {
    fn drop(&mut self) {
        if self.active {
            let _ = delete_app_container_profile(&self.profile);
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn helper_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_appcontainer_probe"))
}

#[test]
fn appcontainer_confines_authority_or_fails_closed_when_policy_blocks_launch() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is monotonic enough")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mcp-appcontainer-{unique}"));
    let profile = format!("com.ma-zierl.kestral.test-probe-{:016x}", unique as u64);
    let cleanup = TestCleanup {
        profile: profile.clone(),
        root: root.clone(),
        active: true,
    };
    let payload_dir = root.join("payload");
    let data_dir = root.join("data");
    let outside_dir = root.join("outside");
    fs::create_dir_all(&payload_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&outside_dir).unwrap();

    fs::write(payload_dir.join("payload.txt"), b"payload-ok").unwrap();
    fs::write(outside_dir.join("blocked.txt"), b"outside-secret").unwrap();
    let loopback_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    loopback_listener.set_nonblocking(true).unwrap();
    let loopback_target = loopback_listener.local_addr().unwrap().to_string();

    let helper = payload_dir.join("probe.exe");
    fs::copy(helper_binary(), &helper).unwrap();

    let transport = match StdioTransport::spawn_sandboxed(
        &profile,
        helper.to_str().expect("helper path is UTF-8"),
        &[],
        &payload_dir,
        &data_dir,
        &[
            ("APP_HOST_PAYLOAD_DIR", payload_dir.to_str().unwrap()),
            ("APP_HOST_DATA_DIR", data_dir.to_str().unwrap()),
            (
                "APP_HOST_OUTSIDE_PATH",
                outside_dir.join("blocked.txt").to_str().unwrap(),
            ),
            ("APP_HOST_LOOPBACK_TARGET", &loopback_target),
        ],
    ) {
        Ok(transport) => transport,
        Err(McpError::Transport(message)) if policy_blocked_launch(&message) => return,
        Err(error) => panic!("AppContainer launch failed unexpectedly: {error:?}"),
    };

    let client = McpClient::connect(Box::new(transport)).expect("MCP handshake succeeds");
    let tools = client.list_tools().expect("tools/list answers");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "probe_status");

    let options = RequestOptions::with_timeout(Duration::from_secs(10));
    let arguments = JsonObject::new();
    let result = client
        .call_tool("probe_status", &arguments, &options)
        .expect("probe call answers");
    let result = match result {
        Value::Object(result) => result,
        other => panic!("expected probe object, got {other}"),
    };
    assert_eq!(result["payload_contents"], json!("payload-ok"));
    assert_eq!(result["data_write_ok"], json!(true));
    assert!(result["outside_read_error"].is_number());
    assert!(result["payload_write_error"].is_number());
    assert_eq!(result["host_loopback_connected"], json!(false));
    match loopback_listener.accept() {
        Err(error) if error.kind() == ErrorKind::WouldBlock => {}
        Ok(_) => panic!("AppContainer connected to the host loopback listener"),
        Err(error) => panic!("inspect host loopback listener failed: {error}"),
    }

    client.shutdown();
    assert!(!payload_dir.join("blocked-write.txt").exists());
    assert_eq!(
        fs::read(data_dir.join("allowed.txt")).unwrap(),
        b"payload-ok"
    );
    cleanup.finish();
}

fn policy_blocked_launch(message: &str) -> bool {
    message == "create AppContainer profile failed: 0x80070005"
        || message == "create AppContainer backend failed: 5"
}
