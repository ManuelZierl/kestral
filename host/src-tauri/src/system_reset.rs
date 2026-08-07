use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::atomic_json::{
    load_json_document, persist_json_document, standard_writer, AtomicJsonError,
};
use crate::host_paths::HostPaths;

const RESET_REQUEST_FILE: &str = "kestral-system-reset.json";
const RESET_REQUEST_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetRequest {
    version: u32,
    profile_id: String,
    requested_at: String,
}

pub(crate) fn stage(paths: &HostPaths, confirmation: &str) -> Result<(), String> {
    let expected = format!("RESET {}", paths.profile_slug());
    if confirmation != expected {
        return Err(format!(
            "system reset confirmation must exactly match '{expected}'"
        ));
    }
    persist_json_document(
        &request_path(paths.root()),
        &ResetRequest {
            version: RESET_REQUEST_VERSION,
            profile_id: paths.profile_id().to_string(),
            requested_at: Utc::now().to_rfc3339(),
        },
        "system reset request",
        standard_writer().as_ref(),
    )
    .map_err(AtomicJsonError::into_message)
}

pub(crate) fn apply_pending(paths: &HostPaths) -> Result<bool, String> {
    let marker = request_path(paths.root());
    let Some(request) = load_json_document::<ResetRequest>(&marker, "system reset request")? else {
        return Ok(false);
    };
    if request.version != RESET_REQUEST_VERSION {
        return Err(format!(
            "unsupported system reset request version: {}",
            request.version
        ));
    }
    if request.profile_id != paths.profile_id() {
        return Err("system reset request does not match the active profile identity".into());
    }
    chrono::DateTime::parse_from_rfc3339(&request.requested_at)
        .map_err(|error| format!("system reset request timestamp is invalid: {error}"))?;

    crate::config::clear_all_indexed_secrets(
        paths.secrets_index_path().to_path_buf(),
        paths.profile_id().to_string(),
    )?;
    remove_profile_state(paths, &marker)?;
    fs::remove_file(&marker).map_err(|error| {
        format!(
            "complete system reset by removing '{}': {error}",
            marker.display()
        )
    })?;
    sync_directory(paths.root())
        .map_err(|error| format!("sync reset profile directory failed: {error}"))?;
    Ok(true)
}

pub(crate) fn validate_persisted(paths: &HostPaths) -> Result<(), String> {
    let path = request_path(paths.root());
    let Some(request) = load_json_document::<ResetRequest>(&path, "system reset request")? else {
        return Ok(());
    };
    if request.version != RESET_REQUEST_VERSION {
        return Err(format!(
            "unsupported system reset request version: {}",
            request.version
        ));
    }
    if request.profile_id != paths.profile_id() {
        return Err("system reset request does not match the active profile identity".into());
    }
    chrono::DateTime::parse_from_rfc3339(&request.requested_at)
        .map_err(|error| format!("system reset request timestamp is invalid: {error}"))?;
    Ok(())
}

fn remove_profile_state(paths: &HostPaths, marker: &Path) -> Result<(), String> {
    let entries = fs::read_dir(paths.root()).map_err(|error| {
        format!(
            "inspect profile data for system reset '{}' failed: {error}",
            paths.root().display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("inspect system reset entry failed: {error}"))?;
        let path = entry.path();
        if path == marker
            || crate::profiles::preserve_during_profile_reset(
                paths.root(),
                paths.default_root(),
                &path,
            )
        {
            continue;
        }
        remove_entry(&path)?;
    }
    Ok(())
}

fn remove_entry(path: &Path) -> Result<(), String> {
    let file_type = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect reset target '{}' failed: {error}", path.display()))?
        .file_type();
    let result = if file_type.is_dir() && !file_type.is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|error| format!("remove reset target '{}' failed: {error}", path.display()))
}

fn request_path(root: &Path) -> PathBuf {
    root.join(RESET_REQUEST_FILE)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
