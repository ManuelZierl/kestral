use super::*;
use crate::atomic_json::{FailingAtomicFileWriter, FailingFileOperation};
use app_host_kernel::ids::{AppId, CapabilityName, GrantId, RunId};
use app_host_kernel::primitives::capability::CapabilityRef;
use std::sync::Arc;
use uuid::Uuid;

fn notice(sequence: usize) -> ChromeNotice {
    ChromeNotice::GrantUse {
        app_id: AppId::new(format!("app-{sequence}")),
        capability: CapabilityRef {
            provider: AppId::new("notes"),
            capability: CapabilityName::new("create_note"),
        },
        grant_id: GrantId::new(format!("grant-{sequence}")),
        run_id: RunId::new(format!("run-{sequence}")),
    }
}

#[test]
fn records_survive_restart_and_sequence_continues() {
    let path = std::env::temp_dir().join(format!("trusted-notices-{}.json", Uuid::new_v4()));

    let first_sequence = {
        let mut store = TrustedNoticeStore::new(path.clone()).unwrap();
        assert_eq!(store.record(notice(0)).unwrap().sequence, 0);
        assert_eq!(store.record(notice(1)).unwrap().sequence, 1);
        store.recent()
    };
    assert_eq!(first_sequence.len(), 2);
    assert_eq!(first_sequence[0].sequence, 1);
    assert_eq!(first_sequence[1].sequence, 0);

    let mut reloaded = TrustedNoticeStore::new(path.clone()).unwrap();
    assert_eq!(reloaded.recent().len(), 2);
    assert_eq!(reloaded.record(notice(2)).unwrap().sequence, 2);
    assert_eq!(reloaded.recent()[0].sequence, 2);

    let _ = std::fs::remove_file(path);
}

#[test]
fn missing_grant_id_in_current_notice_is_rejected() {
    let path = std::env::temp_dir().join(format!("trusted-notices-{}.json", Uuid::new_v4()));
    std::fs::write(
        &path,
        r#"{"version":1,"next_sequence":1,"records":[{"sequence":0,"recorded_at":"2026-07-25T00:00:00Z","acknowledged_at":null,"notice":{"kind":"grant-use","app_id":"chat","capability":{"provider":"notes","capability":"create_note"},"run_id":"run-1"}}]}"#,
    )
    .unwrap();

    let error = TrustedNoticeStore::new(path.clone()).err().unwrap();

    assert!(error.to_string().contains("grant_id"), "{error}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn corrupt_trusted_notice_store_fails_fast_and_preserves_the_file() {
    let path = std::env::temp_dir().join(format!("trusted-notices-{}.json", Uuid::new_v4()));
    let corrupt = "{not-json";
    std::fs::write(&path, corrupt).unwrap();

    let error = TrustedNoticeStore::new(path.clone()).err().unwrap();

    assert!(error
        .to_string()
        .contains("parse trusted notice store failed"));
    assert!(error.to_string().contains("preserved"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), corrupt);
}

#[test]
fn trusted_notice_retention_is_bounded_and_recent_first() {
    let path = std::env::temp_dir().join(format!("trusted-notices-{}.json", Uuid::new_v4()));
    let mut store = TrustedNoticeStore::new(path.clone()).unwrap();

    for sequence in 0..1005 {
        assert_eq!(
            store.record(notice(sequence)).unwrap().sequence,
            sequence as u64
        );
    }

    let recent = store.recent();
    assert_eq!(recent.len(), 1000);
    assert_eq!(recent.first().unwrap().sequence, 1004);
    assert_eq!(recent.last().unwrap().sequence, 5);

    let _ = std::fs::remove_file(path);
}

#[test]
fn failed_trusted_notice_write_keeps_memory_and_disk_unchanged() {
    let path = std::env::temp_dir().join(format!("trusted-notices-{}.json", Uuid::new_v4()));
    let writer = Arc::new(FailingAtomicFileWriter::new(FailingFileOperation::Write));
    let mut store = TrustedNoticeStore::with_writer(path.clone(), writer).unwrap();

    let error = store.record(notice(0)).unwrap_err();

    assert!(error.to_string().contains("injected write failure"));
    assert!(store.recent().is_empty());
    assert!(!path.exists());
}
