use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mcp_adapter::stdio::{app_container_moniker, StdioTransport};
use mcp_adapter::{McpTransport, RequestOptions};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::atomic_json::{load_json_document, persist_json_document, standard_writer};
use crate::package::{AppData, AppDataMigration, Backend, BackendAuthorityMode};

const STATE_VERSION: u32 = 1;
const STATE_FILE: &str = "app-data-state-v1.json";
const REVISIONS_DIR: &str = "app-data-revisions";
const SURFACE_STATE_FILE: &str = "surface-state-v2.json";
const MIGRATION_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct AppDataState {
    version: u32,
    active_revision_id: String,
    revisions: Vec<AppDataRevision>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AppDataRevision {
    pub revision_id: String,
    pub format_version: u32,
    pub package_revision_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct MigrationResponse {
    protocol_version: u32,
    format_version: u32,
}

pub(crate) fn validate_all(data_root: &Path) -> Result<(), String> {
    if !data_root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(data_root).map_err(|error| {
        format!(
            "read app-data root '{}' failed: {error}",
            data_root.display()
        )
    })? {
        let entry = entry.map_err(|error| format!("read app-data entry failed: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect app-data entry failed: {error}"))?;
        if file_type.is_symlink() {
            return Err(format!(
                "app-data root contains a symlink: {}",
                entry.path().display()
            ));
        }
        if file_type.is_dir() && entry.path().join(STATE_FILE).exists() {
            load_state(&entry.path())?;
        }
    }
    Ok(())
}

pub(crate) fn active_dir(
    apps_root: &Path,
    app_id: &str,
    package_revision_id: &str,
    data: &AppData,
    created_at: &str,
) -> Result<PathBuf, String> {
    let root = app_root(apps_root, app_id);
    fs::create_dir_all(&root)
        .map_err(|error| format!("create app-data directory failed: {error}"))?;
    match data {
        AppData::None => {
            if root.join(STATE_FILE).exists() {
                return Err(format!(
                    "app '{app_id}' declares no app-owned data but retained versioned data exists"
                ));
            }
            Ok(root)
        }
        AppData::Versioned { format_version, .. } => {
            let state = match load_state_optional(&root)? {
                Some(state) => state,
                None => initialize_state(&root, *format_version, package_revision_id, created_at)?,
            };
            let active = active_revision(&state)?;
            if active.format_version != *format_version {
                return Err(format!(
                    "app data format {} requires migration before package format {} can start",
                    active.format_version, format_version
                ));
            }
            Ok(revision_dir(&root, &active.revision_id))
        }
        AppData::HostManaged { .. } => Err(
            "host-managed data is available only through the host data API, not a backend directory"
                .into(),
        ),
    }
}

pub(crate) fn current_revision(
    apps_root: &Path,
    app_id: &str,
) -> Result<Option<AppDataRevision>, String> {
    let root = app_root(apps_root, app_id);
    let Some(state) = load_state_optional(&root)? else {
        return Ok(None);
    };
    Ok(Some(active_revision(&state)?.clone()))
}

pub(crate) fn stage_candidate(
    apps_root: &Path,
    app_id: &str,
    candidate: &AppDataRevision,
    source_revision_id: Option<&str>,
) -> Result<(PathBuf, Option<String>), String> {
    let root = app_root(apps_root, app_id);
    let revisions = root.join(REVISIONS_DIR);
    fs::create_dir_all(&revisions)
        .map_err(|error| format!("create app-data revisions directory failed: {error}"))?;
    let candidate_dir = revision_dir(&root, &candidate.revision_id);
    if candidate_dir.exists() {
        fs::remove_dir_all(&candidate_dir)
            .map_err(|error| format!("remove stale app-data candidate failed: {error}"))?;
    }
    fs::create_dir(&candidate_dir)
        .map_err(|error| format!("create app-data candidate failed: {error}"))?;

    let source_digest = if let Some(source_revision_id) = source_revision_id {
        let source_dir = revision_dir(&root, source_revision_id);
        let source_before = tree_digest(&source_dir)?;
        copy_tree(&source_dir, &candidate_dir)?;
        let source_after = tree_digest(&source_dir)?;
        if source_before != source_after {
            let _ = fs::remove_dir_all(&candidate_dir);
            return Err("app data changed while the migration candidate was staged".into());
        }
        Some(source_before)
    } else {
        None
    };
    Ok((candidate_dir, source_digest))
}

pub(crate) fn revision_digest(
    apps_root: &Path,
    app_id: &str,
    revision_id: &str,
) -> Result<String, String> {
    tree_digest(&revision_dir(&app_root(apps_root, app_id), revision_id))
}

pub(crate) fn run_migration_command(
    payload_dir: &Path,
    candidate_dir: &Path,
    app_id: &str,
    backend: &Backend,
    migration: &AppDataMigration,
    from: u32,
    to: u32,
) -> Result<(), String> {
    let payload = payload_dir
        .to_str()
        .ok_or_else(|| "package payload path is not UTF-8".to_string())?;
    let data = candidate_dir
        .to_str()
        .ok_or_else(|| "app-data candidate path is not UTF-8".to_string())?;
    let environment = [
        ("APP_HOST_PAYLOAD_DIR", payload),
        ("APP_HOST_DATA_DIR", data),
    ];
    let resolved_args = resolve_payload_args(payload_dir, &migration.args);
    let args: Vec<&str> = resolved_args.iter().map(String::as_str).collect();
    let authority_mode = backend_authority_mode(backend)?;
    let command = if migration.command == migration.entry {
        payload_dir
            .join(&migration.command)
            .to_string_lossy()
            .to_string()
    } else {
        migration.command.clone()
    };
    let transport: Box<dyn McpTransport> = match authority_mode {
        BackendAuthorityMode::Unsandboxed => Box::new(
            StdioTransport::spawn_in_isolated(&command, &args, candidate_dir, &environment)
                .map_err(|error| format!("start app-data migration command failed: {error}"))?,
        ),
        BackendAuthorityMode::Sandboxed => {
            let command = resolve_sandboxed_command(&command, payload_dir)?;
            Box::new(
                StdioTransport::spawn_sandboxed(
                    &app_container_moniker(app_id),
                    &command,
                    &args,
                    payload_dir,
                    candidate_dir,
                    &environment,
                )
                .map_err(|error| format!("start sandboxed app-data migration failed: {error}"))?,
            )
        }
    };
    let result = transport.request(
        "kestral/app-data/migrate",
        json!({
            "protocol_version": migration.protocol_version,
            "from_format_version": from,
            "to_format_version": to,
        }),
        &RequestOptions::with_timeout(MIGRATION_TIMEOUT),
    );
    transport.shutdown();
    let value = result.map_err(|error| format!("app-data migration failed: {error}"))?;
    let response: MigrationResponse = serde_json::from_value(value)
        .map_err(|error| format!("invalid app-data migration response: {error}"))?;
    if response.protocol_version != 1 || response.format_version != to {
        return Err(format!(
            "app-data migration returned protocol {} format {}; expected protocol 1 format {to}",
            response.protocol_version, response.format_version
        ));
    }
    Ok(())
}

pub(crate) fn commit_candidate(
    apps_root: &Path,
    app_id: &str,
    source_revision_id: Option<&str>,
    candidate: AppDataRevision,
) -> Result<(), String> {
    let root = app_root(apps_root, app_id);
    let mut state = load_state_optional(&root)?.unwrap_or(AppDataState {
        version: STATE_VERSION,
        active_revision_id: candidate.revision_id.clone(),
        revisions: Vec::new(),
    });
    if let Some(source_revision_id) = source_revision_id {
        if state.active_revision_id != source_revision_id {
            if state.active_revision_id == candidate.revision_id {
                return Ok(());
            }
            return Err("active app-data revision changed before migration commit".into());
        }
    }
    if !revision_dir(&root, &candidate.revision_id).is_dir() {
        return Err("validated app-data migration candidate is missing".into());
    }
    state.active_revision_id = candidate.revision_id.clone();
    if !state
        .revisions
        .iter()
        .any(|revision| revision.revision_id == candidate.revision_id)
    {
        state.revisions.push(candidate);
    }
    persist_state(&root, &state)
}

pub(crate) fn rollback_transition(
    apps_root: &Path,
    app_id: &str,
    source_revision_id: Option<&str>,
    candidate_revision_id: &str,
) -> Result<(), String> {
    if let Some(source_revision_id) = source_revision_id {
        let root = app_root(apps_root, app_id);
        let mut state = load_state(&root)?;
        if !state
            .revisions
            .iter()
            .any(|revision| revision.revision_id == source_revision_id)
        {
            return Err(format!(
                "app-data rollback revision '{source_revision_id}' is absent"
            ));
        }
        state.active_revision_id = source_revision_id.to_string();
        state
            .revisions
            .retain(|revision| revision.revision_id != candidate_revision_id);
        persist_state(&root, &state)?;
        return discard_candidate(apps_root, app_id, candidate_revision_id);
    }
    let root = app_root(apps_root, app_id);
    if let Some(state) = load_state_optional(&root)? {
        if state.active_revision_id != candidate_revision_id {
            return Err("app-data rollback found an unexpected active revision".into());
        }
    }
    match fs::remove_file(root.join(STATE_FILE)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "remove app data state during rollback failed: {error}"
            ))
        }
    }
    discard_candidate(apps_root, app_id, candidate_revision_id)
}

pub(crate) fn prune_backups(apps_root: &Path, app_id: &str, retention: u32) -> Result<(), String> {
    if retention == 0 {
        return Err("app-data backup retention must be at least 1".into());
    }
    let root = app_root(apps_root, app_id);
    let mut state = load_state(&root)?;
    let mut backups: Vec<AppDataRevision> = state
        .revisions
        .iter()
        .filter(|revision| revision.revision_id != state.active_revision_id)
        .cloned()
        .collect();
    backups.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    let remove: BTreeSet<String> = backups
        .into_iter()
        .skip(retention as usize)
        .map(|revision| revision.revision_id)
        .collect();
    if remove.is_empty() {
        return Ok(());
    }

    let mut renamed = Vec::new();
    for revision_id in &remove {
        let source = revision_dir(&root, revision_id);
        let destination = root
            .join(REVISIONS_DIR)
            .join(format!(".pruning-{revision_id}"));
        fs::rename(&source, &destination)
            .map_err(|error| format!("stage old app-data backup removal failed: {error}"))?;
        renamed.push((source, destination));
    }
    state
        .revisions
        .retain(|revision| !remove.contains(&revision.revision_id));
    if let Err(error) = persist_state(&root, &state) {
        for (source, destination) in renamed.into_iter().rev() {
            let _ = fs::rename(destination, source);
        }
        return Err(error);
    }
    for (_, destination) in renamed {
        fs::remove_dir_all(&destination)
            .map_err(|error| format!("remove old app-data backup failed: {error}"))?;
    }
    Ok(())
}

pub(crate) fn discard_candidate(
    apps_root: &Path,
    app_id: &str,
    revision_id: &str,
) -> Result<(), String> {
    let path = revision_dir(&app_root(apps_root, app_id), revision_id);
    if !path.exists() {
        return Ok(());
    }
    fs::remove_dir_all(path).map_err(|error| format!("discard app-data candidate failed: {error}"))
}

fn initialize_state(
    root: &Path,
    format_version: u32,
    package_revision_id: &str,
    created_at: &str,
) -> Result<AppDataState, String> {
    ensure_no_legacy_app_data(root)?;
    let revision = AppDataRevision {
        revision_id: Uuid::new_v4().to_string(),
        format_version,
        package_revision_id: package_revision_id.to_string(),
        created_at: created_at.to_string(),
    };
    fs::create_dir_all(revision_dir(root, &revision.revision_id))
        .map_err(|error| format!("create initial app-data revision failed: {error}"))?;
    let state = AppDataState {
        version: STATE_VERSION,
        active_revision_id: revision.revision_id.clone(),
        revisions: vec![revision],
    };
    persist_state(root, &state)?;
    Ok(state)
}

fn ensure_no_legacy_app_data(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    for entry in
        fs::read_dir(root).map_err(|error| format!("read app-data directory failed: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read app-data entry failed: {error}"))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == SURFACE_STATE_FILE || name == REVISIONS_DIR || name == STATE_FILE {
            continue;
        }
        return Err(format!(
            "unversioned app data '{}' cannot be adopted automatically",
            entry.path().display()
        ));
    }
    Ok(())
}

fn load_state_optional(root: &Path) -> Result<Option<AppDataState>, String> {
    let state = load_json_document::<AppDataState>(&root.join(STATE_FILE), "app data state")?;
    state.map(|state| validate_state(root, state)).transpose()
}

fn load_state(root: &Path) -> Result<AppDataState, String> {
    load_state_optional(root)?.ok_or_else(|| "app data state is missing".into())
}

fn validate_state(root: &Path, state: AppDataState) -> Result<AppDataState, String> {
    if state.version != STATE_VERSION {
        return Err(format!(
            "unsupported app data state version {}; expected {STATE_VERSION}",
            state.version
        ));
    }
    let mut ids = BTreeSet::new();
    for revision in &state.revisions {
        Uuid::parse_str(&revision.revision_id)
            .map_err(|_| "app data state contains an invalid revision id".to_string())?;
        if revision.format_version == 0 || !ids.insert(revision.revision_id.as_str()) {
            return Err("app data state contains an invalid or duplicate revision".into());
        }
        validate_tree(&revision_dir(root, &revision.revision_id))?;
    }
    if !ids.contains(state.active_revision_id.as_str()) {
        return Err("app data state active revision is absent".into());
    }
    Ok(state)
}

fn active_revision(state: &AppDataState) -> Result<&AppDataRevision, String> {
    state
        .revisions
        .iter()
        .find(|revision| revision.revision_id == state.active_revision_id)
        .ok_or_else(|| "app data state active revision is absent".into())
}

fn persist_state(root: &Path, state: &AppDataState) -> Result<(), String> {
    persist_json_document(
        &root.join(STATE_FILE),
        state,
        "app data state",
        standard_writer().as_ref(),
    )
    .map_err(|error| error.into_message())
}

fn app_root(apps_root: &Path, app_id: &str) -> PathBuf {
    apps_root.join(".data").join(app_id)
}

fn revision_dir(root: &Path, revision_id: &str) -> PathBuf {
    root.join(REVISIONS_DIR).join(revision_id)
}

fn validate_tree(root: &Path) -> Result<(), String> {
    if !root.is_dir() {
        return Err(format!("app data revision '{}' is missing", root.display()));
    }
    for entry in
        fs::read_dir(root).map_err(|error| format!("read app data revision failed: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read app data entry failed: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect app data entry failed: {error}"))?;
        if file_type.is_symlink() {
            return Err(format!(
                "app data contains unsupported symlink '{}'",
                entry.path().display()
            ));
        }
        if file_type.is_dir() {
            validate_tree(&entry.path())?;
        } else if !file_type.is_file() {
            return Err(format!(
                "app data contains unsupported file type '{}'",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    validate_tree(source)?;
    for entry in
        fs::read_dir(source).map_err(|error| format!("read app data source failed: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read app data source entry failed: {error}"))?;
        let destination_entry = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect app data source entry failed: {error}"))?;
        if file_type.is_dir() {
            fs::create_dir(&destination_entry)
                .map_err(|error| format!("create app data candidate directory failed: {error}"))?;
            copy_tree(&entry.path(), &destination_entry)?;
        } else {
            fs::copy(entry.path(), &destination_entry)
                .map_err(|error| format!("copy app data file failed: {error}"))?;
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&destination_entry)
                .and_then(|file| file.sync_all())
                .map_err(|error| format!("sync app data candidate file failed: {error}"))?;
        }
    }
    Ok(())
}

fn tree_digest(root: &Path) -> Result<String, String> {
    validate_tree(root)?;
    let mut paths = Vec::new();
    collect_paths(root, root, &mut paths)?;
    paths.sort();
    let mut hasher = Sha256::new();
    for relative in paths {
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        let path = root.join(&relative);
        if path.is_dir() {
            hasher.update(b"directory");
            continue;
        }
        let mut file = File::open(&path)
            .map_err(|error| format!("open app data for hashing failed: {error}"))?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("hash app data failed: {error}"))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_paths(root: &Path, current: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(current)
        .map_err(|error| format!("read app data for hashing failed: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read app data entry failed: {error}"))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "app data path escaped its root".to_string())?
            .to_path_buf();
        paths.push(relative);
        if entry
            .file_type()
            .map_err(|error| format!("inspect app data entry failed: {error}"))?
            .is_dir()
        {
            collect_paths(root, &path, paths)?;
        }
    }
    Ok(())
}

fn backend_authority_mode(backend: &Backend) -> Result<BackendAuthorityMode, String> {
    match backend {
        Backend::McpStdio { authority_mode, .. } | Backend::Executable { authority_mode, .. } => {
            Ok(*authority_mode)
        }
        _ => Err("versioned app data requires a local native backend".into()),
    }
}

fn resolve_payload_args(payload_dir: &Path, args: &[String]) -> Vec<String> {
    args.iter()
        .map(|argument| {
            let candidate = payload_dir.join(argument);
            if !argument.contains("..") && !Path::new(argument).is_absolute() && candidate.is_file()
            {
                candidate.to_string_lossy().to_string()
            } else {
                argument.clone()
            }
        })
        .collect()
}

fn resolve_sandboxed_command(command: &str, payload_dir: &Path) -> Result<String, String> {
    let candidate = Path::new(command);
    if candidate.is_absolute() && candidate.is_file() {
        return Ok(candidate.to_string_lossy().to_string());
    }
    let payload_candidate = payload_dir.join(command);
    if payload_candidate.is_file() {
        return Ok(payload_candidate.to_string_lossy().to_string());
    }
    Err(format!(
        "sandboxed app-data migration command not found under payload: {command}"
    ))
}

#[cfg(test)]
mod tests;
