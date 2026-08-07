use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use app_host_kernel::JsonObject;
use mcp_adapter::transport::RequestOptions;
use mcp_adapter::{McpClient, StdioTransport};
use serde_json::{json, Value};

struct TestDirectory(PathBuf);

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn appcontainer_probe_speaks_the_strict_mcp_contract() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is monotonic enough")
        .as_nanos();
    let root =
        TestDirectory(std::env::temp_dir().join(format!("mcp-appcontainer-protocol-{unique}")));
    let payload_dir = root.0.join("payload");
    let data_dir = root.0.join("data");
    let outside_path = root.0.join("outside.txt");
    fs::create_dir_all(&payload_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(payload_dir.join("payload.txt"), b"payload-ok").unwrap();
    fs::write(&outside_path, b"outside").unwrap();
    let loopback_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let loopback_target = loopback_listener.local_addr().unwrap().to_string();

    let payload = payload_dir.to_string_lossy();
    let data = data_dir.to_string_lossy();
    let outside = outside_path.to_string_lossy();
    let transport = StdioTransport::spawn_in_isolated(
        env!("CARGO_BIN_EXE_appcontainer_probe"),
        &[],
        &payload_dir,
        &[
            ("APP_HOST_PAYLOAD_DIR", payload.as_ref()),
            ("APP_HOST_DATA_DIR", data.as_ref()),
            ("APP_HOST_OUTSIDE_PATH", outside.as_ref()),
            ("APP_HOST_LOOPBACK_TARGET", &loopback_target),
        ],
    )
    .expect("probe launches");
    let client = McpClient::connect(Box::new(transport)).expect("probe initializes");

    assert_eq!(client.server_name(), "appcontainer-probe");
    let tools = client.list_tools().expect("tools/list answers");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "probe_status");

    let result = client
        .call_tool(
            "probe_status",
            &JsonObject::new(),
            &RequestOptions::with_timeout(Duration::from_secs(10)),
        )
        .expect("tools/call answers");
    let result = match result {
        Value::Object(result) => result,
        other => panic!("expected probe object, got {other}"),
    };
    assert_eq!(result["payload_contents"], json!("payload-ok"));
    assert_eq!(result["data_write_ok"], json!(true));
    assert_eq!(result["host_loopback_connected"], json!(true));
    loopback_listener
        .accept()
        .expect("unsandboxed probe reaches the host listener");

    client.shutdown();
}
