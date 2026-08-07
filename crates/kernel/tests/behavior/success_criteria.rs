use crate::helpers::*;

/// Criterion 1: chat uses no privileged API - install, start a run,
/// invoke a granted capability; every step is the public surface.
#[test]
fn criterion_1_chat_remains_ordinary() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);

    let run_id = chat_message_run(&mut kernel, "note that the demo went well");
    let result = kernel
        .invoke(
            &run_id,
            &create_note_ref(),
            obj(json!({"text": "demo went well"})),
        )
        .unwrap();
    kernel
        .end_run(&run_id, RunTerminalState::Completed)
        .unwrap();

    assert!(matches!(result, InvocationResult::Completed { .. }));
    assert_eq!(kernel.grants_for(&chat_app()).len(), 1);
}

/// Criterion 3: an external app with the same grants replicates the
/// bundled chat app move for move.
#[test]
fn criterion_3_third_party_parity() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let third_party = AppId::new("replichat");
    kernel
        .install(
            seal(chat_manifest(third_party.clone(), vec![])),
            BTreeMap::new(),
        )
        .unwrap();

    let mut create_note_via = |app_id: AppId| {
        let run_id = kernel
            .start_run(
                Initiator::App {
                    app_id,
                    reason: "chat message".into(),
                },
                "same goal",
            )
            .unwrap();
        let result = kernel
            .invoke(
                &run_id,
                &create_note_ref(),
                obj(json!({"text": "same text"})),
            )
            .unwrap();
        kernel
            .end_run(&run_id, RunTerminalState::Completed)
            .unwrap();
        match result {
            InvocationResult::Completed { result, artifacts } => {
                (result, artifacts[0].content.clone())
            }
            other => panic!("expected completion, got {other:?}"),
        }
    };

    let bundled = create_note_via(chat_app());
    let replicated = create_note_via(third_party);
    assert_eq!(replicated, bundled);
}

/// Criterion 4: any artifact traces through the ledger to a run, a
/// capability, a grant, and an initiator.
#[test]
fn criterion_4_every_action_is_attributable() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "traceable note");
    let result = kernel
        .invoke(
            &run_id,
            &create_note_ref(),
            obj(json!({"text": "trace me"})),
        )
        .unwrap();
    let InvocationResult::Completed { artifacts, .. } = result else {
        panic!("expected completion");
    };

    let provenance = kernel
        .artifact(&artifacts[0].artifact_id)
        .unwrap()
        .provenance
        .clone();
    assert_eq!(provenance.run_id, run_id);
    assert_eq!(provenance.capability, create_note_ref());
    assert_eq!(provenance.produced_by, notes_app());

    let view = kernel.run_view(&provenance.run_id).unwrap();
    assert!(view.grants_exercised.contains(&provenance.grant_id));
    assert_eq!(
        view.initiator,
        Initiator::App {
            app_id: chat_app(),
            reason: "chat message".into()
        }
    );
    assert!(view.artifacts_produced.contains(&artifacts[0].artifact_id));
}
