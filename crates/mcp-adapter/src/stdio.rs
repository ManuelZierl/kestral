//! MCP stdio transport: newline-delimited JSON-RPC 2.0 over a child
//! process's stdin/stdout.
//!
//! Deliberately synchronous — the kernel's handler contract is synchronous
//! and the host runs handlers on blocking workers. One request in flight at
//! a time, no async runtime, no SDK dependency. A reader thread owns stdout
//! and forwards parsed messages over a channel so every wait has a real
//! timeout — a hung server fails the request, never the host.

#[cfg(windows)]
use std::ffi::OsStr;
use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
#[cfg(windows)]
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::errors::McpError;
use crate::transport::{McpTransport, RequestOptions};

/// How long a cancel probe can go unchecked while waiting on the channel.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);
const WRITE_QUEUE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(10);

/// Depth of the reader thread's response queue.
///
/// Only one request is ever in flight, so a healthy server keeps this near
/// empty. A server that answers late — after its request already timed out —
/// leaves messages nothing will ever consume, so the queue is bounded instead
/// of growing for the life of the connection.
const INCOMING_QUEUE_LIMIT: usize = 4;
const OUTGOING_QUEUE_LIMIT: usize = 8;
#[cfg(windows)]
const STDERR_DIAGNOSTIC_BYTES: usize = 16 * 1024;

enum IncomingMessage {
    Message(Value),
    ProtocolError(String),
    TransportError(String),
}

struct WriteRequest {
    bytes: Vec<u8>,
    completion: mpsc::SyncSender<Result<(), String>>,
}

/// Launch policy for a native backend process.
///
/// `Unsandboxed` provides process-tree cleanup and environment minimization,
/// but makes no claim of filesystem or network sandboxing. `Sandboxed` is a
/// request for deny-by-default filesystem and network policy; this crate probes
/// that request and fails closed because the current dependencies do not
/// enforce it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeBackendLaunchPolicy {
    Unsandboxed,
    Sandboxed(NativeBackendSandboxRequest),
}

/// The filesystem/network policy a native backend wants to run under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeBackendSandboxRequest {
    pub filesystem: NativeBackendFilesystemPolicy,
    pub network: NativeBackendNetworkPolicy,
}

impl NativeBackendSandboxRequest {
    /// Read-only payload plus writable app data, with deny-by-default network.
    pub const fn deny_by_default_filesystem_and_network() -> Self {
        Self {
            filesystem: NativeBackendFilesystemPolicy::ReadOnlyPayloadAndReadWriteAppData,
            network: NativeBackendNetworkPolicy::DenyByDefault,
        }
    }

    const fn filesystem_label(self) -> &'static str {
        match self.filesystem {
            NativeBackendFilesystemPolicy::ReadOnlyPayloadAndReadWriteAppData => {
                "read-only payload + read-write app data"
            }
        }
    }

    const fn network_label(self) -> &'static str {
        match self.network {
            NativeBackendNetworkPolicy::DenyByDefault => "deny-by-default network",
        }
    }
}

/// The intended filesystem split for a sandboxed native backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeBackendFilesystemPolicy {
    ReadOnlyPayloadAndReadWriteAppData,
}

/// The intended network policy for a sandboxed native backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeBackendNetworkPolicy {
    DenyByDefault,
}

/// Typed support probe for the current platform and dependency set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeBackendSandboxSupport {
    pub filesystem: SandboxFeatureSupport,
    pub network: SandboxFeatureSupport,
    pub process_cleanup: ProcessTreeCleanupSupport,
}

/// A single sandbox dimension: either genuinely supported, or unsupported
/// with an explicit reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxFeatureSupport {
    Supported,
    Unsupported { reason: &'static str },
}

impl SandboxFeatureSupport {
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::Supported)
    }

    pub const fn reason(self) -> Option<&'static str> {
        match self {
            Self::Supported => None,
            Self::Unsupported { reason } => Some(reason),
        }
    }
}

/// Process lifetime containment is the only native control we currently have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessTreeCleanupSupport {
    #[cfg(windows)]
    WindowsJobObjectKillOnClose,
    #[cfg(unix)]
    UnixProcessGroupKill,
    #[cfg(not(any(unix, windows)))]
    Unsupported,
}

/// Probe the current platform and dependencies for native backend sandboxing.
///
/// Honest result: the current transport can isolate process lifetime. On
/// Windows, sandboxed AppContainer launch is available through the dedicated
/// launcher below; other platforms remain unsupported for sandboxed native
/// backends.
pub fn native_backend_sandbox_support() -> NativeBackendSandboxSupport {
    NativeBackendSandboxSupport {
        filesystem: native_filesystem_sandbox_support(),
        network: native_network_sandbox_support(),
        process_cleanup: process_tree_cleanup_support(),
    }
}

#[cfg(windows)]
fn native_filesystem_sandbox_support() -> SandboxFeatureSupport {
    SandboxFeatureSupport::Unsupported {
        reason: "Windows AppContainer backend launch is not yet proven on this build",
    }
}

#[cfg(not(windows))]
fn native_filesystem_sandbox_support() -> SandboxFeatureSupport {
    SandboxFeatureSupport::Unsupported {
        reason:
            "no OS filesystem sandbox API is wired for read-only payload plus writable app data",
    }
}

#[cfg(windows)]
fn native_network_sandbox_support() -> SandboxFeatureSupport {
    SandboxFeatureSupport::Unsupported {
        reason: "Windows AppContainer backend launch is not yet proven on this build",
    }
}

#[cfg(not(windows))]
fn native_network_sandbox_support() -> SandboxFeatureSupport {
    SandboxFeatureSupport::Unsupported {
        reason: "no deny-by-default network sandbox API is wired for native backends",
    }
}

impl NativeBackendSandboxSupport {
    pub fn supports_sandboxed_execution(self) -> bool {
        self.filesystem.is_supported() && self.network.is_supported()
    }
}

/// A running MCP server reached over stdio. Dropping (or shutting down) the
/// transport kills the child process.
pub struct StdioTransport {
    process: Mutex<Option<BackendProcess>>,
    outgoing: Mutex<Option<mpsc::SyncSender<WriteRequest>>>,
    incoming: Mutex<mpsc::Receiver<IncomingMessage>>,
    next_request_id: AtomicU64,
    #[cfg(windows)]
    stderr_forwarder: Mutex<Option<JoinHandle<()>>>,
}

impl StdioTransport {
    /// Launch `command args…` and wire up the message pump. No MCP handshake
    /// happens here — that is [`crate::client::McpClient::connect`]'s job.
    pub fn spawn(command: &str, args: &[&str]) -> Result<Self, McpError> {
        Self::spawn_in(command, args, None)
    }

    /// Like [`Self::spawn`], but runs the child in `working_dir`. Packaged app
    /// backends need this so relative script paths and data files resolve
    /// against the app's payload directory rather than the host's cwd.
    pub fn spawn_in(
        command: &str,
        args: &[&str],
        working_dir: Option<&Path>,
    ) -> Result<Self, McpError> {
        let mut command_builder = build_command(command, args);
        if let Some(dir) = working_dir {
            command_builder.current_dir(dir);
        }
        Self::spawn_command(command, command_builder)
    }

    /// Launch a packaged backend with no inherited environment except the
    /// explicit allowlist. This limits accidental credential disclosure; it is
    /// an unsandboxed contract, not an OS filesystem or network sandbox.
    pub fn spawn_in_isolated(
        command: &str,
        args: &[&str],
        working_dir: &Path,
        environment: &[(&str, &str)],
    ) -> Result<Self, McpError> {
        Self::spawn_with_policy(
            command,
            args,
            Some(working_dir),
            environment,
            NativeBackendLaunchPolicy::Unsandboxed,
        )
    }

    /// Launch a native backend in a real Windows AppContainer. Unsupported
    /// platforms fail closed before starting a process.
    pub fn spawn_sandboxed(
        app_container_name: &str,
        command: &str,
        args: &[&str],
        payload_dir: &Path,
        data_dir: &Path,
        environment: &[(&str, &str)],
    ) -> Result<Self, McpError> {
        #[cfg(windows)]
        {
            Self::spawn_windows_sandboxed(WindowsSandboxedLaunch::new(
                app_container_name,
                command,
                args,
                payload_dir,
                data_dir,
                environment,
            )?)
        }
        #[cfg(not(windows))]
        {
            let _ = (
                app_container_name,
                command,
                args,
                payload_dir,
                data_dir,
                environment,
            );
            Err(McpError::Transport(
                "sandboxed native backend launch requires Windows AppContainer".into(),
            ))
        }
    }

    /// Request a native launch policy. `Sandboxed` is fail-closed: unsupported
    /// filesystem/network capabilities refuse before any child process starts.
    pub fn spawn_with_policy(
        command: &str,
        args: &[&str],
        working_dir: Option<&Path>,
        environment: &[(&str, &str)],
        policy: NativeBackendLaunchPolicy,
    ) -> Result<Self, McpError> {
        if let NativeBackendLaunchPolicy::Sandboxed(request) = policy {
            return Err(McpError::Transport(format!(
                "native sandboxed backend launch requires the AppContainer launcher; this launch path is unsandboxed (filesystem={}, network={})",
                request.filesystem_label(),
                request.network_label(),
            )));
        }

        let mut command_builder = build_command(command, args);
        if let Some(dir) = working_dir {
            command_builder.current_dir(dir);
        }
        command_builder.env_clear();
        for name in ["PATH", "SystemRoot", "WINDIR"] {
            if let Some(value) = std::env::var_os(name) {
                command_builder.env(name, value);
            }
        }
        command_builder.envs(environment.iter().copied());
        Self::spawn_command(command, command_builder)
    }

    fn spawn_command(command: &str, mut command_builder: Command) -> Result<Self, McpError> {
        let mut child = command_builder.spawn().map_err(|error| {
            McpError::Transport(format!("failed to spawn '{command}': {error}"))
        })?;
        let stdin: Box<dyn Write + Send> = Box::new(child.stdin.take().expect("stdin is piped"));
        let stdout: Box<dyn Read + Send> = Box::new(child.stdout.take().expect("stdout is piped"));
        let outgoing = spawn_writer_thread(stdin);

        // The reader thread owns stdout: responses go to the channel,
        // server-initiated pings are answered inline, notifications and
        // anything unexpected are dropped. It ends when the pipe closes.
        let (sender, receiver) = mpsc::sync_channel::<IncomingMessage>(INCOMING_QUEUE_LIMIT);
        spawn_reader_thread(stdout, outgoing.clone(), sender);

        #[cfg(windows)]
        let stderr_forwarder = Mutex::new(child.stderr.take().map(spawn_stderr_drain));

        Ok(Self {
            process: Mutex::new(Some(BackendProcess::Child(child))),
            outgoing: Mutex::new(Some(outgoing)),
            incoming: Mutex::new(receiver),
            next_request_id: AtomicU64::new(1),
            #[cfg(windows)]
            stderr_forwarder,
        })
    }

    #[cfg(windows)]
    fn spawn_windows_sandboxed(launch: WindowsSandboxedLaunch<'_>) -> Result<Self, McpError> {
        let spawned = launch.spawn()?;
        let WindowsSandboxedProcess {
            stdin,
            stdout,
            stderr,
            backend,
        } = spawned;

        let outgoing = spawn_writer_thread(Box::new(stdin.into_file()) as Box<dyn Write + Send>);
        let (sender, receiver) = mpsc::sync_channel::<IncomingMessage>(INCOMING_QUEUE_LIMIT);
        spawn_reader_thread(stdout.into_file(), outgoing.clone(), sender);

        let stderr_forwarder = Mutex::new(Some(spawn_stderr_drain(stderr.into_file())));

        Ok(Self {
            process: Mutex::new(Some(BackendProcess::Windows(backend))),
            outgoing: Mutex::new(Some(outgoing)),
            incoming: Mutex::new(receiver),
            next_request_id: AtomicU64::new(1),
            #[cfg(windows)]
            stderr_forwarder,
        })
    }

    fn send_until(
        &self,
        message: Value,
        method: &str,
        options: &RequestOptions,
        deadline: Instant,
    ) -> Result<(), McpError> {
        let mut bytes = serde_json::to_vec(&message)
            .map_err(|error| McpError::Protocol(format!("serialize JSON-RPC message: {error}")))?;
        if bytes.len() > crate::transport::MAX_MESSAGE_BYTES {
            return Err(McpError::Protocol(format!(
                "outbound JSON-RPC message exceeded {} bytes",
                crate::transport::MAX_MESSAGE_BYTES
            )));
        }
        bytes.push(b'\n');
        let outgoing = self
            .outgoing
            .lock()
            .map_err(|_| McpError::Transport("stdio transport poisoned".into()))?
            .clone()
            .ok_or_else(|| McpError::Transport("stdio transport closed".into()))?;
        let (completion, completed) = mpsc::sync_channel(1);
        let mut request = WriteRequest { bytes, completion };
        loop {
            if options.is_cancelled() {
                self.shutdown();
                return Err(McpError::Cancelled);
            }
            if Instant::now() >= deadline {
                self.shutdown();
                return Err(McpError::Timeout {
                    method: method.to_string(),
                    timeout: options.timeout,
                });
            }
            match outgoing.try_send(request) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(returned)) => {
                    request = returned;
                    std::thread::sleep(WRITE_QUEUE_POLL_INTERVAL);
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return Err(McpError::Transport("stdio transport closed".into()))
                }
            }
        }
        loop {
            if options.is_cancelled() {
                self.shutdown();
                return Err(McpError::Cancelled);
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                self.shutdown();
                return Err(McpError::Timeout {
                    method: method.to_string(),
                    timeout: options.timeout,
                });
            };
            match completed.recv_timeout(remaining.min(CANCEL_POLL_INTERVAL)) {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(error)) => {
                    return Err(McpError::Transport(format!(
                        "MCP server unreachable: {error}"
                    )))
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(McpError::Transport("stdio writer stopped".into()))
                }
            }
        }
    }
}

impl McpTransport for StdioTransport {
    fn request(
        &self,
        method: &str,
        params: Value,
        options: &RequestOptions,
    ) -> Result<Value, McpError> {
        if options.is_cancelled() {
            return Err(McpError::Cancelled);
        }
        // One request in flight at a time (this transport's contract). Hold
        // the receiver lock across the whole request — id allocation, send,
        // and response consumption — so two concurrent callers can never send
        // before either owns the receiver and then read/discard each other's
        // valid responses. `McpTransport: Send + Sync` makes concurrent calls
        // type-legal, so the serialization must be enforced here.
        let incoming = self
            .incoming
            .lock()
            .map_err(|_| McpError::Transport("stdio transport poisoned".into()))?;
        let deadline =
            Instant::now()
                .checked_add(options.timeout)
                .ok_or_else(|| McpError::Timeout {
                    method: method.to_string(),
                    timeout: options.timeout,
                })?;
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        self.send_until(
            json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "method": method,
                "params": params,
            }),
            method,
            options,
            deadline,
        )?;
        loop {
            if options.is_cancelled() {
                return Err(McpError::Cancelled);
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| McpError::Timeout {
                    method: method.to_string(),
                    timeout: options.timeout,
                })?;
            // Wait in short slices so a cancel probe is honored promptly
            // even while the server stays silent.
            let message = match incoming.recv_timeout(remaining.min(CANCEL_POLL_INTERVAL)) {
                Ok(IncomingMessage::Message(message)) => message,
                Ok(IncomingMessage::ProtocolError(error)) => {
                    drop(incoming);
                    self.shutdown();
                    return Err(McpError::Protocol(error));
                }
                Ok(IncomingMessage::TransportError(error)) => {
                    drop(incoming);
                    self.shutdown();
                    return Err(McpError::Transport(error));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(McpError::Transport(format!(
                        "MCP server stopped answering during '{method}'"
                    )))
                }
            };
            if message.get("id").and_then(Value::as_u64) != Some(request_id) {
                continue; // stale response to an abandoned request
            }
            return crate::protocol::extract_result_or_server_error(&message);
        }
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), McpError> {
        let mut message = json!({"jsonrpc": "2.0", "method": method});
        if !params.is_null() {
            message["params"] = params;
        }
        let options = RequestOptions::with_timeout(NOTIFY_TIMEOUT);
        let deadline =
            Instant::now()
                .checked_add(options.timeout)
                .ok_or_else(|| McpError::Timeout {
                    method: method.to_string(),
                    timeout: options.timeout,
                })?;
        self.send_until(message, method, &options, deadline)
    }

    fn shutdown(&self) {
        if let Ok(mut process) = self.process.lock() {
            if let Some(mut process) = process.take() {
                process.shutdown();
            }
        }
        if let Ok(mut outgoing) = self.outgoing.lock() {
            let _ = outgoing.take();
        }
        #[cfg(windows)]
        if let Ok(mut forwarder) = self.stderr_forwarder.lock() {
            // The process was terminated above, so the bounded drainer will
            // reach EOF. Detach rather than letting a blocked console sink
            // hold shutdown indefinitely.
            let _ = forwarder.take();
        }
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Spawns the reader thread that owns `stdout` for a stdio backend: responses
/// go to `sender`, server-initiated pings are answered inline over
/// `ping_stdin`, and notifications/anything unexpected are dropped. Ends
/// when the pipe closes. Shared by the plain-child and Windows AppContainer
/// launch paths, which differ only in the concrete stdout/stdin handle types.
fn spawn_reader_thread(
    stdout: impl Read + Send + 'static,
    ping_outgoing: mpsc::SyncSender<WriteRequest>,
    sender: mpsc::SyncSender<IncomingMessage>,
) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        // A read error here includes the oversized-message case, which ends
        // the transport rather than letting one server grow host memory
        // without bound.
        loop {
            let line = match crate::transport::read_line_capped(&mut reader) {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(error) => {
                    let terminal = if error.kind() == std::io::ErrorKind::InvalidData {
                        IncomingMessage::ProtocolError(error.to_string())
                    } else {
                        IncomingMessage::TransportError(format!(
                            "MCP stdout reader failed: {error}"
                        ))
                    };
                    let _ = sender.send(terminal);
                    break;
                }
            };
            let message = match serde_json::from_str::<Value>(&line) {
                Ok(message) => message,
                Err(error) => {
                    let _ = sender.send(IncomingMessage::ProtocolError(format!(
                        "MCP server emitted malformed JSON: {error}"
                    )));
                    break;
                }
            };
            if message.get("method").and_then(Value::as_str) == Some("ping") {
                if let Some(id) = message.get("id") {
                    let pong = json!({"jsonrpc": "2.0", "id": id, "result": {}});
                    if let Ok(mut bytes) = serde_json::to_vec(&pong) {
                        bytes.push(b'\n');
                        let (completion, _) = mpsc::sync_channel(1);
                        let _ = ping_outgoing.try_send(WriteRequest { bytes, completion });
                    }
                }
                continue;
            }
            if message.get("id").is_some()
                && message.get("method").is_none()
                && sender.send(IncomingMessage::Message(message)).is_err()
            {
                break; // transport gone
            }
        }
    });
}

fn spawn_writer_thread(mut writer: Box<dyn Write + Send>) -> mpsc::SyncSender<WriteRequest> {
    let (sender, receiver) = mpsc::sync_channel::<WriteRequest>(OUTGOING_QUEUE_LIMIT);
    std::thread::spawn(move || {
        while let Ok(request) = receiver.recv() {
            let result = writer
                .write_all(&request.bytes)
                .and_then(|()| writer.flush())
                .map_err(|error| error.to_string());
            let failed = result.is_err();
            let _ = request.completion.send(result);
            if failed {
                break;
            }
        }
    });
    sender
}

#[cfg(windows)]
fn spawn_stderr_drain(mut stderr: impl Read + Send + 'static) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut diagnostic = Vec::with_capacity(STDERR_DIAGNOSTIC_BYTES);
        let mut buffer = [0u8; 4096];
        loop {
            let read = match stderr.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            let remaining = STDERR_DIAGNOSTIC_BYTES.saturating_sub(diagnostic.len());
            diagnostic.extend_from_slice(&buffer[..read.min(remaining)]);
        }
        if !diagnostic.is_empty() {
            eprint!("{}", String::from_utf8_lossy(&diagnostic));
        }
    })
}

fn build_command(command: &str, args: &[&str]) -> Command {
    let mut built = Command::new(command);
    built
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Server logs stay visible in the host's console.
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        // No flashing console window for the child on Windows.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        built.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        built.process_group(0);
    }
    built
}

fn process_tree_cleanup_support() -> ProcessTreeCleanupSupport {
    #[cfg(windows)]
    {
        ProcessTreeCleanupSupport::WindowsJobObjectKillOnClose
    }
    #[cfg(unix)]
    {
        ProcessTreeCleanupSupport::UnixProcessGroupKill
    }
    #[cfg(not(any(unix, windows)))]
    {
        ProcessTreeCleanupSupport::Unsupported
    }
}

enum BackendProcess {
    Child(Child),
    #[cfg(windows)]
    Windows(WindowsBackendProcess),
}

impl BackendProcess {
    fn shutdown(&mut self) {
        match self {
            Self::Child(child) => {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                #[cfg(windows)]
                let _ = child.kill();
                #[cfg(not(any(unix, windows)))]
                let _ = child.kill();
                let _ = child.wait();
            }
            #[cfg(windows)]
            Self::Windows(process) => process.shutdown(),
        }
    }
}

#[cfg(windows)]
struct WindowsBackendProcess {
    process: WindowsHandle,
    job: WindowsHandle,
    thread: WindowsHandle,
    guards: WindowsSandboxGuards,
}

#[cfg(windows)]
unsafe impl Send for WindowsBackendProcess {}

#[cfg(windows)]
unsafe impl Sync for WindowsBackendProcess {}

#[cfg(windows)]
impl WindowsBackendProcess {
    fn shutdown(&mut self) {
        unsafe {
            let wait = windows_sys::Win32::System::Threading::WaitForSingleObject(
                self.process.as_raw(),
                250,
            );
            if wait == windows_sys::Win32::Foundation::WAIT_TIMEOUT {
                let _ = windows_sys::Win32::System::JobObjects::TerminateJobObject(
                    self.job.as_raw(),
                    1,
                );
                let _ = windows_sys::Win32::System::Threading::WaitForSingleObject(
                    self.process.as_raw(),
                    windows_sys::Win32::System::Threading::INFINITE,
                );
            }
        }
        self.thread.close();
        self.process.close();
        self.job.close();
        self.guards.acl_guards.clear();
        let _ = self.guards.app_container_sid.take();
    }
}

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

#[cfg(windows)]
struct WindowsHandle {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
unsafe impl Send for WindowsHandle {}

#[cfg(windows)]
unsafe impl Sync for WindowsHandle {}

#[cfg(windows)]
impl WindowsHandle {
    fn new(handle: windows_sys::Win32::Foundation::HANDLE) -> Self {
        Self { handle }
    }

    fn as_raw(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.handle
    }

    fn into_file(mut self) -> std::fs::File {
        use std::os::windows::io::FromRawHandle;
        let handle = self.handle;
        self.handle = std::ptr::null_mut();
        unsafe { std::fs::File::from_raw_handle(handle as _) }
    }

    fn close(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                let _ = windows_sys::Win32::Foundation::CloseHandle(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsHandle {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(windows)]
struct WindowsAppContainerSid {
    sid: windows_sys::Win32::Security::PSID,
    name: Vec<u16>,
    delete_profile_on_drop: bool,
}

#[cfg(windows)]
impl WindowsAppContainerSid {
    fn as_raw(&self) -> windows_sys::Win32::Security::PSID {
        self.sid
    }

    fn preserve_profile(&mut self) {
        self.delete_profile_on_drop = false;
    }
}

#[cfg(windows)]
impl Drop for WindowsAppContainerSid {
    fn drop(&mut self) {
        unsafe {
            if self.delete_profile_on_drop {
                let _ = windows_sys::Win32::Security::Isolation::DeleteAppContainerProfile(
                    self.name.as_ptr(),
                );
            }
            if !self.sid.is_null() {
                let _ = windows_sys::Win32::Security::FreeSid(self.sid);
            }
        }
    }
}

#[cfg(windows)]
struct WindowsDirectoryAclGuard {
    path: Vec<u16>,
    original_dacl: *mut windows_sys::Win32::Security::ACL,
    security_descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
}

#[cfg(windows)]
unsafe impl Send for WindowsDirectoryAclGuard {}

#[cfg(windows)]
unsafe impl Sync for WindowsDirectoryAclGuard {}

#[cfg(windows)]
impl Drop for WindowsDirectoryAclGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = set_file_dacl(self.path.as_ptr(), self.original_dacl);
            let _ = windows_sys::Win32::Foundation::LocalFree(self.security_descriptor as _);
        }
    }
}

#[cfg(windows)]
struct WindowsSandboxGuards {
    app_container_sid: Option<WindowsAppContainerSid>,
    acl_guards: Vec<WindowsDirectoryAclGuard>,
}

#[cfg(windows)]
struct WindowsSandboxedLaunch<'a> {
    app_container_name: &'a str,
    command: &'a str,
    args: &'a [&'a str],
    payload_dir: &'a Path,
    data_dir: &'a Path,
    environment: &'a [(&'a str, &'a str)],
}

#[cfg(windows)]
impl<'a> WindowsSandboxedLaunch<'a> {
    fn new(
        app_container_name: &'a str,
        command: &'a str,
        args: &'a [&'a str],
        payload_dir: &'a Path,
        data_dir: &'a Path,
        environment: &'a [(&'a str, &'a str)],
    ) -> Result<Self, McpError> {
        Ok(Self {
            app_container_name,
            command,
            args,
            payload_dir,
            data_dir,
            environment,
        })
    }

    fn spawn(self) -> Result<WindowsSandboxedProcess, McpError> {
        use std::mem::{size_of, zeroed};
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::Security::Isolation::{
            CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_READ_ATTRIBUTES,
            FILE_TRAVERSE,
        };
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        use windows_sys::Win32::System::Threading::{
            CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
            ResumeThread, UpdateProcThreadAttribute, CREATE_NO_WINDOW, CREATE_SUSPENDED,
            CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST,
            PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTUPINFOEXW,
        };

        let app_container_name_w = to_wide_str(self.app_container_name);
        let mut sid = ensure_app_container_sid(
            &app_container_name_w,
            CreateAppContainerProfile,
            DeriveAppContainerSidFromAppContainerName,
        )?;

        let mut acl_guards = apply_traverse_acls(
            &[self.payload_dir, self.data_dir],
            sid.as_raw(),
            FILE_TRAVERSE | FILE_READ_ATTRIBUTES,
        )?;
        acl_guards.extend(apply_acl_tree(
            self.payload_dir,
            sid.as_raw(),
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        )?);
        acl_guards.extend(apply_acl_tree(
            self.data_dir,
            sid.as_raw(),
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE,
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE,
        )?);
        let mut stdin_read = WindowsHandle::new(std::ptr::null_mut());
        let mut stdin_write = WindowsHandle::new(std::ptr::null_mut());
        let mut stdout_read = WindowsHandle::new(std::ptr::null_mut());
        let mut stdout_write = WindowsHandle::new(std::ptr::null_mut());
        let mut stderr_read = WindowsHandle::new(std::ptr::null_mut());
        let mut stderr_write = WindowsHandle::new(std::ptr::null_mut());
        create_pipe_pair(&mut stdin_read, &mut stdin_write)?;
        create_pipe_pair(&mut stdout_read, &mut stdout_write)?;
        create_pipe_pair(&mut stderr_read, &mut stderr_write)?;

        let mut attr_size = 0usize;
        unsafe {
            let _ = InitializeProcThreadAttributeList(std::ptr::null_mut(), 2, 0, &mut attr_size);
        }
        let mut attr_buf = vec![0u8; attr_size];
        let attr_list = attr_buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        unsafe {
            if InitializeProcThreadAttributeList(attr_list, 2, 0, &mut attr_size) == 0 {
                return Err(McpError::Transport(format!(
                    "initialize process attribute list failed: {}",
                    last_error()
                )));
            }
        }

        let handle_list = [
            stdin_read.as_raw(),
            stdout_write.as_raw(),
            stderr_write.as_raw(),
        ];
        let mut security_capabilities = windows_sys::Win32::Security::SECURITY_CAPABILITIES {
            AppContainerSid: sid.as_raw(),
            Capabilities: std::ptr::null_mut(),
            CapabilityCount: 0,
            Reserved: 0,
        };

        unsafe {
            if UpdateProcThreadAttribute(
                attr_list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handle_list.as_ptr() as *mut _,
                size_of::<[windows_sys::Win32::Foundation::HANDLE; 3]>(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) == 0
            {
                DeleteProcThreadAttributeList(attr_list);
                return Err(McpError::Transport(format!(
                    "install handle list attribute failed: {}",
                    last_error()
                )));
            }
            if UpdateProcThreadAttribute(
                attr_list,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                &mut security_capabilities as *mut _ as *mut _,
                size_of::<windows_sys::Win32::Security::SECURITY_CAPABILITIES>(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) == 0
            {
                DeleteProcThreadAttributeList(attr_list);
                return Err(McpError::Transport(format!(
                    "install AppContainer attribute failed: {}",
                    last_error()
                )));
            }
        }

        let mut environment_vars: Vec<(String, String)> = self
            .environment
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        for name in [
            "LOCALAPPDATA",
            "PATH",
            "SystemDrive",
            "SystemRoot",
            "TEMP",
            "TMP",
            "WINDIR",
        ] {
            if let Some(value) = std::env::var_os(name) {
                if let Some(value) = value.to_str() {
                    environment_vars.push((name.to_string(), value.to_string()));
                }
            }
        }
        environment_vars.sort_by(|left, right| {
            left.0
                .to_ascii_lowercase()
                .cmp(&right.0.to_ascii_lowercase())
        });
        let environment = to_environment_block_owned(&environment_vars);
        let mut command_line = to_wide_str(&build_command_line(self.command, self.args));
        let application_name = to_wide_str(self.command);
        let mut current_directory = to_wide_os(self.payload_dir.as_os_str());

        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        let mut desktop = to_wide_str("winsta0\\default");
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.lpAttributeList = attr_list;
        startup.StartupInfo.lpDesktop = desktop.as_mut_ptr();
        startup.StartupInfo.dwFlags = windows_sys::Win32::System::Threading::STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = stdin_read.as_raw();
        startup.StartupInfo.hStdOutput = stdout_write.as_raw();
        startup.StartupInfo.hStdError = stderr_write.as_raw();

        let mut process_info: PROCESS_INFORMATION = unsafe { zeroed() };
        unsafe {
            if CreateProcessW(
                application_name.as_ptr(),
                command_line.as_mut_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                EXTENDED_STARTUPINFO_PRESENT
                    | CREATE_SUSPENDED
                    | CREATE_NO_WINDOW
                    | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr() as *const _,
                current_directory.as_mut_ptr(),
                &startup.StartupInfo,
                &mut process_info,
            ) == 0
            {
                DeleteProcThreadAttributeList(attr_list);
                return Err(McpError::Transport(format!(
                    "create AppContainer backend failed: {}",
                    last_error()
                )));
            }

            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                let _ = windows_sys::Win32::System::Threading::TerminateProcess(
                    process_info.hProcess,
                    1,
                );
                let _ = CloseHandle(process_info.hThread);
                let _ = CloseHandle(process_info.hProcess);
                DeleteProcThreadAttributeList(attr_list);
                return Err(McpError::Transport(format!(
                    "create backend Job Object failed: {}",
                    last_error()
                )));
            }

            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
                || AssignProcessToJobObject(job, process_info.hProcess) == 0
            {
                let _ = windows_sys::Win32::System::JobObjects::TerminateJobObject(job, 1);
                let _ = CloseHandle(job);
                let _ = windows_sys::Win32::System::Threading::TerminateProcess(
                    process_info.hProcess,
                    1,
                );
                let _ = CloseHandle(process_info.hThread);
                let _ = CloseHandle(process_info.hProcess);
                DeleteProcThreadAttributeList(attr_list);
                return Err(McpError::Transport(format!(
                    "contain backend in Job Object failed: {}",
                    last_error()
                )));
            }

            if ResumeThread(process_info.hThread) == u32::MAX {
                let _ = windows_sys::Win32::System::JobObjects::TerminateJobObject(job, 1);
                let _ = CloseHandle(job);
                let _ = CloseHandle(process_info.hThread);
                let _ = CloseHandle(process_info.hProcess);
                DeleteProcThreadAttributeList(attr_list);
                return Err(McpError::Transport(format!(
                    "resume backend failed: {}",
                    last_error()
                )));
            }
            DeleteProcThreadAttributeList(attr_list);
            sid.preserve_profile();

            drop(stdin_read);
            drop(stdout_write);
            drop(stderr_write);

            Ok(WindowsSandboxedProcess {
                stdin: stdin_write,
                stdout: stdout_read,
                stderr: stderr_read,
                backend: WindowsBackendProcess {
                    process: WindowsHandle::new(process_info.hProcess),
                    job: WindowsHandle::new(job),
                    thread: WindowsHandle::new(process_info.hThread),
                    guards: WindowsSandboxGuards {
                        app_container_sid: Some(sid),
                        acl_guards,
                    },
                },
            })
        }
    }
}

#[cfg(windows)]
struct WindowsSandboxedProcess {
    stdin: WindowsHandle,
    stdout: WindowsHandle,
    stderr: WindowsHandle,
    backend: WindowsBackendProcess,
}

#[cfg(windows)]
fn create_pipe_pair(read: &mut WindowsHandle, write: &mut WindowsHandle) -> Result<(), McpError> {
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Pipes::CreatePipe;

    unsafe {
        let mut read_handle = std::ptr::null_mut();
        let mut write_handle = std::ptr::null_mut();
        let attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        if CreatePipe(&mut read_handle, &mut write_handle, &attrs, 0) == 0 {
            return Err(McpError::Transport(format!(
                "create pipe failed: {}",
                last_error()
            )));
        }
        *read = WindowsHandle::new(read_handle);
        *write = WindowsHandle::new(write_handle);
        Ok(())
    }
}

#[cfg(windows)]
fn ensure_app_container_sid(
    name: &[u16],
    create: unsafe extern "system" fn(
        windows_sys::core::PCWSTR,
        windows_sys::core::PCWSTR,
        windows_sys::core::PCWSTR,
        *const windows_sys::Win32::Security::SID_AND_ATTRIBUTES,
        u32,
        *mut *mut core::ffi::c_void,
    ) -> i32,
    derive: unsafe extern "system" fn(
        windows_sys::core::PCWSTR,
        *mut *mut core::ffi::c_void,
    ) -> i32,
) -> Result<WindowsAppContainerSid, McpError> {
    unsafe {
        let mut sid: *mut core::ffi::c_void = std::ptr::null_mut();
        let hr = create(
            name.as_ptr(),
            name.as_ptr(),
            name.as_ptr(),
            std::ptr::null(),
            0,
            &mut sid,
        );
        if hr == 0 {
            return Ok(WindowsAppContainerSid {
                sid: sid as _,
                name: name.to_vec(),
                delete_profile_on_drop: true,
            });
        }
        if hr == 0x8007_00b7u32 as i32 {
            let hr = derive(name.as_ptr(), &mut sid);
            if hr != 0 {
                return Err(McpError::Transport(format!(
                    "derive AppContainer SID failed: {hr:#x}"
                )));
            }
            return Ok(WindowsAppContainerSid {
                sid: sid as _,
                name: name.to_vec(),
                delete_profile_on_drop: false,
            });
        }
        Err(McpError::Transport(format!(
            "create AppContainer profile failed: {hr:#x}"
        )))
    }
}

#[cfg(windows)]
fn apply_directory_acl(
    path: &Path,
    sid: windows_sys::Win32::Security::PSID,
    access_mask: u32,
) -> Result<WindowsDirectoryAclGuard, McpError> {
    apply_path_acl(
        path,
        sid,
        access_mask,
        windows_sys::Win32::Security::OBJECT_INHERIT_ACE
            | windows_sys::Win32::Security::CONTAINER_INHERIT_ACE,
    )
}

#[cfg(windows)]
fn apply_path_acl(
    path: &Path,
    sid: windows_sys::Win32::Security::PSID,
    access_mask: u32,
    inheritance: u32,
) -> Result<WindowsDirectoryAclGuard, McpError> {
    use std::mem::zeroed;
    use windows_sys::Win32::Security::Authorization::{
        BuildTrusteeWithSidW, GetNamedSecurityInfoW, SetEntriesInAclW, EXPLICIT_ACCESS_W,
        GRANT_ACCESS, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};

    type Pacl = *mut ACL;

    let path = to_wide_os(path.as_os_str());
    unsafe {
        let mut original_dacl: Pacl = std::ptr::null_mut();
        let mut security_descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let status = GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut original_dacl,
            std::ptr::null_mut(),
            &mut security_descriptor,
        );
        if status != 0 {
            return Err(McpError::Transport(format!(
                "read directory security failed: {status}"
            )));
        }

        let mut access: EXPLICIT_ACCESS_W = zeroed();
        BuildTrusteeWithSidW(&mut access.Trustee, sid);
        access.grfAccessPermissions = access_mask;
        access.grfAccessMode = GRANT_ACCESS;
        access.grfInheritance = inheritance;

        let mut new_dacl: Pacl = std::ptr::null_mut();
        let status = SetEntriesInAclW(1, &access, original_dacl, &mut new_dacl);
        if status != 0 {
            let _ = windows_sys::Win32::Foundation::LocalFree(security_descriptor as _);
            return Err(McpError::Transport(format!(
                "build directory ACL failed: {status}"
            )));
        }

        let status = set_file_dacl(path.as_ptr(), new_dacl);
        let _ = windows_sys::Win32::Foundation::LocalFree(new_dacl as _);
        if status != 0 {
            let _ = windows_sys::Win32::Foundation::LocalFree(security_descriptor as _);
            return Err(McpError::Transport(format!(
                "apply ACL to '{}' failed: {status}",
                String::from_utf16_lossy(&path[..path.len().saturating_sub(1)])
            )));
        }

        Ok(WindowsDirectoryAclGuard {
            path,
            original_dacl,
            security_descriptor,
        })
    }
}

#[cfg(windows)]
unsafe fn set_file_dacl(path: *const u16, dacl: *mut windows_sys::Win32::Security::ACL) -> u32 {
    use std::mem::zeroed;
    use windows_sys::Win32::Security::{
        InitializeSecurityDescriptor, SetFileSecurityW, SetSecurityDescriptorDacl,
        DACL_SECURITY_INFORMATION, SECURITY_DESCRIPTOR,
    };

    let mut descriptor: SECURITY_DESCRIPTOR = zeroed();
    if InitializeSecurityDescriptor(&mut descriptor as *mut _ as _, 1) == 0
        || SetSecurityDescriptorDacl(&mut descriptor as *mut _ as _, 1, dacl, 0) == 0
        || SetFileSecurityW(
            path,
            DACL_SECURITY_INFORMATION,
            &mut descriptor as *mut _ as _,
        ) == 0
    {
        last_error()
    } else {
        0
    }
}

#[cfg(windows)]
fn apply_traverse_acls(
    roots: &[&Path],
    sid: windows_sys::Win32::Security::PSID,
    access_mask: u32,
) -> Result<Vec<WindowsDirectoryAclGuard>, McpError> {
    let mut ancestors = std::collections::BTreeSet::new();
    for root in roots {
        let mut current = root.parent();
        while let Some(path) = current {
            if path.parent().and_then(Path::parent).is_none() {
                break;
            }
            ancestors.insert(path.to_path_buf());
            current = path.parent();
        }
    }
    ancestors
        .into_iter()
        .map(|path| apply_path_acl(&path, sid, access_mask, 0))
        .collect()
}

#[cfg(windows)]
fn apply_acl_tree(
    root: &Path,
    sid: windows_sys::Win32::Security::PSID,
    directory_access_mask: u32,
    file_access_mask: u32,
) -> Result<Vec<WindowsDirectoryAclGuard>, McpError> {
    let mut guards = vec![apply_directory_acl(root, sid, directory_access_mask)?];
    if root.is_dir() {
        apply_acl_tree_children(root, sid, file_access_mask, &mut guards)?;
    }
    Ok(guards)
}

#[cfg(windows)]
fn apply_acl_tree_children(
    root: &Path,
    sid: windows_sys::Win32::Security::PSID,
    file_access_mask: u32,
    guards: &mut Vec<WindowsDirectoryAclGuard>,
) -> Result<(), McpError> {
    for entry in std::fs::read_dir(root)
        .map_err(|error| McpError::Transport(format!("read package directory failed: {error}")))?
    {
        let entry = entry
            .map_err(|error| McpError::Transport(format!("read package entry failed: {error}")))?;
        let file_type = entry.file_type().map_err(|error| {
            McpError::Transport(format!("inspect package entry failed: {error}"))
        })?;
        let path = entry.path();
        if file_type.is_symlink() {
            return Err(McpError::Transport(format!(
                "package symlinks are unsupported: {}",
                path.display()
            )));
        }
        if file_type.is_dir() {
            guards.push(apply_directory_acl(&path, sid, file_access_mask)?);
            apply_acl_tree_children(&path, sid, file_access_mask, guards)?;
        } else if file_type.is_file() {
            guards.push(apply_directory_acl(&path, sid, file_access_mask)?);
        }
    }
    Ok(())
}

#[cfg(windows)]
fn to_wide_str(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(windows)]
fn to_wide_os(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn build_command_line(command: &str, args: &[&str]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(quote_windows_argument(command));
    parts.extend(args.iter().map(|arg| quote_windows_argument(arg)));
    parts.join(" ")
}

#[cfg(windows)]
fn quote_windows_argument(argument: &str) -> String {
    if argument.is_empty() {
        return "\"\"".into();
    }
    if !argument.chars().any(|ch| ch.is_whitespace() || ch == '"') {
        return argument.to_string();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for ch in argument.chars() {
        match ch {
            '\\' => {
                backslashes += 1;
                quoted.push('\\');
            }
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                backslashes = 0;
                quoted.push(ch);
            }
        }
    }
    if backslashes > 0 {
        quoted.extend(std::iter::repeat_n('\\', backslashes));
    }
    quoted.push('"');
    quoted
}

#[cfg(windows)]
fn to_environment_block_owned(environment: &[(String, String)]) -> Vec<u16> {
    let mut block = Vec::new();
    for (name, value) in environment {
        block.extend(name.encode_utf16());
        block.push('=' as u16);
        block.extend(value.encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(windows)]
fn last_error() -> u32 {
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}

/// A `node` invocation for an .mjs server script, the common case for
/// JavaScript/TypeScript app authors.
pub fn spawn_node_server(script: &Path) -> Result<StdioTransport, McpError> {
    let script = script
        .to_str()
        .ok_or_else(|| McpError::Transport(format!("non-UTF-8 script path: {script:?}")))?;
    StdioTransport::spawn("node", &[script])
}

/// Stable AppContainer moniker derived from the app id. Host uninstall uses
/// the same name to remove the profile.
pub fn app_container_moniker(app_id: &str) -> String {
    format!("Kestral.AppContainer.{app_id}")
}

#[cfg(windows)]
pub fn delete_app_container_profile(app_container_name: &str) -> Result<(), McpError> {
    use windows_sys::Win32::Security::Isolation::DeleteAppContainerProfile;

    let name: Vec<u16> = app_container_name.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let hr = DeleteAppContainerProfile(name.as_ptr());
        if hr == 0 {
            return Ok(());
        }
        if hr == 0x8007_0002u32 as i32 || hr == 0x8007_0006u32 as i32 {
            return Ok(());
        }
        Err(McpError::Transport(format!(
            "delete AppContainer profile failed: {hr:#x}"
        )))
    }
}

#[cfg(not(windows))]
pub fn delete_app_container_profile(_app_container_name: &str) -> Result<(), McpError> {
    Ok(())
}

#[cfg(test)]
mod tests;
