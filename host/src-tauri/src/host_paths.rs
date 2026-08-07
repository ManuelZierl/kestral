use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::profiles::{
    load_or_create_runtime_identity, profile_registry_path, ProfileIdentity,
    ProfileLaunchInstructions, ProfileSource,
};

const CONFIG_FILE: &str = "host-config.json";
const SECRETS_INDEX_FILE: &str = "host-secrets.json";
const CHAT_STORE_FILE: &str = "chat-threads.json";
const KERNEL_STATE_FILE: &str = "kernel-state-v1.json";
const APP_STORE_FILE: &str = "installed-apps.json";
const APP_RECORDS_DIR: &str = "apps";
const NOTICES_FILE: &str = "trusted-notices.json";
const TRUST_STORE_FILE: &str = "trust-store.json";
const UPDATE_JOURNAL_FILE: &str = "update-journal.json";
const MCP_AUDIT_FILE: &str = "mcp-gateway-audit.jsonl";
const FILE_RESOURCE_REGISTRY_FILE: &str = "file-resources-v1.json";
const REMOTE_OWNER_AUTH_FILE: &str = "remote-owner-auth-v1.json";
const REMOTE_OWNER_PAIRING_FILE: &str = "remote-owner-pairing-v1.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostPaths {
    default_root: PathBuf,
    profile: ProfileIdentity,
    allow_unsafe_native_backends: bool,
    config_path: PathBuf,
    secrets_index_path: PathBuf,
    chat_store_path: PathBuf,
    app_store_path: PathBuf,
    app_records_root: PathBuf,
    notices_path: PathBuf,
    trust_store_path: PathBuf,
    update_journal_path: PathBuf,
    mcp_audit_path: PathBuf,
    file_resource_registry_path: PathBuf,
}

impl HostPaths {
    pub(crate) fn resolve_startup(default_root: PathBuf) -> Result<Self, String> {
        Self::resolve_startup_from(default_root, std::env::args_os().skip(1), |name| {
            std::env::var_os(name)
        })
    }

    pub(crate) fn resolve_startup_from<I, F>(
        default_root: PathBuf,
        args: I,
        mut env: F,
    ) -> Result<Self, String>
    where
        I: IntoIterator<Item = OsString>,
        F: FnMut(&str) -> Option<OsString>,
    {
        let cli = parse_selection(args)?;
        let env = parse_env(&mut env)?;
        let selection = if let Some(root) = cli.data_dir {
            Selection::DataDir(root)
        } else if let Some(profile) = cli.profile {
            Selection::Profile(profile)
        } else if let Some(root) = env.data_dir {
            Selection::DataDir(root)
        } else if let Some(profile) = env.profile {
            Selection::Profile(profile)
        } else {
            Selection::Default
        };
        let allow_unsafe_native_backends =
            cli.allow_unsafe_native_backends || env.allow_unsafe_native_backends;

        let mut registry = crate::profiles::ProfileRegistryService::open(default_root.clone())?;
        let profile = match selection {
            Selection::Default => registry.selected_profile_identity()?,
            Selection::Profile(slug) => {
                validate_profile_identifier(&slug)?;
                registry.select_profile_by_slug(&slug)?
            }
            Selection::DataDir(root) => load_or_create_runtime_identity(
                &root,
                ProfileSource::CustomDataDir,
                None,
                None,
                None,
            )?,
        };

        Ok(Self::new(
            default_root,
            profile,
            allow_unsafe_native_backends,
        ))
    }

    pub(crate) fn default_root(&self) -> &Path {
        &self.default_root
    }

    pub(crate) fn root(&self) -> &Path {
        &self.profile.root
    }

    pub(crate) fn profile_id(&self) -> &str {
        &self.profile.profile_id
    }

    pub(crate) fn profile_slug(&self) -> &str {
        &self.profile.slug
    }

    pub(crate) fn profile_source(&self) -> ProfileSource {
        self.profile.source
    }

    pub(crate) fn profile_registry_path(&self) -> PathBuf {
        profile_registry_path(self.default_root())
    }

    pub(crate) fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub(crate) fn secrets_index_path(&self) -> &Path {
        &self.secrets_index_path
    }

    pub(crate) fn chat_store_path(&self) -> &Path {
        &self.chat_store_path
    }

    pub(crate) fn kernel_state_path(&self) -> PathBuf {
        self.profile.root.join(KERNEL_STATE_FILE)
    }

    pub(crate) fn app_store_path(&self) -> &Path {
        &self.app_store_path
    }

    pub(crate) fn app_records_root(&self) -> &Path {
        &self.app_records_root
    }

    pub(crate) fn notices_path(&self) -> &Path {
        &self.notices_path
    }

    pub(crate) fn trust_store_path(&self) -> &Path {
        &self.trust_store_path
    }

    pub(crate) fn update_journal_path(&self) -> &Path {
        &self.update_journal_path
    }

    pub(crate) fn mcp_audit_path(&self) -> &Path {
        &self.mcp_audit_path
    }

    pub(crate) fn file_resource_registry_path(&self) -> &Path {
        &self.file_resource_registry_path
    }

    pub(crate) fn remote_owner_auth_path(&self) -> PathBuf {
        self.profile.root.join(REMOTE_OWNER_AUTH_FILE)
    }

    pub(crate) fn remote_owner_pairing_path(&self) -> PathBuf {
        self.profile.root.join(REMOTE_OWNER_PAIRING_FILE)
    }

    pub(crate) fn allow_unsafe_native_backends(&self) -> bool {
        self.allow_unsafe_native_backends
    }

    pub(crate) fn launch_instructions(&self) -> ProfileLaunchInstructions {
        crate::profiles::launch_instructions_for(
            self.profile_source(),
            self.profile_slug(),
            self.root(),
        )
    }

    pub(crate) fn profile_identity(&self) -> &ProfileIdentity {
        &self.profile
    }

    fn new(
        default_root: PathBuf,
        profile: ProfileIdentity,
        allow_unsafe_native_backends: bool,
    ) -> Self {
        let root = profile.root.clone();
        Self {
            config_path: root.join(CONFIG_FILE),
            secrets_index_path: root.join(SECRETS_INDEX_FILE),
            chat_store_path: root.join(CHAT_STORE_FILE),
            app_store_path: root.join(APP_STORE_FILE),
            app_records_root: root.join(APP_RECORDS_DIR),
            notices_path: root.join(NOTICES_FILE),
            trust_store_path: root.join(TRUST_STORE_FILE),
            update_journal_path: root.join(UPDATE_JOURNAL_FILE),
            mcp_audit_path: root.join(MCP_AUDIT_FILE),
            file_resource_registry_path: root.join(FILE_RESOURCE_REGISTRY_FILE),
            default_root,
            profile,
            allow_unsafe_native_backends,
        }
    }
}

#[derive(Debug, Clone)]
enum Selection {
    Default,
    Profile(String),
    DataDir(PathBuf),
}

#[derive(Debug, Clone, Default)]
struct SelectionSet {
    data_dir: Option<PathBuf>,
    profile: Option<String>,
    allow_unsafe_native_backends: bool,
}

#[derive(Debug, Clone, Default)]
struct EnvSelectionSet {
    data_dir: Option<PathBuf>,
    profile: Option<String>,
    allow_unsafe_native_backends: bool,
}

fn parse_selection<I>(args: I) -> Result<SelectionSet, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut parsed = SelectionSet::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg.as_os_str() == std::ffi::OsStr::new("--profile") {
            let value = args
                .next()
                .ok_or_else(|| "missing value for --profile".to_string())?;
            set_profile(&mut parsed.profile, value)?;
            continue;
        }
        if arg.as_os_str() == std::ffi::OsStr::new("--data-dir") {
            let value = args
                .next()
                .ok_or_else(|| "missing value for --data-dir".to_string())?;
            set_data_dir(&mut parsed.data_dir, value)?;
            continue;
        }
        if let Some(text) = arg.to_str() {
            if let Some(value) = text.strip_prefix("--profile=") {
                set_profile(&mut parsed.profile, OsString::from(value))?;
                continue;
            }
            if let Some(value) = text.strip_prefix("--data-dir=") {
                set_data_dir(&mut parsed.data_dir, OsString::from(value))?;
                continue;
            }
            if text == "--allow-unsafe-native-backends" {
                if parsed.allow_unsafe_native_backends {
                    return Err("duplicate --allow-unsafe-native-backends option".into());
                }
                parsed.allow_unsafe_native_backends = true;
                continue;
            }
        }
    }
    if parsed.profile.is_some() && parsed.data_dir.is_some() {
        return Err("--profile and --data-dir are mutually exclusive".into());
    }
    Ok(parsed)
}

fn parse_env<F>(env: &mut F) -> Result<EnvSelectionSet, String>
where
    F: FnMut(&str) -> Option<OsString>,
{
    let profile = env("KESTRAL_PROFILE")
        .map(os_string_to_string)
        .transpose()?;
    let data_dir = env("KESTRAL_DATA_DIR").map(PathBuf::from);
    let allow_unsafe_native_backends = match env("KESTRAL_ALLOW_UNSAFE_NATIVE_BACKENDS") {
        None => false,
        Some(value) => match value.into_string() {
            Ok(text) => matches!(text.as_str(), "1" | "true" | "yes"),
            Err(value) => return Err(format!("value must be valid UTF-8: {:?}", value)),
        },
    };
    if profile.is_some() && data_dir.is_some() {
        return Err("KESTRAL_PROFILE and KESTRAL_DATA_DIR are mutually exclusive".into());
    }
    Ok(EnvSelectionSet {
        data_dir,
        profile,
        allow_unsafe_native_backends,
    })
}

fn set_profile(target: &mut Option<String>, value: OsString) -> Result<(), String> {
    let profile = os_string_to_string(value)?;
    validate_profile_identifier(&profile)?;
    if target.replace(profile).is_some() {
        return Err("duplicate --profile option".into());
    }
    Ok(())
}

fn set_data_dir(target: &mut Option<PathBuf>, value: OsString) -> Result<(), String> {
    if target.replace(PathBuf::from(value)).is_some() {
        return Err("duplicate --data-dir option".into());
    }
    Ok(())
}

fn os_string_to_string(value: OsString) -> Result<String, String> {
    value
        .into_string()
        .map_err(|value| format!("value must be valid UTF-8: {:?}", value))
}

fn validate_profile_identifier(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("invalid profile identifier: ".into());
    }
    let mut chars = value.chars();
    let first = chars
        .next()
        .ok_or_else(|| "invalid profile identifier: ".to_string())?;
    if !first.is_ascii_alphanumeric() {
        return Err(format!("invalid profile identifier: {value}"));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err(format!("invalid profile identifier: {value}"));
    }
    if value.ends_with('-') {
        return Err(format!("invalid profile identifier: {value}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
