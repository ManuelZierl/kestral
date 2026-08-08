use super::{
    load_or_create_runtime_identity, ProfileRecord, ProfileRegistryDocument,
    ProfileRegistryService, ProfileSource, ProfileTransition,
};
use crate::atomic_json::{FailingAtomicFileWriter, FailingFileOperation};
use std::sync::Arc;

#[test]
fn registry_creates_default_and_managed_profiles() {
    let root = temp_root("registry");
    let mut registry = ProfileRegistryService::open(root.clone()).unwrap();
    let runtime_profile_id = registry.selected_profile_identity().unwrap().profile_id;

    let profiles = registry.list_profiles(&runtime_profile_id).unwrap();
    let runtime = profiles
        .iter()
        .find(|profile| profile.current_runtime)
        .unwrap();
    assert!(runtime.selected_for_next_launch);
    assert_eq!(runtime.source, ProfileSource::Managed);
    assert_eq!(runtime.launch_args[0], "--profile");

    let created = registry
        .create_clean_profile("Work profile".into(), "work".into())
        .unwrap();
    assert!(!created.current_runtime);
    assert!(created.selected_for_next_launch);
    assert_eq!(
        created.launch_args,
        vec!["--profile".to_string(), "work".to_string()]
    );

    let profiles = registry.list_profiles(&runtime_profile_id).unwrap();
    assert!(profiles
        .iter()
        .any(|profile| profile.profile.slug == "work"));
}

#[test]
fn custom_data_dir_identity_is_persistent_inside_an_empty_root() {
    let root = temp_root("custom-root");
    let first =
        load_or_create_runtime_identity(&root, ProfileSource::CustomDataDir, None, None, None)
            .unwrap();
    let second =
        load_or_create_runtime_identity(&root, ProfileSource::CustomDataDir, None, None, None)
            .unwrap();

    assert_eq!(first.profile_id, second.profile_id);
    assert_eq!(first.root, second.root);
    assert_eq!(first.source, ProfileSource::CustomDataDir);
}

#[test]
fn default_profile_recovers_after_indeterminate_identity_and_registry_writes() {
    let root = temp_root("indeterminate-default");
    let error = ProfileRegistryService::with_writer(
        root.clone(),
        Arc::new(FailingAtomicFileWriter::new(
            FailingFileOperation::SyncParent,
        )),
    )
    .err()
    .unwrap();
    assert!(error.contains("durability is indeterminate"), "{error}");

    let recovered = ProfileRegistryService::open(root).unwrap();
    let identity = recovered.selected_profile_identity().unwrap();
    assert_eq!(identity.slug, "default");
    assert_eq!(identity.source, ProfileSource::Managed);
}

#[test]
fn missing_current_profile_selection_field_is_rejected() {
    let document = serde_json::json!({
        "version": 1,
        "active_profile_id": "profile-default",
        "profiles": []
    });

    assert!(serde_json::from_value::<ProfileRegistryDocument>(document).is_err());
}

#[test]
fn non_empty_custom_root_without_identity_is_rejected() {
    let root = temp_root("custom-root-with-data");
    std::fs::write(root.join("kernel-state-v1.lock"), []).unwrap();
    std::fs::write(root.join("existing-data"), "not a profile").unwrap();

    let error =
        load_or_create_runtime_identity(&root, ProfileSource::CustomDataDir, None, None, None)
            .err()
            .unwrap();

    assert!(error.contains("missing"), "{error}");
    assert!(error.contains("kestral-profile.json"), "{error}");
}

#[test]
fn non_empty_default_root_without_identity_is_rejected() {
    let root = temp_root("default-root-with-data");
    std::fs::write(root.join("kestral-profiles.lock"), []).unwrap();
    std::fs::write(root.join("host-config.json"), "test data").unwrap();

    let error = ProfileRegistryService::open(root).err().unwrap();

    assert!(error.contains("missing"), "{error}");
    assert!(error.contains("kestral-profile.json"), "{error}");
}

#[test]
fn coordination_name_is_ignored_only_for_a_regular_file() {
    let root = temp_root("lock-directory-without-identity");
    std::fs::create_dir(root.join("kestral-profiles.lock")).unwrap();

    let error = ProfileRegistryService::open(root).err().unwrap();

    assert!(error.contains("missing"), "{error}");
    assert!(error.contains("kestral-profile.json"), "{error}");
}

#[cfg(unix)]
#[test]
fn symlinked_coordination_name_is_not_ignored() {
    use std::os::unix::fs::symlink;

    let root = temp_root("lock-symlink-without-identity");
    let target = root.with_extension("lock-target");
    std::fs::write(&target, []).unwrap();
    symlink(&target, root.join("kestral-profiles.lock")).unwrap();

    let error = ProfileRegistryService::open(root).err().unwrap();

    assert!(error.contains("missing"), "{error}");
    assert!(error.contains("kestral-profile.json"), "{error}");
}

#[test]
fn inconsistent_profile_registry_is_rejected() {
    let root = temp_root("inconsistent-registry");
    let registry = ProfileRegistryService::open(root.clone()).unwrap();
    drop(registry);

    let path = root.join("kestral-profiles.json");
    let mut document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    document["profiles"][0]["display_name"] = serde_json::Value::String("Mismatch".into());
    std::fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();

    let error = ProfileRegistryService::open(root).err().unwrap();

    assert!(error.contains("does not match"), "{error}");
}

#[test]
fn unknown_profiles_are_rejected_for_deletion() {
    let root = temp_root("delete-active");
    let mut registry = ProfileRegistryService::open(root).unwrap();
    let runtime_profile_id = registry.selected_profile_identity().unwrap().profile_id;
    let _created = registry
        .create_clean_profile("Work profile".into(), "work".into())
        .unwrap();

    let error = registry
        .delete_profile("profile-not-real", &runtime_profile_id)
        .unwrap_err();
    assert!(error.contains("unknown Kestral profile"), "{error}");
}

#[test]
fn inactive_profiles_can_be_deleted() {
    let root = temp_root("delete-inactive");
    let mut registry = ProfileRegistryService::open(root).unwrap();
    let runtime_profile_id = registry.selected_profile_identity().unwrap().profile_id;
    let first = registry
        .create_clean_profile("Work profile".into(), "work".into())
        .unwrap();
    let second = registry
        .create_clean_profile("Personal profile".into(), "personal".into())
        .unwrap();

    let error = registry
        .delete_profile(&second.profile.profile_id, &runtime_profile_id)
        .unwrap_err();
    assert!(
        error.contains("selected for the next Kestral launch"),
        "{error}"
    );

    registry.select_profile_by_slug("work").unwrap();
    registry
        .delete_profile(&second.profile.profile_id, &runtime_profile_id)
        .unwrap();
    let profiles = registry.list_profiles(&runtime_profile_id).unwrap();
    assert!(profiles
        .iter()
        .any(|profile| profile.profile.profile_id == first.profile.profile_id));
    assert!(!profiles
        .iter()
        .any(|profile| profile.profile.profile_id == second.profile.profile_id));
}

#[test]
fn running_profile_cannot_be_deleted_after_next_launch_selection_changes() {
    let root = temp_root("protect-runtime");
    let mut registry = ProfileRegistryService::open(root).unwrap();
    let runtime_profile_id = registry.selected_profile_identity().unwrap().profile_id;
    registry
        .create_clean_profile("Work profile".into(), "work".into())
        .unwrap();

    let error = registry
        .delete_profile(&runtime_profile_id, &runtime_profile_id)
        .unwrap_err();

    assert!(error.contains("running Kestral process"), "{error}");
    let profiles = registry.list_profiles(&runtime_profile_id).unwrap();
    let runtime = profiles
        .iter()
        .find(|profile| profile.current_runtime)
        .unwrap();
    assert!(!runtime.selected_for_next_launch);
}

#[test]
fn startup_removes_root_from_interrupted_uncommitted_create() {
    let root = temp_root("recover-create");
    let registry = ProfileRegistryService::open(root.clone()).unwrap();
    let profile = ProfileRecord {
        profile_id: "profile-interrupted-create".into(),
        display_name: "Interrupted profile".into(),
        slug: "interrupted".into(),
        root: root.join("profiles").join("profile-interrupted-create"),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    registry
        .persist_transition(ProfileTransition::Create {
            profile: profile.clone(),
        })
        .unwrap();
    registry.create_profile_root(&profile).unwrap();
    assert!(profile.root.exists());
    drop(registry);

    let recovered = ProfileRegistryService::open(root).unwrap();

    assert!(!profile.root.exists());
    assert!(recovered
        .list_profiles(recovered.selected_next_launch_profile_id())
        .unwrap()
        .iter()
        .all(|view| view.profile.profile_id != profile.profile_id));
}

#[test]
fn startup_finishes_root_cleanup_after_committed_delete() {
    let root = temp_root("recover-delete");
    let mut registry = ProfileRegistryService::open(root.clone()).unwrap();
    let runtime_profile_id = registry.selected_profile_identity().unwrap().profile_id;
    let created = registry
        .create_clean_profile("Work profile".into(), "work".into())
        .unwrap();
    registry.select_profile_by_slug("default").unwrap();
    let profile = created.profile;
    registry
        .persist_transition(ProfileTransition::Delete {
            profile: profile.clone(),
        })
        .unwrap();
    let mut candidate = registry.document.clone();
    candidate
        .profiles
        .retain(|record| record.profile_id != profile.profile_id);
    registry.persist_document(&candidate).unwrap();
    assert!(profile.root.exists());
    drop(registry);

    let recovered = ProfileRegistryService::open(root).unwrap();

    assert!(!profile.root.exists());
    assert!(recovered
        .list_profiles(&runtime_profile_id)
        .unwrap()
        .iter()
        .all(|view| view.profile.profile_id != profile.profile_id));
}

fn temp_root(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir()
        .join(format!("kestral-profiles-{label}"))
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&path).unwrap();
    path
}
