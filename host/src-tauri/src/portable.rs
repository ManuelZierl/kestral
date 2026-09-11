use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use app_host_kernel::durable::DurableKernelState;
use app_host_kernel::primitives::grant::GrantScope;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zip::write::SimpleFileOptions;

use crate::atomic_json::{
    load_json_document, persist_json_document, standard_writer, AtomicJsonError,
};
use crate::host_paths::HostPaths;
use crate::profiles::{
    ProfileIdentity, ProfileRecord, ProfileRegistryService, ProfileSource, ProfileView,
};

const FORMAT_VERSION: u32 = 1;
const MANIFEST: &str = "kestral-portable.json";
const RECOVERY_FILE: &str = "portable-recovery-v1.json";
const REQUEST_FILE: &str = ".kestral-portable-import.json";
const WORK_DIR: &str = ".kestral-portable-import";
const BACKUP_DIR: &str = ".kestral-profile-backups";
const MAX_ENTRIES: usize = 100_000;
const MAX_ARCHIVE_BYTES: u64 = 8 * 1024 * 1024 * 1024;

const ROOT_FILES: &[&str] = &[
    "kernel-state-v1.json",
    "host-config.json",
    "host-secrets.json",
    "chat-threads.json",
    "installed-apps.json",
    "trust-store.json",
    "trusted-notices.json",
    "file-resources-v1.json",
    "mcp-gateway-audit.jsonl",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableManifest {
    format_version: u32,
    source_profile_id: String,
    source_profile_name: String,
    source_profile_slug: String,
    source_app_version: String,
    captured_at: String,
    contents: Vec<PortableContent>,
    apps: Vec<PortableApp>,
    secrets: Vec<PortableSecret>,
    file_resources: Vec<PortableFileResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableContent {
    rel: String,
    sha256: String,
    size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PortableApp {
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub package_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PortableSecret {
    pub owner: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PortableFileResource {
    pub resource_id: String,
    pub display_name: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryStatus {
    version: u32,
    imported_at: String,
    apps: Vec<PortableApp>,
    secrets: Vec<PortableSecret>,
    file_resources: Vec<PortableFileResource>,
}

pub(crate) fn recovery_status(root: &Path) -> Result<Option<RecoveryStatus>, String> {
    let status = load_json_document::<RecoveryStatus>(
        &root.join(RECOVERY_FILE),
        "portable recovery status",
    )?;
    if let Some(status) = status.as_ref() {
        if status.version != FORMAT_VERSION {
            return Err(format!(
                "unsupported portable recovery status version: {}",
                status.version
            ));
        }
        chrono::DateTime::parse_from_rfc3339(&status.imported_at)
            .map_err(|error| format!("portable recovery timestamp is invalid: {error}"))?;
    }
    Ok(status)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum ImportPhase {
    Staged,
    BackupRetained,
    CommitStarted,
    Committed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportRequest {
    version: u32,
    transaction_id: String,
    profile_id: String,
    candidate_sha256: String,
    phase: ImportPhase,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PortableExportResult {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub files: usize,
    pub excluded_secrets: usize,
    pub reinstall_apps: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum PortableImportTarget {
    Preview,
    Fresh { display_name: String, slug: String },
    OverwriteCurrent { confirmation: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PortableImportResult {
    pub target: String,
    pub restart_required: bool,
    pub restart_instructions: String,
    pub apps: Vec<PortableApp>,
    pub secrets: Vec<PortableSecret>,
    pub file_resources: Vec<PortableFileResource>,
}

pub(crate) fn export(
    paths: &HostPaths,
    destination: &Path,
) -> Result<PortableExportResult, String> {
    if destination.extension().and_then(|value| value.to_str()) != Some("zip") {
        return Err("portable export destination must end in .zip".into());
    }
    let entries = collect_export_entries(paths.root())?;
    let source_digest = digest_entries(&entries);
    let apps = read_portable_apps(paths.app_store_path())?;
    let secrets = read_portable_secrets(paths.secrets_index_path())?;
    let file_resources = read_portable_file_resources(paths.file_resource_registry_path())?;
    let contents = entries
        .iter()
        .map(|(rel, bytes)| PortableContent {
            rel: rel.clone(),
            sha256: hex_digest(bytes),
            size: bytes.len() as u64,
        })
        .collect();
    let manifest = PortableManifest {
        format_version: FORMAT_VERSION,
        source_profile_id: paths.profile_id().to_string(),
        source_profile_name: paths.profile_identity().display_name.clone(),
        source_profile_slug: paths.profile_slug().to_string(),
        source_app_version: env!("CARGO_PKG_VERSION").to_string(),
        captured_at: Utc::now().to_rfc3339(),
        contents,
        apps,
        secrets,
        file_resources,
    };
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create export directory failed: {error}"))?;
    }
    let temp = destination.with_file_name(format!(
        ".{}.{}.tmp",
        destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        Uuid::new_v4()
    ));
    write_archive(&temp, &manifest, &entries)?;
    let current_entries = collect_export_entries(paths.root())?;
    if digest_entries(&current_entries) != source_digest {
        let _ = fs::remove_file(&temp);
        return Err("profile changed during export; no archive was committed".into());
    }
    fs::rename(&temp, destination)
        .map_err(|error| format!("commit portable archive failed: {error}"))?;
    sync_parent(destination)?;
    let bytes = fs::read(destination)
        .map_err(|error| format!("verify portable archive failed: {error}"))?;
    Ok(PortableExportResult {
        path: destination.display().to_string(),
        sha256: hex_digest(&bytes),
        size: bytes.len() as u64,
        files: manifest.contents.len(),
        excluded_secrets: manifest.secrets.len(),
        reinstall_apps: manifest.apps.len(),
    })
}

pub(crate) fn import(
    paths: &HostPaths,
    profiles: &mut ProfileRegistryService,
    archive: &Path,
    target: PortableImportTarget,
) -> Result<PortableImportResult, String> {
    let transaction_id = Uuid::new_v4().to_string();
    let work = paths.default_root().join(WORK_DIR).join(&transaction_id);
    let candidate = work.join("candidate");
    fs::create_dir_all(&candidate)
        .map_err(|error| format!("create portable import candidate failed: {error}"))?;
    let manifest = extract_validated_archive(archive, &candidate)?;
    prepare_candidate(&candidate, &manifest)?;

    match target {
        PortableImportTarget::Preview => {
            let _ = fs::remove_dir_all(&work);
            Ok(PortableImportResult {
                target: "preview".into(),
                restart_required: false,
                restart_instructions: String::new(),
                apps: manifest.apps,
                secrets: manifest.secrets,
                file_resources: manifest.file_resources,
            })
        }
        PortableImportTarget::Fresh { display_name, slug } => {
            let manifest_for_copy = manifest.clone();
            let candidate_for_copy = candidate.clone();
            let default_root = paths.default_root().to_path_buf();
            let view = profiles.create_imported_profile(display_name, slug, move |profile| {
                copy_candidate_contents(&candidate_for_copy, &profile.root)?;
                write_recovery(&profile.root, &manifest_for_copy)?;
                let imported_paths = paths_for_record(default_root, profile.clone());
                crate::profile_migration::validate_profile_contents(
                    &profile.root,
                    &imported_paths,
                    false,
                )
            })?;
            let _ = fs::remove_dir_all(&work);
            Ok(result_for_profile("fresh-profile", false, &view, manifest))
        }
        PortableImportTarget::OverwriteCurrent { confirmation } => {
            let expected = format!("RESTORE {}", paths.profile_slug());
            if confirmation != expected {
                return Err(format!(
                    "portable overwrite confirmation must exactly match '{expected}'"
                ));
            }
            rewrite_identity(&candidate, paths.profile_identity())?;
            write_recovery(&candidate, &manifest)?;
            crate::profile_migration::validate_profile_contents(&candidate, paths, false)?;
            let candidate_sha256 = tree_digest(&candidate)?;
            persist_request(
                paths.root(),
                &ImportRequest {
                    version: FORMAT_VERSION,
                    transaction_id,
                    profile_id: paths.profile_id().to_string(),
                    candidate_sha256,
                    phase: ImportPhase::Staged,
                },
            )?;
            Ok(PortableImportResult {
                target: "overwrite-current".into(),
                restart_required: true,
                restart_instructions: "Restart Kestral to apply the validated portable workspace."
                    .into(),
                apps: manifest.apps,
                secrets: manifest.secrets,
                file_resources: manifest.file_resources,
            })
        }
    }
}

pub(crate) fn apply_pending(paths: &HostPaths) -> Result<bool, String> {
    let request_path = paths.root().join(REQUEST_FILE);
    let Some(mut request) =
        load_json_document::<ImportRequest>(&request_path, "portable import request")?
    else {
        return Ok(false);
    };
    if request.version != FORMAT_VERSION || request.profile_id != paths.profile_id() {
        return Err("portable import request does not match the active profile".into());
    }
    let work = paths
        .default_root()
        .join(WORK_DIR)
        .join(&request.transaction_id);
    let candidate = work.join("candidate");
    let backup = paths.root().join(BACKUP_DIR).join(&request.transaction_id);
    if tree_digest(&candidate)? != request.candidate_sha256 {
        return Err(
            "portable import candidate checksum mismatch; current profile preserved".into(),
        );
    }
    crate::profile_migration::validate_profile_contents(&candidate, paths, false)?;
    if request.phase == ImportPhase::Staged {
        remove_if_exists(&backup)?;
        copy_profile_contents(paths.root(), &backup)?;
        request.phase = ImportPhase::BackupRetained;
        persist_request(paths.root(), &request)?;
    }
    if request.phase == ImportPhase::BackupRetained {
        request.phase = ImportPhase::CommitStarted;
        persist_request(paths.root(), &request)?;
    }
    if request.phase == ImportPhase::CommitStarted {
        if let Err(error) = commit_overwrite(paths, &candidate, &backup, || Ok(())) {
            fs::remove_file(&request_path).map_err(|cleanup| {
                format!(
                    "{error}; original profile was restored, but clearing the failed import request failed: {cleanup}"
                )
            })?;
            sync_parent(&request_path)?;
            return Err(error);
        }
        request.phase = ImportPhase::Committed;
        persist_request(paths.root(), &request)?;
    }
    if request.phase == ImportPhase::Committed {
        fs::remove_file(&request_path)
            .map_err(|error| format!("complete portable import failed: {error}"))?;
        let _ = fs::remove_dir_all(&work);
        sync_parent(&request_path)?;
    }
    Ok(true)
}

fn commit_overwrite(
    paths: &HostPaths,
    candidate: &Path,
    backup: &Path,
    after_remove: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    remove_operational_profile_contents(paths)?;
    let commit = after_remove()
        .and_then(|()| copy_candidate_contents(candidate, paths.root()))
        .and_then(|()| crate::profile_migration::validate_profile(paths.root(), paths, false));
    if let Err(error) = commit {
        let rollback = remove_operational_profile_contents(paths)
            .and_then(|()| copy_candidate_contents(backup, paths.root()))
            .and_then(|()| crate::profile_migration::validate_profile(paths.root(), paths, false));
        return Err(match rollback {
            Ok(()) => format!("portable import commit failed; original profile restored: {error}"),
            Err(rollback) => format!(
                "portable import commit failed: {error}; restoring the retained original also failed: {rollback}"
            ),
        });
    }
    Ok(())
}

fn collect_export_entries(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut entries = BTreeMap::new();
    for name in ROOT_FILES {
        let path = root.join(name);
        if path.exists() {
            let archive_name = match *name {
                "installed-apps.json" => continue,
                "file-resources-v1.json" => continue,
                _ => format!("secure/{name}"),
            };
            entries.insert(archive_name, read_regular_file(&path)?);
        }
    }
    let data_root = root.join("apps/.data");
    if data_root.exists() {
        collect_tree(&data_root, &data_root, "apps/.data", &mut entries)?;
    }
    Ok(entries)
}

fn collect_tree(
    root: &Path,
    directory: &Path,
    prefix: &str,
    entries: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let mut children = fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "read portable source '{}' failed: {error}",
                directory.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read portable source entry failed: {error}"))?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = child.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("inspect portable source failed: {error}"))?;
        if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(format!(
                "portable export refuses non-regular path '{}'",
                path.display()
            ));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "portable path escaped data root".to_string())?;
        let rel = format!("{prefix}/{}", slash_path(relative)?);
        if metadata.is_dir() {
            collect_tree(root, &path, prefix, entries)?;
        } else {
            entries.insert(rel, read_regular_file(&path)?);
        }
    }
    Ok(())
}

fn write_archive(
    path: &Path,
    manifest: &PortableManifest,
    entries: &BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create portable archive failed: {error}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600);
    zip.start_file(MANIFEST, options)
        .map_err(|error| format!("write portable manifest failed: {error}"))?;
    zip.write_all(&serde_json::to_vec_pretty(manifest).map_err(|error| error.to_string())?)
        .map_err(|error| format!("write portable manifest failed: {error}"))?;
    for (rel, bytes) in entries {
        zip.start_file(rel, options)
            .map_err(|error| format!("write portable entry '{rel}' failed: {error}"))?;
        zip.write_all(bytes)
            .map_err(|error| format!("write portable entry '{rel}' failed: {error}"))?;
    }
    let file = zip
        .finish()
        .map_err(|error| format!("finish portable archive failed: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("sync portable archive failed: {error}"))
}

fn extract_validated_archive(path: &Path, destination: &Path) -> Result<PortableManifest, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("inspect portable archive failed: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_ARCHIVE_BYTES {
        return Err("portable archive is not a supported regular file".into());
    }
    let file =
        File::open(path).map_err(|error| format!("open portable archive failed: {error}"))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|error| format!("open portable zip failed: {error}"))?;
    if zip.is_empty() || zip.len() > MAX_ENTRIES {
        return Err("portable archive entry count is invalid".into());
    }
    if zip.by_index(0).map_err(|error| error.to_string())?.name() != MANIFEST {
        return Err("portable archive manifest must be the first entry".into());
    }
    let manifest: PortableManifest = {
        let mut entry = zip.by_index(0).map_err(|error| error.to_string())?;
        serde_json::from_reader(&mut entry)
            .map_err(|error| format!("parse portable manifest failed: {error}"))?
    };
    validate_manifest(&manifest)?;
    let expected: BTreeMap<_, _> = manifest
        .contents
        .iter()
        .map(|entry| (entry.rel.as_str(), entry))
        .collect();
    let mut seen = BTreeSet::new();
    for index in 1..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| format!("read portable entry failed: {error}"))?;
        let name = entry.name().to_string();
        validate_archive_path(&name)?;
        if entry.is_dir() || !seen.insert(name.clone()) {
            return Err(format!(
                "portable archive contains invalid or duplicate entry '{name}'"
            ));
        }
        let expected_entry = expected
            .get(name.as_str())
            .ok_or_else(|| format!("portable archive contains unmanifested entry '{name}'"))?;
        if entry.size() != expected_entry.size {
            return Err(format!("portable entry size mismatch: {name}"));
        }
        let target = destination.join(&name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create portable candidate directory failed: {error}"))?;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read portable entry '{name}' failed: {error}"))?;
        if hex_digest(&bytes) != expected_entry.sha256 {
            return Err(format!("portable entry checksum mismatch: {name}"));
        }
        fs::write(&target, bytes)
            .map_err(|error| format!("write portable candidate failed: {error}"))?;
    }
    if seen.len() != expected.len() {
        return Err("portable archive is missing a manifested entry".into());
    }
    Ok(manifest)
}

fn validate_manifest(manifest: &PortableManifest) -> Result<(), String> {
    if manifest.format_version != FORMAT_VERSION {
        return Err(format!(
            "unsupported portable workspace version: {}",
            manifest.format_version
        ));
    }
    chrono::DateTime::parse_from_rfc3339(&manifest.captured_at)
        .map_err(|error| format!("portable capture timestamp is invalid: {error}"))?;
    let mut paths = BTreeSet::new();
    for entry in &manifest.contents {
        validate_archive_path(&entry.rel)?;
        if !paths.insert(&entry.rel) || entry.sha256.len() != 64 {
            return Err("portable manifest contains a duplicate path or invalid checksum".into());
        }
    }
    Ok(())
}

fn prepare_candidate(root: &Path, manifest: &PortableManifest) -> Result<(), String> {
    let secure = root.join("secure");
    if secure.exists() {
        for entry in fs::read_dir(&secure).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            fs::rename(entry.path(), root.join(entry.file_name()))
                .map_err(|error| format!("stage portable store failed: {error}"))?;
        }
        fs::remove_dir(&secure)
            .map_err(|error| format!("remove portable secure directory failed: {error}"))?;
    }
    fs::write(
        root.join("installed-apps.json"),
        serde_json::to_vec_pretty(&json!({"version": 4, "apps": []})).unwrap(),
    )
    .map_err(|error| format!("write dormant app registry failed: {error}"))?;
    fs::write(
        root.join("file-resources-v1.json"),
        serde_json::to_vec_pretty(&json!({"version": 1, "resources": [], "pending_removals": []}))
            .unwrap(),
    )
    .map_err(|error| format!("write unmatched file registry failed: {error}"))?;
    fs::write(root.join("host-secrets.json"), serde_json::to_vec_pretty(&json!({
        "version": 2,
        "secrets": manifest.secrets.iter().map(|secret| json!({"owner": secret.owner, "name": secret.name, "status": "stored"})).collect::<Vec<_>>()
    })).unwrap()).map_err(|error| format!("write secret reference index failed: {error}"))?;
    remove_file(root.join("remote-owner-auth-v1.json"))?;
    remove_file(root.join("remote-owner-pairing-v1.json"))?;
    remove_file(root.join("update-journal.json"))?;
    narrow_imported_kernel(root, &manifest.apps)?;
    disable_unavailable_oauth_default(root)?;
    Ok(())
}

fn narrow_imported_kernel(root: &Path, apps: &[PortableApp]) -> Result<(), String> {
    let path = root.join("kernel-state-v1.json");
    let Some(mut state) = crate::kernel_state::FileKernelStateStore::read_persisted(&path)? else {
        return Ok(());
    };
    let ids: BTreeSet<_> = apps.iter().map(|app| app.id.as_str()).collect();
    state
        .installed_apps
        .retain(|app| !ids.contains(app.manifest.app_id.as_str()));
    for grant in &state.grants {
        let provider = match &grant.scope {
            GrantScope::ExactCapability { provider, .. }
            | GrantScope::AllProviderCapabilities { provider } => provider.as_str(),
        };
        if (ids.contains(grant.holder.as_str()) || ids.contains(provider))
            && !state.revoked_grant_ids.contains(&grant.grant_id)
        {
            state.revoked_grant_ids.push(grant.grant_id.clone());
        }
    }
    write_kernel_state(&path, &state)
}

fn write_kernel_state(path: &Path, state: &DurableKernelState) -> Result<(), String> {
    let state_bytes = serde_json::to_vec(state).map_err(|error| error.to_string())?;
    let document = json!({
        "format_version": 1,
        "state_sha256": hex_digest(&state_bytes),
        "state": state,
    });
    persist_json_document(
        path,
        &document,
        "portable kernel state",
        standard_writer().as_ref(),
    )
    .map_err(AtomicJsonError::into_message)
}

fn disable_unavailable_oauth_default(root: &Path) -> Result<(), String> {
    let path = root.join("host-config.json");
    if !path.exists() {
        return Ok(());
    }
    let mut value: Value =
        serde_json::from_slice(&fs::read(&path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("parse portable host config failed: {error}"))?;
    if let Some(host) = value
        .pointer_mut("/config/host")
        .and_then(Value::as_object_mut)
    {
        host.insert("default_llm_profile".into(), Value::Null);
    }
    fs::write(
        &path,
        serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("rewrite portable host config failed: {error}"))
}

fn read_portable_apps(path: &Path) -> Result<Vec<PortableApp>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
        .map_err(|error| format!("parse installed apps for export failed: {error}"))?;
    let mut result = Vec::new();
    for app in value
        .get("apps")
        .and_then(Value::as_array)
        .ok_or("installed-apps store has no apps array")?
    {
        let active_id = app
            .get("active_revision_id")
            .and_then(Value::as_str)
            .ok_or("installed app has no active revision")?;
        let revision = app
            .get("revisions")
            .and_then(Value::as_array)
            .and_then(|revisions| {
                revisions.iter().find(|revision| {
                    revision.get("revision_id").and_then(Value::as_str) == Some(active_id)
                })
            })
            .ok_or("installed app active revision is missing")?;
        result.push(PortableApp {
            id: required_string(app, "id")?,
            display_name: required_string(revision, "display_name")?,
            version: required_string(revision, "version")?,
            package_digest: required_string(revision, "package_digest")?,
        });
    }
    Ok(result)
}

fn read_portable_secrets(path: &Path) -> Result<Vec<PortableSecret>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
        .map_err(|error| format!("parse secret index for export failed: {error}"))?;
    value
        .get("secrets")
        .and_then(Value::as_array)
        .ok_or_else(|| "secret index has no secrets array".to_string())?
        .iter()
        .map(|entry| {
            Ok(PortableSecret {
                owner: required_string(entry, "owner")?,
                name: required_string(entry, "name")?,
            })
        })
        .collect()
}

fn read_portable_file_resources(path: &Path) -> Result<Vec<PortableFileResource>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
        .map_err(|error| format!("parse file resources for export failed: {error}"))?;
    value
        .get("resources")
        .and_then(Value::as_array)
        .ok_or_else(|| "file resource store has no resources array".to_string())?
        .iter()
        .map(|entry| {
            Ok(PortableFileResource {
                resource_id: required_string(entry, "resource_id")?,
                display_name: required_string(entry, "display_name")?,
                kind: required_string(entry, "kind")?,
            })
        })
        .collect()
}

fn required_string(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("portable source field '{field}' is missing"))
}

fn write_recovery(root: &Path, manifest: &PortableManifest) -> Result<(), String> {
    persist_json_document(
        &root.join(RECOVERY_FILE),
        &RecoveryStatus {
            version: FORMAT_VERSION,
            imported_at: Utc::now().to_rfc3339(),
            apps: manifest.apps.clone(),
            secrets: manifest.secrets.clone(),
            file_resources: manifest.file_resources.clone(),
        },
        "portable recovery status",
        standard_writer().as_ref(),
    )
    .map_err(AtomicJsonError::into_message)
}

fn rewrite_identity(root: &Path, identity: &ProfileIdentity) -> Result<(), String> {
    let record = ProfileRecord {
        profile_id: identity.profile_id.clone(),
        display_name: identity.display_name.clone(),
        slug: identity.slug.clone(),
        root: identity.root.clone(),
        created_at: identity.created_at.clone(),
    };
    fs::write(
        root.join("kestral-profile.json"),
        serde_json::to_vec_pretty(
            &json!({"version": 1, "profile": record, "source": identity.source}),
        )
        .unwrap(),
    )
    .map_err(|error| format!("rewrite portable profile identity failed: {error}"))
}

fn paths_for_record(default_root: PathBuf, profile: ProfileRecord) -> HostPaths {
    HostPaths::new(
        default_root,
        ProfileIdentity {
            profile_id: profile.profile_id,
            display_name: profile.display_name,
            slug: profile.slug,
            root: profile.root,
            created_at: profile.created_at,
            source: ProfileSource::Managed,
        },
        false,
    )
}

fn result_for_profile(
    target: &str,
    restart_required: bool,
    view: &ProfileView,
    manifest: PortableManifest,
) -> PortableImportResult {
    PortableImportResult {
        target: target.into(),
        restart_required,
        restart_instructions: view.restart_instructions.clone(),
        apps: manifest.apps,
        secrets: manifest.secrets,
        file_resources: manifest.file_resources,
    }
}

fn persist_request(root: &Path, request: &ImportRequest) -> Result<(), String> {
    persist_json_document(
        &root.join(REQUEST_FILE),
        request,
        "portable import request",
        standard_writer().as_ref(),
    )
    .map_err(AtomicJsonError::into_message)
}

fn remove_operational_profile_contents(paths: &HostPaths) -> Result<(), String> {
    for entry in fs::read_dir(paths.root()).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path == paths.root().join(REQUEST_FILE)
            || crate::profiles::preserve_during_profile_reset(
                paths.root(),
                paths.default_root(),
                &path,
            )
            || entry.file_name() == BACKUP_DIR
            || entry.file_name() == WORK_DIR
        {
            continue;
        }
        remove_if_exists(&path)?;
    }
    Ok(())
}

fn copy_profile_contents(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if matches!(
            entry.file_name().to_str(),
            Some(
                REQUEST_FILE
                    | WORK_DIR
                    | BACKUP_DIR
                    | "kernel-state-v1.lock"
                    | "kestral-profiles.lock"
            )
        ) {
            continue;
        }
        copy_entry(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

fn copy_candidate_contents(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        copy_entry(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

fn copy_entry(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "portable import refuses symlink '{}'",
            source.display()
        ));
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination).map_err(|error| error.to_string())?;
        for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            copy_entry(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        fs::copy(source, destination)
            .map_err(|error| format!("copy portable file failed: {error}"))?;
    } else {
        return Err("portable import refuses non-regular file".into());
    }
    Ok(())
}

fn tree_digest(root: &Path) -> Result<String, String> {
    let mut entries = BTreeMap::new();
    collect_tree(root, root, "", &mut entries)?;
    Ok(digest_entries(&entries))
}

fn digest_entries(entries: &BTreeMap<String, Vec<u8>>) -> String {
    let mut hasher = Sha256::new();
    for (path, bytes) in entries {
        hasher.update(path.as_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    format!("{:x}", hasher.finalize())
}

fn validate_archive_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!("portable archive path is unsafe: {value}"));
    }
    Ok(())
}

fn slash_path(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| "portable workspace requires UTF-8 paths".into())
}

fn read_regular_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "portable export refuses non-regular file '{}'",
            path.display()
        ));
    }
    fs::read(path)
        .map_err(|error| format!("read portable file '{}' failed: {error}", path.display()))
}

fn remove_file(path: PathBuf) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|error| format!("remove '{}' failed: {error}", path.display()))
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sync_parent(_path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    if let Some(parent) = _path.parent() {
        File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|error| format!("sync portable directory failed: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
