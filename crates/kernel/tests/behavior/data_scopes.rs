use crate::helpers::*;
use app_host_kernel::services::artifacts::{ArtifactStore, MAX_ARTIFACT_CONTENT_BYTES};
use app_host_kernel::CapabilityAuthorizationView;

#[test]
fn resource_scoped_grant_reaches_the_handler() {
    let (mut kernel, _, _) = test_kernel();
    let seen_scope = Arc::new(Mutex::new(None::<DataScope>));
    let capture = seen_scope.clone();
    install_notes_with(
        &mut kernel,
        Box::new(move |_, context| {
            *capture.lock().unwrap() = Some(context.authorized_data_scope.clone());
            Ok(CapabilityOutcome {
                result: json!({"authorized_data_scope": context.authorized_data_scope}),
                artifacts: vec![],
            })
        }),
    );
    kernel
        .install(
            seal(AppManifest {
                app_id: AppId::new("scoped-chat"),
                version: "1.0.0".into(),
                display_name: "Scoped Chat".into(),
                description: "Requests a resource-scoped note grant".into(),
                capabilities: vec![],
                surfaces: vec![],
                agents: vec![],
                skills: vec![],
                assistant_profiles: vec![],
                automations: vec![],
                connectors: vec![],
                config_declarations: vec![],
                artifact_types: vec![],
                extension_points: vec![],
                extension_contributions: vec![],
                grant_requests: vec![GrantRequest {
                    scope: GrantScope::ExactCapability {
                        provider: notes_app(),
                        capability: create_note(),
                    },
                    data_scope: resource_scope(&["doc-1"]),
                    condition: GrantCondition::Silent,
                    reason: "Need access to doc-1".into(),
                    duration: GrantDuration::NonExpiring,
                }],
                event_subscriptions: vec![],
            }),
            BTreeMap::new(),
        )
        .expect("consumer installs");

    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: AppId::new("scoped-chat"),
                reason: "resource-scoped request".into(),
            },
            "note",
        )
        .expect("run starts");

    let result = kernel
        .invoke_with_data_scope(
            &run_id,
            &create_note_ref(),
            resource_scope(&["doc-1"]),
            obj(json!({"text": "ok"})),
        )
        .unwrap();

    assert!(matches!(result, InvocationResult::Completed { .. }));
    assert_eq!(
        *seen_scope.lock().unwrap(),
        Some(resource_scope(&["doc-1"]))
    );
}

#[test]
fn all_resources_grant_covers_exact_invocation_and_preserves_requested_scope() {
    let (mut kernel, _, _) = test_kernel();
    let seen_scope = Arc::new(Mutex::new(None::<DataScope>));
    let capture = seen_scope.clone();
    install_notes_with(
        &mut kernel,
        Box::new(move |_, context| {
            *capture.lock().unwrap() = Some(context.authorized_data_scope.clone());
            Ok(CapabilityOutcome {
                result: json!({}),
                artifacts: vec![],
            })
        }),
    );
    let mut consumer = chat_manifest(AppId::new("all-resources-reader"), vec![]);
    consumer.grant_requests = vec![GrantRequest {
        scope: GrantScope::ExactCapability {
            provider: notes_app(),
            capability: create_note(),
        },
        data_scope: DataScope::AllResources,
        condition: GrantCondition::Silent,
        reason: "Read every resource from this provider".into(),
        duration: GrantDuration::NonExpiring,
    }];
    kernel.install(seal(consumer), BTreeMap::new()).unwrap();

    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: AppId::new("all-resources-reader"),
                reason: "read one resource".into(),
            },
            "note",
        )
        .unwrap();
    let result = kernel
        .invoke_with_data_scope(
            &run_id,
            &create_note_ref(),
            resource_scope(&["doc-created-after-install"]),
            obj(json!({"text": "ok"})),
        )
        .unwrap();

    assert!(matches!(result, InvocationResult::Completed { .. }));
    assert_eq!(
        *seen_scope.lock().unwrap(),
        Some(resource_scope(&["doc-created-after-install"]))
    );
    assert!(matches!(
        kernel.run_view(&run_id).unwrap().invocations.last(),
        Some(InvocationRecord::Completed { data_scope, .. })
            if data_scope == &resource_scope(&["doc-created-after-install"])
    ));

    let wildcard_run = kernel
        .start_run(
            Initiator::App {
                app_id: AppId::new("all-resources-reader"),
                reason: "invalid wildcard invocation".into(),
            },
            "note",
        )
        .unwrap();
    assert!(matches!(
        kernel.invoke_with_data_scope(
            &wildcard_run,
            &create_note_ref(),
            DataScope::AllResources,
            obj(json!({"text": "not exact"})),
        ),
        Err(KernelError::InvalidGrantDataScope { .. })
    ));
}

#[test]
fn data_scope_coverage_is_asymmetric_and_kind_safe() {
    let exact = resource_scope(&["doc-1"]);

    assert_eq!(
        serde_json::to_value(&DataScope::AllResources).unwrap(),
        json!({"kind": "all-resources"})
    );
    assert_eq!(
        serde_json::from_value::<DataScope>(json!({"kind": "all-resources"})).unwrap(),
        DataScope::AllResources
    );
    assert!(DataScope::AllResources.covers(&exact));
    assert!(DataScope::AllResources.covers(&DataScope::AllResources));
    assert!(!exact.covers(&DataScope::AllResources));
    assert!(!DataScope::AllResources.covers(&DataScope::None));
    assert!(!DataScope::None.covers(&DataScope::AllResources));
    assert!(DataScope::AllResources.validate_invocation().is_err());
}

#[test]
fn wrong_resource_is_refused() {
    let (mut kernel, _, _) = test_kernel();
    install_notes_with(
        &mut kernel,
        Box::new(|_, _| {
            Ok(CapabilityOutcome {
                result: json!({}),
                artifacts: vec![],
            })
        }),
    );

    kernel
        .install(
            seal(AppManifest {
                app_id: AppId::new("scoped-chat"),
                version: "1.0.0".into(),
                display_name: "Scoped Chat".into(),
                description: "Requests a resource-scoped note grant".into(),
                capabilities: vec![],
                surfaces: vec![],
                agents: vec![],
                skills: vec![],
                assistant_profiles: vec![],
                automations: vec![],
                connectors: vec![],
                config_declarations: vec![],
                artifact_types: vec![],
                extension_points: vec![],
                extension_contributions: vec![],
                grant_requests: vec![GrantRequest {
                    scope: GrantScope::ExactCapability {
                        provider: notes_app(),
                        capability: create_note(),
                    },
                    data_scope: resource_scope(&["doc-1"]),
                    condition: GrantCondition::Silent,
                    reason: "Need access to doc-1".into(),
                    duration: GrantDuration::NonExpiring,
                }],
                event_subscriptions: vec![],
            }),
            BTreeMap::new(),
        )
        .expect("consumer installs");

    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: AppId::new("scoped-chat"),
                reason: "resource-scoped request".into(),
            },
            "note",
        )
        .expect("run starts");

    let result = kernel
        .invoke_with_data_scope(
            &run_id,
            &create_note_ref(),
            resource_scope(&["doc-2"]),
            obj(json!({"text": "nope"})),
        )
        .unwrap();

    assert_eq!(
        result,
        InvocationResult::Refused {
            reason: RefusalReason::NoGrant,
        }
    );
}

#[test]
fn unscoped_invocation_cannot_use_a_resource_grant() {
    let (mut kernel, _, _) = test_kernel();
    install_notes_with(
        &mut kernel,
        Box::new(|_, _| {
            Ok(CapabilityOutcome {
                result: Value::Null,
                artifacts: vec![],
            })
        }),
    );

    kernel
        .install(
            seal(AppManifest {
                app_id: AppId::new("scoped-chat"),
                version: "1.0.0".into(),
                display_name: "Scoped Chat".into(),
                description: "Requests a resource-scoped note grant".into(),
                capabilities: vec![],
                surfaces: vec![],
                agents: vec![],
                skills: vec![],
                assistant_profiles: vec![],
                automations: vec![],
                connectors: vec![],
                config_declarations: vec![],
                artifact_types: vec![],
                extension_points: vec![],
                extension_contributions: vec![],
                grant_requests: vec![GrantRequest {
                    scope: GrantScope::ExactCapability {
                        provider: notes_app(),
                        capability: create_note(),
                    },
                    data_scope: resource_scope(&["doc-1"]),
                    condition: GrantCondition::Silent,
                    reason: "Need access to doc-1".into(),
                    duration: GrantDuration::NonExpiring,
                }],
                event_subscriptions: vec![],
            }),
            BTreeMap::new(),
        )
        .expect("consumer installs");

    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: AppId::new("scoped-chat"),
                reason: "resource-scoped request".into(),
            },
            "note",
        )
        .expect("run starts");

    let result = kernel
        .invoke(
            &run_id,
            &create_note_ref(),
            obj(json!({"text": "no scope"})),
        )
        .unwrap();

    assert_eq!(
        result,
        InvocationResult::Refused {
            reason: RefusalReason::NoGrant,
        }
    );
}

#[test]
fn empty_or_duplicate_resource_scopes_are_rejected() {
    assert!(matches!(
        DataScope::resources(vec![]),
        Err(KernelError::InvalidGrantDataScope { .. })
    ));

    let repeated = ResourceId::new("doc-1");
    assert!(matches!(
        DataScope::resources(vec![repeated.clone(), repeated]),
        Err(KernelError::InvalidGrantDataScope { .. })
    ));
}

#[test]
fn capability_views_and_ledger_keep_scope_bound_to_its_condition() {
    let (mut kernel, _, _) = test_kernel();
    install_notes_with(
        &mut kernel,
        Box::new(|_, _| {
            Ok(CapabilityOutcome {
                result: json!({}),
                artifacts: vec![],
            })
        }),
    );
    let mut consumer = chat_manifest(chat_app(), vec![]);
    consumer.grant_requests = vec![
        GrantRequest {
            scope: GrantScope::ExactCapability {
                provider: notes_app(),
                capability: create_note(),
            },
            data_scope: resource_scope(&["project-a"]),
            condition: GrantCondition::Silent,
            reason: "Use project A".into(),
            duration: GrantDuration::NonExpiring,
        },
        GrantRequest {
            scope: GrantScope::ExactCapability {
                provider: notes_app(),
                capability: create_note(),
            },
            data_scope: resource_scope(&["project-b"]),
            condition: GrantCondition::RequiresApproval,
            reason: "Use project B with approval".into(),
            duration: GrantDuration::NonExpiring,
        },
    ];
    kernel.install(seal(consumer), BTreeMap::new()).unwrap();

    let capability = kernel
        .available_capabilities_for(&chat_app())
        .unwrap()
        .into_iter()
        .find(|view| view.capability == create_note())
        .unwrap();
    assert_eq!(
        capability.authorizations,
        vec![
            CapabilityAuthorizationView {
                data_scope: resource_scope(&["project-a"]),
                condition: GrantCondition::Silent,
            },
            CapabilityAuthorizationView {
                data_scope: resource_scope(&["project-b"]),
                condition: GrantCondition::RequiresApproval,
            },
        ]
    );

    let run_id = chat_message_run(&mut kernel, "project B");
    let result = kernel
        .invoke_with_data_scope(
            &run_id,
            &create_note_ref(),
            resource_scope(&["project-b"]),
            obj(json!({"text": "audited"})),
        )
        .unwrap();
    assert!(
        matches!(result, InvocationResult::Completed { .. }),
        "{result:?}"
    );
    let view = kernel.run_view(&run_id).unwrap();
    assert!(
        matches!(
            view.invocations.last(),
            Some(InvocationRecord::Completed {
                data_scope: DataScope::Resources { resource_ids },
                ..
            }) if resource_ids == &vec![ResourceId::new("project-b")]
        ),
        "{:?}",
        view.invocations
    );
    assert!(view
        .approvals
        .iter()
        .all(|approval| { approval.data_scope == resource_scope(&["project-b"]) }));
}

#[test]
fn artifact_snapshot_query_is_stable_and_cursor_strict() {
    let mut store = ArtifactStore::new();
    store.put_all(vec![
        artifact_with_id("artifact-1"),
        artifact_with_id("artifact-2"),
    ]);
    let resolver = store.snapshot_resolver_for(
        &DataScope::resources(vec![
            ResourceId::new("artifact-1"),
            ResourceId::new("artifact-2"),
        ])
        .unwrap(),
    );
    let first = resolver.query(None, 1).unwrap();
    assert_eq!(first.items.len(), 1);
    let cursor = first.next_cursor.clone().unwrap();
    let second = resolver.query(Some(cursor.as_str()), 1).unwrap();
    assert_eq!(second.items.len(), 1);
    assert!(resolver.query(Some("not-a-number"), 1).is_err());
}

#[test]
fn artifact_snapshot_read_bounds_content() {
    let mut store = ArtifactStore::new();
    let mut artifact = artifact_with_id("artifact-1");
    artifact.content = json!({"blob": "x".repeat(MAX_ARTIFACT_CONTENT_BYTES)});
    store.put_all(vec![artifact]);
    let resolver = store
        .snapshot_resolver_for(&DataScope::resources(vec![ResourceId::new("artifact-1")]).unwrap());
    assert!(resolver
        .read(&app_host_kernel::ids::ArtifactId::new("artifact-1"))
        .is_err());
}

fn artifact_with_id(id: &str) -> app_host_kernel::primitives::artifact::Artifact {
    app_host_kernel::primitives::artifact::Artifact {
        artifact_id: app_host_kernel::ids::ArtifactId::new(id),
        artifact_type: ArtifactTypeName::new("note"),
        title: id.into(),
        content: json!({"text": id}),
        provenance: app_host_kernel::primitives::artifact::Provenance {
            run_id: new_run_id(),
            capability: CapabilityRef {
                provider: chat_app(),
                capability: CapabilityName::new("chat.read"),
            },
            grant_id: app_host_kernel::ids::new_grant_id(),
            produced_by: chat_app(),
            recorded_at: Utc::now(),
        },
    }
}
