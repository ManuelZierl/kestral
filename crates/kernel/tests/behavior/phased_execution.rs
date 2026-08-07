//! Phased prepare/execute/finalize invocation behavior.

use crate::helpers::*;

#[test]
fn finalize_rejects_a_result_after_its_grant_is_revoked() {
    let (mut kernel, _chrome, _clock) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: chat_app(),
                reason: "test".into(),
            },
            "test",
        )
        .unwrap();
    let prepared = match kernel
        .prepare_invocation(
            &run_id,
            &create_note_ref(),
            InvocationRequest {
                input: obj(json!({"text": "late"})),
                data_scope: DataScope::None,
            },
        )
        .unwrap()
    {
        app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
    };
    let grant_id = kernel
        .grants_for(&chat_app())
        .into_iter()
        .next()
        .unwrap()
        .grant_id
        .clone();
    let approval = prepared.await_approval();
    kernel.revoke_grant(&grant_id).unwrap();
    let result = match kernel.authorize_invocation(approval).unwrap() {
        app_host_kernel::AuthorizeInvocation::Refused(result) => result,
        app_host_kernel::AuthorizeInvocation::Authorized(_) => {
            panic!("revoked grant dispatched a handler")
        }
    };
    assert_eq!(
        result,
        InvocationResult::Refused {
            reason: RefusalReason::GrantRevoked
        }
    );
    assert!(!kernel
        .records_for_run(&run_id)
        .any(|record| matches!(record.event, LedgerEvent::CapabilityCompleted { .. })));
}

#[test]
fn finalize_rejects_a_result_when_prepare_grant_is_replaced_by_another_covering_grant() {
    let (mut kernel, _chrome, _clock) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let issued = kernel
        .issue_grant(
            &chat_app(),
            &create_note_request(GrantCondition::Silent, GrantDuration::NonExpiring),
        )
        .unwrap();
    assert!(matches!(issued, IssueResult::Issued(_)));
    assert_eq!(kernel.grants_for(&chat_app()).len(), 2);

    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: chat_app(),
                reason: "test".into(),
            },
            "test",
        )
        .unwrap();
    let prepared = match kernel
        .prepare_invocation(
            &run_id,
            &create_note_ref(),
            InvocationRequest {
                input: obj(json!({"text": "stale"})),
                data_scope: DataScope::None,
            },
        )
        .unwrap()
    {
        app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
    };

    let prepared_grant_id = kernel
        .records_for_run(&run_id)
        .find_map(|record| match &record.event {
            LedgerEvent::CapabilityInvoked { grant_id, .. } => Some(grant_id.clone()),
            _ => None,
        })
        .expect("prepared invocation records the selected grant");

    kernel.revoke_grant(&prepared_grant_id).unwrap();

    let approval = prepared.await_approval();
    let result = kernel.authorize_invocation(approval).unwrap();
    assert!(matches!(
        result,
        app_host_kernel::AuthorizeInvocation::Refused(InvocationResult::Refused {
            reason: RefusalReason::Cancelled
        })
    ));
    assert!(kernel
        .records_for_run(&run_id)
        .any(|record| matches!(record.event, LedgerEvent::InvocationCancelled { .. })));
}

#[test]
fn prepared_execution_does_not_hold_the_kernel_for_unrelated_reads() {
    let (mut kernel, _chrome, _clock) = test_kernel();
    let (started_tx, started_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let finish_rx = Arc::new(Mutex::new(finish_rx));
    install_notes_with(
        &mut kernel,
        Box::new(move |_input, _context| {
            started_tx.send(()).unwrap();
            finish_rx.lock().unwrap().recv().unwrap();
            Ok(CapabilityOutcome {
                result: json!({}),
                artifacts: vec![],
            })
        }),
    );
    install_chat(&mut kernel);
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: chat_app(),
                reason: "test".into(),
            },
            "test",
        )
        .unwrap();
    let prepared = match kernel
        .prepare_invocation(
            &run_id,
            &create_note_ref(),
            InvocationRequest {
                input: obj(json!({"text": "parallel"})),
                data_scope: DataScope::None,
            },
        )
        .unwrap()
    {
        app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
    };
    let authorized = match kernel
        .authorize_invocation(prepared.await_approval())
        .unwrap()
    {
        app_host_kernel::AuthorizeInvocation::Authorized(authorized) => authorized,
        app_host_kernel::AuthorizeInvocation::Refused(_) => panic!("grant should be live"),
    };
    let worker = std::thread::spawn(move || authorized.execute());
    started_rx.recv().unwrap();
    // The token owns provider work and no Kernel reference, so reads proceed
    // while the handler waits in another thread.
    assert!(kernel.run_view(&run_id).unwrap().is_active());
    finish_tx.send(()).unwrap();
    let result = kernel.finalize_invocation(worker.join().unwrap()).unwrap();
    assert!(matches!(result, InvocationResult::Completed { .. }));
}

#[test]
fn revocation_during_execution_rejects_the_late_result() {
    let (mut kernel, _chrome, _clock) = test_kernel();
    let (started_tx, started_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let finish_rx = Arc::new(Mutex::new(finish_rx));
    install_notes_with(
        &mut kernel,
        Box::new(move |_input, _context| {
            started_tx.send(()).unwrap();
            finish_rx.lock().unwrap().recv().unwrap();
            Ok(CapabilityOutcome {
                result: json!({}),
                artifacts: vec![],
            })
        }),
    );
    install_chat(&mut kernel);
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: chat_app(),
                reason: "test".into(),
            },
            "test",
        )
        .unwrap();
    let prepared = match kernel
        .prepare_invocation(
            &run_id,
            &create_note_ref(),
            InvocationRequest {
                input: obj(json!({"text": "revoke"})),
                data_scope: DataScope::None,
            },
        )
        .unwrap()
    {
        app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
    };
    let grant_id = kernel
        .grants_for(&chat_app())
        .into_iter()
        .next()
        .unwrap()
        .grant_id
        .clone();
    let authorized = match kernel
        .authorize_invocation(prepared.await_approval())
        .unwrap()
    {
        app_host_kernel::AuthorizeInvocation::Authorized(authorized) => authorized,
        app_host_kernel::AuthorizeInvocation::Refused(_) => panic!("grant should be live"),
    };
    let worker = std::thread::spawn(move || authorized.execute());
    started_rx.recv().unwrap();
    kernel.revoke_grant(&grant_id).unwrap();
    finish_tx.send(()).unwrap();
    let result = kernel.finalize_invocation(worker.join().unwrap()).unwrap();
    assert_eq!(
        result,
        InvocationResult::Refused {
            reason: RefusalReason::GrantRevoked
        }
    );
}

#[test]
fn provider_uninstall_during_execution_rejects_the_late_result() {
    let (mut kernel, _chrome, _clock) = test_kernel();
    let (started_tx, started_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let finish_rx = Arc::new(Mutex::new(finish_rx));
    install_notes_with(
        &mut kernel,
        Box::new(move |_input, _context| {
            started_tx.send(()).unwrap();
            finish_rx.lock().unwrap().recv().unwrap();
            Ok(CapabilityOutcome {
                result: json!({}),
                artifacts: vec![],
            })
        }),
    );
    install_chat(&mut kernel);
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: chat_app(),
                reason: "test".into(),
            },
            "test",
        )
        .unwrap();
    let prepared = match kernel
        .prepare_invocation(
            &run_id,
            &create_note_ref(),
            InvocationRequest {
                input: obj(json!({"text": "uninstall"})),
                data_scope: DataScope::None,
            },
        )
        .unwrap()
    {
        app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
    };
    let authorized = match kernel
        .authorize_invocation(prepared.await_approval())
        .unwrap()
    {
        app_host_kernel::AuthorizeInvocation::Authorized(authorized) => authorized,
        app_host_kernel::AuthorizeInvocation::Refused(_) => panic!("grant should be live"),
    };
    let worker = std::thread::spawn(move || authorized.execute());
    started_rx.recv().unwrap();
    kernel.uninstall(&notes_app()).unwrap();
    finish_tx.send(()).unwrap();
    assert!(matches!(
        kernel.finalize_invocation(worker.join().unwrap()),
        Err(KernelError::PreparedInvocationConsumed)
    ));
    assert!(kernel
        .records_for_run(&run_id)
        .any(|record| matches!(record.event, LedgerEvent::InvocationCancelled { .. })));
}

#[test]
fn cancelled_run_never_dispatches_or_commits_handler_output() {
    let (mut kernel, _chrome, _clock) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: chat_app(),
                reason: "test".into(),
            },
            "test",
        )
        .unwrap();
    let prepared = match kernel
        .prepare_invocation(
            &run_id,
            &create_note_ref(),
            InvocationRequest {
                input: obj(json!({"text": "cancel"})),
                data_scope: DataScope::None,
            },
        )
        .unwrap()
    {
        app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
    };
    kernel
        .end_run(&run_id, RunTerminalState::Cancelled)
        .unwrap();
    assert!(matches!(
        kernel.authorize_invocation(prepared.await_approval()),
        Err(KernelError::PreparedInvocationConsumed)
    ));
    assert!(!kernel
        .records_for_run(&run_id)
        .any(|record| matches!(record.event, LedgerEvent::CapabilityCompleted { .. })));
}

#[test]
fn approval_is_revalidated_before_phased_finalization() {
    let (mut kernel, _chrome, _clock) = test_kernel();
    install_notes(&mut kernel, GrantCondition::RequiresApproval);
    install_chat_with_grant_condition(&mut kernel, GrantCondition::RequiresApproval);
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: chat_app(),
                reason: "test".into(),
            },
            "test",
        )
        .unwrap();
    let prepared = match kernel
        .prepare_invocation(
            &run_id,
            &create_note_ref(),
            InvocationRequest {
                input: obj(json!({"text": "approval"})),
                data_scope: DataScope::None,
            },
        )
        .unwrap()
    {
        app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
    };
    let grant_id = kernel
        .grants_for(&chat_app())
        .into_iter()
        .next()
        .unwrap()
        .grant_id
        .clone();
    let approval = prepared.await_approval();
    kernel.revoke_grant(&grant_id).unwrap();
    assert!(matches!(
        kernel.check_grant(&chat_app(), &create_note_ref()),
        app_host_kernel::services::broker::GrantCheck::Denied(DenialReason::Revoked)
    ));
    let result = match kernel.authorize_invocation(approval).unwrap() {
        app_host_kernel::AuthorizeInvocation::Refused(result) => result,
        app_host_kernel::AuthorizeInvocation::Authorized(_) => {
            panic!("revoked grant dispatched a handler")
        }
    };
    assert_eq!(
        result,
        InvocationResult::Refused {
            reason: RefusalReason::GrantRevoked
        }
    );
    assert!(!kernel
        .records_for_run(&run_id)
        .any(|record| matches!(record.event, LedgerEvent::CapabilityInvoked { .. })));
}

#[test]
fn timed_out_prepared_invocation_rejects_late_output() {
    let (mut kernel, _chrome, _clock) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: chat_app(),
                reason: "test".into(),
            },
            "test",
        )
        .unwrap();
    let prepared = match kernel
        .prepare_invocation_with_timeout(
            &run_id,
            &create_note_ref(),
            InvocationRequest {
                input: obj(json!({"text": "timeout"})),
                data_scope: DataScope::None,
            },
            std::time::Duration::ZERO,
        )
        .unwrap()
    {
        app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
    };
    assert!(matches!(
        kernel.authorize_invocation(prepared.await_approval()),
        Ok(app_host_kernel::AuthorizeInvocation::Refused(_))
    ));
}

#[test]
fn timed_out_invocation_is_reported_as_cancelled_not_revoked() {
    // Regression: a deadline crossing is not a grant revocation. The refusal
    // reason must be `Cancelled` and the ledger must record InvocationCancelled,
    // never InvocationRefused(Revoked) — the grant stayed valid.
    let (mut kernel, _chrome, _clock) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "timeout");
    let prepared = match kernel
        .prepare_invocation_with_timeout(
            &run_id,
            &create_note_ref(),
            InvocationRequest {
                input: obj(json!({"text": "x"})),
                data_scope: DataScope::None,
            },
            std::time::Duration::ZERO,
        )
        .unwrap()
    {
        app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
    };
    let result = match kernel
        .authorize_invocation(prepared.await_approval())
        .unwrap()
    {
        app_host_kernel::AuthorizeInvocation::Refused(result) => result,
        app_host_kernel::AuthorizeInvocation::Authorized(_) => {
            panic!("timed-out invocation dispatched a handler")
        }
    };
    assert_eq!(
        result,
        InvocationResult::Refused {
            reason: RefusalReason::Cancelled
        }
    );
    assert!(kernel
        .records_for_run(&run_id)
        .any(|record| matches!(record.event, LedgerEvent::InvocationCancelled { .. })));
    assert!(!kernel
        .records_for_run(&run_id)
        .any(|record| matches!(record.event, LedgerEvent::InvocationRefused { .. })));
}

#[test]
fn abort_prepared_invocation_records_cancellation_and_reclaims_state() {
    // Regression: an explicit abort of a prepared token reclaims its pending
    // reservation and records an honest cancellation, instead of leaking the
    // entry (which a silent drop would).
    let (mut kernel, _chrome, _clock) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "abort me");
    let prepared = match kernel
        .prepare_invocation(
            &run_id,
            &create_note_ref(),
            InvocationRequest {
                input: obj(json!({"text": "x"})),
                data_scope: DataScope::None,
            },
        )
        .unwrap()
    {
        app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
    };
    let result = kernel.abort_prepared_invocation(prepared).unwrap();
    assert_eq!(
        result,
        InvocationResult::Refused {
            reason: RefusalReason::Cancelled
        }
    );
    assert!(kernel
        .records_for_run(&run_id)
        .any(|record| matches!(record.event, LedgerEvent::InvocationCancelled { .. })));
    assert!(!kernel
        .records_for_run(&run_id)
        .any(|record| matches!(record.event, LedgerEvent::CapabilityCompleted { .. })));
}

#[test]
fn dropped_phase_token_is_cancelled_and_reclaimed_on_the_next_prepare() {
    let (mut kernel, _chrome, _clock) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "drop token");
    let request = || InvocationRequest {
        input: obj(json!({"text": "x"})),
        data_scope: DataScope::None,
    };
    let dropped = match kernel
        .prepare_invocation(&run_id, &create_note_ref(), request())
        .unwrap()
    {
        app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
    };
    drop(dropped);

    let next = match kernel
        .prepare_invocation(&run_id, &create_note_ref(), request())
        .unwrap()
    {
        app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
    };
    assert_eq!(
        kernel
            .records_for_run(&run_id)
            .filter(|record| matches!(record.event, LedgerEvent::InvocationCancelled { .. }))
            .count(),
        1
    );
    kernel.abort_prepared_invocation(next).unwrap();
}

#[test]
fn late_result_is_rejected_after_provider_uninstall() {
    let (mut kernel, _chrome, _clock) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: chat_app(),
                reason: "test".into(),
            },
            "test",
        )
        .unwrap();
    let prepared = match kernel
        .prepare_invocation(
            &run_id,
            &create_note_ref(),
            InvocationRequest {
                input: obj(json!({"text": "uninstall"})),
                data_scope: DataScope::None,
            },
        )
        .unwrap()
    {
        app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
        app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
    };
    kernel.uninstall(&notes_app()).unwrap();
    assert!(matches!(
        kernel.authorize_invocation(prepared.await_approval()),
        Err(KernelError::PreparedInvocationConsumed)
    ));
    assert!(kernel
        .records_for_run(&run_id)
        .any(|record| matches!(record.event, LedgerEvent::InvocationCancelled { .. })));
}

#[test]
fn phase_tokens_cannot_cross_kernel_instances() {
    fn prepared(
        kernel: &mut Kernel,
        run_id: &app_host_kernel::ids::RunId,
    ) -> app_host_kernel::PreparedInvocation {
        match kernel
            .prepare_invocation(
                run_id,
                &create_note_ref(),
                InvocationRequest {
                    input: obj(json!({"text": "isolated"})),
                    data_scope: DataScope::None,
                },
            )
            .unwrap()
        {
            app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
            app_host_kernel::PrepareInvocation::Refused(_) => panic!("grant should be live"),
        }
    }

    let (mut first, _, _) = test_kernel();
    let (mut second, _, _) = test_kernel();
    for kernel in [&mut first, &mut second] {
        install_notes(kernel, GrantCondition::Silent);
        install_chat(kernel);
    }
    let first_run = chat_message_run(&mut first, "first");
    let second_run = chat_message_run(&mut second, "second");
    let first_prepared = prepared(&mut first, &first_run);
    let second_prepared = prepared(&mut second, &second_run);

    assert!(matches!(
        second.authorize_invocation(first_prepared.await_approval()),
        Err(KernelError::PreparedInvocationConsumed)
    ));
    let second_authorized = match second
        .authorize_invocation(second_prepared.await_approval())
        .unwrap()
    {
        app_host_kernel::AuthorizeInvocation::Authorized(authorized) => authorized,
        app_host_kernel::AuthorizeInvocation::Refused(_) => panic!("grant should be live"),
    };

    let first_prepared = prepared(&mut first, &first_run);
    let first_authorized = match first
        .authorize_invocation(first_prepared.await_approval())
        .unwrap()
    {
        app_host_kernel::AuthorizeInvocation::Authorized(authorized) => authorized,
        app_host_kernel::AuthorizeInvocation::Refused(_) => panic!("grant should be live"),
    };
    assert!(matches!(
        second.finalize_invocation(first_authorized.execute()),
        Err(KernelError::PreparedInvocationConsumed)
    ));
    assert!(matches!(
        second
            .finalize_invocation(second_authorized.execute())
            .unwrap(),
        InvocationResult::Completed { .. }
    ));
    first
        .end_run(&first_run, RunTerminalState::Cancelled)
        .unwrap();
}

#[test]
fn unrepresentable_invocation_timeout_is_rejected_without_panicking() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "timeout overflow");

    assert!(matches!(
        kernel.prepare_invocation_with_timeout(
            &run_id,
            &create_note_ref(),
            InvocationRequest {
                input: obj(json!({"text": "x"})),
                data_scope: DataScope::None,
            },
            std::time::Duration::MAX,
        ),
        Err(KernelError::InvalidInvocationTimeout)
    ));
}
