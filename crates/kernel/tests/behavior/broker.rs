use crate::helpers::*;
use app_host_kernel::PreparedGrant;

#[test]
fn grant_issuance_goes_through_trusted_chrome() {
    let (mut kernel, chrome, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let prompts = chrome.grant_prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].app_id, notes_app());
    assert!(matches!(
        kernel.check_grant(&notes_app(), &create_note_ref()),
        GrantCheck::Allowed(_)
    ));
    assert!(kernel
        .grants_for(&notes_app())
        .iter()
        .all(|grant| grant.origin == GrantOrigin::ManifestRequested));
}

#[test]
fn user_refusal_means_no_grant() {
    let (mut kernel, chrome, _) = test_kernel();
    chrome.set_grant_decision(ApprovalDecision::Denied);
    install_notes(&mut kernel, GrantCondition::Silent);
    assert_eq!(
        kernel.check_grant(&notes_app(), &create_note_ref()),
        GrantCheck::Denied(DenialReason::NoGrant)
    );
}

#[test]
fn prepared_grant_is_stale_after_provider_replacement() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let request = create_note_request(GrantCondition::Silent, GrantDuration::NonExpiring);
    let approval = kernel
        .prepare_grant(&chat_app(), request)
        .unwrap()
        .await_approval();

    kernel.uninstall(&notes_app()).unwrap();
    let mut replacement = notes_manifest(GrantCondition::Silent);
    replacement.version = "2.0.0".into();
    kernel.install(seal(replacement), notes_handlers()).unwrap();

    assert!(matches!(
        kernel.commit_grant(approval),
        Err(KernelError::PreparedInstallStale)
    ));
}

#[test]
fn grouped_grant_approval_rejects_mixed_holders() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    let request = create_note_request(GrantCondition::Silent, GrantDuration::NonExpiring);
    let notes_grant = kernel.prepare_grant(&notes_app(), request.clone()).unwrap();
    let chat_grant = kernel.prepare_grant(&chat_app(), request).unwrap();

    assert!(matches!(
        PreparedGrant::await_grouped_approvals(vec![notes_grant, chat_grant]),
        Err(KernelError::PreparedGrantGroupMismatch)
    ));
}

#[test]
fn grant_refused_variant_returned_when_user_declines() {
    let (mut kernel, chrome, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    chrome.set_grant_decision(ApprovalDecision::Denied);
    let result = kernel
        .issue_grant(
            &notes_app(),
            &create_note_request(GrantCondition::Silent, GrantDuration::NonExpiring),
        )
        .unwrap();
    assert!(matches!(result, IssueResult::Refused));
}

#[test]
fn editor_issued_grants_validate_targets_and_record_user_origin() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let issued = kernel
        .issue_grant(
            &notes_app(),
            &create_note_request(GrantCondition::Silent, GrantDuration::NonExpiring),
        )
        .unwrap();
    assert!(matches!(issued, IssueResult::Issued(grant) if grant.origin == GrantOrigin::UserAdded));

    let invalid_capability = GrantRequest {
        scope: GrantScope::ExactCapability {
            provider: notes_app(),
            capability: CapabilityName::new("not-declared"),
        },
        data_scope: DataScope::None,
        condition: GrantCondition::Silent,
        reason: "Invalid target".into(),
        duration: GrantDuration::NonExpiring,
    };
    assert!(matches!(
        kernel.issue_grant(&notes_app(), &invalid_capability),
        Err(KernelError::UndeclaredCapability { .. })
    ));

    let blank_reason = GrantRequest {
        reason: "  ".into(),
        ..create_note_request(GrantCondition::Silent, GrantDuration::NonExpiring)
    };
    assert!(matches!(
        kernel.issue_grant(&notes_app(), &blank_reason),
        Err(KernelError::GrantReasonRequired)
    ));
}

#[test]
fn install_rejects_unresolved_grants_before_prompting_or_mutating_state() {
    let (mut kernel, chrome, _) = test_kernel();
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest.grant_requests = vec![GrantRequest {
        scope: GrantScope::ExactCapability {
            provider: AppId::new("missing-provider"),
            capability: CapabilityName::new("missing-capability"),
        },
        data_scope: DataScope::None,
        condition: GrantCondition::Silent,
        reason: "Use a dependency that is not installed".into(),
        duration: GrantDuration::NonExpiring,
    }];

    assert!(matches!(
        kernel.install(seal(manifest), notes_handlers()),
        Err(KernelError::UnknownApp(app)) if app == AppId::new("missing-provider")
    ));
    assert!(kernel.installed_app(&notes_app()).is_err());
    assert!(chrome.grant_prompts.lock().unwrap().is_empty());
}

#[test]
fn install_validates_self_scoped_capabilities_before_prompting() {
    let (mut kernel, chrome, _) = test_kernel();
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest.grant_requests = vec![GrantRequest {
        scope: GrantScope::ExactCapability {
            provider: notes_app(),
            capability: CapabilityName::new("not-declared"),
        },
        data_scope: DataScope::None,
        condition: GrantCondition::Silent,
        reason: "Invalid self target".into(),
        duration: GrantDuration::NonExpiring,
    }];

    assert!(matches!(
        kernel.install(seal(manifest), notes_handlers()),
        Err(KernelError::UndeclaredCapability { .. })
    ));
    assert!(kernel.installed_app(&notes_app()).is_err());
    assert!(chrome.grant_prompts.lock().unwrap().is_empty());
}

#[test]
fn requires_approval_grant_asks_per_use() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::RequiresApproval);
    assert!(matches!(
        kernel.check_grant(&notes_app(), &create_note_ref()),
        GrantCheck::ApprovalRequired(_)
    ));
}

#[test]
fn revoked_grant_is_denied() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let grant_id = kernel.grants_for(&notes_app())[0].grant_id.clone();
    kernel.revoke_grant(&grant_id).unwrap();
    assert_eq!(
        kernel.check_grant(&notes_app(), &create_note_ref()),
        GrantCheck::Denied(DenialReason::Revoked)
    );
    assert!(kernel.grants_for(&notes_app()).is_empty());
}

#[test]
fn expired_grant_is_denied() {
    let (mut kernel, _, clock) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    for grant_id in kernel
        .grants_for(&notes_app())
        .iter()
        .map(|g| g.grant_id.clone())
        .collect::<Vec<_>>()
    {
        kernel.revoke_grant(&grant_id).unwrap();
    }
    let issued = kernel
        .issue_grant(
            &notes_app(),
            &create_note_request(GrantCondition::Silent, expires_after(3600)),
        )
        .unwrap();
    assert!(matches!(issued, IssueResult::Issued(_)));

    assert!(matches!(
        kernel.check_grant(&notes_app(), &create_note_ref()),
        GrantCheck::Allowed(_)
    ));
    clock.advance_to(start_time() + Duration::hours(2));
    assert_eq!(
        kernel.check_grant(&notes_app(), &create_note_ref()),
        GrantCheck::Denied(DenialReason::Expired)
    );
    // A permissions view built on grants_for must not list dead grants.
    assert!(kernel.grants_for(&notes_app()).is_empty());
}

#[test]
fn least_interactive_covering_grant_wins() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::RequiresApproval);
    kernel
        .issue_grant(
            &notes_app(),
            &create_note_request(GrantCondition::Silent, GrantDuration::NonExpiring),
        )
        .unwrap();
    match kernel.check_grant(&notes_app(), &create_note_ref()) {
        GrantCheck::Allowed(grant) => {
            assert_eq!(grant.condition, GrantCondition::Silent)
        }
        other => panic!("expected Allowed, got {other:?}"),
    }
}

#[test]
fn equally_interactive_grants_pick_the_earliest_issued() {
    let (mut kernel, _, clock) = test_kernel();
    install_notes(&mut kernel, GrantCondition::RequiresApproval);
    let first_id = kernel.grants_for(&notes_app())[0].grant_id.clone();
    clock.advance_to(start_time() + Duration::seconds(10));
    kernel
        .issue_grant(
            &notes_app(),
            &create_note_request(GrantCondition::RequiresApproval, GrantDuration::NonExpiring),
        )
        .unwrap();
    match kernel.check_grant(&notes_app(), &create_note_ref()) {
        // Deterministic: the earliest-issued covering grant wins, so
        // identical histories yield identical grant_ids in the ledger.
        GrantCheck::ApprovalRequired(grant) => assert_eq!(grant.grant_id, first_id),
        other => panic!("expected ApprovalRequired, got {other:?}"),
    }
}

#[test]
fn secrets_resolve_only_declared_names() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    install_chat(&mut kernel);
    kernel.put_secret(
        SecretRef {
            owner: notes_app(),
            name: api_token_secret(),
        },
        "s3cret".into(),
    );

    let notes_resolver = kernel.secret_resolver_for(&notes_app()).unwrap();
    assert_eq!(
        notes_resolver.resolve(&api_token_secret()).unwrap(),
        "s3cret"
    );

    let chat_resolver = kernel.secret_resolver_for(&chat_app()).unwrap();
    assert!(matches!(
        chat_resolver.resolve(&api_token_secret()).unwrap_err(),
        KernelError::UndeclaredSecret(_)
    ));
}

#[test]
fn stored_nowhere_secret_fails_loud() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let resolver = kernel.secret_resolver_for(&notes_app()).unwrap();
    assert!(matches!(
        resolver.resolve(&api_token_secret()).unwrap_err(),
        KernelError::UnknownSecret(_)
    ));
}

#[test]
fn secret_resolver_is_a_snapshot() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    // A resolver handed to a handler must not observe later broker state.
    let resolver = kernel.secret_resolver_for(&notes_app()).unwrap();
    kernel.put_secret(
        SecretRef {
            owner: notes_app(),
            name: api_token_secret(),
        },
        "stored-later".into(),
    );
    assert!(matches!(
        resolver.resolve(&api_token_secret()).unwrap_err(),
        KernelError::UnknownSecret(_)
    ));
}

#[test]
fn malicious_app_cannot_resolve_another_owners_matching_secret_name() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let malicious_app = AppId::new("malicious");
    let manifest = AppManifest {
        app_id: malicious_app.clone(),
        version: "1.0.0".into(),
        display_name: "Malicious app".into(),
        description: "Attempts secret-name collisions".into(),
        capabilities: vec![],
        surfaces: vec![],
        agents: vec![],
        skills: vec![],
        assistant_profiles: vec![],
        automations: vec![],
        connectors: vec![ConnectorDeclaration {
            name: "stolen-notes-backend".into(),
            description: "Declares the notes secret name".into(),
            secret_names: vec![api_token_secret()],
            config_schema: None,
        }],
        config_declarations: vec![],
        artifact_types: vec![],
        extension_points: vec![],
        extension_contributions: vec![],
        grant_requests: vec![],
        event_subscriptions: vec![],
    };
    kernel.install(seal(manifest), BTreeMap::new()).unwrap();

    // The attacker declares the same local name, but only notes owns this
    // stored value.
    kernel.put_secret(
        SecretRef {
            owner: notes_app(),
            name: api_token_secret(),
        },
        "notes-value".into(),
    );

    let notes_resolver = kernel.secret_resolver_for(&notes_app()).unwrap();
    assert_eq!(
        notes_resolver.resolve(&api_token_secret()).unwrap(),
        "notes-value"
    );

    let malicious_resolver = kernel.secret_resolver_for(&malicious_app).unwrap();
    assert!(matches!(
        malicious_resolver.resolve(&api_token_secret()).unwrap_err(),
        KernelError::UnknownSecret(_)
    ));
}

#[test]
fn uninstall_clears_owned_secrets() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    kernel.put_secret(
        SecretRef {
            owner: notes_app(),
            name: api_token_secret(),
        },
        "old-value".into(),
    );

    kernel.uninstall(&notes_app()).unwrap();

    // Reinstall - must not inherit the old secret.
    install_notes(&mut kernel, GrantCondition::Silent);
    let resolver = kernel.secret_resolver_for(&notes_app()).unwrap();
    assert!(matches!(
        resolver.resolve(&api_token_secret()).unwrap_err(),
        KernelError::UnknownSecret(_)
    ));
}

#[test]
fn one_authority_requested_twice_asks_the_user_once() {
    // Two structurally identical grant requests denote the same authority: one
    // issued grant satisfies both. Prompting per request would ask the user
    // the same question twice and let them give two answers to it, which the
    // commit path could only resolve by guessing.
    let (mut kernel, chrome, _) = test_kernel();
    let mut manifest = notes_manifest(GrantCondition::Silent);
    let duplicate = manifest.grant_requests[0].clone();
    manifest.grant_requests.push(duplicate);

    let results = kernel
        .install(seal(manifest), notes_handlers())
        .expect("notes installs");

    assert_eq!(
        chrome.grant_prompts.lock().unwrap().len(),
        1,
        "the same authority must be confirmed once, not once per request"
    );
    assert!(matches!(
        kernel.check_grant(&notes_app(), &create_note_ref()),
        GrantCheck::Allowed(_)
    ));
    assert_eq!(results.len(), 2, "one result is returned per declaration");
    assert_eq!(kernel.grants_for(&notes_app()).len(), 1);
}

#[test]
fn non_adjacent_duplicate_grants_keep_decisions_aligned() {
    let (mut kernel, chrome, _) = test_kernel();
    let mut manifest = notes_manifest(GrantCondition::Silent);
    let duplicate = manifest.grant_requests[0].clone();
    manifest.grant_requests.push(GrantRequest {
        condition: GrantCondition::Notify,
        ..duplicate.clone()
    });
    manifest.grant_requests.push(duplicate);

    let results = kernel
        .install(seal(manifest), notes_handlers())
        .expect("notes installs");

    assert_eq!(chrome.grant_prompts.lock().unwrap().len(), 2);
    assert_eq!(results.len(), 3);
    assert_eq!(kernel.grants_for(&notes_app()).len(), 2);
}

#[test]
fn refusing_a_duplicated_authority_grants_nothing() {
    let (mut kernel, chrome, _) = test_kernel();
    chrome.set_grant_decision(ApprovalDecision::Denied);
    let mut manifest = notes_manifest(GrantCondition::Silent);
    let duplicate = manifest.grant_requests[0].clone();
    manifest.grant_requests.push(duplicate);

    kernel
        .install(seal(manifest), notes_handlers())
        .expect("notes installs");

    // A single denial must cover every request for that authority; the second
    // copy must not become a second chance to get it issued.
    assert_eq!(
        kernel.check_grant(&notes_app(), &create_note_ref()),
        GrantCheck::Denied(DenialReason::NoGrant)
    );
}
