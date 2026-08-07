//! Safe acquisition of app packages from public Git repositories.
//!
//! Repositories are fetched into a bare store and exported from Git objects.
//! No worktree means repository hooks and clean/smudge filters cannot run.

use std::fs::{self, File};
use std::io::Write;
use std::net::{IpAddr, ToSocketAddrs};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use reqwest::Url;
use uuid::Uuid;

const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_FETCH_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PACKAGE_FILES: usize = 10_000;

#[derive(Debug)]
struct ValidatedGitUrl {
    url: Url,
    host: String,
    port: u16,
    address: IpAddr,
}

pub(crate) struct ExportedRepository {
    temporary_root: PathBuf,
    export_root: PathBuf,
}

impl ExportedRepository {
    pub(crate) fn path(&self) -> &Path {
        &self.export_root
    }
}

impl Drop for ExportedRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.temporary_root);
    }
}

pub(crate) fn export_public_repository(
    git_url: &str,
    staging_root: &Path,
) -> Result<ExportedRepository, String> {
    let url = validate_public_git_url(git_url)?;
    export_repository(&url, staging_root)
}

fn validate_public_git_url(git_url: &str) -> Result<ValidatedGitUrl, String> {
    let url = Url::parse(git_url).map_err(|error| format!("invalid Git URL: {error}"))?;
    if url.scheme() != "https" {
        return Err("public Git URLs must use HTTPS".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("public Git URLs must not contain credentials".into());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "public Git URL must include a host".to_string())?
        .to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses: Vec<_> = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|error| format!("could not resolve Git host '{host}': {error}"))?
        .collect();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err("Git URL host must resolve only to public network addresses".into());
    }
    Ok(ValidatedGitUrl {
        url,
        host,
        port,
        address: addresses[0].ip(),
    })
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || octets[0] == 0
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
                || (octets[0] == 192 && octets[1] == 88 && octets[2] == 99)
                || (octets[0] == 198 && (18..=19).contains(&octets[1]))
                || octets[0] >= 224)
        }
        IpAddr::V6(ip) => {
            if let Some(ipv4) = ip.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(ipv4));
            }
            let segments = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
                || (segments[0] == 0x0100
                    && segments[1] == 0
                    && segments[2] == 0
                    && segments[3] == 0))
        }
    }
}

fn export_repository(
    git_url: &ValidatedGitUrl,
    staging_root: &Path,
) -> Result<ExportedRepository, String> {
    fs::create_dir_all(staging_root)
        .map_err(|error| format!("create package staging directory failed: {error}"))?;
    let temporary_root = staging_root.join(format!(".git-source-{}", Uuid::new_v4()));
    let repository = temporary_root.join("repository.git");
    let archive = temporary_root.join("repository.tar");
    let export_root = temporary_root.join("export");
    fs::create_dir_all(&temporary_root)
        .map_err(|error| format!("create temporary Git directory failed: {error}"))?;

    let result = (|| {
        run_git(
            Command::new("git")
                .args(["init", "--bare"])
                .arg(&repository),
            &temporary_root,
            "initialize temporary repository",
        )?;
        let address = match git_url.address {
            IpAddr::V4(address) => address.to_string(),
            IpAddr::V6(address) => format!("[{address}]"),
        };
        let pinned_address = format!("{}:{}:{address}", git_url.host, git_url.port);
        let mut fetch = Command::new("git");
        fetch
            .args(["-c", "http.followRedirects=false"])
            .args(["-c", "http.proxy="])
            .args(["-c", "protocol.allow=never"])
            .args(["-c", "protocol.https.allow=always"])
            .args(["-c", &format!("http.curloptResolve={pinned_address}")])
            .arg("--git-dir")
            .arg(&repository)
            .args(["fetch", "--depth=1", "--no-tags"])
            .arg(git_url.url.as_str())
            .arg("HEAD");
        run_git(&mut fetch, &temporary_root, "fetch repository")?;
        run_git(
            Command::new("git")
                .args(["--git-dir"])
                .arg(&repository)
                .args(["archive", "--format=tar", "--output"])
                .arg(&archive)
                .arg("FETCH_HEAD"),
            &temporary_root,
            "export repository",
        )?;
        extract_archive(&archive, &export_root)
    })();

    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temporary_root);
        return Err(error);
    }
    Ok(ExportedRepository {
        temporary_root,
        export_root,
    })
}

fn run_git(command: &mut Command, limit_root: &Path, action: &str) -> Result<(), String> {
    let stdout_path = limit_root.join("git.stdout");
    let stderr_path = limit_root.join("git.stderr");
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null_device())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            File::create(&stdout_path).map_err(|error| error.to_string())?,
        ))
        .stderr(Stdio::from(
            File::create(&stderr_path).map_err(|error| error.to_string())?,
        ));
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|error| format!("{action} failed to start Git: {error}"))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("{action} failed while waiting for Git: {error}"))?
        {
            if status.success() {
                return Ok(());
            }
            let detail = fs::read_to_string(&stderr_path).unwrap_or_default();
            return Err(format!("{action} failed: {}", detail.trim()));
        }
        if started.elapsed() >= FETCH_TIMEOUT || directory_size(limit_root)? > MAX_FETCH_BYTES {
            terminate_process_tree(&mut child);
            let _ = child.wait();
            return Err(format!("{action} exceeded the time or size limit"));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

fn null_device() -> &'static str {
    if cfg!(windows) {
        "NUL"
    } else {
        "/dev/null"
    }
}

fn directory_size(root: &Path) -> Result<u64, String> {
    let mut total = 0_u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("read temporary Git directory failed: {error}"))?
        {
            let entry =
                entry.map_err(|error| format!("read temporary Git entry failed: {error}"))?;
            let metadata = entry
                .metadata()
                .map_err(|error| format!("read temporary Git metadata failed: {error}"))?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}

fn extract_archive(archive_path: &Path, export_root: &Path) -> Result<(), String> {
    fs::create_dir_all(export_root)
        .map_err(|error| format!("create Git export directory failed: {error}"))?;
    let file = File::open(archive_path)
        .map_err(|error| format!("open Git export archive failed: {error}"))?;
    let mut archive = tar::Archive::new(file);
    let mut files = 0_usize;
    let mut bytes = 0_u64;
    let started = Instant::now();
    for entry in archive
        .entries()
        .map_err(|error| format!("read Git export archive failed: {error}"))?
    {
        let mut entry = entry.map_err(|error| format!("read Git archive entry failed: {error}"))?;
        files += 1;
        if files > MAX_PACKAGE_FILES || started.elapsed() >= FETCH_TIMEOUT {
            return Err(
                "Git repository package exceeds the entry count or extraction time limit".into(),
            );
        }
        let entry_type = entry.header().entry_type();
        if entry_type.is_pax_global_extensions() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|error| format!("read Git archive path failed: {error}"))?
            .into_owned();
        if path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(format!(
                "Git repository contains an unsafe path: {}",
                path.display()
            ));
        }
        let destination = export_root.join(&path);
        if entry_type.is_dir() {
            fs::create_dir_all(&destination)
                .map_err(|error| format!("create Git export directory failed: {error}"))?;
            continue;
        }
        if !entry_type.is_file() {
            return Err(format!(
                "Git repository contains unsupported non-regular entry: {}",
                path.display()
            ));
        }
        bytes = bytes.saturating_add(entry.size());
        if bytes > MAX_PACKAGE_BYTES {
            return Err("Git repository package exceeds the file count or size limit".into());
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create Git export parent failed: {error}"))?;
        }
        let mut output = File::create(&destination)
            .map_err(|error| format!("create Git export file failed: {error}"))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| format!("write Git export file failed: {error}"))?;
        output
            .flush()
            .map_err(|error| format!("flush Git export file failed: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
