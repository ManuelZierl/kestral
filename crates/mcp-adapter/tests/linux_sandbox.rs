#![cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use mcp_adapter::stdio::{native_backend_sandbox_support, StdioTransport};
use mcp_adapter::transport::{McpTransport, RequestOptions};
use serde_json::json;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("kestral-sandbox-test-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn packaged_backend_has_stdio_and_app_data_but_no_ambient_files_or_sockets() {
    let fixture = Fixture::new();
    let payload = fixture.0.join("payload");
    let data = fixture.0.join("data");
    let outside = fixture.0.join("outside");
    fs::create_dir(&payload).unwrap();
    fs::create_dir(&data).unwrap();
    fs::write(&outside, "synthetic private data").unwrap();

    let overlapping = StdioTransport::spawn_sandboxed(
        "sandbox-test",
        "/usr/bin/true",
        &[],
        &payload,
        &payload,
        &[],
    );
    assert!(matches!(overlapping, Err(e) if e.to_string().contains("must be separate")));

    let support = native_backend_sandbox_support();
    if !support.supports_sandboxed_execution() {
        assert!(
            std::env::var_os("KESTRAL_REQUIRE_LINUX_SANDBOX_TEST").is_none(),
            "sandbox required in CI: {support:?}"
        );
        eprintln!("sandbox integration not exercised: {support:?}");
        return;
    }

    let script = payload.join("probe.py");
    fs::write(&script, include_str!("fixtures/linux-sandbox-probe.py")).unwrap();
    let transport = StdioTransport::spawn_sandboxed(
        "sandbox-test",
        "/usr/bin/python3",
        &[script.to_str().unwrap(), outside.to_str().unwrap()],
        &payload,
        &data,
        &[("KESTRAL_TEST_AMBIENT", "not forwarded")],
    )
    .unwrap();
    let report = transport
        .request(
            "probe",
            json!({}),
            &RequestOptions::with_timeout(Duration::from_secs(5)),
        )
        .unwrap();
    assert_eq!(
        report,
        json!({
            "payload_read_only": true, "outside_hidden": true, "cwd_is_app_data": true,
            "scratch_writable": true, "environment_cleared": true, "tcp_denied": true,
            "unix_socket_denied": true,
        })
    );
    assert_eq!(fs::read_to_string(data.join("result")).unwrap(), "ok");
    assert!(!payload.join("unexpected").exists());
}
