use crate::helpers::*;
use std::sync::atomic::{AtomicU8, Ordering};

struct UpgradeStore {
    inner: Arc<MemoryKernelStateStore>,
    failure: AtomicU8,
}

impl UpgradeStore {
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

impl KernelStateStore for UpgradeStore {
    fn load(&self) -> Result<Option<DurableKernelState>, String> {
        self.inner.load()
    }

    fn commit(&self, state: &DurableKernelState) -> app_host_kernel::durable::CommitOutcome {
        use app_host_kernel::durable::CommitOutcome;

        match self.failure.swap(0, Ordering::AcqRel) {
            1 => CommitOutcome::NotCommitted("injected upgrade failure before commit".into()),
            2 => {
                assert_eq!(self.inner.commit(state), CommitOutcome::Committed);
                CommitOutcome::Indeterminate("injected upgrade crash after durable write".into())
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

fn upgraded_notes_manifest() -> AppManifest {
    let mut manifest = notes_manifest(GrantCondition::Silent);
    manifest.version = "2.0.0".into();
    manifest.display_name = "Notes 2".into();
    manifest.description = "Creates note artifacts, updated".into();
    manifest
}

fn upgraded_notes_handler() -> CapabilityHandler {
    Box::new(|input, _context| {
        Ok(CapabilityOutcome {
            result: json!({"created": true, "revision": 2}),
            artifacts: vec![ArtifactDraft {
                artifact_type: ArtifactTypeName::new("note"),
                title: "Updated note".into(),
                content: json!({"text": input["text"]}),
            }],
        })
    })
}

fn upgraded_notes_handlers() -> BTreeMap<CapabilityName, CapabilityHandler> {
    BTreeMap::from([(create_note(), upgraded_notes_handler())])
}

#[test]
fn metadata_and_version_upgrade_preserves_authority_and_history() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let grant_id = kernel.grants_for(&notes_app())[0].grant_id.clone();
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: notes_app(),
                reason: "upgrade history".into(),
            },
            "create before upgrade",
        )
        .unwrap();
    let artifact_id = match kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "before"})))
        .unwrap()
    {
        InvocationResult::Completed { artifacts, .. } => artifacts[0].artifact_id.clone(),
        other => panic!("unexpected invocation result: {other:?}"),
    };
    let record_count = kernel.records().len();

    kernel
        .upgrade_app(seal(upgraded_notes_manifest()), upgraded_notes_handlers())
        .expect("presentation-only upgrade succeeds");

    let installed = kernel.installed_app(&notes_app()).unwrap();
    assert_eq!(installed.manifest.version, "2.0.0");
    assert_eq!(kernel.grants_for(&notes_app())[0].grant_id, grant_id);
    assert_eq!(kernel.records().len(), record_count);
    assert_eq!(
        kernel.artifact(&artifact_id).unwrap().provenance.run_id,
        run_id
    );
    assert_eq!(kernel.run_view(&run_id).unwrap().terminal_state, None);

    let result = kernel
        .invoke(&run_id, &create_note_ref(), obj(json!({"text": "after"})))
        .unwrap();
    assert!(matches!(result, InvocationResult::Completed { .. }));
    assert!(kernel.records().len() > record_count);
}

#[test]
fn authority_and_behavioral_contract_changes_are_refused_without_mutation() {
    let (mut kernel, _, _) = test_kernel();
    install_notes(&mut kernel, GrantCondition::Silent);
    let original_manifest = kernel.installed_app(&notes_app()).unwrap().manifest.clone();
    let original_hash = kernel
        .installed_app(&notes_app())
        .unwrap()
        .content_hash
        .clone();
    let original_grant = kernel.grants_for(&notes_app())[0].clone();
    let original_records = kernel.records().to_vec();

    let mut changed_capability = upgraded_notes_manifest();
    changed_capability.capabilities[0].effect = CapabilityEffect::Destructive;
    assert!(matches!(
        kernel.upgrade_app(seal(changed_capability), notes_handlers()),
        Err(KernelError::AppUpgradeContractChanged { .. })
    ));

    let mut changed_grant = upgraded_notes_manifest();
    changed_grant.grant_requests[0].condition = GrantCondition::RequiresApproval;
    assert!(matches!(
        kernel.upgrade_app(seal(changed_grant), notes_handlers()),
        Err(KernelError::AppUpgradeContractChanged { .. })
    ));

    let mut changed_schema = upgraded_notes_manifest();
    changed_schema.capabilities[0].input_schema = obj(json!({
        "type": "object",
        "properties": {"text": {"type": "number"}},
        "required": ["text"],
        "additionalProperties": false,
    }));
    assert!(matches!(
        kernel.upgrade_app(seal(changed_schema), notes_handlers()),
        Err(KernelError::AppUpgradeContractChanged { .. })
    ));

    let installed = kernel.installed_app(&notes_app()).unwrap();
    assert_eq!(installed.manifest, original_manifest);
    assert_eq!(installed.content_hash, original_hash);
    assert_eq!(kernel.grants_for(&notes_app())[0], &original_grant);
    assert_eq!(kernel.records(), original_records.as_slice());
}

#[test]
fn upgrade_not_committed_leaves_live_and_durable_state_unchanged() {
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let inner = MemoryKernelStateStore::shared();
    let store = UpgradeStore::new(inner.clone());
    let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
    install_notes(&mut kernel, GrantCondition::Silent);
    let original_hash = kernel
        .installed_app(&notes_app())
        .unwrap()
        .content_hash
        .clone();
    let original_grant = kernel.grants_for(&notes_app())[0].grant_id.clone();

    store.fail_before_commit();
    assert!(matches!(
        kernel.upgrade_app(seal(upgraded_notes_manifest()), upgraded_notes_handlers()),
        Err(KernelError::Durability(_))
    ));
    assert_eq!(
        kernel.installed_app(&notes_app()).unwrap().content_hash,
        original_hash
    );
    assert_eq!(kernel.grants_for(&notes_app())[0].grant_id, original_grant);
    drop(kernel);

    let recovered = persistent_kernel(chrome, clock, inner);
    assert_eq!(
        recovered
            .installed_app(&notes_app())
            .unwrap()
            .manifest
            .version,
        "1.0.0"
    );
}

#[test]
fn indeterminate_upgrade_commit_requires_recovery_and_durable_state_wins() {
    let chrome = FakeChrome::new();
    let clock = FixedClock::new(start_time());
    let inner = MemoryKernelStateStore::shared();
    let store = UpgradeStore::new(inner.clone());
    let mut kernel = persistent_kernel(chrome.clone(), clock.clone(), store.clone());
    install_notes(&mut kernel, GrantCondition::Silent);

    store.fail_after_durable_write();
    let error = kernel
        .upgrade_app(seal(upgraded_notes_manifest()), upgraded_notes_handlers())
        .unwrap_err();
    assert!(matches!(
        error,
        KernelError::Durability(message) if message.contains("recovery")
    ));
    assert_eq!(
        kernel.installed_app(&notes_app()).unwrap().manifest.version,
        "1.0.0"
    );
    assert!(matches!(
        kernel.upgrade_app(seal(upgraded_notes_manifest()), upgraded_notes_handlers()),
        Err(KernelError::Durability(message)) if message.contains("recovery required")
    ));
    drop(kernel);

    let recovered = persistent_kernel(chrome, clock, inner);
    assert_eq!(
        recovered
            .installed_app(&notes_app())
            .unwrap()
            .manifest
            .version,
        "2.0.0"
    );
    assert_eq!(recovered.grants_for(&notes_app()).len(), 1);
}
