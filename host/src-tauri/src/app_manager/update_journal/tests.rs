use super::*;

#[test]
fn update_journal_serializes_strictly() {
    let journal = UpdateJournal::new(
        "transition".into(),
        "com.example.app".into(),
        ManagedAppOperation::Update,
        Some("revision-a".into()),
        AppRevision {
            revision_id: "revision-b".into(),
            version: "1.0.1".into(),
            display_name: "Example".into(),
            description: "Example".into(),
            backend_kind: "none".into(),
            publisher: None,
            signature_verdict: "unsigned".into(),
            signature_key_id: None,
            min_host_version: "0.0.1".into(),
            installed_at: "2026-07-18T00:00:00Z".into(),
            payload_dir: "apps/com.example.app/revisions/revision-b".into(),
            package_digest: "sha256-test".into(),
        },
        Vec::new(),
        true,
    );

    let json = serde_json::to_value(&journal).unwrap();
    assert_eq!(json["version"], JOURNAL_VERSION);
    assert_eq!(json["phase"], "prepared");
    assert!(serde_json::from_value::<UpdateJournal>(json).is_ok());
}

#[test]
fn update_journal_rejects_a_non_current_version() {
    let mut journal = UpdateJournal::new(
        "transition".into(),
        "com.example.app".into(),
        ManagedAppOperation::Update,
        None,
        AppRevision {
            revision_id: "revision".into(),
            version: "1.0.0".into(),
            display_name: "Example".into(),
            description: "Example".into(),
            backend_kind: "none".into(),
            publisher: None,
            signature_verdict: "unsigned".into(),
            signature_key_id: None,
            min_host_version: "0.0.1".into(),
            installed_at: "2026-07-18T00:00:00Z".into(),
            payload_dir: "apps/com.example.app/revisions/revision".into(),
            package_digest: "sha256-test".into(),
        },
        Vec::new(),
        true,
    );
    journal.version += 1;

    let error = journal.validate_version().unwrap_err();
    assert_eq!(
        error,
        "unsupported app update journal version 3; expected 2"
    );
}

#[test]
fn data_transition_journal_round_trips_every_recovery_phase() {
    let phases = [
        UpdatePhase::Prepared,
        UpdatePhase::Deactivated,
        UpdatePhase::DataCandidateValidated,
        UpdatePhase::DataCommitted,
        UpdatePhase::Activated,
        UpdatePhase::RollingBack,
        UpdatePhase::DataRollbackCommitted,
        UpdatePhase::RolledBack,
        UpdatePhase::Committed,
    ];
    for phase in phases {
        let source_digest = if matches!(phase, UpdatePhase::Prepared | UpdatePhase::Deactivated) {
            None
        } else {
            Some("sha256-source".into())
        };
        let candidate_digest = source_digest
            .as_ref()
            .map(|_| "sha256-candidate".to_string());
        let mut journal = UpdateJournal::new(
            "transition".into(),
            "com.example.app".into(),
            ManagedAppOperation::Update,
            Some("revision-a".into()),
            AppRevision {
                revision_id: "revision-b".into(),
                version: "2.0.0".into(),
                display_name: "Example".into(),
                description: "Example".into(),
                backend_kind: "mcp-stdio".into(),
                publisher: None,
                signature_verdict: "unsigned".into(),
                signature_key_id: None,
                min_host_version: "0.0.1".into(),
                installed_at: "2026-08-03T00:00:00Z".into(),
                payload_dir: "apps/com.example.app/revisions/revision-b".into(),
                package_digest: "sha256-test".into(),
            },
            Vec::new(),
            true,
        )
        .with_data_transition(Some(AppDataTransitionJournal {
            source_revision_id: Some("11111111-1111-4111-8111-111111111111".into()),
            source_format_version: Some(1),
            source_digest,
            candidate: crate::app_data::AppDataRevision {
                revision_id: "22222222-2222-4222-8222-222222222222".into(),
                format_version: 2,
                package_revision_id: "revision-b".into(),
                created_at: "2026-08-03T00:00:00Z".into(),
            },
            candidate_digest,
            migration_revision_id: Some("revision-b".into()),
            destructive: false,
        }));
        journal.phase = phase.clone();
        let encoded = serde_json::to_value(&journal).unwrap();
        let decoded: UpdateJournal = serde_json::from_value(encoded).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded.phase, phase);
        assert_eq!(decoded.data_transition, journal.data_transition);
    }
}
