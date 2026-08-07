use std::sync::{atomic::AtomicBool, Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use super::{CancellationHandle, ProgressReportStatus, ProgressReporter};

#[test]
fn progress_rejects_untyped_and_oversized_events() {
    let reporter = ProgressReporter::default();
    assert_eq!(
        reporter.report(json!({"content": "missing kind"})),
        ProgressReportStatus::Invalid
    );
    assert_eq!(
        reporter.report(json!({"kind": "chunk", "content": "x".repeat(65 * 1024)})),
        ProgressReportStatus::Oversized
    );
}

#[test]
fn progress_applies_backpressure_without_dropping_deltas() {
    let values = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&values);
    let reporter = ProgressReporter::new(move |value| captured.lock().unwrap().push(value));

    assert_eq!(
        reporter.report(json!({"kind": "first"})),
        ProgressReportStatus::Emitted
    );
    assert_eq!(
        reporter.report(json!({"kind": "second"})),
        ProgressReportStatus::Emitted
    );
    assert_eq!(values.lock().unwrap().len(), 2);
}

#[test]
fn progress_consumer_failure_requests_cancellation() {
    let cancellation = CancellationHandle::new(
        Arc::new(AtomicBool::new(false)),
        Instant::now() + Duration::from_secs(1),
    );
    let reporter =
        ProgressReporter::new_checked(|_| Err(())).with_cancellation(cancellation.clone());

    assert_eq!(
        reporter.report(json!({"kind": "chunk"})),
        ProgressReportStatus::ConsumerGone
    );
    assert!(cancellation.is_cancelled());
}
