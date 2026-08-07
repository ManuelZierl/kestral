//! Outbound MCP gateway: exposes selected host capabilities to remote MCP
//! clients over Streamable HTTP — entirely outside the kernel.
//!
//! Security model, in order:
//!
//! - **Nothing is exported by default.** Remote reach is exactly the live
//!   grants of a `mcp-export/<profile>` virtual principal ([`crate::mcp_export`]),
//!   confirmed through local trusted chrome when the profile is enabled.
//! - **No unauthenticated mode.** Every request must carry a bearer token
//!   minted for one profile; the token maps to that profile and nothing
//!   else. A tunnel (e.g. Cloudflare) in front of the listener is transport,
//!   not authentication. OAuth 2.1 protected-resource metadata is staged
//!   behind a config flag that validation keeps disabled until the
//!   implementation is correct.
//! - **Remote clients never submit app ids, grant ids, or capability
//!   references.** They call opaque tool names; the gateway resolves names
//!   against the profile and the principal's live grants on every call.
//! - **Local chrome stays authoritative.** A `requires-approval` grant
//!   prompts the local user on every remote call; the remote client just
//!   waits (or times out).
//! - Loopback bind by default (config validation enforces it), Origin
//!   validation, body caps, per-profile rate limits, session caps and idle
//!   expiry, per-call invocation timeouts, structured JSONL audit logging.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use app_host_kernel::invocation::InvocationResult;
use app_host_kernel::kernel::{Kernel, PrepareInvocation};
use app_host_kernel::primitives::capability::{CapabilityEffect, CapabilityRef};
use app_host_kernel::primitives::run::{Initiator, RunTerminalState};
use app_host_kernel::JsonObject;
use mcp_adapter::protocol::LATEST_PROTOCOL_VERSION;

use crate::config::{HostConfigService, McpExportProfile};
use crate::mcp_export::principal_app_id;
use crate::tool_mapping::cap_ref_to_tool_name;

const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_SESSIONS: usize = 32;
const SESSION_IDLE_LIMIT: Duration = Duration::from_secs(30 * 60);
/// Generous enough for a local approval prompt (trusted chrome times out
/// after 5 minutes) plus handler work.
const CALL_TIMEOUT: Duration = Duration::from_secs(6 * 60);
const WORKER_THREADS: usize = 4;
const RECV_POLL: Duration = Duration::from_millis(200);
/// How long a worker waits for a request body before giving up on it.
/// `tiny_http` 0.12 exposes no per-connection socket, so a read deadline
/// cannot be set on the stream; the read is instead handed to a helper thread
/// the worker can walk away from.
const BODY_READ_TIMEOUT: Duration = Duration::from_secs(15);
/// Ceiling on reads abandoned that way at any one time. A worker never blocks
/// on a slow client, but the helper thread it left behind cannot be killed, so
/// past this many outstanding stalled reads the gateway sheds load instead of
/// spawning more.
const MAX_STALLED_READS: usize = 16;

// -- Authentication seam -------------------------------------------------------

/// The gateway's authentication seam. Implementations map request
/// credentials to exactly one export profile — there is no anonymous
/// outcome, only `None` (reject).
pub trait GatewayAuth: Send + Sync {
    fn authenticate(&self, authorization: Option<&str>) -> Option<String>;
}

/// Bearer tokens minted per profile (`rotate_mcp_export_token`), read from
/// owner-scoped secret storage and compared in constant time.
pub struct BearerProfileAuth {
    config: Arc<Mutex<HostConfigService>>,
}

impl BearerProfileAuth {
    pub fn new(config: Arc<Mutex<HostConfigService>>) -> Self {
        Self { config }
    }
}

impl GatewayAuth for BearerProfileAuth {
    fn authenticate(&self, authorization: Option<&str>) -> Option<String> {
        let presented = authorization?.strip_prefix("Bearer ")?.trim();
        if presented.is_empty() {
            return None;
        }
        let config = self.config.lock().ok()?;
        for view in config.list_mcp_export_profiles() {
            if !view.profile.enabled {
                continue;
            }
            let Some(expected) = config.mcp_export_token(&view.id) else {
                continue; // profile without a token is unreachable, by design
            };
            if constant_time_eq(expected.as_bytes(), presented.as_bytes()) {
                return Some(view.id);
            }
        }
        None
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

// -- Audit log -------------------------------------------------------------------

/// How many recent audit events are kept in memory for the Settings activity
/// view. Small on purpose: it is a "what just happened" window, not history.
const RECENT_ACTIVITY_CAP: usize = 50;

/// Structured JSONL audit trail; one line per security-relevant event. Also
/// keeps the most recent events in memory so the Settings UI can show what a
/// remote client actually did, closing the "remote use is unobservable" gap.
pub struct AuditLog {
    path: Option<PathBuf>,
    file: Mutex<Option<std::fs::File>>,
    recent: Mutex<VecDeque<Value>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedAuditRecord {
    format_version: u32,
    at: String,
    event: Map<String, Value>,
}

impl AuditLog {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            file: Mutex::new(None),
            recent: Mutex::new(VecDeque::new()),
        }
    }

    /// The most recent audit events, newest last. Cloned so callers never
    /// hold the lock.
    pub fn recent(&self) -> Vec<Value> {
        self.recent
            .lock()
            .map(|entries| entries.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn validate_persisted(path: &Path) -> Result<(), String> {
        if !path.exists() {
            return Ok(());
        }
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("read MCP gateway audit log failed: {error}"))?;
        for (index, line) in text.lines().enumerate() {
            let record: PersistedAuditRecord = serde_json::from_str(line).map_err(|error| {
                format!(
                    "parse MCP gateway audit event at line {} failed: {error}",
                    index + 1
                )
            })?;
            if record.format_version != 1 {
                return Err(format!(
                    "unsupported MCP gateway audit event version {} at line {}",
                    record.format_version,
                    index + 1
                ));
            }
            chrono::DateTime::parse_from_rfc3339(&record.at).map_err(|error| {
                format!(
                    "MCP gateway audit timestamp at line {} is invalid: {error}",
                    index + 1
                )
            })?;
            if record.event.get("event").and_then(Value::as_str).is_none() {
                return Err(format!(
                    "MCP gateway audit event at line {} has no event name",
                    index + 1
                ));
            }
        }
        Ok(())
    }

    fn record(&self, mut entry: Value) -> Result<(), String> {
        let event = entry
            .as_object()
            .cloned()
            .filter(|event| event.get("event").and_then(Value::as_str).is_some())
            .ok_or_else(|| "MCP gateway audit entry must contain an event name".to_string())?;
        let at = chrono::Utc::now().to_rfc3339();
        entry
            .as_object_mut()
            .expect("validated audit entry is an object")
            .insert("at".to_string(), Value::String(at.clone()));
        // Keep the in-memory window current even if file persistence fails
        // below — the UI's value is showing that something happened at all.
        if let Ok(mut recent) = self.recent.lock() {
            recent.push_back(entry.clone());
            while recent.len() > RECENT_ACTIVITY_CAP {
                recent.pop_front();
            }
        }
        let line = serde_json::to_string(&PersistedAuditRecord {
            format_version: 1,
            at,
            event,
        })
        .map_err(|error| format!("serialize MCP gateway audit event failed: {error}"))?;
        let Some(path) = &self.path else {
            eprintln!("mcp-gateway audit: {line}");
            return Ok(());
        };
        let mut slot = self
            .file
            .lock()
            .map_err(|_| "MCP gateway audit lock poisoned".to_string())?;
        if slot.is_none() {
            *slot = Some(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|error| format!("open MCP gateway audit log failed: {error}"))?,
            );
        }
        if let Some(file) = slot.as_mut() {
            use std::io::Write;
            writeln!(file, "{line}")
                .and_then(|_| file.sync_data())
                .map_err(|error| format!("persist MCP gateway audit event failed: {error}"))?;
        }
        Ok(())
    }
}

// -- Gateway state ---------------------------------------------------------------

struct SessionState {
    profile_id: String,
    last_seen: Instant,
}

pub struct GatewayContext {
    pub kernel: Arc<Mutex<Kernel>>,
    pub config: Arc<Mutex<HostConfigService>>,
    pub auth: Arc<dyn GatewayAuth>,
    pub audit: Arc<AuditLog>,
    pub cancel_pending_approvals: Arc<dyn Fn() + Send + Sync>,
}

struct GatewayState {
    context: GatewayContext,
    sessions: Mutex<HashMap<String, SessionState>>,
    /// Sliding one-minute window of `tools/call` instants per profile.
    call_windows: Mutex<HashMap<String, Vec<Instant>>>,
    allowed_origins: Vec<String>,
    /// Body reads a worker gave up on and left running. Bounded by
    /// [`MAX_STALLED_READS`].
    stalled_reads: Arc<AtomicUsize>,
}

pub struct RunningGateway {
    local_addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    workers: Vec<std::thread::JoinHandle<()>>,
    cancel_pending_approvals: Arc<dyn Fn() + Send + Sync>,
}

impl RunningGateway {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn stop(mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        (self.cancel_pending_approvals)();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

/// Bind the listener and start serving `/mcp`. The bind address must be
/// loopback (config validation enforces the default; this enforces direct
/// callers too).
pub fn start_gateway(
    bind_address: &str,
    context: GatewayContext,
) -> Result<RunningGateway, String> {
    let requested: SocketAddr = bind_address
        .parse()
        .map_err(|_| format!("invalid MCP gateway bind address: {bind_address}"))?;
    if !requested.ip().is_loopback() {
        return Err(format!(
            "MCP gateway refuses to bind non-loopback address {bind_address}; use a local \
             tunnel for public exposure"
        ));
    }
    let allowed_origins = context
        .config
        .lock()
        .map_err(|_| "config lock poisoned".to_string())?
        .mcp_gateway_settings()
        .allowed_origins;
    let cancel_pending_approvals = context.cancel_pending_approvals.clone();
    let server = tiny_http::Server::http(requested)
        .map_err(|error| format!("MCP gateway failed to bind {bind_address}: {error}"))?;
    let local_addr = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| "MCP gateway unexpectedly received a non-IP listen address".to_string())?;
    context.audit.record(json!({
        "event": "gateway-started",
        "bind": local_addr.to_string(),
    }))?;

    let server = Arc::new(server);
    let state = Arc::new(GatewayState {
        context,
        sessions: Mutex::new(HashMap::new()),
        call_windows: Mutex::new(HashMap::new()),
        allowed_origins,
        stalled_reads: Arc::new(AtomicUsize::new(0)),
    });
    let shutdown = Arc::new(AtomicBool::new(false));
    let workers = (0..WORKER_THREADS)
        .map(|_| {
            let server = server.clone();
            let state = state.clone();
            let shutdown = shutdown.clone();
            std::thread::spawn(move || {
                while !shutdown.load(Ordering::Relaxed) {
                    match server.recv_timeout(RECV_POLL) {
                        Ok(Some(request)) => handle_request(&state, request),
                        Ok(None) => continue,
                        Err(_) => break,
                    }
                }
            })
        })
        .collect();

    Ok(RunningGateway {
        local_addr,
        shutdown,
        workers,
        cancel_pending_approvals,
    })
}

// -- HTTP layer ------------------------------------------------------------------

enum BodyRead {
    Complete {
        body: String,
        request: tiny_http::Request,
    },
    TooLarge(tiny_http::Request),
    /// The read is still running on a helper thread; ownership of the request
    /// went with it and it answers (or drops) on its own.
    Abandoned,
    /// Too many reads are already stalled to start another.
    Overloaded(tiny_http::Request),
}

/// Read a request body without letting one slow client pin a worker thread.
///
/// A client that trickles its body a byte at a time would otherwise hold a
/// worker for as long as it likes; with only [`WORKER_THREADS`] of them, a few
/// such requests stop every export profile from being served. The read
/// therefore happens on a helper thread that the worker stops waiting on after
/// [`BODY_READ_TIMEOUT`]. `respond` does not drain the body, so abandoning a
/// request here is safe: the helper owns it and answers when its read ends.
fn read_body_within(state: &Arc<GatewayState>, request: tiny_http::Request) -> BodyRead {
    if state.stalled_reads.load(Ordering::Relaxed) >= MAX_STALLED_READS {
        return BodyRead::Overloaded(request);
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let stalled = state.stalled_reads.clone();
    stalled.fetch_add(1, Ordering::Relaxed);
    std::thread::spawn(move || {
        let mut request = request;
        let mut body = String::new();
        let outcome = request
            .as_reader()
            .take(MAX_BODY_BYTES as u64 + 1)
            .read_to_string(&mut body);
        let read = match outcome {
            Ok(_) if body.len() <= MAX_BODY_BYTES => BodyRead::Complete { body, request },
            _ => BodyRead::TooLarge(request),
        };
        // If the worker already moved on, the request rides back here and is
        // dropped, which answers the client.
        let _ = sender.send(read);
        stalled.fetch_sub(1, Ordering::Relaxed);
    });
    receiver
        .recv_timeout(BODY_READ_TIMEOUT)
        .unwrap_or(BodyRead::Abandoned)
}

fn header_value(request: &tiny_http::Request, name: &str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|header| header.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str().to_string())
}

fn respond_status(request: tiny_http::Request, status: u16) {
    let _ = request.respond(tiny_http::Response::empty(status));
}

fn respond_unauthorized(request: tiny_http::Request) {
    let response = tiny_http::Response::empty(401).with_header(
        tiny_http::Header::from_bytes(&b"WWW-Authenticate"[..], &b"Bearer"[..])
            .expect("static header"),
    );
    let _ = request.respond(response);
}

/// Non-browser clients send no Origin; browsers must match localhost or the
/// configured allowlist (DNS-rebinding defense per the MCP transport spec).
fn origin_allowed(origin: Option<&str>, allowed: &[String]) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    if allowed.iter().any(|entry| entry == origin) {
        return true;
    }
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    if rest.contains(['/', '?', '#', '@']) {
        return false;
    }
    if let Some(port) = rest.strip_prefix("localhost:") {
        return !port.is_empty() && port.chars().all(|character| character.is_ascii_digit());
    }
    if let Some(port) = rest.strip_prefix("127.0.0.1:") {
        return !port.is_empty() && port.chars().all(|character| character.is_ascii_digit());
    }
    if let Some(port) = rest.strip_prefix("[::1]:") {
        return !port.is_empty() && port.chars().all(|character| character.is_ascii_digit());
    }
    matches!(rest, "localhost" | "127.0.0.1" | "[::1]")
}

fn handle_request(state: &Arc<GatewayState>, request: tiny_http::Request) {
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or(&url);
    let remote = request
        .remote_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|| "unknown".into());

    // Staged, deliberately disabled: OAuth 2.1 protected-resource metadata
    // for ChatGPT-style clients. Until audience validation is implemented
    // correctly this endpoint answers 404 and the config flag cannot be
    // enabled (validation rejects it).
    if path == "/.well-known/oauth-protected-resource" {
        let _ = state.context.audit.record(json!({
            "event": "oauth-metadata-requested",
            "outcome": "staged-disabled",
            "remote": remote,
        }));
        respond_status(request, 404);
        return;
    }
    if path != "/mcp" {
        respond_status(request, 404);
        return;
    }

    let origin = header_value(&request, "Origin");
    if !origin_allowed(origin.as_deref(), &state.allowed_origins) {
        let _ = state.context.audit.record(json!({
            "event": "origin-rejected",
            "origin": origin,
            "remote": remote,
        }));
        respond_status(request, 403);
        return;
    }

    // Every request authenticates; there is no anonymous surface at all.
    let authorization = header_value(&request, "Authorization");
    let Some(profile_id) = state.context.auth.authenticate(authorization.as_deref()) else {
        let _ = state.context.audit.record(json!({
            "event": "auth-failed",
            "remote": remote,
        }));
        respond_unauthorized(request);
        return;
    };

    let method = request.method().clone();
    if method == tiny_http::Method::Delete {
        let removed = header_value(&request, "Mcp-Session-Id").and_then(|session_id| {
            let mut sessions = state.sessions.lock().ok()?;
            let owned = sessions
                .get(&session_id)
                .is_some_and(|session| session.profile_id == profile_id);
            owned.then(|| sessions.remove(&session_id)).flatten()
        });
        let _ = state.context.audit.record(json!({
            "event": "session-deleted",
            "profile": profile_id,
            "found": removed.is_some(),
            "remote": remote,
        }));
        respond_status(request, if removed.is_some() { 200 } else { 404 });
        return;
    }
    if method != tiny_http::Method::Post {
        respond_status(request, 405);
        return;
    }

    // Body cap before and during the read.
    if request
        .body_length()
        .is_some_and(|length| length > MAX_BODY_BYTES)
    {
        respond_status(request, 413);
        return;
    }
    let (body, request) = match read_body_within(state, request) {
        BodyRead::Complete { body, request } => (body, request),
        BodyRead::TooLarge(request) => {
            respond_status(request, 413);
            return;
        }
        // The worker walked away from a stalled read, or refused to start one.
        // Either way this request is finished as far as this thread cares.
        BodyRead::Abandoned => {
            let _ = state.context.audit.record(json!({
                "event": "request-body-timeout",
                "profile": profile_id,
                "remote": remote,
            }));
            return;
        }
        BodyRead::Overloaded(request) => {
            let _ = state.context.audit.record(json!({
                "event": "request-shed",
                "profile": profile_id,
                "remote": remote,
            }));
            respond_status(request, 503);
            return;
        }
    };
    let Ok(message) = serde_json::from_str::<Value>(&body) else {
        respond_mcp(
            request,
            400,
            json_rpc_error(Value::Null, -32700, "parse error"),
            false,
            None,
        );
        return;
    };
    let Some(object) = message.as_object() else {
        respond_mcp(
            request,
            400,
            json_rpc_error(Value::Null, -32600, "invalid request"),
            false,
            None,
        );
        return;
    };
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    let notification = !object.contains_key("id");
    let Some(rpc_method) = object.get("method").and_then(Value::as_str) else {
        respond_mcp(
            request,
            400,
            json_rpc_error(id, -32600, "method is required"),
            false,
            None,
        );
        return;
    };
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    let accepts_sse = header_value(&request, "Accept").is_some_and(|accept| {
        accept
            .split(',')
            .any(|part| part.trim().starts_with("text/event-stream"))
    });
    let current_session = match validate_session(state, &request, &profile_id, rpc_method) {
        Ok(session) => session,
        Err(error) => {
            respond_mcp(
                request,
                404,
                json_rpc_error(id, -32001, &error),
                false,
                None,
            );
            return;
        }
    };
    let mut response = match rpc_method {
        "initialize" => initialize_response(state, &profile_id, params),
        "ping" => Ok(json!({})),
        "tools/list" => tools_list_response(state, &profile_id),
        "tools/call" => tools_call_response(state, &profile_id, params, &remote),
        method if method.starts_with("notifications/") => Ok(Value::Null),
        _ => Err((-32601, "method not found".to_string())),
    };
    if notification {
        respond_status(request, 202);
        return;
    }
    let created_session = response
        .as_ref()
        .ok()
        .and_then(|result| result.get("_session_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Ok(result) = &mut response {
        result
            .as_object_mut()
            .map(|object| object.remove("_session_id"));
    }
    let body = match response {
        Ok(result) => json_rpc_result(id, result),
        Err((code, message)) => json_rpc_error(id, code, &message),
    };
    respond_mcp(
        request,
        200,
        body,
        accepts_sse,
        created_session.as_deref().or(current_session.as_deref()),
    );
}

fn respond_mcp(
    request: tiny_http::Request,
    status: u16,
    body: Value,
    as_sse: bool,
    session_id: Option<&str>,
) {
    let mut response = if as_sse {
        tiny_http::Response::from_string(format!("event: message\ndata: {body}\n\n"))
            .with_status_code(status)
            .with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream"[..])
                    .expect("static header"),
            )
    } else {
        tiny_http::Response::from_string(body.to_string())
            .with_status_code(status)
            .with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                    .expect("static header"),
            )
    };
    if let Some(session_id) = session_id {
        response = response.with_header(
            tiny_http::Header::from_bytes(&b"Mcp-Session-Id"[..], session_id.as_bytes())
                .expect("host-generated session id"),
        );
    }
    let _ = request.respond(response);
}

fn json_rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn json_rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn validate_session(
    state: &GatewayState,
    request: &tiny_http::Request,
    profile_id: &str,
    method: &str,
) -> Result<Option<String>, String> {
    if method == "initialize" {
        return Ok(None);
    }
    let session_id = header_value(request, "Mcp-Session-Id")
        .ok_or_else(|| "MCP session is required".to_string())?;
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "session lock poisoned".to_string())?;
    sessions.retain(|_, session| session.last_seen.elapsed() <= SESSION_IDLE_LIMIT);
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| "MCP session is unknown or expired".to_string())?;
    if session.profile_id != profile_id {
        return Err("MCP session does not belong to this authenticated profile".into());
    }
    session.last_seen = Instant::now();
    Ok(Some(session_id))
}

fn initialize_response(
    state: &GatewayState,
    profile_id: &str,
    params: Value,
) -> Result<Value, (i64, String)> {
    let version = params.get("protocolVersion").and_then(Value::as_str);
    if version != Some(LATEST_PROTOCOL_VERSION) {
        return Err((
            -32602,
            format!("unsupported MCP protocol version; expected '{LATEST_PROTOCOL_VERSION}'"),
        ));
    }
    let session_id = uuid::Uuid::new_v4().to_string();
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| (-32000, "session lock poisoned".into()))?;
    sessions.retain(|_, session| session.last_seen.elapsed() <= SESSION_IDLE_LIMIT);
    if sessions.len() >= MAX_SESSIONS {
        return Err((-32002, "MCP session limit reached".into()));
    }
    sessions.insert(
        session_id.clone(),
        SessionState {
            profile_id: profile_id.to_string(),
            last_seen: Instant::now(),
        },
    );
    Ok(
        json!({"_session_id": session_id, "protocolVersion": LATEST_PROTOCOL_VERSION, "serverInfo": {"name": "ai-app-host", "version": env!("CARGO_PKG_VERSION")}, "capabilities": {"tools": {}}}),
    )
}

fn profile_for(state: &GatewayState, profile_id: &str) -> Result<McpExportProfile, (i64, String)> {
    let config = state
        .context
        .config
        .lock()
        .map_err(|_| (-32000, "config lock poisoned".into()))?;
    let profile = config
        .mcp_export_profile(profile_id)
        .ok_or_else(|| (-32001, "export profile is unavailable".into()))?;
    if !profile.enabled {
        return Err((-32001, "export profile is disabled".into()));
    }
    Ok(profile)
}

fn live_capabilities(
    state: &GatewayState,
    profile_id: &str,
    profile: &McpExportProfile,
) -> Result<Vec<app_host_kernel::kernel::CapabilityUseView>, (i64, String)> {
    let mut capabilities = state
        .context
        .kernel
        .lock()
        .map_err(|_| (-32000, "kernel lock poisoned".into()))?
        .available_capabilities_for(&principal_app_id(profile_id))
        .map_err(|error| (-32000, error.to_string()))?;
    capabilities.retain(|capability| {
        profile.capabilities.iter().any(|exported| {
            exported.provider == capability.provider_app_id.as_str()
                && exported.capability == capability.capability.as_str()
        })
    });
    Ok(capabilities)
}

fn tools_list_response(state: &GatewayState, profile_id: &str) -> Result<Value, (i64, String)> {
    let profile = profile_for(state, profile_id)?;
    let kernel = state
        .context
        .kernel
        .lock()
        .map_err(|_| (-32000, "kernel lock poisoned".into()))?;
    let mut views = kernel
        .available_capabilities_for(&principal_app_id(profile_id))
        .map_err(|error| (-32000, error.to_string()))?;
    views.retain(|capability| {
        profile.capabilities.iter().any(|exported| {
            exported.provider == capability.provider_app_id.as_str()
                && exported.capability == capability.capability.as_str()
        })
    });
    let tools = views.into_iter().map(|view| {
        let reference = CapabilityRef { provider: view.provider_app_id, capability: view.capability };
        let effect = kernel.capability_declaration(&reference).map(|declaration| declaration.effect).unwrap_or(CapabilityEffect::Unspecified);
        json!({"name": mcp_tool_name(&reference), "description": view.description, "inputSchema": view.input_schema, "annotations": {"readOnlyHint": matches!(effect, CapabilityEffect::ReadOnly)}})
    }).collect::<Vec<_>>();
    Ok(json!({"tools": tools}))
}

fn tools_call_response(
    state: &GatewayState,
    profile_id: &str,
    params: Value,
    remote: &str,
) -> Result<Value, (i64, String)> {
    let profile = profile_for(state, profile_id)?;
    enforce_rate_limit(state, profile_id, profile.rate_limit_per_minute)?;
    state
        .context
        .audit
        .record(json!({"event": "tool-call-started", "profile": profile_id, "remote": remote}))
        .map_err(|error| (-32000, error))?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| (-32602, "tool name is required".into()))?;
    let input: JsonObject = serde_json::from_value(
        params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
    .map_err(|_| (-32602, "tool arguments must be an object".into()))?;
    let capability = live_capabilities(state, profile_id, &profile)?
        .into_iter()
        .map(|view| CapabilityRef {
            provider: view.provider_app_id,
            capability: view.capability,
        })
        .find(|capability| mcp_tool_name(capability) == name)
        .ok_or_else(|| (-32602, "unknown tool".into()))?;
    let (run_id, prepared) = {
        let mut kernel = state
            .context
            .kernel
            .lock()
            .map_err(|_| (-32000, "kernel lock poisoned".into()))?;
        let run_id = kernel
            .start_run(
                Initiator::App {
                    app_id: principal_app_id(profile_id),
                    reason: "authenticated remote MCP request".into(),
                },
                &format!("Remote MCP call: {name}"),
            )
            .map_err(|error| (-32000, error.to_string()))?;
        let prepared = match kernel.prepare_invocation_with_timeout(
            &run_id,
            &capability,
            app_host_kernel::invocation::InvocationRequest {
                input,
                data_scope: app_host_kernel::primitives::grant::DataScope::None,
            },
            CALL_TIMEOUT,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                let _ = kernel.end_run(&run_id, RunTerminalState::Failed);
                return Err((-32000, error.to_string()));
            }
        };
        (run_id, prepared)
    };
    let mut tool_executed = false;
    let result = match prepared {
        PrepareInvocation::Refused(result) => result,
        PrepareInvocation::Prepared(prepared) => {
            let approval = prepared.await_approval();
            let authorized = match state
                .context
                .kernel
                .lock()
                .map_err(|_| (-32000, "kernel lock poisoned".into()))?
                .authorize_invocation(approval)
            {
                Ok(authorized) => authorized,
                Err(error) => {
                    if let Ok(mut kernel) = state.context.kernel.lock() {
                        let _ = kernel.end_run(&run_id, RunTerminalState::Failed);
                    }
                    return Err((-32000, error.to_string()));
                }
            };
            match authorized {
                app_host_kernel::AuthorizeInvocation::Refused(result) => result,
                app_host_kernel::AuthorizeInvocation::Authorized(authorized) => {
                    tool_executed = true;
                    let executed = authorized.execute();
                    // From here on the external action has already happened.
                    // Failures below are recording failures, not execution
                    // failures, and must say so — a plain error would invite
                    // the remote caller to retry a non-idempotent action.
                    match state
                        .context
                        .kernel
                        .lock()
                        .map_err(|_| recording_failure("kernel lock poisoned"))?
                        .finalize_invocation(executed)
                    {
                        Ok(result) => result,
                        Err(error) => {
                            if let Ok(mut kernel) = state.context.kernel.lock() {
                                let _ = kernel.end_run(&run_id, RunTerminalState::Failed);
                            }
                            return Err(recording_failure(&error.to_string()));
                        }
                    }
                }
            }
        }
    };
    let terminal = match &result {
        InvocationResult::Completed { .. } => RunTerminalState::Completed,
        InvocationResult::Failed { .. } => RunTerminalState::Failed,
        InvocationResult::Refused { .. } => RunTerminalState::Cancelled,
    };
    // Auditing stays fail-closed, but once the tool ran the error must carry
    // the executed-but-unrecorded signal instead of a plain failure.
    let bookkeeping_failure = |message: String| {
        if tool_executed {
            recording_failure(&message)
        } else {
            (-32000, message)
        }
    };
    state
        .context
        .kernel
        .lock()
        .map_err(|_| bookkeeping_failure("kernel lock poisoned".into()))?
        .end_run(&run_id, terminal)
        .map_err(|error| bookkeeping_failure(error.to_string()))?;
    state
        .context
        .audit
        .record(json!({"event": "tool-called", "profile": profile_id, "tool": name, "run_id": run_id, "remote": remote, "outcome": invocation_outcome(&result)}))
        .map_err(bookkeeping_failure)?;
    Ok(mcp_tool_result(result, &profile))
}

/// Error for failures that occur after provider code has already run: the
/// external action may have completed even though the host could not record
/// it. The distinct code lets remote clients tell "not executed" (safe to
/// retry) from "executed but not recorded" (verify effects first).
const POST_EXECUTION_ERROR_CODE: i64 = -32004;

fn recording_failure(detail: &str) -> (i64, String) {
    (
        POST_EXECUTION_ERROR_CODE,
        format!(
            "tool call already executed, but the host failed to record the result: {detail}; \
             verify the external effect before retrying"
        ),
    )
}

fn enforce_rate_limit(
    state: &GatewayState,
    profile_id: &str,
    limit: u32,
) -> Result<(), (i64, String)> {
    let mut windows = state
        .call_windows
        .lock()
        .map_err(|_| (-32000, "rate-limit lock poisoned".into()))?;
    let window = windows.entry(profile_id.to_string()).or_default();
    window.retain(|instant| instant.elapsed() < Duration::from_secs(60));
    if window.len() >= limit as usize {
        return Err((-32003, "remote call rate limit exceeded".into()));
    }
    window.push(Instant::now());
    Ok(())
}

fn invocation_outcome(result: &InvocationResult) -> &'static str {
    match result {
        InvocationResult::Completed { .. } => "completed",
        InvocationResult::Failed { .. } => "failed",
        InvocationResult::Refused { .. } => "refused",
    }
}

fn mcp_tool_result(result: InvocationResult, profile: &McpExportProfile) -> Value {
    match result {
        InvocationResult::Completed { result, artifacts } => {
            let mut content = if profile.expose_results {
                vec![json!({"type": "text", "text": result.to_string()})]
            } else {
                vec![json!({"type": "text", "text": "Capability completed."})]
            };
            if profile.expose_artifacts {
                content.extend(artifacts.into_iter().map(|artifact| json!({"type": "resource", "resource": {"uri": format!("artifact:{}", artifact.artifact_id), "name": artifact.artifact_type}})));
            }
            json!({"content": content})
        }
        InvocationResult::Failed { error } => {
            json!({"content": [{"type": "text", "text": error}], "isError": true})
        }
        InvocationResult::Refused { reason } => {
            json!({"content": [{"type": "text", "text": format!("Invocation refused: {reason:?}")}], "isError": true})
        }
    }
}

/// Gateway names retain a readable prefix but add a digest of the exact
/// capability reference, preventing sanitized-name collisions.
fn mcp_tool_name(capability: &CapabilityRef) -> String {
    let raw = capability.qualified_name();
    let readable = cap_ref_to_tool_name(capability);
    let suffix = format!("{:x}", Sha256::digest(raw.as_bytes()))[..12].to_string();
    let prefix = readable.chars().take(51).collect::<String>();
    format!("{prefix}_{suffix}")
}

#[cfg(test)]
mod tests;
