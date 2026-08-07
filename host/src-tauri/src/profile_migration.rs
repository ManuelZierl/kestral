use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use app_host_kernel::primitives::grant::{DataScope, Grant, GrantCondition, GrantScope};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::atomic_json::{
    load_json_document, persist_json_document, standard_writer, AtomicJsonError,
};
use crate::host_paths::HostPaths;
use crate::profiles::{ProfileRecord, ProfileSource};

const JOURNAL_VERSION: u32 = 1;
const JOURNAL_FILE: &str = ".kestral-profile-migration.json";
const WORK_DIR: &str = ".kestral-profile-migration";
const BACKUP_DIR: &str = ".kestral-profile-backups";
const LEGACY_UPDATE_JOURNAL: &str = "update-journal.jsonl";
const UPDATE_JOURNAL: &str = "update-journal.json";
const PROFILE_LOCK_FILE: &str = "kernel-state-v1.lock";
const REGISTRY_LOCK_FILE: &str = "kestral-profiles.lock";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DurableStoreOwner {
    pub(crate) store: &'static str,
    pub(crate) owner: &'static str,
    pub(crate) format: &'static str,
    pub(crate) location: &'static str,
}

/// Canonical migration ownership inventory. Dynamic app payload/data paths are
/// represented once because their bytes are copied as a tree under one owner.
pub(crate) const DURABLE_STORE_OWNERS: &[DurableStoreOwner] = &[
    DurableStoreOwner {
        store: "profile registry and identity",
        owner: "profiles",
        format: "v1",
        location: "default root and profile root",
    },
    DurableStoreOwner {
        store: "host config",
        owner: "config",
        format: "storage v3 / domain v2",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "secret-reference index and vault identities",
        owner: "config",
        format: "index v2 / derived vault accounts",
        location: "profile root and OS vault",
    },
    DurableStoreOwner {
        store: "kernel state",
        owner: "kernel_state",
        format: "v1",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "chat threads",
        owner: "chat_store",
        format: "v4",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "installed-app registry and payload records",
        owner: "app_manager",
        format: "registry v4 / package format 1",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "app update journal",
        owner: "app_manager::update_journal",
        format: "v2 JSON document",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "app-data revision index and backups",
        owner: "app_data",
        format: "index v1 / opaque revisions",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "private surface state",
        owner: "surface_state envelope / app-owned values",
        format: "envelope v2 / opaque values",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "host-managed app data",
        owner: "managed_data",
        format: "store v1 / package-declared schemas",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "trusted notices",
        owner: "chrome::notices",
        format: "v1",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "publisher trust",
        owner: "publisher_trust",
        format: "v1",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "file resources",
        owner: "file_resources",
        format: "v1",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "remote-owner passkeys",
        owner: "remote_auth",
        format: "v1",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "pending reset",
        owner: "system_reset",
        format: "v1",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "gateway audit",
        owner: "mcp_gateway",
        format: "JSONL event v1",
        location: "profile root",
    },
    DurableStoreOwner {
        store: "browser custom themes",
        owner: "frontend theme store",
        format: "v2",
        location: "browser localStorage",
    },
    DurableStoreOwner {
        store: "pending-send recovery",
        owner: "frontend chatThreads store",
        format: "v1",
        location: "browser localStorage",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EphemeralStateDisposition {
    pub(crate) state: &'static str,
    pub(crate) owner: &'static str,
    pub(crate) restart: &'static str,
}

pub(crate) const EPHEMERAL_STATE_DISPOSITIONS: &[EphemeralStateDisposition] = &[
    EphemeralStateDisposition {
        state: "workers",
        owner: "node_worker and app_manager",
        restart: "recreate per invocation or activation",
    },
    EphemeralStateDisposition {
        state: "MCP sessions",
        owner: "mcp adapter",
        restart: "disconnect; require explicit reconnect",
    },
    EphemeralStateDisposition {
        state: "pending approvals and OAuth ceremonies",
        owner: "trusted chrome",
        restart: "deny or abandon",
    },
    EphemeralStateDisposition {
        state: "owner sessions",
        owner: "remote_auth",
        restart: "revoke",
    },
    EphemeralStateDisposition {
        state: "surfaces",
        owner: "kernel surface manager",
        restart: "recreate from declarations",
    },
    EphemeralStateDisposition {
        state: "leases",
        owner: "kernel router",
        restart: "discard",
    },
    EphemeralStateDisposition {
        state: "event inboxes",
        owner: "kernel router",
        restart: "discard",
    },
    EphemeralStateDisposition {
        state: "transient progress",
        owner: "invocation runtime",
        restart: "interrupt",
    },
    EphemeralStateDisposition {
        state: "live streams",
        owner: "chat and remote transports",
        restart: "interrupt",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum MigrationPhase {
    Planned,
    CandidateStaged,
    CandidateValidated,
    BackupRetained,
    CommitStarted,
    Committed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationJournal {
    version: u32,
    transaction_id: String,
    profile_id: String,
    source_sha256: String,
    candidate_sha256: Option<String>,
    phase: MigrationPhase,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileIdentityDocument {
    version: u32,
    profile: ProfileRecord,
    source: ProfileSource,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileRegistryDocument {
    version: u32,
    selected_next_launch_profile_id: String,
    profiles: Vec<ProfileRecord>,
}

pub(crate) fn run(paths: &HostPaths) -> Result<(), String> {
    debug_assert!(DURABLE_STORE_OWNERS.iter().all(|entry| {
        !entry.store.is_empty()
            && !entry.owner.is_empty()
            && !entry.format.is_empty()
            && !entry.location.is_empty()
    }));
    debug_assert!(EPHEMERAL_STATE_DISPOSITIONS.iter().all(|entry| {
        !entry.state.is_empty() && !entry.owner.is_empty() && !entry.restart.is_empty()
    }));
    run_with_observer(paths, |_| Ok(()))
}

fn run_with_observer(
    paths: &HostPaths,
    observer: impl Fn(MigrationPhase) -> Result<(), String>,
) -> Result<(), String> {
    let journal_path = paths.root().join(JOURNAL_FILE);
    let reset_path = paths.root().join("kestral-system-reset.json");
    if reset_path.exists() && !journal_path.exists() {
        crate::system_reset::validate_persisted(paths)?;
        return Ok(());
    }
    validate_profile(paths.root(), paths, true)?;
    let legacy_journal = paths.root().join(LEGACY_UPDATE_JOURNAL);
    let current_journal = paths.root().join(UPDATE_JOURNAL);
    if !journal_path.exists() && !legacy_journal.exists() {
        return Ok(());
    }
    if !journal_path.exists() && current_journal.exists() {
        return Err(format!(
            "both legacy and current app update journals exist; preserved '{}' and '{}'",
            legacy_journal.display(),
            current_journal.display()
        ));
    }

    let mut journal = if let Some(journal) =
        load_json_document::<MigrationJournal>(&journal_path, "profile migration journal")?
    {
        validate_journal(&journal, paths)?;
        journal
    } else {
        let journal = MigrationJournal {
            version: JOURNAL_VERSION,
            transaction_id: Uuid::new_v4().to_string(),
            profile_id: paths.profile_id().to_string(),
            source_sha256: tree_digest(paths.root(), true)?,
            candidate_sha256: None,
            phase: MigrationPhase::Planned,
        };
        persist_journal(&journal_path, &journal)?;
        observer(MigrationPhase::Planned)?;
        journal
    };

    let work_root = paths.root().join(WORK_DIR).join(&journal.transaction_id);
    let candidate_root = work_root.join("candidate");
    let backup_root = paths.root().join(BACKUP_DIR).join(&journal.transaction_id);

    loop {
        match journal.phase {
            MigrationPhase::Planned => {
                remove_dir_if_present(&candidate_root)?;
                copy_profile_tree(paths.root(), &candidate_root)?;
                apply_alpha_1_path_migrations(&candidate_root)?;
                set_phase(
                    &journal_path,
                    &mut journal,
                    MigrationPhase::CandidateStaged,
                    &observer,
                )?;
            }
            MigrationPhase::CandidateStaged => {
                validate_profile(&candidate_root, paths, false)?;
                validate_authority_not_widened(paths.root(), &candidate_root)?;
                journal.candidate_sha256 = Some(tree_digest(&candidate_root, false)?);
                set_phase(
                    &journal_path,
                    &mut journal,
                    MigrationPhase::CandidateValidated,
                    &observer,
                )?;
            }
            MigrationPhase::CandidateValidated => {
                require_source_unchanged(paths.root(), &journal)?;
                remove_dir_if_present(&backup_root)?;
                copy_profile_tree(paths.root(), &backup_root)?;
                let backup_digest = tree_digest(&backup_root, false)?;
                if backup_digest != journal.source_sha256 {
                    return Err("profile migration backup verification failed".into());
                }
                set_phase(
                    &journal_path,
                    &mut journal,
                    MigrationPhase::BackupRetained,
                    &observer,
                )?;
            }
            MigrationPhase::BackupRetained => {
                require_source_unchanged(paths.root(), &journal)?;
                set_phase(
                    &journal_path,
                    &mut journal,
                    MigrationPhase::CommitStarted,
                    &observer,
                )?;
            }
            MigrationPhase::CommitStarted => {
                commit_alpha_1_path_migrations(paths.root(), &candidate_root)?;
                validate_profile(paths.root(), paths, false)?;
                validate_authority_not_widened(&backup_root, paths.root())?;
                let committed = tree_digest(paths.root(), true)?;
                if Some(committed) != journal.candidate_sha256 {
                    return Err("committed profile does not match the validated candidate".into());
                }
                set_phase(
                    &journal_path,
                    &mut journal,
                    MigrationPhase::Committed,
                    &observer,
                )?;
            }
            MigrationPhase::Committed => {
                validate_profile(paths.root(), paths, false)?;
                remove_dir_if_present(&work_root)?;
                fs::remove_file(&journal_path).map_err(|error| {
                    format!(
                        "remove completed profile migration journal '{}' failed: {error}",
                        journal_path.display()
                    )
                })?;
                sync_directory(paths.root())?;
                return Ok(());
            }
        }
    }
}

fn validate_journal(journal: &MigrationJournal, paths: &HostPaths) -> Result<(), String> {
    if journal.version != JOURNAL_VERSION {
        return Err(format!(
            "unsupported profile migration journal version: {}",
            journal.version
        ));
    }
    if journal.profile_id != paths.profile_id() {
        return Err("profile migration journal does not match the active profile".into());
    }
    if journal.transaction_id.is_empty() {
        return Err("profile migration transaction id cannot be empty".into());
    }
    Ok(())
}

fn set_phase(
    path: &Path,
    journal: &mut MigrationJournal,
    phase: MigrationPhase,
    observer: &impl Fn(MigrationPhase) -> Result<(), String>,
) -> Result<(), String> {
    journal.phase = phase;
    persist_journal(path, journal)?;
    observer(phase)
}

fn persist_journal(path: &Path, journal: &MigrationJournal) -> Result<(), String> {
    persist_json_document(
        path,
        journal,
        "profile migration journal",
        standard_writer().as_ref(),
    )
    .map_err(AtomicJsonError::into_message)
}

fn require_source_unchanged(root: &Path, journal: &MigrationJournal) -> Result<(), String> {
    let actual = tree_digest(root, true)?;
    if actual != journal.source_sha256 {
        return Err(
            "profile changed while its migration candidate was staged; original preserved".into(),
        );
    }
    Ok(())
}

fn apply_alpha_1_path_migrations(candidate_root: &Path) -> Result<(), String> {
    let legacy = candidate_root.join(LEGACY_UPDATE_JOURNAL);
    let current = candidate_root.join(UPDATE_JOURNAL);
    if legacy.exists() {
        if current.exists() {
            return Err("candidate contains both legacy and current app update journals".into());
        }
        fs::rename(&legacy, &current).map_err(|error| {
            format!(
                "rename staged app update journal '{}' to '{}' failed: {error}",
                legacy.display(),
                current.display()
            )
        })?;
    }
    // Pairing is a short-lived ceremony marker, not authority to carry through
    // a profile migration. A new ceremony can be started after recovery.
    remove_file_if_present(&candidate_root.join("remote-owner-pairing-v1.json"))?;
    Ok(())
}

fn commit_alpha_1_path_migrations(root: &Path, candidate_root: &Path) -> Result<(), String> {
    let legacy = root.join(LEGACY_UPDATE_JOURNAL);
    let current = root.join(UPDATE_JOURNAL);
    let staged = candidate_root.join(UPDATE_JOURNAL);
    if staged.exists() {
        let expected = file_digest(&staged)?;
        if current.exists() {
            if file_digest(&current)? != expected {
                return Err(
                    "current app update journal differs from the validated candidate".into(),
                );
            }
        } else {
            fs::copy(&staged, &current).map_err(|error| {
                format!(
                    "commit app update journal '{}' failed: {error}",
                    current.display()
                )
            })?;
            sync_file(&current)?;
            sync_directory(root)?;
        }
    }
    remove_file_if_present(&legacy)?;
    remove_file_if_present(&root.join("remote-owner-pairing-v1.json"))?;
    sync_directory(root)
}

fn validate_profile(root: &Path, paths: &HostPaths, allow_legacy: bool) -> Result<(), String> {
    validate_identity(root, paths)?;
    validate_registry(root, paths)?;
    crate::config::HostConfigService::validate_persisted_documents(
        &root.join("host-config.json"),
        &root.join("host-secrets.json"),
    )?;
    crate::kernel_state::FileKernelStateStore::validate_persisted(
        &root.join("kernel-state-v1.json"),
    )?;
    crate::chat_store::ChatStore::validate_persisted(&root.join("chat-threads.json"))?;
    let update_journal = if allow_legacy && root.join(LEGACY_UPDATE_JOURNAL).exists() {
        root.join(LEGACY_UPDATE_JOURNAL)
    } else {
        root.join(UPDATE_JOURNAL)
    };
    crate::app_manager::AppManager::validate_persisted_profile(
        &root.join("installed-apps.json"),
        &root.join("trust-store.json"),
        &root.join("apps"),
        paths.app_records_root(),
        &update_journal,
    )?;
    if root.join("trusted-notices.json").exists() {
        crate::chrome::TrustedNoticeStore::new(root.join("trusted-notices.json"))
            .map_err(|error| error.to_string())?;
    }
    if root.join("file-resources-v1.json").exists() {
        crate::file_resources::FileResourceRegistryService::new(
            root.join("file-resources-v1.json"),
        )?;
    }
    crate::remote_auth::validate_persisted_owner_auth(&root.join("remote-owner-auth-v1.json"))?;
    if root == paths.root() {
        crate::system_reset::validate_persisted(paths)?;
    } else if root.join("kestral-system-reset.json").exists() {
        validate_staged_reset(&root.join("kestral-system-reset.json"), paths.profile_id())?;
    }
    crate::mcp_gateway::AuditLog::validate_persisted(&root.join("mcp-gateway-audit.jsonl"))?;
    crate::surface_state::SurfaceStateStore::validate_all(&root.join("apps/.data"))?;
    crate::managed_data::ManagedDataStore::validate_all(&root.join("apps/.data"))?;
    crate::app_data::validate_all(&root.join("apps/.data"))?;
    validate_package_documents(&root.join("apps"))?;
    Ok(())
}

fn validate_identity(root: &Path, paths: &HostPaths) -> Result<(), String> {
    let identity_path = root.join("kestral-profile.json");
    let document =
        load_json_document::<ProfileIdentityDocument>(&identity_path, "profile identity")?
            .ok_or_else(|| format!("missing profile identity: {}", identity_path.display()))?;
    if document.version != 1 {
        return Err(format!(
            "unsupported profile identity version: {}",
            document.version
        ));
    }
    let expected = paths.profile_identity();
    if document.profile.profile_id != expected.profile_id
        || document.profile.display_name != expected.display_name
        || document.profile.slug != expected.slug
        || document.profile.root != expected.root
        || document.profile.created_at != expected.created_at
        || document.source != expected.source
    {
        return Err("profile identity disagrees with the selected runtime profile".into());
    }
    Ok(())
}

fn validate_registry(root: &Path, paths: &HostPaths) -> Result<(), String> {
    let registry_path = if paths.root() == paths.default_root() {
        root.join("kestral-profiles.json")
    } else {
        paths.profile_registry_path()
    };
    let document =
        load_json_document::<ProfileRegistryDocument>(&registry_path, "profile registry")?
            .ok_or_else(|| format!("missing profile registry: {}", registry_path.display()))?;
    if document.version != 1 {
        return Err(format!(
            "unsupported profile registry version: {}",
            document.version
        ));
    }
    if document.selected_next_launch_profile_id.is_empty() {
        return Err("profile registry has an empty next-launch profile id".into());
    }
    let mut ids = BTreeSet::new();
    for profile in &document.profiles {
        if !ids.insert(profile.profile_id.as_str()) {
            return Err("profile registry contains duplicate profile ids".into());
        }
    }
    if paths.profile_source() == ProfileSource::Managed {
        let expected = paths.profile_identity();
        let Some(record) = document
            .profiles
            .iter()
            .find(|profile| profile.profile_id == expected.profile_id)
        else {
            return Err("active managed profile is absent from the profile registry".into());
        };
        if record.display_name != expected.display_name
            || record.slug != expected.slug
            || record.root != expected.root
            || record.created_at != expected.created_at
        {
            return Err("profile registry and active profile identity disagree".into());
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StagedResetRequest {
    version: u32,
    profile_id: String,
    requested_at: String,
}

fn validate_staged_reset(path: &Path, profile_id: &str) -> Result<(), String> {
    let request = load_json_document::<StagedResetRequest>(path, "system reset request")?
        .ok_or_else(|| "staged system reset request disappeared".to_string())?;
    if request.version != 1 {
        return Err(format!(
            "unsupported system reset request version: {}",
            request.version
        ));
    }
    if request.profile_id != profile_id {
        return Err("system reset request does not match the active profile identity".into());
    }
    chrono::DateTime::parse_from_rfc3339(&request.requested_at)
        .map_err(|error| format!("system reset request timestamp is invalid: {error}"))?;
    Ok(())
}

fn validate_versioned_json(
    path: &Path,
    version_field: &str,
    expected: u64,
    label: &str,
) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let value: Value = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("read {label} failed: {error}"))?,
    )
    .map_err(|error| format!("parse {label} failed: {error}"))?;
    let actual = value.get(version_field).and_then(Value::as_u64);
    if actual != Some(expected) {
        return Err(format!(
            "unsupported {label} version: {:?}; expected {expected}",
            actual
        ));
    }
    Ok(())
}

fn validate_package_documents(apps_root: &Path) -> Result<(), String> {
    let revisions_root_name = "revisions";
    if !apps_root.exists() {
        return Ok(());
    }
    visit_files(apps_root, &mut |path| {
        if path.file_name().and_then(|name| name.to_str()) == Some("app.json")
            && path
                .components()
                .any(|part| part.as_os_str() == revisions_root_name)
        {
            validate_versioned_json(path, "format_version", 1, "app package")?;
        }
        Ok(())
    })
}

fn validate_authority_not_widened(source_root: &Path, candidate_root: &Path) -> Result<(), String> {
    let source = crate::kernel_state::FileKernelStateStore::read_persisted(
        &source_root.join("kernel-state-v1.json"),
    )?;
    let candidate = crate::kernel_state::FileKernelStateStore::read_persisted(
        &candidate_root.join("kernel-state-v1.json"),
    )?;
    let (source, candidate) = match (source, candidate) {
        (Some(source), Some(candidate)) => (source, candidate),
        (None, None) => return Ok(()),
        _ => return Err("migration cannot add or remove the durable kernel projection".into()),
    };
    validate_kernel_authority(&source, &candidate)
}

fn validate_kernel_authority(
    source: &app_host_kernel::durable::DurableKernelState,
    candidate: &app_host_kernel::durable::DurableKernelState,
) -> Result<(), String> {
    let source_grants: BTreeMap<_, _> = source
        .grants
        .iter()
        .map(|grant| (grant.grant_id.clone(), grant))
        .collect();
    for grant in &candidate.grants {
        let Some(previous) = source_grants.get(&grant.grant_id) else {
            return Err(format!("migration introduced grant '{}'", grant.grant_id));
        };
        validate_grant_not_widened(previous, grant)?;
    }
    let candidate_ids: BTreeSet<_> = candidate
        .grants
        .iter()
        .map(|grant| &grant.grant_id)
        .collect();
    if source_grants
        .keys()
        .any(|grant_id| !candidate_ids.contains(grant_id))
    {
        return Err("migration removed an issued grant fact".into());
    }
    let candidate_revoked: BTreeSet<_> = candidate.revoked_grant_ids.iter().collect();
    if source
        .revoked_grant_ids
        .iter()
        .any(|grant_id| !candidate_revoked.contains(grant_id))
    {
        return Err("migration revived a revoked grant".into());
    }
    Ok(())
}

fn validate_grant_not_widened(previous: &Grant, candidate: &Grant) -> Result<(), String> {
    if previous.holder != candidate.holder
        || previous.origin != candidate.origin
        || previous.issued_at != candidate.issued_at
        || !scope_is_same_or_narrower(&previous.scope, &candidate.scope)
        || !data_scope_is_same_or_narrower(&previous.data_scope, &candidate.data_scope)
        || condition_strength(candidate.condition) < condition_strength(previous.condition)
        || expiry_extended(previous, candidate)
    {
        return Err(format!(
            "migration widened or rewrote grant '{}'",
            previous.grant_id
        ));
    }
    Ok(())
}

fn scope_is_same_or_narrower(previous: &GrantScope, candidate: &GrantScope) -> bool {
    match (previous, candidate) {
        (
            GrantScope::ExactCapability {
                provider: left_provider,
                capability: left_capability,
            },
            GrantScope::ExactCapability {
                provider: right_provider,
                capability: right_capability,
            },
        ) => left_provider == right_provider && left_capability == right_capability,
        (
            GrantScope::AllProviderCapabilities {
                provider: left_provider,
            },
            GrantScope::AllProviderCapabilities {
                provider: right_provider,
            },
        ) => left_provider == right_provider,
        (
            GrantScope::AllProviderCapabilities {
                provider: left_provider,
            },
            GrantScope::ExactCapability {
                provider: right_provider,
                ..
            },
        ) => left_provider == right_provider,
        _ => false,
    }
}

fn data_scope_is_same_or_narrower(previous: &DataScope, candidate: &DataScope) -> bool {
    match (previous, candidate) {
        (DataScope::None, DataScope::None) => true,
        (DataScope::AllResources, _) => true,
        (
            DataScope::Resources {
                resource_ids: previous,
            },
            DataScope::Resources {
                resource_ids: candidate,
            },
        ) => candidate.iter().all(|resource| previous.contains(resource)),
        _ => false,
    }
}

fn condition_strength(condition: GrantCondition) -> u8 {
    match condition {
        GrantCondition::Silent => 0,
        GrantCondition::Notify => 1,
        GrantCondition::RequiresApproval => 2,
    }
}

fn expiry_extended(previous: &Grant, candidate: &Grant) -> bool {
    match (previous.expires_at, candidate.expires_at) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(previous), Some(candidate)) => candidate > previous,
    }
}

fn copy_profile_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| {
        format!(
            "create migration directory '{}' failed: {error}",
            destination.display()
        )
    })?;
    let mut entries = fs::read_dir(source)
        .map_err(|error| format!("read profile '{}' failed: {error}", source.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read profile entry failed: {error}"))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if excluded_root_name(&entry.file_name()) {
            continue;
        }
        copy_entry(&entry.path(), &destination.join(entry.file_name()))?;
    }
    sync_directory(destination)
}

fn copy_entry(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|error| {
        format!(
            "inspect migration source '{}' failed: {error}",
            source.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "profile migration refuses symlink '{}'",
            source.display()
        ));
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination).map_err(|error| {
            format!(
                "create migration directory '{}' failed: {error}",
                destination.display()
            )
        })?;
        let mut entries = fs::read_dir(source)
            .map_err(|error| {
                format!(
                    "read migration directory '{}' failed: {error}",
                    source.display()
                )
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read migration entry failed: {error}"))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            copy_entry(&entry.path(), &destination.join(entry.file_name()))?;
        }
        return sync_directory(destination);
    }
    if !metadata.is_file() {
        return Err(format!(
            "profile migration refuses non-file '{}'",
            source.display()
        ));
    }
    fs::copy(source, destination)
        .map_err(|error| format!("copy migration file '{}' failed: {error}", source.display()))?;
    sync_file(destination)
}

fn tree_digest(root: &Path, exclude_runtime: bool) -> Result<String, String> {
    let mut hasher = Sha256::new();
    digest_directory(root, root, exclude_runtime, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn digest_directory(
    root: &Path,
    directory: &Path,
    exclude_runtime: bool,
    hasher: &mut Sha256,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "read profile digest directory '{}' failed: {error}",
                directory.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read profile digest entry failed: {error}"))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if exclude_runtime && directory == root && excluded_root_name(&entry.file_name()) {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "inspect profile digest path '{}' failed: {error}",
                path.display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "profile migration refuses symlink '{}'",
                path.display()
            ));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "profile digest path escaped its root".to_string())?;
        let relative = relative
            .to_str()
            .ok_or_else(|| "profile migration requires UTF-8 paths".to_string())?;
        hasher.update(relative.len().to_le_bytes());
        hasher.update(relative.as_bytes());
        if metadata.is_dir() {
            hasher.update(b"directory");
            digest_directory(root, &path, exclude_runtime, hasher)?;
        } else if metadata.is_file() {
            hasher.update(b"file");
            let bytes = fs::read(&path).map_err(|error| {
                format!("read profile file '{}' failed: {error}", path.display())
            })?;
            hasher.update(bytes.len().to_le_bytes());
            hasher.update(bytes);
        } else {
            return Err(format!(
                "profile migration refuses non-file '{}'",
                path.display()
            ));
        }
    }
    Ok(())
}

fn excluded_root_name(name: &std::ffi::OsStr) -> bool {
    name == WORK_DIR
        || name == BACKUP_DIR
        || name == JOURNAL_FILE
        || name == PROFILE_LOCK_FILE
        || name == REGISTRY_LOCK_FILE
}

fn file_digest(path: &Path) -> Result<String, String> {
    fs::read(path)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .map_err(|error| format!("read '{}' for checksum failed: {error}", path.display()))
}

fn visit_files(
    path: &Path,
    visitor: &mut impl FnMut(&Path) -> Result<(), String>,
) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect '{}' failed: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "profile migration refuses symlink '{}'",
            path.display()
        ));
    }
    if metadata.is_file() {
        return visitor(path);
    }
    for entry in
        fs::read_dir(path).map_err(|error| format!("read '{}' failed: {error}", path.display()))?
    {
        visit_files(
            &entry
                .map_err(|error| format!("read directory entry failed: {error}"))?
                .path(),
            visitor,
        )?;
    }
    Ok(())
}

fn remove_file_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("remove '{}' failed: {error}", path.display())),
    }
}

fn remove_dir_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("remove '{}' failed: {error}", path.display())),
    }
}

fn sync_file(path: &Path) -> Result<(), String> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("sync '{}' failed: {error}", path.display()))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync directory '{}' failed: {error}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests;
