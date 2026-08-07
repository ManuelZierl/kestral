use super::*;

use std::ffi::OsString;

use crate::host_paths::HostPaths;

fn test_paths(label: &str) -> HostPaths {
    let root = std::env::temp_dir().join(format!(
        "kestral-system-reset-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    HostPaths::resolve_startup_from(root, std::iter::empty::<OsString>(), |_| None).unwrap()
}

#[test]
fn reset_requires_the_exact_profile_specific_phrase() {
    let paths = test_paths("confirmation");

    let error = stage(&paths, "RESET").unwrap_err();

    assert!(error.contains("RESET default"));
    assert!(!request_path(paths.root()).exists());
    std::fs::remove_dir_all(paths.root()).unwrap();
}

#[test]
fn pending_reset_removes_profile_state_but_preserves_profile_infrastructure() {
    let paths = test_paths("apply");
    let identity = crate::profiles::profile_identity_path(paths.root());
    let registry = crate::profiles::profile_registry_path(paths.default_root());
    let other_profile = paths.root().join("profiles/profile-other/keep.txt");
    std::fs::create_dir_all(other_profile.parent().unwrap()).unwrap();
    std::fs::write(&other_profile, "other profile").unwrap();
    std::fs::write(paths.root().join("kernel-state-v1.json"), "state").unwrap();
    std::fs::create_dir_all(paths.root().join("apps/example/.data")).unwrap();
    std::fs::write(paths.root().join("apps/example/.data/value"), "private").unwrap();
    std::fs::write(paths.root().join("mcp-gateway-audit.jsonl"), "audit").unwrap();
    stage(&paths, "RESET default").unwrap();

    assert!(apply_pending(&paths).unwrap());

    assert!(identity.exists());
    assert!(registry.exists());
    assert!(other_profile.exists());
    assert!(!paths.root().join("kernel-state-v1.json").exists());
    assert!(!paths.root().join("apps").exists());
    assert!(!paths.root().join("mcp-gateway-audit.jsonl").exists());
    assert!(!request_path(paths.root()).exists());
    assert!(!apply_pending(&paths).unwrap());
    std::fs::remove_dir_all(paths.root()).unwrap();
}

#[test]
fn reset_keeps_the_request_and_data_when_the_secret_index_cannot_be_read() {
    let paths = test_paths("bad-secrets");
    let data = paths.root().join("chat-threads.json");
    std::fs::write(&data, "chat").unwrap();
    std::fs::write(paths.secrets_index_path(), "{").unwrap();
    stage(&paths, "RESET default").unwrap();

    let error = apply_pending(&paths).unwrap_err();

    assert!(error.contains("parse secret reference store failed"));
    assert!(data.exists());
    assert!(request_path(paths.root()).exists());
    std::fs::remove_dir_all(paths.root()).unwrap();
}

#[test]
fn reset_refuses_a_marker_for_another_profile() {
    let paths = test_paths("identity-mismatch");
    persist_json_document(
        &request_path(paths.root()),
        &ResetRequest {
            version: RESET_REQUEST_VERSION,
            profile_id: "profile-other".into(),
            requested_at: Utc::now().to_rfc3339(),
        },
        "system reset request",
        standard_writer().as_ref(),
    )
    .unwrap();
    let data = paths.root().join("installed-apps.json");
    std::fs::write(&data, "apps").unwrap();

    assert!(apply_pending(&paths)
        .unwrap_err()
        .contains("does not match the active profile identity"));
    assert!(data.exists());
    std::fs::remove_dir_all(paths.root()).unwrap();
}
