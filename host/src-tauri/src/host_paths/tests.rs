use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use super::HostPaths;
use crate::profiles::{ProfileRegistryService, ProfileSource};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn env(values: &[(&str, &str)]) -> BTreeMap<String, OsString> {
    values
        .iter()
        .map(|(key, value)| (key.to_string(), OsString::from(value)))
        .collect()
}

#[test]
fn default_root_derives_all_paths() {
    let root = temp_root("default");
    let paths =
        HostPaths::resolve_startup_from(root.clone(), Vec::<OsString>::new(), |_| None).unwrap();

    assert_eq!(paths.default_root(), root.as_path());
    assert_eq!(paths.root(), root.as_path());
    assert_eq!(paths.profile_slug(), "default");
    assert_eq!(
        paths.profile_identity().display_name,
        "Default Kestral profile"
    );
    assert_eq!(paths.profile_source(), ProfileSource::Managed);
    assert_eq!(
        paths.profile_registry_path(),
        root.join("kestral-profiles.json")
    );
    assert_eq!(paths.config_path(), root.join("host-config.json"));
    assert_eq!(paths.secrets_index_path(), root.join("host-secrets.json"));
    assert_eq!(paths.chat_store_path(), root.join("chat-threads.json"));
    assert_eq!(paths.kernel_state_path(), root.join("kernel-state-v1.json"));
    assert_eq!(paths.app_store_path(), root.join("installed-apps.json"));
    assert_eq!(paths.app_records_root(), root.join("apps"));
    assert_eq!(paths.notices_path(), root.join("trusted-notices.json"));
    assert_eq!(
        paths.remote_owner_auth_path(),
        root.join("remote-owner-auth-v1.json")
    );
    assert_eq!(
        paths.remote_owner_pairing_path(),
        root.join("remote-owner-pairing-v1.json")
    );
    assert_eq!(paths.trust_store_path(), root.join("trust-store.json"));
    assert_eq!(
        paths.update_journal_path(),
        root.join("update-journal.json")
    );
    assert_eq!(paths.mcp_audit_path(), root.join("mcp-gateway-audit.jsonl"));
    assert_eq!(
        paths.file_resource_registry_path(),
        root.join("file-resources-v1.json")
    );
}

#[test]
fn profile_selection_is_scoped_under_profiles_directory() {
    let default_root = temp_root("managed-profile");
    let mut registry = ProfileRegistryService::open(default_root.clone()).unwrap();
    let created = registry
        .create_clean_profile("Work profile".into(), "work".into())
        .unwrap();

    let paths =
        HostPaths::resolve_startup_from(default_root.clone(), args(&["--profile", "work"]), |_| {
            None
        })
        .unwrap();

    assert_eq!(paths.root(), created.profile.root.as_path());
    assert_eq!(paths.profile_id(), created.profile.profile_id);
    assert_eq!(paths.profile_source(), ProfileSource::Managed);
}

#[test]
fn cli_data_dir_and_profile_are_mutually_exclusive() {
    let error = HostPaths::resolve_startup_from(
        temp_root("cli-conflict"),
        args(&["--profile", "cli-profile", "--data-dir", "/tmp/cli-data"]),
        |_| None,
    )
    .unwrap_err();

    assert!(error.contains("mutually exclusive"), "{error}");
}

#[test]
fn cli_profile_wins_over_env_data_dir() {
    let default_root = temp_root("cli-profile");
    let env = env(&[("KESTRAL_DATA_DIR", "/tmp/env-data")]);
    let mut registry = ProfileRegistryService::open(default_root.clone()).unwrap();
    let created = registry
        .create_clean_profile("Team profile".into(), "team".into())
        .unwrap();

    let paths =
        HostPaths::resolve_startup_from(default_root, args(&["--profile", "team"]), |key| {
            env.get(key).cloned()
        })
        .unwrap();

    assert_eq!(paths.root(), created.profile.root.as_path());
    assert_eq!(paths.profile_id(), created.profile.profile_id);
}

#[test]
fn cli_data_dir_wins_over_env_profile() {
    let default_root = temp_root("cli-data");
    let env = env(&[("KESTRAL_PROFILE", "env-profile")]);

    let paths = HostPaths::resolve_startup_from(
        default_root,
        args(&["--data-dir", "/tmp/cli-data-only"]),
        |key| env.get(key).cloned(),
    )
    .unwrap();

    assert_eq!(paths.root(), PathBuf::from("/tmp/cli-data-only").as_path());
    assert_eq!(paths.profile_source(), ProfileSource::CustomDataDir);
}

#[test]
fn env_data_dir_and_profile_are_mutually_exclusive() {
    let error =
        HostPaths::resolve_startup_from(temp_root("env-conflict"), Vec::<OsString>::new(), |key| {
            match key {
                "KESTRAL_PROFILE" => Some(OsString::from("env-profile")),
                "KESTRAL_DATA_DIR" => Some(OsString::from("/tmp/env-data")),
                _ => None,
            }
        })
        .unwrap_err();

    assert!(error.contains("mutually exclusive"), "{error}");
}

#[test]
fn env_data_dir_is_used_when_profile_is_absent() {
    let default_root = temp_root("env-data");
    let paths =
        HostPaths::resolve_startup_from(default_root, Vec::<OsString>::new(), |key| match key {
            "KESTRAL_DATA_DIR" => Some(OsString::from("/tmp/env-data-only")),
            _ => None,
        })
        .unwrap();

    assert_eq!(paths.root(), PathBuf::from("/tmp/env-data-only").as_path());
    assert_eq!(paths.profile_source(), ProfileSource::CustomDataDir);
}

#[test]
fn env_profile_is_used_when_data_dir_is_absent() {
    let default_root = temp_root("env-profile");
    let mut registry = ProfileRegistryService::open(default_root.clone()).unwrap();
    let created = registry
        .create_clean_profile("Env profile".into(), "env-profile".into())
        .unwrap();

    let paths =
        HostPaths::resolve_startup_from(default_root, Vec::<OsString>::new(), |key| match key {
            "KESTRAL_PROFILE" => Some(OsString::from("env-profile")),
            _ => None,
        })
        .unwrap();

    assert_eq!(paths.profile_id(), created.profile.profile_id);
    assert_eq!(paths.root(), created.profile.root.as_path());
}

#[test]
fn invalid_profile_identifier_is_rejected() {
    let error = HostPaths::resolve_startup_from(
        temp_root("invalid-profile"),
        args(&["--profile", "../bad"]),
        |_| None,
    )
    .unwrap_err();

    assert!(error.contains("invalid profile identifier"), "{error}");
}

fn temp_root(label: &str) -> PathBuf {
    let path = std::env::temp_dir()
        .join(format!("kestral-host-paths-{label}"))
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&path).unwrap();
    path
}
