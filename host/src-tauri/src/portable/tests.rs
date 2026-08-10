use super::*;

use std::ffi::OsString;
use std::path::PathBuf;

fn test_paths(label: &str) -> HostPaths {
    let root = std::env::temp_dir().join(format!("kestral-portable-{label}-{}", Uuid::new_v4()));
    HostPaths::resolve_startup_from(root, std::iter::empty::<OsString>(), |_| None).unwrap()
}

fn cleanup(paths: &HostPaths) {
    let _ = fs::remove_dir_all(paths.default_root());
}

#[test]
fn export_import_round_trip_preserves_profile_files_and_app_data() {
    let paths = test_paths("round-trip");
    fs::write(paths.chat_store_path(), br#"{"version":4,"threads":[]}"#).unwrap();
    let app_data = paths.root().join("apps/.data/com.example/data.json");
    fs::create_dir_all(app_data.parent().unwrap()).unwrap();
    fs::write(&app_data, br#"{"kept":true}"#).unwrap();
    let archive = paths.default_root().join("round-trip.kestral-portable.zip");
    export(&paths, &archive).unwrap();

    let mut registry = ProfileRegistryService::open(paths.default_root().to_path_buf()).unwrap();
    let result = import(
        &paths,
        &mut registry,
        &archive,
        PortableImportTarget::Fresh {
            display_name: "Imported".into(),
            slug: "imported".into(),
        },
    )
    .unwrap();

    assert_eq!(result.target, "fresh-profile");
    let imported = registry.selected_profile_identity().unwrap();
    assert_eq!(
        fs::read(imported.root.join("chat-threads.json")).unwrap(),
        br#"{"version":4,"threads":[]}"#
    );
    assert_eq!(
        fs::read(imported.root.join("apps/.data/com.example/data.json")).unwrap(),
        br#"{"kept":true}"#
    );
    cleanup(&paths);
}

#[test]
fn tampered_archive_is_rejected_without_creating_a_profile() {
    let paths = test_paths("tampered");
    let archive = paths.default_root().join("tampered.kestral-portable.zip");
    export(&paths, &archive).unwrap();
    let mut bytes = fs::read(&archive).unwrap();
    let index = bytes.len() / 2;
    bytes[index] ^= 0xff;
    fs::write(&archive, bytes).unwrap();
    let mut registry = ProfileRegistryService::open(paths.default_root().to_path_buf()).unwrap();
    let before = registry.list_profiles(paths.profile_id()).unwrap().len();

    assert!(import(
        &paths,
        &mut registry,
        &archive,
        PortableImportTarget::Preview
    )
    .is_err());
    assert_eq!(
        registry.list_profiles(paths.profile_id()).unwrap().len(),
        before
    );
    cleanup(&paths);
}

#[test]
fn overwrite_is_staged_then_retains_a_backup_on_restart() {
    let paths = test_paths("overwrite");
    fs::write(paths.chat_store_path(), br#"{"version":4,"threads":[]}"#).unwrap();
    let archive = paths.default_root().join("overwrite.kestral-portable.zip");
    export(&paths, &archive).unwrap();
    fs::write(
        paths.chat_store_path(),
        br#"{"version":4,"threads":[{"bad":true}]}"#,
    )
    .unwrap();
    let mut registry = ProfileRegistryService::open(paths.default_root().to_path_buf()).unwrap();

    let staged = import(
        &paths,
        &mut registry,
        &archive,
        PortableImportTarget::OverwriteCurrent {
            confirmation: format!("RESTORE {}", paths.profile_slug()),
        },
    )
    .unwrap();
    assert!(staged.restart_required);
    apply_pending(&paths).unwrap();

    assert_eq!(
        fs::read(paths.chat_store_path()).unwrap(),
        br#"{"version":4,"threads":[]}"#
    );
    assert_eq!(
        fs::read_dir(paths.root().join(BACKUP_DIR)).unwrap().count(),
        1
    );
    assert!(!paths.root().join(REQUEST_FILE).exists());
    cleanup(&paths);
}

#[test]
fn failed_overwrite_commit_restores_the_retained_original() {
    let paths = test_paths("overwrite-rollback");
    fs::write(paths.chat_store_path(), br#"{"version":4,"threads":[]}"#).unwrap();
    let staging = std::env::temp_dir().join(format!("kestral-portable-staging-{}", Uuid::new_v4()));
    let candidate = staging.join("candidate");
    let backup = staging.join("backup");
    copy_profile_contents(paths.root(), &candidate).unwrap();
    copy_profile_contents(paths.root(), &backup).unwrap();

    let error = commit_overwrite(&paths, &candidate, &backup, || {
        Err("injected copy failure".into())
    })
    .unwrap_err();

    assert!(error.contains("original profile restored"), "{error}");
    assert_eq!(
        fs::read(paths.chat_store_path()).unwrap(),
        br#"{"version":4,"threads":[]}"#
    );
    crate::profile_migration::validate_profile(paths.root(), &paths, false).unwrap();
    fs::remove_dir_all(staging).unwrap();
    cleanup(&paths);
}

#[test]
fn unsafe_archive_paths_are_refused() {
    for path in ["../escape", "/absolute", "secure\\windows"] {
        assert!(validate_archive_path(path).is_err(), "{path}");
    }
}

#[test]
fn checked_in_v1_fixture_is_strict_and_contains_no_sensitive_payloads() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/portable/v1/empty");
    let manifest: PortableManifest =
        serde_json::from_slice(&fs::read(root.join(MANIFEST)).unwrap()).unwrap();

    validate_manifest(&manifest).unwrap();
    assert!(manifest.contents.is_empty());
    assert!(manifest.apps.is_empty());
    assert!(manifest.secrets.is_empty());
    assert!(manifest.file_resources.is_empty());
}
