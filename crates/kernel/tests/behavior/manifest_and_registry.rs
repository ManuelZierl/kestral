use crate::helpers::*;

#[test]
fn tampered_manifest_is_rejected_at_install() {
    let (mut kernel, _, _) = test_kernel();
    let mut sealed = seal(notes_manifest(GrantCondition::Silent));
    sealed.manifest.version = "6.6.6".into();
    let error = kernel.install(sealed, notes_handlers()).unwrap_err();
    assert!(matches!(error, KernelError::ManifestContentHashMismatch(_)));
}

#[test]
fn duplicate_capability_names_are_rejected() {
    let (mut kernel, _, _) = test_kernel();
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest.capabilities.push(manifest.capabilities[0].clone());
    let error = kernel
        .install(seal(manifest), notes_handlers())
        .unwrap_err();
    assert!(matches!(
        error,
        KernelError::ManifestContributionInvalid { .. }
    ));
}

#[test]
fn duplicate_assistant_profile_names_are_rejected() {
    let (mut kernel, _, _) = test_kernel();
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest.skills.push(SkillDeclaration {
        name: "writing-guidance".into(),
        description: "Guidance for writing".into(),
        instructions: "Be concise".into(),
    });
    manifest.assistant_profiles = vec![
        AssistantProfileDeclaration {
            profile_name: "default".into(),
            title: "Default".into(),
            description: "Default profile".into(),
            instruction_skill_refs: vec!["writing-guidance".into()],
            suggested_capability_refs: vec![],
            suggested_agent_engine_contract: None,
            starter_prompts: vec![],
        },
        AssistantProfileDeclaration {
            profile_name: "default".into(),
            title: "Also Default".into(),
            description: "Duplicate profile".into(),
            instruction_skill_refs: vec!["writing-guidance".into()],
            suggested_capability_refs: vec![],
            suggested_agent_engine_contract: None,
            starter_prompts: vec![],
        },
    ];
    let error = kernel
        .install(seal(manifest), notes_handlers())
        .unwrap_err();
    assert!(matches!(
        error,
        KernelError::ManifestContributionInvalid { .. }
    ));
}

#[test]
fn assistant_profile_fields_must_be_non_empty() {
    let (mut kernel, _, _) = test_kernel();
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest.skills.push(SkillDeclaration {
        name: "writing-guidance".into(),
        description: "Guidance for writing".into(),
        instructions: "Be concise".into(),
    });
    manifest.assistant_profiles = vec![AssistantProfileDeclaration {
        profile_name: "".into(),
        title: "".into(),
        description: "".into(),
        instruction_skill_refs: vec!["writing-guidance".into()],
        suggested_capability_refs: vec![],
        suggested_agent_engine_contract: None,
        starter_prompts: vec![],
    }];
    let error = kernel
        .install(seal(manifest), notes_handlers())
        .unwrap_err();
    assert!(matches!(
        error,
        KernelError::ManifestIdentityInvalid {
            field: "profile_name"
        }
    ));
}

#[test]
fn assistant_profile_instruction_skills_must_be_local() {
    let (mut kernel, _, _) = test_kernel();
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest.assistant_profiles = vec![AssistantProfileDeclaration {
        profile_name: "default".into(),
        title: "Default".into(),
        description: "Default profile".into(),
        instruction_skill_refs: vec!["missing-skill".into()],
        suggested_capability_refs: vec![],
        suggested_agent_engine_contract: None,
        starter_prompts: vec![],
    }];
    let error = kernel
        .install(seal(manifest), notes_handlers())
        .unwrap_err();
    assert!(matches!(
        error,
        KernelError::ManifestContributionInvalid { .. }
    ));
}

#[test]
fn assistant_profile_collection_fields_are_required_on_the_wire() {
    let error = serde_json::from_value::<AssistantProfileDeclaration>(serde_json::json!({
        "profile_name": "default",
        "title": "Default",
        "description": "Default profile"
    }))
    .unwrap_err();

    assert!(error.to_string().contains("instruction_skill_refs"));
}

#[test]
fn extension_contribution_must_reference_its_own_surface() {
    let (mut kernel, _, _) = test_kernel();
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest
        .extension_contributions
        .push(ExtensionContribution {
            target_app: AppId::new("chat"),
            extension_point: ExtensionPointName::new("message-actions"),
            contract_version: 1,
            surface: SurfaceName::new("not-declared"),
        });
    let error = kernel
        .install(seal(manifest), notes_handlers())
        .unwrap_err();
    assert!(matches!(
        error,
        KernelError::ManifestExtensionContributionInvalid { .. }
    ));
}

#[test]
fn extension_contribution_can_install_dormant_before_its_target() {
    let (mut kernel, _, _) = test_kernel();
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest
        .extension_contributions
        .push(ExtensionContribution {
            target_app: AppId::new("chat"),
            extension_point: ExtensionPointName::new("message-actions"),
            contract_version: 1,
            surface: manifest.surfaces[0].name.clone(),
        });
    kernel.install(seal(manifest), notes_handlers()).unwrap();
    assert!(kernel.installed_app(&notes_app()).is_ok());
}

#[test]
fn empty_identity_is_rejected() {
    let (mut kernel, _, _) = test_kernel();
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest.app_id = AppId::new("");
    let error = kernel
        .install(seal(manifest), notes_handlers())
        .unwrap_err();
    assert!(matches!(
        error,
        KernelError::ManifestIdentityInvalid { field: "app_id" }
    ));
}

#[test]
fn double_install_is_refused() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let error = kernel
        .install(
            seal(notes_manifest(GrantCondition::Silent)),
            notes_handlers(),
        )
        .unwrap_err();
    assert!(matches!(error, KernelError::AppAlreadyInstalled(_)));
}

#[test]
fn unknown_app_is_refused() {
    let (kernel, _, _) = test_kernel();
    let error = kernel.installed_app(&AppId::new("ghost")).unwrap_err();
    assert!(matches!(error, KernelError::UnknownApp(_)));
}

#[test]
fn undeclared_capability_does_not_exist() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let error = kernel
        .capability_declaration(&CapabilityRef {
            provider: notes_app(),
            capability: CapabilityName::new("delete_all"),
        })
        .unwrap_err();
    assert!(matches!(error, KernelError::UndeclaredCapability { .. }));
}

#[test]
fn undeclared_surface_does_not_exist() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let error = kernel
        .open_surface(&notes_app(), &SurfaceName::new("admin-panel"))
        .unwrap_err();
    assert!(matches!(error, KernelError::UndeclaredSurface { .. }));
}

#[test]
fn undeclared_artifact_type_fails_the_invocation() {
    let (mut kernel, _, _) = test_kernel();
    install_notes_with(
        &mut kernel,
        Box::new(|_, _| {
            Ok(CapabilityOutcome {
                result: json!({}),
                artifacts: vec![ArtifactDraft {
                    artifact_type: ArtifactTypeName::new("ghost-type"),
                    title: "Sneaky".into(),
                    content: json!({}),
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
            assert!(error.contains("does not declare artifact type"))
        }
        other => panic!("expected failure, got {other:?}"),
    }
    assert_eq!(kernel.artifacts().count(), 0);
}

#[test]
fn invalid_input_schema_is_rejected_at_install() {
    let (mut kernel, _, _) = test_kernel();
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest.capabilities[0].input_schema = obj(json!({"type": "definitely-not-a-type"}));
    let error = kernel
        .install(seal(manifest), notes_handlers())
        .unwrap_err();
    assert!(matches!(error, KernelError::InvalidSchema { .. }));
}

#[test]
fn unknown_event_topic_is_rejected_at_install() {
    let (mut kernel, _, _) = test_kernel();
    let error = kernel
        .install(
            seal(chat_manifest(
                chat_app(),
                vec![EventTopic::new("definitely-not-a-topic")],
            )),
            BTreeMap::new(),
        )
        .unwrap_err();
    assert!(matches!(error, KernelError::UnknownEventTopic { .. }));
}

#[test]
fn handlers_must_match_declared_capabilities_exactly() {
    let (mut kernel, _, _) = test_kernel();
    // Missing handler for a declared capability.
    let error = kernel
        .install(
            seal(notes_manifest(GrantCondition::Silent)),
            BTreeMap::new(),
        )
        .unwrap_err();
    assert!(matches!(error, KernelError::HandlerBindingMismatch { .. }));
    // Extra handler for an undeclared capability.
    let error = kernel
        .install(
            seal(chat_manifest(chat_app(), vec![])),
            BTreeMap::from([(create_note(), create_note_handler())]),
        )
        .unwrap_err();
    assert!(matches!(error, KernelError::HandlerBindingMismatch { .. }));
}

#[test]
fn uninstall_leaves_no_authority_behind() {
    let (mut kernel, chrome, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let binding = kernel
        .open_surface(&notes_app(), &composer_surface())
        .unwrap();
    kernel.uninstall(&notes_app()).unwrap();

    // A different app reinstalled under the same AppId gets nothing:
    // the user declines its grant request, and the old grant must not
    // revive (it exists but is revoked - not silently live).
    chrome.set_grant_decision(ApprovalDecision::Denied);
    install_notes(&mut kernel, GrantCondition::Silent);
    assert_eq!(
        kernel.check_grant(&notes_app(), &create_note_ref()),
        GrantCheck::Denied(DenialReason::Revoked)
    );
    assert!(kernel.grants_for(&notes_app()).is_empty());

    // The pre-uninstall surface binding died with the old app.
    let error = kernel
        .submit_action(&binding, create_note_intent("stale binding"))
        .unwrap_err();
    assert!(matches!(error, KernelError::SurfaceNotOpen { .. }));
}

#[test]
fn uninstalling_a_provider_revokes_consumer_grants_over_it() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    assert!(matches!(
        kernel.check_grant(&chat_app(), &create_note_ref()),
        GrantCheck::Allowed(_)
    ));

    kernel.uninstall(&notes_app()).unwrap();
    // Chat consented to the old provider's code; different code
    // reinstalled under the same AppId must not be reachable through
    // the surviving grant.
    install_notes(&mut kernel, GrantCondition::Silent);

    assert_eq!(
        kernel.check_grant(&chat_app(), &create_note_ref()),
        GrantCheck::Denied(DenialReason::Revoked)
    );
}

#[test]
fn uninstall_discards_inbox_for_a_later_reinstall() {
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
    let run_id = chat_message_run(&mut kernel, "queued event");
    kernel
        .end_run(&run_id, RunTerminalState::Completed)
        .unwrap();

    kernel.uninstall(&chat_app()).unwrap();
    kernel
        .install(
            seal(chat_manifest(
                chat_app(),
                vec![EventTopic::new("run-ended")],
            )),
            BTreeMap::new(),
        )
        .unwrap();

    assert!(kernel.drain_inbox(&chat_app()).unwrap().is_empty());
}

#[test]
fn event_subscriptions_are_disclosed_and_denial_leaves_no_installed_app() {
    let (mut kernel, chrome, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    chrome.set_subscription_decision(ApprovalDecision::Denied);

    let error = kernel
        .install(
            seal(chat_manifest(
                chat_app(),
                vec![EventTopic::new("run-ended")],
            )),
            BTreeMap::new(),
        )
        .unwrap_err();

    assert!(matches!(error, KernelError::EventSubscriptionDenied(app) if app == chat_app()));
    assert!(matches!(
        kernel.installed_app(&chat_app()),
        Err(KernelError::UnknownApp(_))
    ));
    assert_eq!(
        *chrome.subscription_prompts.lock().unwrap(),
        vec![EventSubscriptionPrompt {
            app_id: chat_app(),
            app_display_name: "Chat".into(),
            topics: vec![EventTopic::new("run-ended")],
        }]
    );
}

#[test]
fn uninstalled_app_cannot_keep_acting_through_an_open_run() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let run_id = chat_message_run(&mut kernel, "outlives its app");
    kernel.uninstall(&chat_app()).unwrap();
    let error = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "x"})))
        .unwrap_err();
    assert!(matches!(error, KernelError::RunAlreadyEnded(_)));
}
