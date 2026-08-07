//! Generic invocation-scoped Node worker transport.

use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Serialize;

pub(crate) const MAX_STDOUT_LINE_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_STDERR_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct WorkerPaths {
    pub(crate) node: PathBuf,
    pub(crate) worker: PathBuf,
}

pub(crate) struct WorkerPathConfig<'a> {
    pub(crate) node_env: &'a str,
    pub(crate) worker_env: &'a str,
    pub(crate) resource_env: &'a str,
    pub(crate) source_directory: &'a str,
    pub(crate) packaged_directory: &'a str,
    pub(crate) packaged_worker: &'a str,
    pub(crate) display_name: &'a str,
}

pub(crate) struct NodeWorkerProcess<E: DeserializeOwned + Send + 'static> {
    child: Child,
    stdin: ChildStdin,
    events: Receiver<Result<E, String>>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<Result<Vec<u8>, String>>>,
    finished: bool,
    display_name: &'static str,
}

impl<E> NodeWorkerProcess<E>
where
    E: DeserializeOwned + Send + 'static,
{
    pub(crate) fn spawn(paths: &WorkerPaths, display_name: &'static str) -> Result<Self, String> {
        let mut command = Command::new(&paths.node);
        command
            .arg(&paths.worker)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        preserve_windows_runtime_environment(&mut command);

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to start {display_name}: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("failed to open {display_name} stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("failed to open {display_name} stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| format!("failed to open {display_name} stderr"))?;

        let (sender, events) = mpsc::channel();
        let stderr_sender = sender.clone();
        let stdout_name = display_name;
        let stderr_name = display_name;
        let stdout_thread = thread::spawn(move || read_worker_events(stdout, sender, stdout_name));
        let stderr_thread = thread::spawn(move || {
            let result = read_bounded_stderr(stderr, stderr_name);
            if let Err(error) = &result {
                let _ = stderr_sender.send(Err(error.clone()));
            }
            result
        });

        Ok(Self {
            child,
            stdin,
            events,
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
            finished: false,
            display_name,
        })
    }

    pub(crate) fn send<C: Serialize>(&mut self, command: &C) -> Result<(), String> {
        let encoded = serde_json::to_vec(command)
            .map_err(|error| format!("worker request serialization failed: {error}"))?;
        self.stdin
            .write_all(&encoded)
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|error| format!("failed to write {} request: {error}", self.display_name))
    }

    pub(crate) fn recv(&self, timeout: Duration) -> Result<E, String> {
        match self.events.recv_timeout(timeout) {
            Ok(Ok(event)) => Ok(event),
            Ok(Err(error)) => Err(error),
            Err(RecvTimeoutError::Timeout) => {
                Err(format!("{} output timed out", self.display_name))
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(format!("{} closed stdout unexpectedly", self.display_name))
            }
        }
    }

    pub(crate) fn try_recv(&self, timeout: Duration) -> Result<Option<E>, String> {
        match self.events.recv_timeout(timeout) {
            Ok(Ok(event)) => Ok(Some(event)),
            Ok(Err(error)) => Err(error),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                Err(format!("{} closed stdout unexpectedly", self.display_name))
            }
        }
    }

    pub(crate) fn wait_for_exit(&mut self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(status)) if status.success() => {
                    self.finished = true;
                    self.join_readers()?;
                    return Ok(());
                }
                Ok(Some(status)) => {
                    self.finished = true;
                    let stderr = self.join_readers()?;
                    return Err(format!(
                        "{} exited with status {status}{}",
                        self.display_name,
                        format_worker_stderr(&stderr)
                    ));
                }
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(error) => {
                    return Err(format!("failed waiting for {}: {error}", self.display_name));
                }
            }
        }
        Err(format!("{} shutdown timed out", self.display_name))
    }

    /// Join the reader threads, returning any captured stderr (up to
    /// `MAX_STDERR_BYTES`) so callers can fold the worker's own diagnostics
    /// into their error messages instead of discarding them.
    fn join_readers(&mut self) -> Result<Vec<u8>, String> {
        if let Some(handle) = self.stdout_thread.take() {
            handle
                .join()
                .map_err(|_| format!("{} stdout reader panicked", self.display_name))?;
        }
        if let Some(handle) = self.stderr_thread.take() {
            return handle
                .join()
                .map_err(|_| format!("{} stderr reader panicked", self.display_name))?;
        }
        Ok(Vec::new())
    }
}

/// Format captured worker stderr for appending to an error message. Returns an
/// empty string when there is nothing useful to show, otherwise `: <text>`.
fn format_worker_stderr(stderr: &[u8]) -> String {
    if stderr.is_empty() {
        return String::new();
    }
    let text = String::from_utf8_lossy(stderr);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!(": {trimmed}")
    }
}

impl<E: DeserializeOwned + Send + 'static> Drop for NodeWorkerProcess<E> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.finished = true;
        }
        let _ = self.join_readers();
    }
}

pub(crate) fn read_worker_events<E: DeserializeOwned>(
    stdout: impl Read,
    sender: mpsc::Sender<Result<E, String>>,
    display_name: &str,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = Vec::new();
        match reader
            .by_ref()
            .take((MAX_STDOUT_LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
        {
            Ok(0) => return,
            Ok(_) if line.len() > MAX_STDOUT_LINE_BYTES => {
                let _ = sender.send(Err(format!("{display_name} output line exceeded 2 MiB")));
                return;
            }
            Ok(_) => {
                if !line.ends_with(b"\n") {
                    let _ = sender.send(Err(format!(
                        "{display_name} output ended with an incomplete line"
                    )));
                    return;
                }
                let event = serde_json::from_slice::<E>(&line)
                    .map_err(|error| format!("malformed {display_name} output: {error}"));
                if sender.send(event).is_err() {
                    return;
                }
            }
            Err(error) => {
                let _ = sender.send(Err(format!(
                    "failed reading {display_name} output: {error}"
                )));
                return;
            }
        }
    }
}

pub(crate) fn read_bounded_stderr(
    mut stderr: impl Read,
    display_name: &str,
) -> Result<Vec<u8>, String> {
    let mut captured = Vec::new();
    stderr
        .by_ref()
        .take((MAX_STDERR_BYTES + 1) as u64)
        .read_to_end(&mut captured)
        .map_err(|error| format!("failed reading {display_name} stderr: {error}"))?;
    if captured.len() > MAX_STDERR_BYTES {
        return Err(format!("{display_name} stderr exceeded 16 KiB"));
    }
    Ok(captured)
}

pub(crate) fn resolve_worker_paths(
    manifest_dir: &Path,
    current_exe: PathBuf,
    config: &WorkerPathConfig<'_>,
) -> Result<WorkerPaths, String> {
    resolve_worker_paths_from(
        env::var_os(config.node_env).map(PathBuf::from),
        env::var_os(config.worker_env).map(PathBuf::from),
        manifest_dir,
        current_exe,
        config,
    )
}

pub(crate) fn resolve_worker_paths_from(
    explicit_node: Option<PathBuf>,
    explicit_worker: Option<PathBuf>,
    manifest_dir: &Path,
    current_exe: PathBuf,
    config: &WorkerPathConfig<'_>,
) -> Result<WorkerPaths, String> {
    match (explicit_node, explicit_worker) {
        (Some(node), Some(worker)) => {
            return validate_worker_paths(node, worker, config.display_name)
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(format!(
                "{} and {} must both be set",
                config.node_env, config.worker_env
            ));
        }
        (None, None) => {}
    }

    let node_name = if cfg!(windows) { "node.exe" } else { "node" };
    let source_root = manifest_dir
        .parent()
        .ok_or_else(|| "failed to resolve host source directory".to_string())?
        .join(config.source_directory);
    let source = WorkerPaths {
        node: source_root.join("runtime").join(node_name),
        worker: source_root.join("dist").join("worker.mjs"),
    };
    if cfg!(debug_assertions) && source.node.is_file() && source.worker.is_file() {
        return Ok(source);
    }

    if let Some(resource_dir) = env::var_os(config.resource_env).map(PathBuf::from) {
        return validate_worker_paths(
            resource_dir
                .join(config.packaged_directory)
                .join("runtime")
                .join(node_name),
            resource_dir
                .join(config.packaged_directory)
                .join(config.packaged_worker),
            config.display_name,
        );
    }

    let executable_dir = current_exe
        .parent()
        .ok_or_else(|| "failed to resolve current executable directory".to_string())?;
    validate_worker_paths(
        executable_dir
            .join(config.packaged_directory)
            .join("runtime")
            .join(node_name),
        executable_dir
            .join(config.packaged_directory)
            .join(config.packaged_worker),
        config.display_name,
    )
}

pub(crate) fn validate_worker_paths(
    node: PathBuf,
    worker: PathBuf,
    display_name: &str,
) -> Result<WorkerPaths, String> {
    if !node.is_file() {
        return Err(format!(
            "{display_name} Node executable not found: {}",
            node.display()
        ));
    }
    if !worker.is_file() {
        return Err(format!(
            "{display_name} script not found: {}",
            worker.display()
        ));
    }
    Ok(WorkerPaths { node, worker })
}

fn preserve_windows_runtime_environment(command: &mut Command) {
    #[cfg(windows)]
    for name in ["SystemRoot", "WINDIR"] {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
    #[cfg(not(windows))]
    let _ = command;
}
