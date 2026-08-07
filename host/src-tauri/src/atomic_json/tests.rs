use super::*;

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct TestDocument {
    value: String,
}

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!("kestral-atomic-json-{label}-{}", Uuid::new_v4()))
        .join("store.json")
}

#[cfg(unix)]
#[test]
fn persisted_documents_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let path = temp_path("new-mode");
    persist_json_document(
        &path,
        &TestDocument {
            value: "private".into(),
        },
        "test store",
        standard_writer().as_ref(),
    )
    .unwrap();

    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[cfg(unix)]
#[test]
fn loading_repairs_permissive_document_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let path = temp_path("repair-mode");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, r#"{"value":"private"}"#).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

    let loaded = load_json_document::<TestDocument>(&path, "test store").unwrap();

    assert_eq!(loaded.unwrap().value, "private");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn parent_sync_failure_reports_indeterminate_after_target_is_replaced() {
    let path = temp_path("sync-parent");
    let writer = FailingAtomicFileWriter::new(FailingFileOperation::SyncParent);

    let error = persist_json_document(
        &path,
        &TestDocument {
            value: "committed candidate".into(),
        },
        "test store",
        &writer,
    )
    .unwrap_err();

    assert!(matches!(error, AtomicJsonError::Indeterminate(_)));
    assert_eq!(
        load_json_document::<TestDocument>(&path, "test store")
            .unwrap()
            .unwrap()
            .value,
        "committed candidate"
    );
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
