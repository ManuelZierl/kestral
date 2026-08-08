use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::atomic_json::{
    load_json_document, persist_json_document, standard_writer, AtomicFileWriter, AtomicJsonError,
};

const REGISTRY_VERSION: u32 = 1;
const IDENTITY_VERSION: u32 = 1;
const PROFILE_REGISTRY_FILE: &str = "kestral-profiles.json";
const PROFILE_IDENTITY_FILE: &str = "kestral-profile.json";
const PROFILE_TRANSITION_FILE: &str = "kestral-profile-transition.json";
pub(crate) const PROFILE_REGISTRY_LOCK_FILE: &str = "kestral-profiles.lock";
pub(crate) const KERNEL_STATE_LOCK_FILE: &str = "kernel-state-v1.lock";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ProfileSource {
    Managed,
    CustomDataDir,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileIdentity {
    pub profile_id: String,
    pub display_name: String,
    pub slug: String,
    pub root: PathBuf,
    pub created_at: String,
    pub source: ProfileSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileRecord {
    pub profile_id: String,
    pub display_name: String,
    pub slug: String,
    pub root: PathBuf,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileRegistryDocument {
    version: u32,
    selected_next_launch_profile_id: String,
    profiles: Vec<ProfileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileTransitionDocument {
    version: u32,
    transition: ProfileTransition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum ProfileTransition {
    Create { profile: ProfileRecord },
    Delete { profile: ProfileRecord },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileIdentityDocument {
    version: u32,
    profile: ProfileRecord,
    source: ProfileSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileView {
    #[serde(flatten)]
    pub profile: ProfileRecord,
    pub current_runtime: bool,
    pub selected_for_next_launch: bool,
    pub source: ProfileSource,
    pub launch_args: Vec<String>,
    pub restart_instructions: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileLaunchInstructions {
    pub launch_args: Vec<String>,
    pub restart_instructions: String,
}

/// The CLI relaunch command for a profile: `--profile <slug>` for a managed
/// profile, `--data-dir <root>` for anything else. Shared by
/// `HostPaths::launch_instructions` (the active profile) and
/// `ProfileRegistryService::launch_instructions` (any listed profile).
pub(crate) fn launch_instructions_for(
    source: ProfileSource,
    slug: &str,
    root: &Path,
) -> ProfileLaunchInstructions {
    if source == ProfileSource::Managed {
        ProfileLaunchInstructions {
            launch_args: vec!["--profile".into(), slug.to_string()],
            restart_instructions: format!("Restart Kestral with: --profile {slug}"),
        }
    } else {
        ProfileLaunchInstructions {
            launch_args: vec!["--data-dir".into(), root.display().to_string()],
            restart_instructions: format!("Restart Kestral with: --data-dir {}", root.display()),
        }
    }
}

pub(crate) struct ProfileRegistryService {
    default_root: PathBuf,
    path: PathBuf,
    document: ProfileRegistryDocument,
    writer: Arc<dyn AtomicFileWriter>,
}

impl ProfileRegistryService {
    pub(crate) fn open(default_root: PathBuf) -> Result<Self, String> {
        Self::with_writer(default_root, standard_writer())
    }

    pub(crate) fn with_writer(
        default_root: PathBuf,
        writer: Arc<dyn AtomicFileWriter>,
    ) -> Result<Self, String> {
        if let Some(parent) = registry_path(&default_root).parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create profile registry directory failed: {error}"))?;
        }
        let path = registry_path(&default_root);
        let service = if let Some(document) =
            load_json_document::<ProfileRegistryDocument>(&path, "profile registry")
                .map_err(|error| format!("load profile registry failed: {error}"))?
        {
            if document.version != REGISTRY_VERSION {
                return Err(format!(
                    "unsupported profile registry version: {}",
                    document.version
                ));
            }
            Self {
                default_root,
                path,
                document: ProfileRegistryDocument {
                    version: document.version,
                    selected_next_launch_profile_id: document.selected_next_launch_profile_id,
                    profiles: document.profiles,
                },
                writer,
            }
        } else {
            let profile = match load_identity_if_present(&default_root)? {
                Some(identity) if identity.source == ProfileSource::Managed => ProfileRecord {
                    profile_id: identity.profile_id,
                    display_name: identity.display_name,
                    slug: identity.slug,
                    root: identity.root,
                    created_at: identity.created_at,
                },
                Some(_) => {
                    return Err(
                        "default profile identity must use the managed profile source".into(),
                    )
                }
                None => default_profile(&default_root, writer.as_ref())?,
            };
            let document = ProfileRegistryDocument {
                version: REGISTRY_VERSION,
                selected_next_launch_profile_id: profile.profile_id.clone(),
                profiles: vec![profile],
            };
            persist_json_document(&path, &document, "profile registry", writer.as_ref())
                .map_err(AtomicJsonError::into_message)?;
            Self {
                default_root,
                path,
                document,
                writer,
            }
        };
        service.recover_transition()?;
        service.validate_registry()?;
        Ok(service)
    }

    pub(crate) fn selected_profile_identity(&self) -> Result<ProfileIdentity, String> {
        let selected = self
            .document
            .profiles
            .iter()
            .find(|profile| profile.profile_id == self.document.selected_next_launch_profile_id)
            .ok_or_else(|| "next-launch profile is missing from the registry".to_string())?;
        load_identity(&selected.root)
    }

    pub(crate) fn selected_next_launch_profile_id(&self) -> &str {
        &self.document.selected_next_launch_profile_id
    }

    pub(crate) fn list_profiles(
        &self,
        runtime_profile_id: &str,
    ) -> Result<Vec<ProfileView>, String> {
        self.document
            .profiles
            .iter()
            .map(|profile| {
                let source = load_identity(&profile.root)?.source;
                Ok(profile_view(
                    profile.clone(),
                    profile.profile_id == runtime_profile_id,
                    profile.profile_id == self.document.selected_next_launch_profile_id,
                    source,
                    self.launch_instructions(&profile.profile_id, source),
                ))
            })
            .collect()
    }

    pub(crate) fn create_clean_profile(
        &mut self,
        display_name: String,
        slug: String,
    ) -> Result<ProfileView, String> {
        validate_display_name(&display_name)?;
        validate_slug(&slug)?;
        if self
            .document
            .profiles
            .iter()
            .any(|profile| profile.slug == slug)
        {
            return Err(format!("profile slug already exists: {slug}"));
        }
        let profile_id = format!("profile-{}", Uuid::new_v4());
        let root = self.default_root.join("profiles").join(&profile_id);
        let profile = ProfileRecord {
            profile_id,
            display_name,
            slug,
            root,
            created_at: Utc::now().to_rfc3339(),
        };
        self.persist_transition(ProfileTransition::Create {
            profile: profile.clone(),
        })?;
        if let Err(error) = self.create_profile_root(&profile) {
            if error.is_indeterminate() {
                return Err(error.into_message());
            }
            return Err(self.abort_create_transition(&profile, error.into_message()));
        }
        let mut candidate = self.document.clone();
        candidate.selected_next_launch_profile_id = profile.profile_id.clone();
        candidate.profiles.push(profile.clone());
        if let Err(error) = self.persist_document(&candidate) {
            if error.is_indeterminate() {
                self.document = candidate;
                return Err(error.into_message());
            }
            return Err(self.abort_create_transition(&profile, error.into_message()));
        }
        self.document = candidate;
        self.clear_transition()?;
        Ok(profile_view(
            profile.clone(),
            false,
            true,
            ProfileSource::Managed,
            self.launch_instructions(&profile.profile_id, ProfileSource::Managed),
        ))
    }

    pub(crate) fn delete_profile(
        &mut self,
        profile_id: &str,
        runtime_profile_id: &str,
    ) -> Result<(), String> {
        if runtime_profile_id == profile_id {
            return Err("cannot delete the profile used by the running Kestral process".into());
        }
        if self.document.selected_next_launch_profile_id == profile_id {
            return Err("cannot delete the profile selected for the next Kestral launch".into());
        }
        let index = self
            .document
            .profiles
            .iter()
            .position(|profile| profile.profile_id == profile_id)
            .ok_or_else(|| format!("unknown Kestral profile: {profile_id}"))?;
        let profile = self.document.profiles[index].clone();
        let managed_root = self.default_root.join("profiles").join(&profile.profile_id);
        if profile.root != managed_root {
            return Err(
                "refusing to delete a profile outside the managed profiles registry".into(),
            );
        }
        self.persist_transition(ProfileTransition::Delete {
            profile: profile.clone(),
        })?;
        let mut candidate = self.document.clone();
        candidate.profiles.remove(index);
        if let Err(error) = self.persist_document(&candidate) {
            if error.is_indeterminate() {
                self.document = candidate;
                return Err(error.into_message());
            }
            let cleanup = self.clear_transition();
            return Err(combine_cleanup_error(
                error.into_message(),
                cleanup,
                "profile transition",
            ));
        }
        self.document = candidate;
        if let Err(error) = fs::remove_dir_all(&profile.root) {
            return Err(format!(
                "profile was removed from the registry, but delete profile root failed; restart Kestral to retry cleanup: {error}"
            ));
        }
        self.clear_transition()
    }

    pub(crate) fn select_profile_by_slug(&mut self, slug: &str) -> Result<ProfileIdentity, String> {
        let profile = self
            .document
            .profiles
            .iter()
            .find(|profile| profile.slug == slug)
            .cloned()
            .ok_or_else(|| format!("unknown Kestral profile slug: {slug}"))?;
        // Validate the target profile's identity BEFORE persisting the
        // selection. Write-then-validate here could brick startup: the next
        // plain launch resolves the same persisted selection and fails
        // identically.
        let identity = load_identity(&profile.root)?;
        let mut candidate = self.document.clone();
        candidate.selected_next_launch_profile_id = profile.profile_id.clone();
        match self.persist_document(&candidate) {
            Ok(()) => {
                self.document = candidate;
                Ok(identity)
            }
            Err(error) if error.is_indeterminate() => {
                self.document = candidate;
                Err(error.into_message())
            }
            Err(error) => Err(error.into_message()),
        }
    }

    fn persist_document(&self, document: &ProfileRegistryDocument) -> Result<(), AtomicJsonError> {
        persist_json_document(
            &self.path,
            document,
            "profile registry",
            self.writer.as_ref(),
        )
    }

    fn create_profile_root(&self, profile: &ProfileRecord) -> Result<(), AtomicJsonError> {
        fs::create_dir_all(&profile.root).map_err(|error| {
            AtomicJsonError::NotCommitted(format!("create profile directory failed: {error}"))
        })?;
        persist_identity(
            &profile.root,
            profile,
            ProfileSource::Managed,
            self.writer.as_ref(),
        )
    }

    fn abort_create_transition(&self, profile: &ProfileRecord, error: String) -> String {
        let root_cleanup = if profile.root.exists() {
            fs::remove_dir_all(&profile.root)
                .map_err(|cleanup| format!("delete staged profile root failed: {cleanup}"))
        } else {
            Ok(())
        };
        let error = combine_cleanup_error(error, root_cleanup, "staged profile root");
        combine_cleanup_error(error, self.clear_transition(), "profile transition")
    }

    fn persist_transition(&self, transition: ProfileTransition) -> Result<(), String> {
        persist_json_document(
            &transition_path(&self.default_root),
            &ProfileTransitionDocument {
                version: REGISTRY_VERSION,
                transition,
            },
            "profile transition",
            self.writer.as_ref(),
        )
        .map_err(AtomicJsonError::into_message)
    }

    fn clear_transition(&self) -> Result<(), String> {
        match self
            .writer
            .remove_file(&transition_path(&self.default_root))
        {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("clear profile transition failed: {error}")),
        }
    }

    fn recover_transition(&self) -> Result<(), String> {
        let path = transition_path(&self.default_root);
        let Some(document) =
            load_json_document::<ProfileTransitionDocument>(&path, "profile transition")?
        else {
            return Ok(());
        };
        if document.version != REGISTRY_VERSION {
            return Err(format!(
                "unsupported profile transition version: {}",
                document.version
            ));
        }
        match document.transition {
            ProfileTransition::Create { profile } => {
                let committed = self
                    .document
                    .profiles
                    .iter()
                    .any(|record| record.profile_id == profile.profile_id);
                if committed {
                    load_identity(&profile.root)?;
                } else if profile.root.exists() {
                    remove_managed_profile_root(&self.default_root, &profile)?;
                }
            }
            ProfileTransition::Delete { profile } => {
                let committed = self
                    .document
                    .profiles
                    .iter()
                    .all(|record| record.profile_id != profile.profile_id);
                if committed && profile.root.exists() {
                    remove_managed_profile_root(&self.default_root, &profile)?;
                } else if !committed {
                    load_identity(&profile.root)?;
                }
            }
        }
        self.clear_transition()
    }

    fn validate_registry(&self) -> Result<(), String> {
        if self.document.profiles.is_empty() {
            return Err("profile registry must contain at least one profile".into());
        }
        if !self
            .document
            .profiles
            .iter()
            .any(|profile| profile.root == self.default_root)
        {
            return Err("profile registry is missing its default-root profile".into());
        }
        if !self
            .document
            .profiles
            .iter()
            .any(|profile| profile.profile_id == self.document.selected_next_launch_profile_id)
        {
            return Err("next-launch profile is missing from the registry".into());
        }

        let mut profile_ids = BTreeSet::new();
        let mut slugs = BTreeSet::new();
        let mut roots = BTreeSet::new();
        for profile in &self.document.profiles {
            if profile.profile_id.is_empty() {
                return Err("profile registry contains an empty profile id".into());
            }
            if !profile_ids.insert(&profile.profile_id) {
                return Err(format!(
                    "profile registry contains duplicate profile id: {}",
                    profile.profile_id
                ));
            }
            validate_display_name(&profile.display_name)?;
            validate_slug(&profile.slug)?;
            if !slugs.insert(&profile.slug) {
                return Err(format!(
                    "profile registry contains duplicate profile slug: {}",
                    profile.slug
                ));
            }
            if !roots.insert(&profile.root) {
                return Err(format!(
                    "profile registry contains duplicate profile root: {}",
                    profile.root.display()
                ));
            }

            let identity = load_identity(&profile.root)?;
            if identity.profile_id != profile.profile_id
                || identity.display_name != profile.display_name
                || identity.slug != profile.slug
                || identity.root != profile.root
                || identity.source != ProfileSource::Managed
            {
                return Err(format!(
                    "profile registry record does not match its on-disk identity: {}",
                    profile.root.display()
                ));
            }
        }
        Ok(())
    }

    fn launch_instructions(
        &self,
        profile_id: &str,
        source: ProfileSource,
    ) -> ProfileLaunchInstructions {
        let slug = self
            .slug_for(profile_id)
            .unwrap_or_else(|| "default".into());
        let root = self
            .document
            .profiles
            .iter()
            .find(|profile| profile.profile_id == profile_id)
            .map(|profile| profile.root.clone())
            .unwrap_or_else(|| self.default_root.clone());
        launch_instructions_for(source, &slug, &root)
    }

    fn slug_for(&self, profile_id: &str) -> Option<String> {
        self.document
            .profiles
            .iter()
            .find(|profile| profile.profile_id == profile_id)
            .map(|profile| profile.slug.clone())
    }
}

pub(crate) fn load_or_create_runtime_identity(
    root: &Path,
    source: ProfileSource,
    profile_id: Option<String>,
    display_name: Option<String>,
    slug: Option<String>,
) -> Result<ProfileIdentity, String> {
    if let Some(identity) = load_identity_if_present(root)? {
        return Ok(identity);
    }
    require_empty_unidentified_root(root)?;
    let profile = ProfileRecord {
        profile_id: profile_id.unwrap_or_else(|| format!("profile-{}", Uuid::new_v4())),
        display_name: display_name.unwrap_or_else(|| default_display_name(root)),
        slug: slug.unwrap_or_else(|| default_slug(root)),
        root: root.to_path_buf(),
        created_at: Utc::now().to_rfc3339(),
    };
    validate_display_name(&profile.display_name)?;
    validate_slug(&profile.slug)?;
    if let Some(parent) = root.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create profile root parent failed: {error}"))?;
    }
    fs::create_dir_all(root).map_err(|error| format!("create profile root failed: {error}"))?;
    persist_identity(root, &profile, source, standard_writer().as_ref())
        .map_err(AtomicJsonError::into_message)?;
    Ok(ProfileIdentity {
        profile_id: profile.profile_id,
        display_name: profile.display_name,
        slug: profile.slug,
        root: profile.root,
        created_at: profile.created_at,
        source,
    })
}

pub(crate) fn profile_identity_path(root: &Path) -> PathBuf {
    root.join(PROFILE_IDENTITY_FILE)
}

pub(crate) fn profile_registry_path(default_root: &Path) -> PathBuf {
    default_root.join(PROFILE_REGISTRY_FILE)
}

pub(crate) fn preserve_during_profile_reset(
    profile_root: &Path,
    default_root: &Path,
    path: &Path,
) -> bool {
    if path == profile_identity_path(profile_root) {
        return true;
    }
    if path == profile_root.join(KERNEL_STATE_LOCK_FILE) {
        return true;
    }
    if profile_root == default_root && path == default_root.join(PROFILE_REGISTRY_LOCK_FILE) {
        return true;
    }
    profile_root == default_root
        && (path == profile_registry_path(default_root)
            || path == transition_path(default_root)
            || path == default_root.join("profiles"))
}

fn profile_view(
    profile: ProfileRecord,
    current_runtime: bool,
    selected_for_next_launch: bool,
    source: ProfileSource,
    launch: ProfileLaunchInstructions,
) -> ProfileView {
    ProfileView {
        profile,
        current_runtime,
        selected_for_next_launch,
        source,
        launch_args: launch.launch_args,
        restart_instructions: launch.restart_instructions,
    }
}

fn persist_identity(
    root: &Path,
    profile: &ProfileRecord,
    source: ProfileSource,
    writer: &dyn AtomicFileWriter,
) -> Result<(), AtomicJsonError> {
    persist_json_document(
        &profile_identity_path(root),
        &ProfileIdentityDocument {
            version: IDENTITY_VERSION,
            profile: profile.clone(),
            source,
        },
        "profile identity",
        writer,
    )
}

fn load_identity(root: &Path) -> Result<ProfileIdentity, String> {
    let document = load_json_document::<ProfileIdentityDocument>(
        &profile_identity_path(root),
        "profile identity",
    )?
    .ok_or_else(|| {
        format!(
            "missing profile identity: {}",
            profile_identity_path(root).display()
        )
    })?;
    if document.version != IDENTITY_VERSION {
        return Err(format!(
            "unsupported profile identity version: {}",
            document.version
        ));
    }
    validate_display_name(&document.profile.display_name)?;
    validate_slug(&document.profile.slug)?;
    if document.profile.root != root {
        return Err(format!(
            "profile identity root mismatch: {}",
            document.profile.root.display()
        ));
    }
    Ok(ProfileIdentity {
        profile_id: document.profile.profile_id,
        display_name: document.profile.display_name,
        slug: document.profile.slug,
        root: document.profile.root,
        created_at: document.profile.created_at,
        source: document.source,
    })
}

fn load_identity_if_present(root: &Path) -> Result<Option<ProfileIdentity>, String> {
    let path = profile_identity_path(root);
    if !path.exists() {
        return Ok(None);
    }
    load_identity(root).map(Some)
}

fn default_profile(
    default_root: &Path,
    writer: &dyn AtomicFileWriter,
) -> Result<ProfileRecord, String> {
    require_empty_unidentified_root(default_root)?;
    let root = default_root.to_path_buf();
    let display_name = "Default Kestral profile".to_string();
    let slug = "default".to_string();
    validate_display_name(&display_name)?;
    validate_slug(&slug)?;
    fs::create_dir_all(&root)
        .map_err(|error| format!("create default profile root failed: {error}"))?;
    let profile = ProfileRecord {
        profile_id: format!("profile-{}", Uuid::new_v4()),
        display_name,
        slug,
        root: root.clone(),
        created_at: Utc::now().to_rfc3339(),
    };
    persist_identity(&root, &profile, ProfileSource::Managed, writer)
        .map_err(AtomicJsonError::into_message)?;
    Ok(profile)
}

fn require_empty_unidentified_root(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    for entry in
        fs::read_dir(root).map_err(|error| format!("inspect profile root failed: {error}"))?
    {
        let entry = entry.map_err(|error| format!("inspect profile root entry failed: {error}"))?;
        let name = entry.file_name();
        let is_coordination_file =
            name == PROFILE_REGISTRY_LOCK_FILE || name == KERNEL_STATE_LOCK_FILE;
        if is_coordination_file
            && entry
                .file_type()
                .map_err(|error| format!("inspect profile root entry type failed: {error}"))?
                .is_file()
        {
            continue;
        }
        return Err(format!(
            "profile root contains data but is missing {}",
            profile_identity_path(root).display()
        ));
    }
    Ok(())
}

fn validate_display_name(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err("profile display name is required".into())
    } else {
        Ok(())
    }
}

fn validate_slug(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("profile slug is required".into());
    }
    let mut chars = value.chars();
    let first = chars
        .next()
        .ok_or_else(|| "profile slug is required".to_string())?;
    if !first.is_ascii_alphanumeric() {
        return Err(format!("invalid profile slug: {value}"));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err(format!("invalid profile slug: {value}"));
    }
    if value.ends_with('-') {
        return Err(format!("invalid profile slug: {value}"));
    }
    Ok(())
}

fn default_display_name(root: &Path) -> String {
    root.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(|name| name.replace('_', " "))
        .unwrap_or_else(|| "Custom Kestral profile".to_string())
}

fn default_slug(root: &Path) -> String {
    let candidate = root
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            name.to_ascii_lowercase()
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "-")
        })
        .unwrap_or_else(|| "custom".into());
    let normalized = candidate.trim_matches('-');
    if normalized.is_empty() {
        "custom".into()
    } else {
        normalized.to_string()
    }
}

fn registry_path(default_root: &Path) -> PathBuf {
    profile_registry_path(default_root)
}

fn transition_path(default_root: &Path) -> PathBuf {
    default_root.join(PROFILE_TRANSITION_FILE)
}

fn remove_managed_profile_root(default_root: &Path, profile: &ProfileRecord) -> Result<(), String> {
    let expected = default_root.join("profiles").join(&profile.profile_id);
    if profile.root != expected {
        return Err("refusing to recover a profile outside the managed profiles directory".into());
    }
    fs::remove_dir_all(&profile.root)
        .map_err(|error| format!("delete staged profile root failed: {error}"))
}

fn combine_cleanup_error(
    error: String,
    cleanup: Result<(), String>,
    cleanup_label: &str,
) -> String {
    match cleanup {
        Ok(()) => error,
        Err(cleanup) => format!("{error}; cleanup of {cleanup_label} failed: {cleanup}"),
    }
}

#[cfg(test)]
mod tests;
