use crate::helpers::*;

#[test]
fn silent_grant_invocation_completes_and_is_recorded() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "note that milk is out");

    let result = kernel
        .invoke(
            &run_id,
            &create_note_ref(),
            obj(json!({"text": "milk is out"})),
        )
        .unwrap();

    assert!(matches!(result, InvocationResult::Completed { .. }));
    let view = kernel.run_view(&run_id).unwrap();
    assert_eq!(view.invocations.len(), 1);
    assert!(matches!(
        view.invocations[0],
        InvocationRecord::Completed { .. }
    ));
    assert!(!view.grants_exercised.is_empty());
}

#[test]
fn notify_grant_shows_chrome_notice() {
    let (mut kernel, chrome, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Notify);
    let grant_id = kernel.grants_for(&notes_app())[0].grant_id.clone();
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: notes_app(),
                reason: "automation fired".into(),
            },
            "daily note",
        )
        .unwrap();
    kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "daily"})))
        .unwrap();
    assert_eq!(
        *chrome.notices.lock().unwrap(),
        vec![ChromeNotice::GrantUse {
            app_id: notes_app(),
            capability: create_note_ref(),
            grant_id,
            run_id,
        }]
    );
}

#[test]
fn direct_provider_surface_action_does_not_show_notify_notice() {
    let (mut kernel, chrome, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Notify);
    let binding = kernel
        .open_surface(&notes_app(), &composer_surface())
        .unwrap();

    let outcome = kernel
        .submit_action(&binding, create_note_intent("direct user action"))
        .unwrap();

    assert!(matches!(outcome.result, InvocationResult::Completed { .. }));
    assert!(chrome.notices.lock().unwrap().is_empty());
}

#[test]
fn direct_provider_surface_action_does_not_request_per_use_approval() {
    let (mut kernel, chrome, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::RequiresApproval);
    chrome.set_capability_decision(ApprovalDecision::Denied);
    let binding = kernel
        .open_surface(&notes_app(), &composer_surface())
        .unwrap();

    let outcome = kernel
        .submit_action(&binding, create_note_intent("direct user action"))
        .unwrap();

    assert!(matches!(outcome.result, InvocationResult::Completed { .. }));
    assert!(chrome.approval_prompts.lock().unwrap().is_empty());
    assert!(kernel
        .run_view(&outcome.run_id)
        .unwrap()
        .approvals
        .is_empty());
}

#[test]
fn notify_grant_notice_persistence_failure_fails_invocation() {
    let (mut kernel, chrome, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Notify);
    chrome.set_notice_error(ChromeNoticeError::Persistence {
        message: "injected notice failure".into(),
    });
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: notes_app(),
                reason: "automation fired".into(),
            },
            "daily note",
        )
        .unwrap();

    let error = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "daily"})))
        .unwrap_err();

    assert!(
        matches!(error, KernelError::Durability(message) if message.contains("trusted notice persistence failed"))
    );
    assert!(chrome.notices.lock().unwrap().is_empty());
}

#[test]
fn requires_approval_approved_runs_and_records_approval() {
    let (mut kernel, chrome, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::RequiresApproval);
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: notes_app(),
                reason: "chat message".into(),
            },
            "approved note",
        )
        .unwrap();

    let result = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "yes"})))
        .unwrap();

    assert!(matches!(result, InvocationResult::Completed { .. }));
    assert_eq!(chrome.approval_prompts.lock().unwrap().len(), 1);
    let view = kernel.run_view(&run_id).unwrap();
    assert_eq!(
        view.approvals
            .iter()
            .map(|a| a.approved)
            .collect::<Vec<_>>(),
        vec![true]
    );
    let kinds: Vec<&str> = kernel
        .records_for_run(&run_id)
        .map(|r| r.event.kind())
        .collect();
    assert_eq!(
        kinds,
        vec![
            "run-started",
            "approval-requested",
            "approval-granted",
            "capability-invoked",
            "artifact-produced",
            "capability-completed",
        ]
    );
}

#[test]
fn requires_approval_denial_refuses_before_any_code_runs() {
    let (mut kernel, chrome, _) = test_kernel();
    let calls = Arc::new(Mutex::new(0usize));
    install_notes_with(&mut kernel, counting_note_handler(calls.clone()));
    kernel
        .revoke_grant(&kernel.grants_for(&notes_app())[0].grant_id.clone())
        .unwrap();
    kernel
        .issue_grant(
            &notes_app(),
            &create_note_request(GrantCondition::RequiresApproval, GrantDuration::NonExpiring),
        )
        .unwrap();
    chrome.set_capability_decision(ApprovalDecision::Denied);
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: notes_app(),
                reason: "chat message".into(),
            },
            "risky note",
        )
        .unwrap();

    let result = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "nope"})))
        .unwrap();

    assert_eq!(
        result,
        InvocationResult::Refused {
            reason: RefusalReason::ApprovalDenied
        }
    );
    let view = kernel.run_view(&run_id).unwrap();
    assert_eq!(*calls.lock().unwrap(), 0);
    assert_eq!(
        view.approvals
            .iter()
            .map(|a| a.approved)
            .collect::<Vec<_>>(),
        vec![false]
    );
    assert!(view.invocations.is_empty());
    assert_eq!(
        chrome.approval_prompts.lock().unwrap()[0].goal,
        "risky note"
    );
    let kinds: Vec<&str> = kernel
        .records_for_run(&run_id)
        .map(|r| r.event.kind())
        .collect();
    assert_eq!(
        kinds,
        vec!["run-started", "approval-requested", "approval-denied"]
    );
}

#[test]
fn approval_cannot_resurrect_an_expired_grant() {
    let (mut kernel, chrome, clock) = test_kernel();
    kernel
        .install(
            seal(notes_manifest_with_duration(
                GrantCondition::RequiresApproval,
                expires_after(3600),
            )),
            notes_handlers(),
        )
        .unwrap();
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: notes_app(),
                reason: "chat message".into(),
            },
            "slow user",
        )
        .unwrap();
    // The user sits on the prompt for two hours; the grant dies meanwhile.
    chrome.advance_clock_on_approval(clock, start_time() + Duration::hours(2));

    let result = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "late"})))
        .unwrap();

    assert_eq!(
        result,
        InvocationResult::Refused {
            reason: RefusalReason::GrantExpired
        }
    );
    let kinds: Vec<&str> = kernel
        .records_for_run(&run_id)
        .map(|r| r.event.kind())
        .collect();
    assert!(kinds.contains(&"invocation-refused"));
    assert!(!kinds.contains(&"capability-invoked"));
}

#[test]
fn no_grant_is_refused_and_recorded() {
    let (mut kernel, chrome, _) = test_kernel();
    let calls = Arc::new(Mutex::new(0usize));
    install_notes_with(&mut kernel, counting_note_handler(calls.clone()));
    chrome.set_grant_decision(ApprovalDecision::Denied); // chat gets no grant
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "g");

    let result = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "denied"})))
        .unwrap();

    assert_eq!(
        result,
        InvocationResult::Refused {
            reason: RefusalReason::NoGrant
        }
    );
    let view = kernel.run_view(&run_id).unwrap();
    assert_eq!(
        view.invocations,
        vec![InvocationRecord::Refused {
            capability: create_note_ref(),
            reason: DenialReason::NoGrant,
            data_scope: DataScope::None,
        }]
    );
    assert_eq!(*calls.lock().unwrap(), 0);
    let kinds: Vec<&str> = kernel
        .records_for_run(&run_id)
        .map(|r| r.event.kind())
        .collect();
    assert!(!kinds.contains(&"capability-invoked"));
}

#[test]
fn revoked_mid_run_grant_refuses_the_next_invocation() {
    let (mut kernel, _, _) = test_kernel();
    let calls = Arc::new(Mutex::new(0usize));
    install_notes_with(&mut kernel, counting_note_handler(calls.clone()));
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "two notes");

    let first = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "one"})))
        .unwrap();
    assert!(matches!(first, InvocationResult::Completed { .. }));
    assert_eq!(*calls.lock().unwrap(), 1);

    let grant_id = kernel.grants_for(&chat_app())[0].grant_id.clone();
    kernel.revoke_grant(&grant_id).unwrap();

    let second = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "two"})))
        .unwrap();
    assert_eq!(
        second,
        InvocationResult::Refused {
            reason: RefusalReason::GrantRevoked
        }
    );
    let view = kernel.run_view(&run_id).unwrap();
    assert!(matches!(
        view.invocations.last().unwrap(),
        InvocationRecord::Refused {
            reason: DenialReason::Revoked,
            ..
        }
    ));
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[test]
fn invoking_on_an_ended_run_is_impossible() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "g");
    kernel
        .end_run(&run_id, RunTerminalState::Completed)
        .unwrap();
    let error = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "late"})))
        .unwrap_err();
    assert!(matches!(error, KernelError::RunAlreadyEnded(_)));
}

#[test]
fn input_violating_declared_schema_fails_at_the_boundary() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "g");
    assert!(matches!(
        kernel
            .invoke(&run_id, &create_note_ref(), obj(json!({"text": 42})))
            .unwrap_err(),
        KernelError::InvalidCapabilityInput { .. }
    ));
    assert!(matches!(
        kernel
            .invoke(
                &run_id,
                &create_note_ref(),
                obj(json!({"text": "ok", "surprise": true}))
            )
            .unwrap_err(),
        KernelError::InvalidCapabilityInput { .. }
    ));
}

#[test]
fn failing_handler_is_contained_and_recorded() {
    let (mut kernel, _, _) = test_kernel();
    install_notes_with(
        &mut kernel,
        Box::new(|_, _| {
            Err(app_host_kernel::invocation::HandlerFailure(
                "backend down".into(),
            ))
        }),
    );
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "g");

    let result = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "boom"})))
        .unwrap();

    assert_eq!(
        result,
        InvocationResult::Failed {
            error: "backend down".into()
        }
    );
    let view = kernel.run_view(&run_id).unwrap();
    assert!(matches!(
        view.invocations[0],
        InvocationRecord::Failed { .. }
    ));
    assert!(view.is_active()); // the kernel survived; the run may continue
}

#[test]
fn panicking_handler_is_contained() {
    let (mut kernel, _, _) = test_kernel();
    install_notes_with(&mut kernel, Box::new(|_, _| panic!("handler bug")));
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "g");

    let result = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "boom"})))
        .unwrap();

    match result {
        InvocationResult::Failed { error } => {
            assert!(error.contains("handler panicked"));
            assert!(error.contains("handler bug"));
        }
        other => panic!("expected failure, got {other:?}"),
    }
    // The kernel survived: the run is still usable afterwards.
    assert!(kernel.run_view(&run_id).unwrap().is_active());
}

#[test]
fn invalid_artifact_from_handler_fails_the_invocation() {
    let (mut kernel, _, _) = test_kernel();
    install_notes_with(
        &mut kernel,
        Box::new(|_, _| {
            Ok(CapabilityOutcome {
                result: Value::Null,
                artifacts: vec![ArtifactDraft {
                    artifact_type: ArtifactTypeName::new("note"),
                    title: "Bad note".into(),
                    content: json!({"wrong_field": "x"}),
                }],
            })
        }),
    );
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "g");

    let result = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "x"})))
        .unwrap();

    match result {
        InvocationResult::Failed { error } => {
            assert!(error.contains("rejected by declared schema"))
        }
        other => panic!("expected failure, got {other:?}"),
    }
    assert_eq!(kernel.artifacts().count(), 0); // nothing half-stored
}

#[test]
fn a_failing_draft_stores_nothing_from_the_same_outcome() {
    let (mut kernel, _, _) = test_kernel();
    install_notes_with(
        &mut kernel,
        Box::new(|_, _| {
            Ok(CapabilityOutcome {
                result: Value::Null,
                artifacts: vec![
                    ArtifactDraft {
                        artifact_type: ArtifactTypeName::new("note"),
                        title: "Good note".into(),
                        content: json!({"text": "valid"}),
                    },
                    ArtifactDraft {
                        artifact_type: ArtifactTypeName::new("note"),
                        title: "Bad note".into(),
                        content: json!({"wrong_field": "x"}),
                    },
                ],
            })
        }),
    );
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "g");

    let result = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "x"})))
        .unwrap();

    assert!(matches!(result, InvocationResult::Failed { .. }));
    // The valid sibling draft must not have been committed either: a
    // failed invocation leaves no artifacts and no produced events.
    assert_eq!(kernel.artifacts().count(), 0);
    let kinds: Vec<&str> = kernel
        .records_for_run(&run_id)
        .map(|r| r.event.kind())
        .collect();
    assert!(!kinds.contains(&"artifact-produced"));
    assert!(kinds.contains(&"capability-failed"));
}

#[test]
fn surface_intent_travels_the_full_path() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let binding = kernel
        .open_surface(&notes_app(), &composer_surface())
        .unwrap();

    let outcome = kernel
        .submit_action(&binding, create_note_intent("from the composer"))
        .unwrap();

    assert!(matches!(outcome.result, InvocationResult::Completed { .. }));
    let view = kernel.run_view(&outcome.run_id).unwrap();
    assert!(matches!(view.initiator, Initiator::SurfaceAction { .. }));
    assert_eq!(view.terminal_state, Some(RunTerminalState::Completed));
}

#[test]
fn closed_surface_cannot_submit() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let binding = kernel
        .open_surface(&notes_app(), &composer_surface())
        .unwrap();
    kernel.close_surface(&binding);
    let error = kernel
        .submit_action(&binding, create_note_intent("too late"))
        .unwrap_err();
    assert!(matches!(error, KernelError::SurfaceNotOpen { .. }));
}

#[test]
fn surface_grant_refusal_ends_the_run_cancelled() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let grant_id = kernel.grants_for(&notes_app())[0].grant_id.clone();
    kernel.revoke_grant(&grant_id).unwrap();
    let binding = kernel
        .open_surface(&notes_app(), &composer_surface())
        .unwrap();

    let outcome = kernel
        .submit_action(&binding, create_note_intent("revoked authority"))
        .unwrap();

    assert_eq!(
        outcome.result,
        InvocationResult::Refused {
            reason: RefusalReason::GrantRevoked
        }
    );
    let view = kernel.run_view(&outcome.run_id).unwrap();
    assert_eq!(view.terminal_state, Some(RunTerminalState::Cancelled));
}

#[test]
fn surface_run_is_cleaned_up_when_input_is_malformed() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let binding = kernel
        .open_surface(&notes_app(), &composer_surface())
        .unwrap();
    let error = kernel
        .submit_action(
            &binding,
            ActionIntent {
                capability: create_note_ref(),
                input: obj(json!({"text": 1})),
                data_scope: DataScope::None,
                goal: "g".into(),
            },
        )
        .unwrap_err();
    assert!(matches!(error, KernelError::InvalidCapabilityInput { .. }));
    let ended: Vec<RunTerminalState> = kernel
        .records()
        .iter()
        .filter_map(|r| match &r.event {
            LedgerEvent::RunEnded { terminal_state, .. } => Some(*terminal_state),
            _ => None,
        })
        .collect();
    assert_eq!(ended, vec![RunTerminalState::Failed]);
}

#[test]
fn forged_binding_with_invented_instance_id_is_rejected() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let binding = kernel
        .open_surface(&notes_app(), &composer_surface())
        .unwrap();
    // A forged binding with the same app/surface but a made-up instance_id
    // is not in the open set (BTreeSet compares all fields).
    let forged = app_host_kernel::services::surfaces::SurfaceBinding {
        app_id: binding.app_id.clone(),
        surface: binding.surface.clone(),
        instance_id: SurfaceInstanceId::new("si-forged"),
    };
    let error = kernel
        .submit_action(&forged, create_note_intent("forged"))
        .unwrap_err();
    assert!(matches!(error, KernelError::SurfaceNotOpen { .. }));
}

#[test]
fn undeclared_intent_is_rejected_before_any_run_or_handler() {
    let (mut kernel, _, _) = test_kernel();
    // Install notes with a second capability that is NOT declared on the
    // composer surface's intents.
    let second_cap = CapabilityName::new("delete_all");
    let second_ref = CapabilityRef {
        provider: notes_app(),
        capability: second_cap.clone(),
    };
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest.capabilities.push(CapabilityDeclaration {
        name: second_cap.clone(),
        description: "Delete all notes".into(),
        input_schema: obj(json!({"type": "object", "additionalProperties": false})),
        effect: CapabilityEffect::Destructive,
        output_schema: Some(obj(json!({"type": "object", "additionalProperties": true}))),
    });
    let handler_calls = Arc::new(Mutex::new(0usize));
    let counting: CapabilityHandler = {
        let calls = handler_calls.clone();
        Box::new(move |_, _| {
            *calls.lock().unwrap() += 1;
            Ok(CapabilityOutcome {
                result: json!({"deleted": true}),
                artifacts: vec![],
            })
        })
    };
    let mut handlers: BTreeMap<CapabilityName, CapabilityHandler> = BTreeMap::new();
    handlers.insert(create_note(), create_note_handler());
    handlers.insert(second_cap.clone(), counting);
    kernel
        .install(seal(manifest), handlers)
        .expect("notes with extra capability installs");
    let binding = kernel
        .open_surface(&notes_app(), &composer_surface())
        .unwrap();
    // The composer surface only declares create_note in its intents.
    // Submitting delete_all should fail with UndeclaredSurfaceIntent.
    let error = kernel
        .submit_action(
            &binding,
            ActionIntent {
                capability: second_ref,
                input: obj(json!({})),
                data_scope: DataScope::None,
                goal: "delete all".into(),
            },
        )
        .unwrap_err();
    assert!(
        matches!(error, KernelError::UndeclaredSurfaceIntent { .. }),
        "expected UndeclaredSurfaceIntent, got {error:?}"
    );
    // No handler was called, no run was recorded.
    assert_eq!(*handler_calls.lock().unwrap(), 0);
    let run_count = kernel
        .records()
        .iter()
        .filter(|r| matches!(&r.event, LedgerEvent::RunStarted { .. }))
        .count();
    assert_eq!(run_count, 0);
}

#[test]
fn invalid_output_is_rejected_by_declared_schema() {
    let (mut kernel, _, _) = test_kernel();
    // Install notes with an output_schema that requires a specific shape.
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest.capabilities[0].output_schema = Some(obj(json!({
        "type": "object",
        "properties": {
            "created": {"type": "boolean"}
        },
        "required": ["created"],
        "additionalProperties": false
    })));
    // Handler returns a result that violates the schema (extra field).
    let bad_handler: CapabilityHandler = Box::new(|_, _| {
        Ok(CapabilityOutcome {
            result: json!({"created": true, "extra": "not allowed"}),
            artifacts: vec![ArtifactDraft {
                artifact_type: ArtifactTypeName::new("note"),
                title: "Would be valid".into(),
                content: json!({"text": "x"}),
            }],
        })
    });
    kernel
        .install(
            seal(manifest),
            BTreeMap::from([(create_note(), bad_handler)]),
        )
        .expect("notes installs");
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "g");
    let result = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "x"})))
        .unwrap();
    match &result {
        InvocationResult::Failed { error } => {
            assert!(
                error.contains("rejected by declared schema"),
                "expected schema rejection, got: {error}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    // No artifacts, no ArtifactProduced or CapabilityCompleted events.
    assert_eq!(kernel.artifacts().count(), 0);
    let kinds: Vec<&str> = kernel
        .records_for_run(&run_id)
        .map(|r| r.event.kind())
        .collect();
    assert!(!kinds.contains(&"artifact-produced"));
    assert!(!kinds.contains(&"capability-completed"));
    assert!(kinds.contains(&"capability-failed"));
}

#[test]
fn surface_run_is_cleaned_up_when_output_is_malformed() {
    let (mut kernel, _, _) = test_kernel();
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest.capabilities[0].output_schema = Some(obj(json!({
        "type": "object",
        "required": ["created"],
        "additionalProperties": false
    })));
    let handler: CapabilityHandler = Box::new(|_, _| {
        Ok(CapabilityOutcome {
            result: json!({"created": true, "unexpected": true}),
            artifacts: vec![ArtifactDraft {
                artifact_type: ArtifactTypeName::new("note"),
                title: "Would be valid".into(),
                content: json!({"text": "x"}),
            }],
        })
    });
    kernel
        .install(seal(manifest), BTreeMap::from([(create_note(), handler)]))
        .expect("notes installs");
    let binding = kernel
        .open_surface(&notes_app(), &composer_surface())
        .unwrap();

    let outcome = kernel
        .submit_action(&binding, create_note_intent("bad output"))
        .unwrap();

    assert!(matches!(outcome.result, InvocationResult::Failed { .. }));
    assert_eq!(kernel.artifacts().count(), 0);
    let view = kernel.run_view(&outcome.run_id).unwrap();
    assert_eq!(view.terminal_state, Some(RunTerminalState::Failed));
    let kinds: Vec<&str> = kernel
        .records_for_run(&outcome.run_id)
        .map(|record| record.event.kind())
        .collect();
    assert!(!kinds.contains(&"artifact-produced"));
    assert!(!kinds.contains(&"capability-completed"));
    assert!(kinds.contains(&"capability-failed"));
}
