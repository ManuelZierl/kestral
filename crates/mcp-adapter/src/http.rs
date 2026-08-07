//! MCP Streamable HTTP transport (spec revision 2025-06-18).
//!
//! Every JSON-RPC message is an HTTP POST to the server's single MCP
//! endpoint. The server answers a request either with a plain
//! `application/json` body or with a `text/event-stream` body that carries
//! the response (and possibly other messages) as SSE events — both are
//! supported. Sessions (`Mcp-Session-Id`) and the negotiated
//! `MCP-Protocol-Version` header are captured from the `initialize`
//! exchange and attached to every subsequent request. Shutdown sends a
//! best-effort HTTP DELETE to end the session.
//!
//! Synchronous by design, like the stdio transport: the host runs MCP work
//! on blocking workers. Timeouts bound the whole request including body
//! reads; cancellation is honored between SSE events and, best-effort,
//! announced to the server as `notifications/cancelled`.

use std::io::{BufRead, BufReader, Read};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Value};

use crate::errors::McpError;
use crate::transport::{McpTransport, RequestOptions};

const NOTIFY_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub struct StreamableHttpTransport {
    endpoint: String,
    http: reqwest::blocking::Client,
    request_headers: HeaderMap,
    session_id: Mutex<Option<String>>,
    negotiated_version: Mutex<Option<String>>,
    next_request_id: AtomicU64,
    closed: AtomicBool,
}

impl StreamableHttpTransport {
    /// Point the transport at a server's MCP endpoint URL. No I/O happens
    /// here; the `initialize` request establishes the session.
    pub fn new(endpoint: &str) -> Result<Self, McpError> {
        Self::with_headers(endpoint, Vec::new())
    }

    /// Configure static secret headers for a host-managed server. Values are
    /// validated once, marked sensitive, and attached to every request in the
    /// MCP session, including cancellation and DELETE shutdown.
    pub fn with_headers(endpoint: &str, headers: Vec<(String, String)>) -> Result<Self, McpError> {
        let request_headers = validated_request_headers(endpoint, &headers)?;
        let http = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| McpError::Transport(format!("HTTP client init failed: {error}")))?;
        Ok(Self {
            endpoint: endpoint.to_string(),
            http,
            request_headers,
            session_id: Mutex::new(None),
            negotiated_version: Mutex::new(None),
            next_request_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
        })
    }

    /// Validate endpoint and header policy without constructing an HTTP
    /// client. Safe for config validation on an asynchronous runtime thread.
    pub fn validate_settings(endpoint: &str, headers: &[(String, String)]) -> Result<(), McpError> {
        validated_request_headers(endpoint, headers).map(|_| ())
    }

    fn post(
        &self,
        method: &str,
        body: &Value,
        timeout: Duration,
    ) -> Result<reqwest::blocking::Response, McpError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(McpError::Transport("MCP HTTP transport is closed".into()));
        }
        let mut request = self
            .http
            .post(&self.endpoint)
            .timeout(timeout)
            .headers(self.request_headers.clone())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        // The negotiated protocol version rides on every request after
        // initialize (the spec's MCP-Protocol-Version requirement).
        if let Some(version) = self.negotiated_version.lock().unwrap().as_deref() {
            request = request.header("MCP-Protocol-Version", version);
        }
        if let Some(session) = self.session_id.lock().unwrap().as_deref() {
            request = request.header("Mcp-Session-Id", session);
        }
        let response = request.body(body.to_string()).send().map_err(|error| {
            if error.is_timeout() {
                McpError::Timeout {
                    method: method.to_string(),
                    timeout,
                }
            } else {
                transport_error("MCP endpoint unreachable", &error)
            }
        })?;
        if response.status().as_u16() == 404 && self.session_id.lock().unwrap().is_some() {
            return Err(McpError::Transport(
                "MCP session expired (server answered 404); reconnect the server".into(),
            ));
        }
        if !response.status().is_success() {
            return Err(McpError::Transport(format!(
                "MCP endpoint answered HTTP {}",
                response.status()
            )));
        }
        Ok(response)
    }

    /// Best-effort `notifications/cancelled` so the server can stop working
    /// on a request the caller abandoned.
    fn announce_cancellation(&self, request_id: u64) {
        let body = json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": {"requestId": request_id, "reason": "client cancelled"},
        });
        let _ = self.post("notifications/cancelled", &body, NOTIFY_TIMEOUT);
    }
}

fn validated_request_headers(
    endpoint: &str,
    headers: &[(String, String)],
) -> Result<HeaderMap, McpError> {
    let endpoint_url = reqwest::Url::parse(endpoint).map_err(|_| {
        McpError::Transport(format!(
            "MCP endpoint must be an absolute http(s) URL, got '{endpoint}'"
        ))
    })?;
    if endpoint_url.scheme() != "http" && endpoint_url.scheme() != "https" {
        return Err(McpError::Transport(format!(
            "MCP endpoint must be an http(s) URL, got '{endpoint}'"
        )));
    }
    if endpoint_url.host_str().is_none()
        || !endpoint_url.username().is_empty()
        || endpoint_url.password().is_some()
        || endpoint_url.query().is_some()
        || endpoint_url.fragment().is_some()
    {
        return Err(McpError::Transport(
            "MCP endpoint must have a host and must not contain credentials, a query, or a fragment"
                .into(),
        ));
    }
    if headers.len() > 16 {
        return Err(McpError::Transport(
            "MCP HTTP authentication has too many headers".into(),
        ));
    }
    if !headers.is_empty() && endpoint_url.scheme() != "https" && !is_loopback_url(&endpoint_url) {
        return Err(McpError::Transport(
            "MCP HTTP credentials require HTTPS unless the endpoint is loopback".into(),
        ));
    }
    let mut request_headers = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| McpError::Transport("MCP authentication header name is invalid".into()))?;
        if is_reserved_header(&name) {
            return Err(McpError::Transport(format!(
                "MCP authentication cannot override reserved header '{name}'"
            )));
        }
        if request_headers.contains_key(&name) {
            return Err(McpError::Transport(format!(
                "MCP authentication header '{name}' is duplicated"
            )));
        }
        let mut value = HeaderValue::from_str(value).map_err(|_| {
            McpError::Transport(format!(
                "MCP authentication header '{name}' has an invalid value"
            ))
        })?;
        value.set_sensitive(true);
        request_headers.insert(name, value);
    }
    Ok(request_headers)
}

impl McpTransport for StreamableHttpTransport {
    fn request(
        &self,
        method: &str,
        params: Value,
        options: &RequestOptions,
    ) -> Result<Value, McpError> {
        if options.is_cancelled() {
            return Err(McpError::Cancelled);
        }
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        });
        let response = self.post(method, &body, options.timeout)?;

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        if content_type.starts_with("application/json") {
            // The session header must be captured before `.json()` consumes
            // the response; the negotiated version sits in the result body.
            let session_header = response_header(&response, "mcp-session-id")?;
            let message = read_json_response(response, method, options.timeout)?;
            let result = unpack_response(&message, request_id, method)?;
            if method == "initialize" {
                if let Some(session) = session_header {
                    *self.session_id.lock().unwrap() = Some(session);
                }
                if let Some(version) = result.get("protocolVersion").and_then(Value::as_str) {
                    *self.negotiated_version.lock().unwrap() = Some(version.to_string());
                }
            }
            return Ok(result);
        }

        if content_type.starts_with("text/event-stream") {
            return self.read_sse_response(response, request_id, method, options);
        }

        Err(McpError::Protocol(format!(
            "'{method}' answered with unsupported content type '{content_type}'"
        )))
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), McpError> {
        let mut body = json!({"jsonrpc": "2.0", "method": method});
        if !params.is_null() {
            body["params"] = params;
        }
        // Servers answer notifications with 202 Accepted and no body; any
        // 2xx is fine, error statuses surfaced by post().
        self.post(method, &body, NOTIFY_TIMEOUT).map(|_| ())
    }

    fn shutdown(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let session = self.session_id.lock().unwrap().take();
        if let Some(session) = session {
            // Explicit session termination per spec; servers MAY answer 405
            // if they do not support it — either way the session is over
            // for us.
            let request = self
                .http
                .delete(&self.endpoint)
                .timeout(SHUTDOWN_TIMEOUT)
                .headers(self.request_headers.clone())
                .header("Mcp-Session-Id", &session);
            let request = if let Some(version) = self.negotiated_version.lock().unwrap().as_deref()
            {
                request.header("MCP-Protocol-Version", version)
            } else {
                request
            };
            let _ = request.send();
        }
    }
}

fn is_loopback_url(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn is_reserved_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "accept"
            | "connection"
            | "content-length"
            | "content-type"
            | "host"
            | "keep-alive"
            | "mcp-protocol-version"
            | "mcp-session-id"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

impl StreamableHttpTransport {
    fn read_sse_response(
        &self,
        response: reqwest::blocking::Response,
        request_id: u64,
        method: &str,
        options: &RequestOptions,
    ) -> Result<Value, McpError> {
        let is_initialize = method == "initialize";
        let session_header = response_header(&response, "mcp-session-id")?;
        let mut reader = BufReader::new(response);
        loop {
            if options.is_cancelled() {
                self.announce_cancellation(request_id);
                return Err(McpError::Cancelled);
            }
            let data = match next_sse_data(&mut reader) {
                Ok(Some(data)) => data,
                Ok(None) => {
                    return Err(McpError::Protocol(format!(
                        "SSE stream ended before the '{method}' response arrived"
                    )))
                }
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                    return Err(McpError::Timeout {
                        method: method.to_string(),
                        timeout: options.timeout,
                    })
                }
                Err(error) => {
                    return Err(McpError::Transport(format!(
                        "SSE stream broke during '{method}': {error}"
                    )))
                }
            };
            let message = serde_json::from_str::<Value>(&data).map_err(|error| {
                McpError::Protocol(format!("SSE carried malformed JSON-RPC data: {error}"))
            })?;
            if message.get("id").and_then(Value::as_u64) == Some(request_id)
                && message.get("method").is_none()
            {
                let result = unpack_response(&message, request_id, method)?;
                if is_initialize {
                    if let Some(session) = session_header {
                        *self.session_id.lock().unwrap() = Some(session);
                    }
                    if let Some(version) = result.get("protocolVersion").and_then(Value::as_str) {
                        *self.negotiated_version.lock().unwrap() = Some(version.to_string());
                    }
                }
                return Ok(result);
            }
            // Server-initiated requests/notifications inside the stream are
            // not consumed by this adapter yet (room for resources, prompts,
            // and MCP Apps); skip them and keep waiting for our response.
        }
    }
}

/// Extract the result from a JSON-RPC response object, mapping `error`
/// members to `McpError::Server`.
fn unpack_response(message: &Value, request_id: u64, method: &str) -> Result<Value, McpError> {
    if message.get("id").and_then(Value::as_u64) != Some(request_id) {
        return Err(McpError::Protocol(format!(
            "'{method}' answered with a mismatched response id"
        )));
    }
    crate::protocol::extract_result_or_server_error(message)
}

fn response_header(
    response: &reqwest::blocking::Response,
    name: &'static str,
) -> Result<Option<String>, McpError> {
    response
        .headers()
        .get(name)
        .map(|value| {
            value.to_str().map(str::to_string).map_err(|_| {
                McpError::Protocol(format!("response carries an invalid {name} header"))
            })
        })
        .transpose()
}

fn read_json_response(
    response: reqwest::blocking::Response,
    method: &str,
    timeout: Duration,
) -> Result<Value, McpError> {
    if response
        .content_length()
        .is_some_and(|length| length > crate::transport::MAX_MESSAGE_BYTES as u64)
    {
        return Err(McpError::Protocol(format!(
            "'{method}' JSON response exceeded {} bytes",
            crate::transport::MAX_MESSAGE_BYTES
        )));
    }
    let mut bytes = Vec::new();
    response
        .take(crate::transport::MAX_MESSAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                McpError::Timeout {
                    method: method.to_string(),
                    timeout,
                }
            } else {
                McpError::Transport(format!("HTTP response broke during '{method}': {error}"))
            }
        })?;
    if bytes.len() > crate::transport::MAX_MESSAGE_BYTES {
        return Err(McpError::Protocol(format!(
            "'{method}' JSON response exceeded {} bytes",
            crate::transport::MAX_MESSAGE_BYTES
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| McpError::Protocol(format!("malformed JSON response: {error}")))
}

/// Read the next SSE event's `data:` payload (joining multi-line data per
/// the SSE spec). Returns `None` at end of stream.
fn next_sse_data(reader: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut data_lines: Vec<String> = Vec::new();
    let mut buffered = 0usize;
    loop {
        let Some(line) = crate::transport::read_line_capped(reader)? else {
            return Ok(None); // stream closed
        };
        let line = line.trim_end_matches(['\r', '\n']);
        // One event can also be split across many `data:` lines, so the
        // joined payload needs the same ceiling as a single line.
        buffered += line.len();
        if buffered > crate::transport::MAX_MESSAGE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "SSE event exceeded the maximum message size",
            ));
        }
        if line.is_empty() {
            if !data_lines.is_empty() {
                return Ok(Some(data_lines.join("\n")));
            }
            continue; // blank line with no pending event
        }
        if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.strip_prefix(' ').unwrap_or(data).to_string());
        }
        // event:, id:, retry:, and comment lines don't carry payload.
    }
}

fn transport_error(context: &str, error: &dyn std::fmt::Display) -> McpError {
    McpError::Transport(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests;
