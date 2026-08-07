use crate::helpers::*;
use app_host_kernel::ids::RunId;

#[test]
fn records_are_sequenced() {
    let (mut kernel, _, _) = test_kernel();
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "take a note");
    kernel
        .end_run(&run_id, RunTerminalState::Completed)
        .unwrap();
    let sequences: Vec<u64> = kernel.records().iter().map(|r| r.sequence).collect();
    assert_eq!(sequences, (0..sequences.len() as u64).collect::<Vec<_>>());
}

#[test]
fn unknown_run_has_no_view() {
    let (kernel, _, _) = test_kernel();
    assert!(matches!(
        kernel
            .run_view(&RunId::new("run-never-started"))
            .unwrap_err(),
        KernelError::UnknownRun(_)
    ));
}

// The append guards are ledger invariants; the kernel exposes no way to
// append directly (that privilege gap is the point), so they are tested
// on the service itself.

#[test]
fn ledger_rejects_events_after_run_end() {
    let mut ledger = RunLedger::new(FixedClock::new(start_time()));
    let run_id = new_run_id();
    ledger
        .append(LedgerEvent::RunStarted {
            run_id: run_id.clone(),
            initiator: Initiator::App {
                app_id: chat_app(),
                reason: "chat message".into(),
            },
            goal: "g".into(),
        })
        .unwrap();
    ledger
        .append(LedgerEvent::RunEnded {
            run_id: run_id.clone(),
            terminal_state: RunTerminalState::Completed,
        })
        .unwrap();
    let error = ledger
        .append(LedgerEvent::RunEnded {
            run_id,
            terminal_state: RunTerminalState::Failed,
        })
        .unwrap_err();
    assert!(matches!(error, KernelError::RunAlreadyEnded(_)));
}

#[test]
fn ledger_rejects_a_second_start_for_the_same_run() {
    let mut ledger = RunLedger::new(FixedClock::new(start_time()));
    let run_id = new_run_id();
    let start = |run_id: RunId, goal: &str| LedgerEvent::RunStarted {
        run_id,
        initiator: Initiator::App {
            app_id: chat_app(),
            reason: "chat message".into(),
        },
        goal: goal.into(),
    };
    ledger.append(start(run_id.clone(), "g")).unwrap();
    let error = ledger.append(start(run_id, "fork history")).unwrap_err();
    assert!(matches!(error, KernelError::RunAlreadyStarted(_)));
}

#[test]
fn child_runs_carry_their_parent() {
    let (mut kernel, _, _) = test_kernel();
    install_chat(&mut kernel);
    let parent = chat_message_run(&mut kernel, "parent goal");
    let child = kernel
        .start_run(
            Initiator::Run {
                app_id: chat_app(),
                parent_run_id: parent.clone(),
            },
            "child goal",
        )
        .unwrap();
    let view = kernel.run_view(&child).unwrap();
    assert_eq!(
        view.initiator,
        Initiator::Run {
            app_id: chat_app(),
            parent_run_id: parent,
        }
    );
}

#[test]
fn child_run_cannot_launder_attribution() {
    let (mut kernel, _, _) = test_kernel();
    install_chat(&mut kernel);
    install_notes(&mut kernel, GrantCondition::Silent);
    let parent = chat_message_run(&mut kernel, "g");
    let error = kernel
        .start_run(
            Initiator::Run {
                app_id: notes_app(),
                parent_run_id: parent,
            },
            "pretend to be notes",
        )
        .unwrap_err();
    assert!(matches!(
        error,
        KernelError::ChildRunAttributionMismatch { .. }
    ));
}

#[test]
fn child_run_requires_active_parent() {
    let (mut kernel, _, _) = test_kernel();
    install_chat(&mut kernel);
    let parent = chat_message_run(&mut kernel, "g");
    kernel
        .end_run(&parent, RunTerminalState::Completed)
        .unwrap();
    let error = kernel
        .start_run(
            Initiator::Run {
                app_id: chat_app(),
                parent_run_id: parent,
            },
            "too late",
        )
        .unwrap_err();
    assert!(matches!(error, KernelError::RunAlreadyEnded(_)));
}
