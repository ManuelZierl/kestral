use std::sync::Arc;

use super::*;
use crate::clock::FixedClock;
use crate::ids::{AppId, CapabilityName, GrantId};
use chrono::TimeZone;

#[test]
fn batch_append_failure_leaves_no_partial_ledger_or_run_state() {
    let clock = Arc::new(FixedClock::new(
        Utc.with_ymd_and_hms(2026, 7, 10, 12, 0, 0).unwrap(),
    ));
    let mut ledger = RunLedger::new(clock);
    let run_id = RunId::new("run-atomic");
    let capability = CapabilityRef {
        provider: AppId::new("notes"),
        capability: CapabilityName::new("create_note"),
    };
    ledger
        .append(LedgerEvent::RunStarted {
            run_id: run_id.clone(),
            initiator: Initiator::App {
                app_id: AppId::new("chat"),
                reason: "test".into(),
            },
            goal: "test atomic batch".into(),
        })
        .unwrap();

    // Inject an invalid final event after a staged RunEnded record. The
    // public ledger must retain neither staged record on rejection.
    let result = ledger.append_batch(vec![
        LedgerEvent::RunEnded {
            run_id: run_id.clone(),
            terminal_state: RunTerminalState::Completed,
        },
        LedgerEvent::CapabilityCompleted {
            run_id: run_id.clone(),
            capability,
            grant_id: GrantId::new("grant-atomic"),
            result_sha256: "digest".into(),
            data_scope: crate::primitives::grant::DataScope::None,
        },
    ]);

    assert!(matches!(result, Err(KernelError::RunAlreadyEnded(_))));
    assert_eq!(ledger.records().len(), 1);
    assert!(ledger.run_view(&run_id).unwrap().is_active());
    assert_eq!(ledger.active_run_ids(), vec![run_id]);
}

#[test]
fn restore_rebuilds_the_active_run_index() {
    let clock = Arc::new(FixedClock::new(
        Utc.with_ymd_and_hms(2026, 7, 10, 12, 0, 0).unwrap(),
    ));
    let mut ledger = RunLedger::new(clock.clone());
    let active_run = RunId::new("run-active");
    let ended_run = RunId::new("run-ended");
    for run_id in [&active_run, &ended_run] {
        ledger
            .append(LedgerEvent::RunStarted {
                run_id: run_id.clone(),
                initiator: Initiator::App {
                    app_id: AppId::new("chat"),
                    reason: "test".into(),
                },
                goal: "test restore".into(),
            })
            .unwrap();
    }
    ledger
        .append(LedgerEvent::RunEnded {
            run_id: ended_run.clone(),
            terminal_state: RunTerminalState::Completed,
        })
        .unwrap();

    let mut restored = RunLedger::restore(clock, ledger.records().to_vec()).unwrap();

    assert_eq!(restored.active_run_ids(), vec![active_run]);
    assert!(matches!(
        restored.append(LedgerEvent::RunEnded {
            run_id: ended_run.clone(),
            terminal_state: RunTerminalState::Completed,
        }),
        Err(KernelError::RunAlreadyEnded(run_id)) if run_id == ended_run
    ));
}
