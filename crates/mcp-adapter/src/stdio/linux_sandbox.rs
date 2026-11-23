use std::fs::{self, File};
use std::io::{self, Seek, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{build_command, McpError};

const LAUNCHER: &str = "/usr/bin/bwrap";
const MISSING: &str = "install the distribution's Bubblewrap package (/usr/bin/bwrap)";
const UNAVAILABLE: &str =
    "Bubblewrap could not establish the required Linux namespaces and syscall filter";
const PAYLOAD: &str = "/kestral-payload";
const DATA: &str = "/kestral-data";
const RUNTIME_DIRS: &[&str] = &[
    "/usr/bin",
    "/usr/lib",
    "/usr/lib64",
    "/bin",
    "/lib",
    "/lib64",
];

fn launcher() -> Result<Command, &'static str> {
    if !Path::new(LAUNCHER).is_file() {
        return Err(MISSING);
    }
    let filter = socket_filter().map_err(|_| UNAVAILABLE)?;
    let mut cmd = build_command(LAUNCHER, &[]);
    cmd.env_clear()
        .current_dir("/")
        .stderr(Stdio::null())
        .args([
            "--unshare-user",
            "--disable-userns",
            "--unshare-ipc",
            "--unshare-pid",
            "--unshare-net",
            "--unshare-uts",
            "--cap-drop",
            "ALL",
            "--die-with-parent",
            "--new-session",
            "--proc",
            "/proc",
            "--remount-ro",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--clearenv",
            "--setenv",
            "PATH",
            "/usr/bin:/bin",
            "--setenv",
            "TMPDIR",
            "/tmp",
            "--seccomp",
        ])
        .arg(filter.as_raw_fd().to_string());

    for dir in RUNTIME_DIRS {
        cmd.args(["--ro-bind-try", dir, dir]);
    }

    // Close ambient handles on exec, including ones opened by dependencies
    // without CLOEXEC. Retain only the standard streams and the filter fd,
    // which Bubblewrap reads and closes before starting the backend.
    // SAFETY: the child closure owns the fd and uses only raw syscalls/errno.
    unsafe {
        cmd.pre_exec(move || {
            if libc::syscall(
                libc::SYS_close_range,
                3u32,
                u32::MAX,
                libc::CLOSE_RANGE_CLOEXEC,
            ) < 0
                || libc::fcntl(filter.as_raw_fd(), libc::F_SETFD, 0) < 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(cmd)
}

pub(super) fn probe() -> Result<(), &'static str> {
    let mut cmd = launcher()?;
    let mut child = cmd
        .args(["--remount-ro", "/", "--", "/usr/bin/true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|_| UNAVAILABLE)?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success().then_some(()).ok_or(UNAVAILABLE),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(UNAVAILABLE);
            }
        }
    }
}

pub(super) fn prepare(
    command: &str,
    args: &[&str],
    payload_dir: &Path,
    data_dir: &Path,
) -> Result<Command, McpError> {
    let payload = checked_dir(payload_dir)?;
    let data = checked_dir(data_dir)?;
    if overlaps(&payload, &data) {
        return Err(McpError::Transport(
            "sandbox payload and data directories must be separate".into(),
        ));
    }

    let relocate = |value: &str| -> PathBuf {
        let path = Path::new(value);
        for (source, canonical, mount) in
            [(payload_dir, &payload, PAYLOAD), (data_dir, &data, DATA)]
        {
            if let Ok(relative) = path
                .strip_prefix(source)
                .or_else(|_| path.strip_prefix(canonical))
            {
                return Path::new(mount).join(relative);
            }
        }
        path.to_owned()
    };

    let mut cmd = launcher().map_err(|reason| McpError::Transport(reason.into()))?;
    cmd.arg("--ro-bind")
        .arg(&payload)
        .arg(PAYLOAD)
        .arg("--bind")
        .arg(&data)
        .arg(DATA)
        .args([
            "--setenv",
            "APP_HOST_PAYLOAD_DIR",
            PAYLOAD,
            "--setenv",
            "APP_HOST_DATA_DIR",
            DATA,
        ])
        .args(["--chdir", DATA, "--remount-ro", "/", "--"])
        .arg(relocate(command))
        .args(args.iter().map(|arg| relocate(arg)));
    Ok(cmd)
}

fn checked_dir(path: &Path) -> Result<PathBuf, McpError> {
    let canonical = fs::canonicalize(path)
        .map_err(|e| McpError::Transport(format!("resolve sandbox directory: {e}")))?;
    if !canonical.is_dir()
        || RUNTIME_DIRS
            .iter()
            .map(fs::canonicalize)
            .filter_map(Result::ok)
            .any(|runtime| overlaps(&runtime, &canonical))
    {
        return Err(McpError::Transport(
            "sandbox directories must not overlap system runtime directories".into(),
        ));
    }
    Ok(canonical)
}

fn overlaps(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

/// Native-ABI classic BPF over `struct seccomp_data` (syscall at byte 0,
/// audit architecture at byte 4). Block socket creation (including non-IP
/// sockets) and io_uring, whose operations do not traverse this syscall filter.
fn socket_filter() -> io::Result<File> {
    const LOAD_WORD: u16 = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
    const JUMP_EQ: u16 = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16;
    const JUMP_GE: u16 = (libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K) as u16;
    const RET: u16 = (libc::BPF_RET | libc::BPF_K) as u16;
    #[cfg(target_arch = "x86_64")]
    const ARCH: u32 = 0xc000_003e; // AUDIT_ARCH_X86_64
    #[cfg(target_arch = "aarch64")]
    const ARCH: u32 = 0xc000_00b7; // AUDIT_ARCH_AARCH64

    let instructions = [
        (LOAD_WORD, 0, 0, 4),
        (JUMP_EQ, 1, 0, ARCH),
        (RET, 0, 0, libc::SECCOMP_RET_KILL_PROCESS),
        (LOAD_WORD, 0, 0, 0),
        (JUMP_GE, 2, 0, 0x4000_0000), // also reject x32 syscall encoding
        (JUMP_EQ, 1, 0, libc::SYS_socket as u32),
        (JUMP_EQ, 0, 1, libc::SYS_io_uring_setup as u32),
        (RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32),
        (RET, 0, 0, libc::SECCOMP_RET_ALLOW),
    ];
    // SAFETY: C string is NUL-terminated; a successful fd is owned here.
    let fd = unsafe { libc::memfd_create(c"kestral-stdio-filter".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    for (code, jt, jf, operand) in instructions {
        file.write_all(&code.to_ne_bytes())?;
        file.write_all(&[jt, jf])?;
        file.write_all(&operand.to_ne_bytes())?;
    }
    file.rewind()?;
    Ok(file)
}
