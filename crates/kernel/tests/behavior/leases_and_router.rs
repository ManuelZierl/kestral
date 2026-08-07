use crate::helpers::*;
use app_host_kernel::ids::RunId;
use app_host_kernel::services::registry::APP_DATA_CHANGED_TOPIC;
use app_host_kernel::services::router::{AppDataChangeKind, LeaseOutcome, LeaseTarget};

fn workspace_file() -> LeaseTarget {
    LeaseTarget::WorkspacePath {
        path: "notes/inbox.md".into(),
    }
}

fn two_runs(kernel: &mut Kernel) -> (RunId, RunId) {
    install_chat(kernel);
    let first = chat_message_run(kernel, "a");
    let second = chat_message_run(kernel, "b");
    (first, second)
}

#[test]
fn conflicting_lease_is_refused_and_surfaced() {
    let (mut kernel, chrome, _) = test_kernel();
    let (first, second) = two_runs(&mut kernel);
    assert!(matches!(
        kernel
            .acquire_lease(&first, workspace_file(), Duration::minutes(5))
            .unwrap(),
        LeaseOutcome::Acquired(_)
    ));
    match kernel
        .acquire_lease(&second, workspace_file(), Duration::minutes(5))
        .unwrap()
    {
        LeaseOutcome::Conflict { holder } => assert_eq!(holder.run_id, first),
        other => panic!("expected conflict, got {other:?}"),
    }
    assert_eq!(
        *chrome.notices.lock().unwrap(),
        vec![ChromeNotice::LeaseConflict {
            resource: "workspace:notes/inbox.md".into(),
            holding_run: first,
            requesting_run: second,
        }]
    );
}

#[test]
fn expired_lease_frees_the_target() {
    let (mut kernel, _, clock) = test_kernel();
    let (first, second) = two_runs(&mut kernel);
    kernel
        .acquire_lease(&first, workspace_file(), Duration::minutes(5))
        .unwrap();
    clock.advance_to(start_time() + Duration::minutes(6));
    assert!(matches!(
        kernel
            .acquire_lease(&second, workspace_file(), Duration::minutes(5))
            .unwrap(),
        LeaseOutcome::Acquired(_)
    ));
}

#[test]
fn same_run_reacquire_renews_the_lease() {
    let (mut kernel, _, clock) = test_kernel();
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "long-running edit");
    let LeaseOutcome::Acquired(original) = kernel
        .acquire_lease(&run_id, workspace_file(), Duration::minutes(5))
        .unwrap()
    else {
        panic!("expected acquisition");
    };
    clock.advance_to(start_time() + Duration::minutes(3));
    let LeaseOutcome::Acquired(renewed) = kernel
        .acquire_lease(&run_id, workspace_file(), Duration::minutes(5))
        .unwrap()
    else {
        panic!("expected renewal");
    };
    // Renewal, not a twin: the same lease, with a pushed-out expiry.
    assert_eq!(renewed.lease_id, original.lease_id);
    assert_eq!(renewed.expires_at, start_time() + Duration::minutes(8));
    assert_eq!(renewed.acquired_at, original.acquired_at);
}

#[test]
fn non_positive_lease_duration_is_rejected() {
    let (mut kernel, _, _) = test_kernel();
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "invalid lease");

    for duration in [Duration::zero(), Duration::seconds(-1)] {
        assert!(matches!(
            kernel.acquire_lease(&run_id, workspace_file(), duration),
            Err(KernelError::InvalidLeaseDuration)
        ));
    }
}

#[test]
fn run_end_releases_its_leases() {
    let (mut kernel, _, _) = test_kernel();
    let (first, second) = two_runs(&mut kernel);
    kernel
        .acquire_lease(&first, workspace_file(), Duration::minutes(5))
        .unwrap();
    kernel.end_run(&first, RunTerminalState::Completed).unwrap();
    assert!(matches!(
        kernel
            .acquire_lease(&second, workspace_file(), Duration::minutes(5))
            .unwrap(),
        LeaseOutcome::Acquired(_)
    ));
}

#[test]
fn uninstall_cancels_runs_and_releases_their_leases() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let chat_run = chat_message_run(&mut kernel, "editing inbox");
    kernel
        .acquire_lease(&chat_run, workspace_file(), Duration::minutes(5))
        .unwrap();

    kernel.uninstall(&chat_app()).unwrap();

    let notes_run = kernel
        .start_run(
            Initiator::App {
                app_id: notes_app(),
                reason: "take over edit".into(),
            },
            "take over edit",
        )
        .unwrap();
    assert!(matches!(
        kernel
            .acquire_lease(&notes_run, workspace_file(), Duration::minutes(5))
            .unwrap(),
        LeaseOutcome::Acquired(_)
    ));
}

#[test]
fn app_data_change_is_delivered_only_to_explicit_subscribers() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    kernel
        .install(
            seal(chat_manifest(
                chat_app(),
                vec![EventTopic::new(APP_DATA_CHANGED_TOPIC)],
            )),
            BTreeMap::new(),
        )
        .unwrap();

    kernel
        .publish_app_data_change(&chat_app(), "chat-thread-1", 2, AppDataChangeKind::Updated)
        .unwrap();

    assert_eq!(
        kernel.drain_inbox(&chat_app()).unwrap(),
        vec![
            app_host_kernel::services::router::AppEventEnvelope::AppDataChanged {
                provider_app_id: chat_app(),
                resource_ref: "chat-thread-1".into(),
                revision: 2,
                change_kind: AppDataChangeKind::Updated,
            }
        ]
    );
}

#[test]
fn app_data_change_rejects_empty_resource_and_allows_initial_revision() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    assert!(kernel
        .publish_app_data_change(&notes_app(), "", 1, AppDataChangeKind::Created)
        .is_err());
    kernel
        .publish_app_data_change(&notes_app(), "chat-thread-1", 0, AppDataChangeKind::Created)
        .unwrap();
}

#[test]
fn only_subscribed_topics_are_delivered() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    kernel
        .install(
            seal(chat_manifest(
                chat_app(),
                vec![EventTopic::new("run-ended")],
            )),
            BTreeMap::new(),
        )
        .unwrap();
    let run_id = chat_message_run(&mut kernel, "g");
    kernel
        .end_run(&run_id, RunTerminalState::Completed)
        .unwrap();

    let delivered = kernel.drain_inbox(&chat_app()).unwrap();
    assert_eq!(
        delivered,
        vec![
            app_host_kernel::services::router::AppEventEnvelope::RunEvent {
                topic: EventTopic::new("run-ended"),
                run_id: run_id.clone(),
                actor: chat_app(),
                capability: None,
                artifact_id: None,
                terminal_state: Some(RunTerminalState::Completed),
            }
        ]
    );
    assert!(kernel.drain_inbox(&chat_app()).unwrap().is_empty());
}

#[test]
fn subscription_payloads_are_minimized_event_views() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    kernel
        .install(
            seal(chat_manifest(
                chat_app(),
                vec![EventTopic::new("capability-invoked")],
            )),
            BTreeMap::new(),
        )
        .unwrap();
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: chat_app(),
                reason: "automation fired".into(),
            },
            "create note",
        )
        .unwrap();
    kernel
        .invoke(
            &run_id,
            &create_note_ref(),
            obj(json!({"text": "secret payload"})),
        )
        .unwrap();

    assert_eq!(
        kernel.drain_inbox(&chat_app()).unwrap(),
        vec![
            app_host_kernel::services::router::AppEventEnvelope::RunEvent {
                topic: EventTopic::new("capability-invoked"),
                run_id,
                actor: chat_app(),
                capability: Some(create_note_ref()),
                artifact_id: None,
                terminal_state: None,
            }
        ]
    );
}

#[test]
fn unknown_app_cannot_drain_an_inbox() {
    let (mut kernel, _, _) = test_kernel();
    let error = kernel.drain_inbox(&AppId::new("ghost")).unwrap_err();
    assert!(matches!(error, KernelError::UnknownApp(_)));
}

#[test]
fn inboxes_are_bounded_and_expose_dropped_event_counts() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    kernel
        .install(
            seal(chat_manifest(
                chat_app(),
                vec![EventTopic::new("run-started")],
            )),
            BTreeMap::new(),
        )
        .unwrap();

    for index in 0..=app_host_kernel::services::router::MessageRouter::MAX_INBOX_EVENTS {
        let run_id = kernel
            .start_run(
                Initiator::App {
                    app_id: chat_app(),
                    reason: format!("event {index}"),
                },
                "bounded inbox",
            )
            .unwrap();
        kernel
            .end_run(&run_id, RunTerminalState::Completed)
            .unwrap();
    }

    assert_eq!(
        kernel.inbox_status(&chat_app()).unwrap(),
        app_host_kernel::services::router::EventInboxStatus {
            queued_events: app_host_kernel::services::router::MessageRouter::MAX_INBOX_EVENTS,
            dropped_events: 1,
        }
    );
    assert_eq!(
        kernel.drain_inbox(&chat_app()).unwrap().len(),
        app_host_kernel::services::router::MessageRouter::MAX_INBOX_EVENTS
    );
    assert_eq!(kernel.inbox_status(&chat_app()).unwrap().dropped_events, 1);
}
