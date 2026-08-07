use crate::helpers::*;
use std::sync::atomic::{AtomicU8, Ordering};

struct InjectedStore {
    inner: Arc<MemoryKernelStateStore>,
    failure: AtomicU8,
}

impl InjectedStore {
    fn new(inner: Arc<MemoryKernelStateStore>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            failure: AtomicU8::new(0),
        })
    }

    fn fail_before_commit(&self) {
        self.failure.store(1, Ordering::Release);
    }

    fn fail_after_durable_write(&self) {
        self.failure.store(2, Ordering::Release);
    }
}

impl KernelStateStore for InjectedStore {
    fn load(&self) -> Result<Option<DurableKernelState>, String> {
        self.inner.load()
    }

    fn commit(&self, state: &DurableKernelState) -> app_host_kernel::durable::CommitOutcome {
        use app_host_kernel::durable::CommitOutcome;
        match self.failure.swap(0, Ordering::AcqRel) {
            1 => CommitOutcome::NotCommitted("injected failure before commit".into()),
            2 => {
                assert_eq!(self.inner.commit(state), CommitOutcome::Committed);
                CommitOutcome::Indeterminate("injected crash after durable write".into())
            }
            _ => self.inner.commit(state),
        }
    }
}

fn persistent_kernel(
    chrome: Arc<FakeChrome>,
    clock: Arc<FixedClock>,
    store: Arc<dyn KernelStateStore>,
) -> Kernel {
    Kernel::with_clock_and_state_store(chrome, clock, store).unwrap()
}

#[test]
fn restart_restores_grants_ledger_artifacts_and_interrupts_active_runs_once() {
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let store = MemoryKernelStateStore::shared();
    let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
    install_notes(&mut kernel, GrantCondition::Silent);
    let grant_id = kernel.grants_for(&notes_app())[0].grant_id.clone();
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: notes_app(),
                reason: "durability test".into(),
            },
            "produce durable artifact",
        )
        .unwrap();
    let artifact_id = match kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text":"durable"})))
        .unwrap()
    {
        InvocationResult::Completed { artifacts, .. } => artifacts[0].artifact_id.clone(),
        other => panic!("unexpected invocation result: {other:?}"),
    };
    kernel.revoke_grant(&grant_id).unwrap();
    drop(kernel);

    let recovered = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
    assert_eq!(
        recovered.run_view(&run_id).unwrap().terminal_state,
        Some(RunTerminalState::Interrupted)
    );
    assert_eq!(
        recovered.artifact(&artifact_id).unwrap().provenance.run_id,
        run_id
    );
    assert!(recovered.grants_for(&notes_app()).is_empty());
    let record_count = recovered.records().len();
    drop(recovered);

    let recovered_again = persistent_kernel(chrome, clock, store);
    assert_eq!(recovered_again.records().len(), record_count);
    assert_eq!(
        recovered_again
            .records()
            .iter()
            .filter(|record| matches!(
                record.event,
                LedgerEvent::RunEnded {
                    terminal_state: RunTerminalState::Interrupted,
                    ..
                }
            ))
            .count(),
        1
    );
}

#[test]
fn restart_tolerates_grant_request_for_an_uninstalled_provider() {
    // A consumer keeps declaring a grant request against a provider the user
    // later uninstalled. Durable recovery must treat that as a dormant request,
    // not corrupt state, so one gone provider cannot brick the host at boot.
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let store = MemoryKernelStateStore::shared();
    {
        let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
        install_notes(&mut kernel, GrantCondition::Silent);
        install_chat(&mut kernel); // manifest requests notes/create_note
        kernel.uninstall(&notes_app()).expect("notes uninstalls");
    }

    // Before the fix this aborted restore with UnknownApp(notes) and the host
    // process died in its setup hook.
    let recovered = persistent_kernel(chrome, clock, store);
    assert!(recovered.installed_app(&chat_app()).is_ok());
    assert!(recovered.installed_app(&notes_app()).is_err());
}

#[test]
fn recovered_consumer_rebinds_while_its_provider_is_absent() {
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let store = MemoryKernelStateStore::shared();
    {
        let mut kernel = persistent_kernel(chrome, clock.clone(), store.clone());
        install_notes(&mut kernel, GrantCondition::Silent);
        install_chat(&mut kernel);
        kernel.uninstall(&notes_app()).unwrap();
    }

    let restart_chrome = FakeChrome::new();
    let mut recovered = persistent_kernel(restart_chrome.clone(), clock, store);
    let results = recovered
        .install(seal(chat_manifest(chat_app(), vec![])), BTreeMap::new())
        .expect("consumer rebind stays usable with dormant authority");

    assert_eq!(results, vec![IssueResult::Refused]);
    assert!(restart_chrome.grant_prompts.lock().unwrap().is_empty());
    assert!(recovered.installed_app(&chat_app()).is_ok());
}

#[test]
fn failure_before_commit_changes_neither_memory_nor_recovered_state() {
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let inner = MemoryKernelStateStore::shared();
    let store = InjectedStore::new(inner.clone());
    let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
    store.fail_before_commit();
    assert!(kernel
        .install(
            seal(notes_manifest(GrantCondition::Silent)),
            notes_handlers()
        )
        .is_err());
    assert_eq!(kernel.installed_apps().count(), 0);
    drop(kernel);
    let recovered = persistent_kernel(chrome, clock, inner);
    assert_eq!(recovered.installed_apps().count(), 0);
}

#[test]
fn recovered_app_with_changed_manifest_reinstalls_instead_of_erroring() {
    // Regression: a bundled app whose manifest drifts across versions is
    // recovered from durable state (no handlers bound). Re-installing the new
    // manifest must succeed — replacing the stale registration — not fail with
    // AppAlreadyInstalled, which previously aborted startup before the host
    // could rehydrate secrets.
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let store = MemoryKernelStateStore::shared();

    let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
    kernel
        .install(
            seal(notes_manifest(GrantCondition::Silent)),
            notes_handlers(),
        )
        .expect("v1 installs");
    assert_eq!(kernel.grants_for(&notes_app()).len(), 1);
    let v1_hash = kernel
        .installed_app(&notes_app())
        .unwrap()
        .content_hash
        .clone();
    drop(kernel);

    // Restart: the app registration is restored without its handlers.
    let mut recovered = persistent_kernel(chrome, clock, store);
    assert_eq!(recovered.installed_apps().count(), 1);

    // A new build ships a changed manifest under the same app id.
    let mut drifted = notes_manifest(GrantCondition::Silent);
    drifted.display_name = "Notes (v2)".into();
    recovered
        .install(seal(drifted), notes_handlers())
        .expect("changed manifest reinstalls rather than erroring");

    let installed = recovered.installed_app(&notes_app()).unwrap();
    assert_eq!(installed.manifest.display_name, "Notes (v2)");
    assert_ne!(installed.content_hash, v1_hash);
    // Grants were re-consented from scratch, not inherited across the code
    // change (same rule as a same-id reinstall after uninstall).
    assert_eq!(recovered.grants_for(&notes_app()).len(), 1);
}

#[test]
fn rebind_re_requests_a_manifest_grant_that_went_missing_while_dormant() {
    // Regression: a bundled app's install-time grant prompt can be lost (a
    // startup race auto-denies it, or the user revokes the grant later). The
    // app then sits installed but without its declared authority, and nothing
    // ever asked for it again — every use failed with "not permitted" until
    // the user found Settings → Permissions. Rebind now reconciles: restoring
    // a dormant app re-requests exactly the missing manifest grants through
    // trusted chrome.
    let denying = FakeChrome::new();
    denying.set_grant_decision(ApprovalDecision::Denied);
    let clock = FixedClock::new(start_time());
    let store = MemoryKernelStateStore::shared();

    let mut kernel = persistent_kernel(denying.clone(), clock.clone(), store.clone());
    kernel
        .install(
            seal(notes_manifest(GrantCondition::Silent)),
            notes_handlers(),
        )
        .expect("the app installs even when its grant prompt is denied");
    assert!(kernel.grants_for(&notes_app()).is_empty());
    assert_eq!(denying.grant_prompts.lock().unwrap().len(), 1);
    drop(kernel);

    // Restart: this time trusted chrome (the user) approves the re-request.
    let approving = FakeChrome::new();
    let mut recovered = persistent_kernel(approving.clone(), clock.clone(), store.clone());
    recovered
        .install(
            seal(notes_manifest(GrantCondition::Silent)),
            notes_handlers(),
        )
        .expect("rebind succeeds");
    assert_eq!(
        approving.grant_prompts.lock().unwrap().len(),
        1,
        "rebind re-requests the missing grant through trusted chrome"
    );
    assert_eq!(recovered.grants_for(&notes_app()).len(), 1);

    // The recovered authority is real: an invocation completes.
    let run_id = recovered
        .start_run(
            Initiator::App {
                app_id: notes_app(),
                reason: "rebind reconciliation test".into(),
            },
            "create a note with re-requested authority",
        )
        .unwrap();
    assert!(matches!(
        recovered
            .invoke(&run_id, &create_note_ref(), obj(json!({"text":"back"})))
            .unwrap(),
        InvocationResult::Completed { .. }
    ));
    drop(recovered);

    // And it is durable: the re-issued grant survives the next restart.
    let after = persistent_kernel(FakeChrome::new(), clock, store);
    assert_eq!(after.grants_for(&notes_app()).len(), 1);
}

#[test]
fn rebind_with_intact_authority_asks_chrome_nothing() {
    // A normal restart must stay silent: rebinding an app whose manifest
    // grants are all active collects zero prompts and keeps the same grant.
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let store = MemoryKernelStateStore::shared();

    let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
    install_notes(&mut kernel, GrantCondition::Silent);
    let grant_id = kernel.grants_for(&notes_app())[0].grant_id.clone();
    drop(kernel);

    let restart_chrome = FakeChrome::new();
    let mut recovered = persistent_kernel(restart_chrome.clone(), clock, store);
    recovered
        .install(
            seal(notes_manifest(GrantCondition::Silent)),
            notes_handlers(),
        )
        .expect("rebind succeeds");
    assert!(
        restart_chrome.grant_prompts.lock().unwrap().is_empty(),
        "intact authority is never re-prompted"
    );
    let grants = recovered.grants_for(&notes_app());
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].grant_id, grant_id);
}

#[test]
fn failed_replacement_preserves_the_previous_app_and_its_grants() {
    // Regression (transactional replacement): if the single durable commit of
    // a drifted-manifest replacement fails, the previously working app, its
    // manifest, and its grants must survive untouched — the update path is
    // transactional, not destroy-then-rebuild.
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let inner = MemoryKernelStateStore::shared();
    let store = InjectedStore::new(inner.clone());

    let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
    kernel
        .install(
            seal(notes_manifest(GrantCondition::Silent)),
            notes_handlers(),
        )
        .expect("v1 installs");
    let v1_grant = kernel.grants_for(&notes_app())[0].grant_id.clone();
    drop(kernel);

    // Restart: the registration is recovered without handlers, so the next
    // install of a drifted manifest goes through the replacement path.
    let mut recovered = persistent_kernel(chrome, clock, store.clone());
    let mut drifted = notes_manifest(GrantCondition::Silent);
    drifted.display_name = "Notes (v2)".into();

    store.fail_before_commit();
    assert!(recovered.install(seal(drifted), notes_handlers()).is_err());

    let installed = recovered.installed_app(&notes_app()).unwrap();
    assert_eq!(installed.manifest.display_name, "Notes");
    let grants = recovered.grants_for(&notes_app());
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].grant_id, v1_grant);
}

#[test]
fn stale_replacement_approval_cannot_overwrite_a_newer_install() {
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let store = MemoryKernelStateStore::shared();
    {
        let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
        install_notes(&mut kernel, GrantCondition::Silent);
    }

    let mut recovered = persistent_kernel(chrome, clock, store);
    let mut v2 = notes_manifest(GrantCondition::Silent);
    v2.version = "2.0.0".into();
    let stale = recovered
        .prepare_install(seal(v2), notes_handlers())
        .unwrap()
        .await_approval();

    recovered.uninstall(&notes_app()).unwrap();
    let mut v3 = notes_manifest(GrantCondition::Silent);
    v3.version = "3.0.0".into();
    recovered.install(seal(v3), notes_handlers()).unwrap();

    assert!(matches!(
        recovered.commit_install(stale),
        Err(KernelError::PreparedInstallStale)
    ));
    assert_eq!(
        recovered
            .installed_app(&notes_app())
            .unwrap()
            .manifest
            .version,
        "3.0.0"
    );
}

#[test]
fn extension_recovery_is_independent_of_app_id_sort_order() {
    // Regression (two-phase recovery): a contributor whose AppId sorts *before*
    // its target must still recover. Restore inserts every app before resolving
    // cross-app contributions, so lexicographic order cannot break a valid pair.
    fn target_manifest() -> AppManifest {
        AppManifest {
            app_id: AppId::new("z-target"),
            version: "1.0.0".into(),
            display_name: "Target".into(),
            description: "Hosts an extension point".into(),
            capabilities: vec![],
            surfaces: vec![],
            agents: vec![],
            skills: vec![],
            assistant_profiles: vec![],
            automations: vec![],
            connectors: vec![],
            config_declarations: vec![],
            artifact_types: vec![],
            extension_points: vec![ExtensionPointDeclaration {
                name: ExtensionPointName::new("sidebar"),
                contract_version: 1,
                context_schema: obj(json!({"type": "object", "additionalProperties": true})),
            }],
            extension_contributions: vec![],
            grant_requests: vec![],
            event_subscriptions: vec![],
        }
    }
    fn contributor_manifest() -> AppManifest {
        AppManifest {
            app_id: AppId::new("a-contributor"),
            version: "1.0.0".into(),
            display_name: "Contributor".into(),
            description: "Contributes a panel".into(),
            capabilities: vec![],
            surfaces: vec![SurfaceDeclaration {
                name: SurfaceName::new("panel"),
                kind: SurfaceKind::Panel,
                title: "Panel".into(),
                description: "A contributed panel".into(),
                intents: vec![],
            }],
            agents: vec![],
            skills: vec![],
            assistant_profiles: vec![],
            automations: vec![],
            connectors: vec![],
            config_declarations: vec![],
            artifact_types: vec![],
            extension_points: vec![],
            extension_contributions: vec![ExtensionContribution {
                target_app: AppId::new("z-target"),
                extension_point: ExtensionPointName::new("sidebar"),
                contract_version: 1,
                surface: SurfaceName::new("panel"),
            }],
            grant_requests: vec![],
            event_subscriptions: vec![],
        }
    }

    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let store = MemoryKernelStateStore::shared();
    let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
    // Target first: live install resolves the contribution eagerly.
    kernel
        .install(seal(target_manifest()), BTreeMap::new())
        .expect("target installs");
    kernel
        .install(seal(contributor_manifest()), BTreeMap::new())
        .expect("contributor installs");
    drop(kernel);

    let recovered = persistent_kernel(chrome, clock, store);
    assert_eq!(recovered.installed_apps().count(), 2);
    assert!(recovered
        .installed_app(&AppId::new("a-contributor"))
        .is_ok());
    assert!(recovered.installed_app(&AppId::new("z-target")).is_ok());
}

#[test]
fn durable_write_wins_if_process_dies_before_memory_swap() {
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let inner = MemoryKernelStateStore::shared();
    let store = InjectedStore::new(inner.clone());
    let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
    store.fail_after_durable_write();
    assert!(kernel
        .install(
            seal(notes_manifest(GrantCondition::Silent)),
            notes_handlers()
        )
        .is_err());
    assert_eq!(kernel.installed_apps().count(), 0);
    let recovery_error = kernel
        .install(
            seal(notes_manifest(GrantCondition::Silent)),
            notes_handlers(),
        )
        .unwrap_err();
    assert!(matches!(
        recovery_error,
        KernelError::Durability(message) if message.contains("recovery required")
    ));
    drop(kernel);

    let mut recovered = persistent_kernel(chrome, clock, inner);
    assert_eq!(recovered.installed_apps().count(), 1);
    recovered
        .install(
            seal(notes_manifest(GrantCondition::Silent)),
            notes_handlers(),
        )
        .unwrap();
    assert_eq!(recovered.installed_apps().count(), 1);
    assert_eq!(recovered.grants_for(&notes_app()).len(), 1);
}

#[test]
fn artifact_completion_batch_is_absent_before_commit_and_unique_after_durable_write() {
    for after_durable_write in [false, true] {
        let chrome = FakeChrome::new();
        let clock = FixedClock::new(start_time());
        let inner = MemoryKernelStateStore::shared();
        let store = InjectedStore::new(inner.clone());
        let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
        install_notes(&mut kernel, GrantCondition::Silent);
        let run_id = kernel
            .start_run(
                Initiator::App {
                    app_id: notes_app(),
                    reason: "completion crash".into(),
                },
                "completion crash",
            )
            .unwrap();
        let prepared = match kernel
            .prepare_invocation(
                &run_id,
                &create_note_ref(),
                InvocationRequest {
                    input: obj(json!({"text":"one"})),
                    data_scope: DataScope::None,
                },
            )
            .unwrap()
        {
            app_host_kernel::PrepareInvocation::Prepared(prepared) => prepared,
            _ => panic!("invocation unexpectedly refused"),
        };
        let authorized = match kernel
            .authorize_invocation(prepared.await_approval())
            .unwrap()
        {
            app_host_kernel::AuthorizeInvocation::Authorized(authorized) => authorized,
            _ => panic!("invocation unexpectedly refused"),
        };
        let executed = authorized.execute();
        if after_durable_write {
            store.fail_after_durable_write();
        } else {
            store.fail_before_commit();
        }
        assert!(kernel.finalize_invocation(executed).is_err());
        assert_eq!(kernel.artifacts().count(), 0);
        assert!(matches!(
            kernel.end_run(&run_id, RunTerminalState::Failed),
            Err(KernelError::Durability(message)) if message.contains("recovery required")
        ));
        drop(kernel);

        let recovered = persistent_kernel(chrome, clock, inner);
        let completed = recovered
            .records_for_run(&run_id)
            .filter(|record| matches!(record.event, LedgerEvent::CapabilityCompleted { .. }))
            .count();
        let produced = recovered
            .records_for_run(&run_id)
            .filter(|record| matches!(record.event, LedgerEvent::ArtifactProduced { .. }))
            .count();
        if after_durable_write {
            assert_eq!(
                (completed, produced, recovered.artifacts().count()),
                (1, 1, 1)
            );
        } else {
            assert_eq!(
                (completed, produced, recovered.artifacts().count()),
                (0, 0, 0)
            );
        }
    }
}

#[test]
fn recovery_rejects_completion_without_an_invocation() {
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let store = MemoryKernelStateStore::shared();
    {
        let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
        install_notes(&mut kernel, GrantCondition::Silent);
        let run_id = kernel
            .start_run(
                Initiator::App {
                    app_id: notes_app(),
                    reason: "corruption fixture".into(),
                },
                "produce fixture",
            )
            .unwrap();
        kernel
            .invoke(&run_id, &create_note_ref(), obj(json!({"text": "fixture"})))
            .unwrap();
    }

    let mut state = store.load().unwrap().unwrap();
    state.ledger_records.retain(|record| {
        !matches!(
            record.event,
            LedgerEvent::CapabilityInvoked { .. } | LedgerEvent::ArtifactProduced { .. }
        )
    });
    for (sequence, record) in state.ledger_records.iter_mut().enumerate() {
        record.sequence = sequence as u64;
    }
    state.artifacts.clear();
    assert_eq!(
        store.commit(&state),
        app_host_kernel::durable::CommitOutcome::Committed
    );

    let error = match Kernel::with_clock_and_state_store(chrome, clock, store) {
        Ok(_) => panic!("corrupt completion must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        KernelError::Durability(message) if message.contains("no matching invocation")
    ));
}

#[test]
fn recovery_rejects_forged_artifact_producer() {
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let store = MemoryKernelStateStore::shared();
    {
        let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
        install_notes(&mut kernel, GrantCondition::Silent);
        let run_id = kernel
            .start_run(
                Initiator::App {
                    app_id: notes_app(),
                    reason: "corruption fixture".into(),
                },
                "produce fixture",
            )
            .unwrap();
        kernel
            .invoke(&run_id, &create_note_ref(), obj(json!({"text": "fixture"})))
            .unwrap();
    }

    let mut state = store.load().unwrap().unwrap();
    state.artifacts[0].provenance.produced_by = AppId::new("forged-producer");
    assert_eq!(
        store.commit(&state),
        app_host_kernel::durable::CommitOutcome::Committed
    );

    let error = match Kernel::with_clock_and_state_store(chrome, clock, store) {
        Ok(_) => panic!("forged producer must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        KernelError::Durability(message) if message.contains("producer disagrees")
    ));
}

#[test]
fn recovery_rejects_non_positive_grant_lifetime() {
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let store = MemoryKernelStateStore::shared();
    {
        let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
        install_notes(&mut kernel, GrantCondition::Silent);
    }

    let mut state = store.load().unwrap().unwrap();
    state.grants[0].expires_at = Some(state.grants[0].issued_at);
    assert_eq!(
        store.commit(&state),
        app_host_kernel::durable::CommitOutcome::Committed
    );

    assert!(matches!(
        Kernel::with_clock_and_state_store(chrome, clock, store),
        Err(KernelError::InvalidGrantDuration)
    ));
}

#[test]
fn revoke_and_uninstall_recover_from_their_single_commit_boundary() {
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let inner = MemoryKernelStateStore::shared();
    let store = InjectedStore::new(inner.clone());
    let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
    install_notes(&mut kernel, GrantCondition::Silent);
    let grant_id = kernel.grants_for(&notes_app())[0].grant_id.clone();

    store.fail_before_commit();
    assert!(kernel.revoke_grant(&grant_id).is_err());
    assert_eq!(kernel.grants_for(&notes_app()).len(), 1);

    store.fail_after_durable_write();
    assert!(kernel.revoke_grant(&grant_id).is_err());
    assert_eq!(kernel.grants_for(&notes_app()).len(), 1);
    drop(kernel);
    let mut recovered = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
    assert!(recovered.grants_for(&notes_app()).is_empty());

    let run_id = recovered
        .start_run(
            Initiator::App {
                app_id: notes_app(),
                reason: "uninstall crash".into(),
            },
            "uninstall crash",
        )
        .unwrap();
    store.fail_after_durable_write();
    assert!(recovered.uninstall(&notes_app()).is_err());
    assert!(recovered.installed_app(&notes_app()).is_ok());
    drop(recovered);

    let recovered = persistent_kernel(chrome, clock, inner);
    assert!(recovered.installed_app(&notes_app()).is_err());
    assert_eq!(
        recovered.run_view(&run_id).unwrap().terminal_state,
        Some(RunTerminalState::Cancelled)
    );
}
