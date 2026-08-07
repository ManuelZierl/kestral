use super::*;
use app_host_kernel::durable::{CommitOutcome, KernelStateStore};
use uuid::Uuid;

fn temp_path() -> PathBuf {
    std::env::temp_dir()
        .join(format!("kernel-state-test-{}", Uuid::new_v4()))
        .join("kernel-state-v1.json")
}

#[test]
fn state_round_trips_and_second_writer_is_locked_out() {
    let path = temp_path();
    let store = FileKernelStateStore::open(path.clone()).unwrap();
    assert_eq!(
        store.commit(&DurableKernelState::empty()),
        CommitOutcome::Committed
    );
    assert_eq!(store.load().unwrap(), Some(DurableKernelState::empty()));
    assert!(FileKernelStateStore::open(path).is_err());
}

#[test]
fn corrupt_state_fails_instead_of_starting_empty() {
    let path = temp_path();
    let store = FileKernelStateStore::open(path.clone()).unwrap();
    assert_eq!(
        store.commit(&DurableKernelState::empty()),
        CommitOutcome::Committed
    );
    drop(store);
    let mut raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    raw["state_sha256"] = serde_json::Value::String("bad".into());
    fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();
    let store = FileKernelStateStore::open(path).unwrap();
    assert!(store.load().unwrap_err().contains("checksum mismatch"));
}

#[test]
fn missing_required_ledger_field_is_rejected() {
    let path = temp_path();
    let state = serde_json::json!({
        "installed_apps": [],
        "grants": [],
        "revoked_grant_ids": [],
        "ledger_records": [{
            "sequence": 0,
            "recorded_at": "2026-01-01T00:00:00Z",
            "event": {
                "kind": "invocation-cancelled",
                "run_id": "run-missing-field",
                "capability": {
                    "provider": "provider",
                    "capability": "old-shape"
                }
            }
        }],
        "artifacts": []
    });
    let document = KernelStateDocument {
        format_version: FORMAT_VERSION,
        state_sha256: FileKernelStateStore::checksum_serializable(&state).unwrap(),
        state,
    };
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();

    let error = FileKernelStateStore::open(path)
        .unwrap()
        .load()
        .unwrap_err();
    assert!(error.contains("data_scope"), "{error}");
}

#[test]
fn raw_checksum_removes_only_json_formatting_whitespace() {
    let raw = RawValue::from_string(
        r#"{ "message": "space and \"quote\" and \\ slash", "nested": [ 1, 2 ] }"#.into(),
    )
    .unwrap();
    let expected =
        Sha256::digest(br#"{"message":"space and \"quote\" and \\ slash","nested":[1,2]}"#);

    assert_eq!(
        FileKernelStateStore::checksum_raw(&raw),
        format!("{expected:x}")
    );
}
