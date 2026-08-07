use std::fs;
use std::io::{self, BufRead, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

struct ProbeResult {
    payload_contents: String,
    data_write_ok: bool,
    outside_read_error: Option<i32>,
    payload_write_error: Option<i32>,
    host_loopback_connected: bool,
    host_loopback_error: Option<i32>,
}

fn main() {
    let payload_dir = PathBuf::from(std::env::var("APP_HOST_PAYLOAD_DIR").expect("payload dir"));
    let data_dir = PathBuf::from(std::env::var("APP_HOST_DATA_DIR").expect("data dir"));
    let outside_path = std::env::var("APP_HOST_OUTSIDE_PATH")
        .ok()
        .map(PathBuf::from);
    let host_loopback_target = std::env::var("APP_HOST_LOOPBACK_TARGET")
        .expect("host loopback target")
        .parse::<SocketAddr>()
        .expect("host loopback target is a socket address");

    let payload_contents = fs::read_to_string(payload_dir.join("payload.txt"))
        .unwrap_or_else(|error| format!("read-error:{error}"));
    let data_write_ok =
        fs::write(data_dir.join("allowed.txt"), payload_contents.as_bytes()).is_ok();
    let outside_read_error = outside_path.as_ref().and_then(|path| {
        fs::read_to_string(path)
            .err()
            .and_then(|error| error.raw_os_error())
    });
    let payload_write_error = fs::write(payload_dir.join("blocked-write.txt"), b"blocked")
        .err()
        .and_then(|error| error.raw_os_error());
    let (host_loopback_connected, host_loopback_error) =
        match TcpStream::connect_timeout(&host_loopback_target, Duration::from_secs(2)) {
            Ok(_) => (true, None),
            Err(error) => (false, error.raw_os_error()),
        };

    let probe = ProbeResult {
        payload_contents,
        data_write_ok,
        outside_read_error,
        payload_write_error,
        host_loopback_connected,
        host_loopback_error,
    };

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let response = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}},
                    "serverInfo": {
                        "name": "appcontainer-probe",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": [{
                        "name": "probe_status",
                        "description": "Return AppContainer probe results.",
                        "inputSchema": {"type": "object", "additionalProperties": false},
                        "outputSchema": {
                            "type": "object",
                            "properties": {
                                "payload_contents": {"type": "string"},
                                "data_write_ok": {"type": "boolean"},
                                "outside_read_error": {"type": ["integer", "null"]},
                                "payload_write_error": {"type": ["integer", "null"]},
                                "host_loopback_connected": {"type": "boolean"},
                                "host_loopback_error": {"type": ["integer", "null"]}
                            },
                            "required": ["payload_contents", "data_write_ok", "outside_read_error", "payload_write_error", "host_loopback_connected", "host_loopback_error"],
                            "additionalProperties": false
                        }
                    }]
                }
            }),
            "tools/call" => {
                let name = message
                    .get("params")
                    .and_then(|params| params.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if name == "probe_status" {
                    let structured_content = json!({
                        "payload_contents": probe.payload_contents,
                        "data_write_ok": probe.data_write_ok,
                        "outside_read_error": probe.outside_read_error,
                        "payload_write_error": probe.payload_write_error,
                        "host_loopback_connected": probe.host_loopback_connected,
                        "host_loopback_error": probe.host_loopback_error
                    });
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{
                                "type": "text",
                                "text": structured_content.to_string()
                            }],
                            "structuredContent": structured_content
                        }
                    })
                } else {
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32602, "message": format!("unknown tool {name}")}
                    })
                }
            }
            "notifications/initialized" => continue,
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("unsupported method {method}")}
            }),
        };
        let _ = writeln!(stdout, "{response}");
        let _ = stdout.flush();
    }
}
