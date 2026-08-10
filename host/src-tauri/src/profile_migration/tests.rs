use super::*;

use std::cell::Cell;
use std::ffi::OsString;
use std::path::PathBuf;

use chrono::{Duration, TimeZone, Utc};

use crate::app_manager::{AppRevision, ManagedAppOperation, UpdateJournal};

fn test_paths(label: &str) -> HostPaths {
    let root = std::env::temp_dir().join(format!(
        "kestral-profile-migration-{label}-{}",
        Uuid::new_v4()
    ));
    HostPaths::resolve_startup_from(root, std::iter::empty::<OsString>(), |_| None).unwrap()
}

fn write_legacy_journal(paths: &HostPaths) -> Vec<u8> {
    let journal = UpdateJournal::new(
        "transition-alpha-1".into(),
        "com.example.app".into(),
        ManagedAppOperation::Update,
        Some("revision-old".into()),
        AppRevision {
            revision_id: "revision-new".into(),
            version: "0.2.0".into(),
            display_name: "Example".into(),
            description: "Example app".into(),
            backend_kind: "none".into(),
            publisher: None,
            signature_verdict: "unsigned".into(),
            signature_key_id: None,
            min_host_version: "0.1.0-alpha.1".into(),
            installed_at: "2026-08-03T00:00:00Z".into(),
            payload_dir: paths
                .app_records_root()
                .join("com.example.app/revisions/revision-new")
                .display()
                .to_string(),
            package_digest: "digest-new".into(),
        },
        Vec::new(),
        true,
    );
    let bytes = serde_json::to_vec_pretty(&journal).unwrap();
    fs::write(paths.root().join(LEGACY_UPDATE_JOURNAL), &bytes).unwrap();
    bytes
}

fn cleanup(paths: &HostPaths) {
    fs::remove_dir_all(paths.root()).unwrap();
}

#[test]
fn migrates_the_legacy_update_journal_and_retains_original_bytes() {
    let paths = test_paths("success");
    let original = write_legacy_journal(&paths);

    run(&paths).unwrap();

    assert!(!paths.root().join(LEGACY_UPDATE_JOURNAL).exists());
    assert_eq!(
        fs::read(paths.root().join(UPDATE_JOURNAL)).unwrap(),
        original
    );
    assert!(!paths.root().join(JOURNAL_FILE).exists());
    let backups = fs::read_dir(paths.root().join(BACKUP_DIR))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        fs::read(backups[0].path().join(LEGACY_UPDATE_JOURNAL)).unwrap(),
        original
    );
    cleanup(&paths);
}

#[test]
fn current_profile_is_a_repeatable_no_op() {
    let paths = test_paths("noop");
    write_legacy_journal(&paths);
    run(&paths).unwrap();
    let before = tree_digest(paths.root(), true).unwrap();

    run(&paths).unwrap();

    assert_eq!(tree_digest(paths.root(), true).unwrap(), before);
    assert!(!paths.root().join(JOURNAL_FILE).exists());
    cleanup(&paths);
}

#[test]
fn recovery_is_idempotent_after_every_persisted_phase() {
    for phase in [
        MigrationPhase::Planned,
        MigrationPhase::CandidateStaged,
        MigrationPhase::CandidateValidated,
        MigrationPhase::BackupRetained,
        MigrationPhase::CommitStarted,
        MigrationPhase::Committed,
    ] {
        let paths = test_paths(&format!("phase-{phase:?}"));
        let original = write_legacy_journal(&paths);
        let failed = Cell::new(false);

        let error = run_with_observer(&paths, |persisted| {
            if persisted == phase && !failed.replace(true) {
                return Err(format!("injected failure after {phase:?}"));
            }
            Ok(())
        })
        .unwrap_err();
        assert!(error.contains("injected failure"), "{error}");

        run(&paths).unwrap();
        assert!(!paths.root().join(LEGACY_UPDATE_JOURNAL).exists());
        assert_eq!(
            fs::read(paths.root().join(UPDATE_JOURNAL)).unwrap(),
            original,
            "phase {phase:?}"
        );
        cleanup(&paths);
    }
}

#[test]
fn corrupt_and_unknown_store_versions_are_preserved_and_refused() {
    let corrupt = test_paths("corrupt");
    fs::write(corrupt.config_path(), b"{").unwrap();
    let error = run(&corrupt).unwrap_err();
    assert!(error.contains("parse host config"), "{error}");
    assert_eq!(fs::read(corrupt.config_path()).unwrap(), b"{");
    cleanup(&corrupt);

    let unknown = test_paths("unknown");
    fs::write(unknown.chat_store_path(), br#"{"version":99,"threads":[]}"#).unwrap();
    let error = run(&unknown).unwrap_err();
    assert!(
        error.contains("unsupported chat storage version"),
        "{error}"
    );
    cleanup(&unknown);
}

#[test]
fn reset_and_gateway_audit_require_their_complete_current_shapes() {
    let reset = test_paths("bad-reset-shape");
    fs::write(
        reset.root().join("kestral-system-reset.json"),
        br#"{"version":1,"profile_id":"profile-other","requested_at":"2026-08-03T00:00:00Z","extra":true}"#,
    )
    .unwrap();
    let error = run(&reset).unwrap_err();
    assert!(
        error.contains("unknown field") || error.contains("does not match"),
        "{error}"
    );
    cleanup(&reset);

    let audit = test_paths("bad-audit-shape");
    fs::write(
        audit.mcp_audit_path(),
        br#"{"format_version":1,"at":"2026-08-03T00:00:00Z","event":{"profile_id":"profile-alpha-1"}}"#,
    )
    .unwrap();
    let error = run(&audit).unwrap_err();
    assert!(error.contains("has no event name"), "{error}");
    cleanup(&audit);
}

#[test]
fn registry_and_profile_identity_disagreement_is_refused() {
    let paths = test_paths("identity");
    let identity_path = paths.root().join("kestral-profile.json");
    let mut identity: Value = serde_json::from_slice(&fs::read(&identity_path).unwrap()).unwrap();
    identity["profile"]["profile_id"] = Value::String("profile-other".into());
    fs::write(
        &identity_path,
        serde_json::to_vec_pretty(&identity).unwrap(),
    )
    .unwrap();

    let error = run(&paths).unwrap_err();

    assert!(error.contains("selected runtime profile"), "{error}");
    cleanup(&paths);
}

#[test]
fn missing_vault_values_do_not_rewrite_secret_references() {
    let paths = test_paths("missing-vault");
    let references = br#"{
  "version": 2,
  "secrets": [
    { "owner": "com.example.app", "name": "api-key", "status": "stored" }
  ]
}"#;
    fs::write(paths.secrets_index_path(), references).unwrap();
    write_legacy_journal(&paths);

    run(&paths).unwrap();

    assert_eq!(fs::read(paths.secrets_index_path()).unwrap(), references);
    cleanup(&paths);
}

fn grant() -> Grant {
    let issued_at = Utc.with_ymd_and_hms(2026, 8, 3, 0, 0, 0).unwrap();
    Grant {
        grant_id: app_host_kernel::ids::GrantId::new("grant-1"),
        holder: app_host_kernel::ids::AppId::new("com.example.consumer"),
        scope: GrantScope::ExactCapability {
            provider: app_host_kernel::ids::AppId::new("com.example.provider"),
            capability: app_host_kernel::ids::CapabilityName::new("read"),
        },
        data_scope: DataScope::resources(vec![app_host_kernel::ids::ResourceId::new("resource-1")])
            .unwrap(),
        condition: GrantCondition::RequiresApproval,
        origin: app_host_kernel::primitives::grant::GrantOrigin::ManifestRequested,
        issued_at,
        expires_at: Some(issued_at + Duration::hours(1)),
    }
}

#[test]
fn grant_migration_cannot_widen_any_authority_dimension() {
    let previous = grant();

    let mut provider_wide = previous.clone();
    provider_wide.scope = GrantScope::AllProviderCapabilities {
        provider: app_host_kernel::ids::AppId::new("com.example.provider"),
    };
    assert!(validate_grant_not_widened(&previous, &provider_wide).is_err());

    let mut all_resources = previous.clone();
    all_resources.data_scope = DataScope::AllResources;
    assert!(validate_grant_not_widened(&previous, &all_resources).is_err());

    let mut silent = previous.clone();
    silent.condition = GrantCondition::Silent;
    assert!(validate_grant_not_widened(&previous, &silent).is_err());

    let mut extended = previous.clone();
    extended.expires_at = previous
        .expires_at
        .map(|expiry| expiry + Duration::seconds(1));
    assert!(validate_grant_not_widened(&previous, &extended).is_err());

    let mut narrowed = previous.clone();
    narrowed.expires_at = previous
        .expires_at
        .map(|expiry| expiry - Duration::seconds(1));
    assert!(validate_grant_not_widened(&previous, &narrowed).is_ok());
}

#[test]
fn inventory_assigns_each_required_store_exactly_one_owner() {
    let stores: BTreeSet<_> = DURABLE_STORE_OWNERS
        .iter()
        .map(|entry| entry.store)
        .collect();
    assert_eq!(stores.len(), DURABLE_STORE_OWNERS.len());
    for required in [
        "profile registry and identity",
        "host config",
        "secret-reference index and vault identities",
        "kernel state",
        "chat threads",
        "installed-app registry and payload records",
        "app update journal",
        "app-data revision index and backups",
        "private surface state",
        "trusted notices",
        "publisher trust",
        "file resources",
        "remote-owner passkeys",
        "pending reset",
        "portable recovery status",
        "gateway audit",
        "browser custom themes",
        "pending-send recovery",
    ] {
        assert!(stores.contains(required), "missing owner for {required}");
    }

    let formats: BTreeMap<_, _> = DURABLE_STORE_OWNERS
        .iter()
        .map(|entry| (entry.store, entry.format))
        .collect();
    for (store, format) in [
        ("profile registry and identity", "v1"),
        ("host config", "storage v3 / domain v2"),
        (
            "secret-reference index and vault identities",
            "index v2 / derived vault accounts",
        ),
        ("kernel state", "v1"),
        ("chat threads", "v4"),
        (
            "installed-app registry and payload records",
            "registry v4 / package format 1",
        ),
        ("app update journal", "v2 JSON document"),
        (
            "app-data revision index and backups",
            "index v1 / opaque revisions",
        ),
        ("private surface state", "envelope v2 / opaque values"),
        ("trusted notices", "v1"),
        ("publisher trust", "v1"),
        ("file resources", "v1"),
        ("remote-owner passkeys", "v1"),
        ("pending reset", "v1"),
        ("portable recovery status", "v1"),
        ("gateway audit", "JSONL event v1"),
        ("browser custom themes", "v2"),
        ("pending-send recovery", "v1"),
    ] {
        assert_eq!(
            formats.get(store),
            Some(&format),
            "format drift for {store}"
        );
    }
}

#[test]
fn ephemeral_state_has_an_explicit_non_migration_disposition() {
    let states: BTreeSet<_> = EPHEMERAL_STATE_DISPOSITIONS
        .iter()
        .map(|entry| entry.state)
        .collect();
    assert_eq!(states.len(), EPHEMERAL_STATE_DISPOSITIONS.len());
    for required in [
        "workers",
        "MCP sessions",
        "pending approvals and OAuth ceremonies",
        "owner sessions",
        "surfaces",
        "leases",
        "event inboxes",
        "transient progress",
        "live streams",
    ] {
        assert!(
            states.contains(required),
            "missing disposition for {required}"
        );
    }
}

#[test]
fn revoked_grants_cannot_revive_and_owner_sessions_are_not_durable_state() {
    let previous = grant();
    let source = app_host_kernel::durable::DurableKernelState {
        installed_apps: Vec::new(),
        grants: vec![previous.clone()],
        revoked_grant_ids: vec![previous.grant_id.clone()],
        ledger_records: Vec::new(),
        artifacts: Vec::new(),
    };
    let mut candidate = source.clone();
    candidate.revoked_grant_ids.clear();

    assert!(validate_kernel_authority(&source, &candidate)
        .unwrap_err()
        .contains("revived a revoked grant"));
    let serialized = serde_json::to_value(source).unwrap();
    assert!(serialized.get("owner_sessions").is_none());
}

#[test]
fn frozen_alpha_1_whole_profile_opens_without_rewriting_fixture_bytes() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/persistence/alpha-1/whole-profile");
    let paths = test_paths("frozen-whole-profile");
    fs::remove_dir_all(paths.root()).unwrap();
    materialize_fixture(&fixture, paths.root(), paths.root()).unwrap();

    let loaded = HostPaths::resolve_startup_from(
        paths.root().to_path_buf(),
        std::iter::empty::<OsString>(),
        |_| None,
    )
    .unwrap();
    run(&loaded).unwrap();

    assert_eq!(loaded.profile_id(), "profile-alpha-1");
    assert!(loaded
        .root()
        .join("apps/.data/com.example.fixture/app-data-state-v1.json")
        .exists());
    assert!(loaded
        .root()
        .join("apps/.data/com.example.fixture/app-data-revisions/11111111-1111-4111-8111-111111111111/opaque-data.json")
        .exists());
    cleanup(&loaded);
}

#[test]
fn frozen_alpha_1_fixture_bytes_match_the_corpus_manifest() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/persistence/alpha-1");
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("CORPUS.json")).unwrap()).unwrap();
    assert_eq!(manifest["release"], "0.1.0-alpha.1");
    let expected = manifest["sha256"].as_object().unwrap();
    let mut actual = BTreeMap::new();
    visit_files(&root, &mut |path| {
        if path.file_name().and_then(|name| name.to_str()) == Some("CORPUS.json") {
            return Ok(());
        }
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        actual.insert(relative, file_digest(path)?);
        Ok(())
    })
    .unwrap();
    assert_eq!(actual.len(), expected.len());
    for (path, digest) in actual {
        assert_eq!(
            expected.get(&path).and_then(Value::as_str),
            Some(digest.as_str()),
            "{path}"
        );
    }
}

#[test]
fn frozen_remote_owner_passkey_fixture_is_strictly_readable() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/persistence/alpha-1/store-samples/remote-owner-auth-v1.json");
    crate::remote_auth::validate_persisted_owner_auth(&path).unwrap();
}

#[test]
fn fixture_profile_roots_are_json_escaped() {
    let path = r"C:\Users\runner\AppData\Local\Kestral";
    let escaped = json_escaped_string_contents(path).unwrap();

    assert_eq!(escaped, r"C:\\Users\\runner\\AppData\\Local\\Kestral");
    assert_eq!(
        serde_json::from_str::<String>(&format!(r#""{escaped}""#)).unwrap(),
        path
    );
}

fn materialize_fixture(
    source: &Path,
    destination: &Path,
    profile_root: &Path,
) -> Result<(), String> {
    let escaped_profile_root = json_escaped_string_contents(&profile_root.display().to_string())?;
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            materialize_fixture(&entry.path(), &target, profile_root)?;
        } else {
            let bytes = fs::read(entry.path()).map_err(|error| error.to_string())?;
            let text = String::from_utf8(bytes).map_err(|error| error.to_string())?;
            fs::write(
                target,
                text.replace("{{PROFILE_ROOT}}", &escaped_profile_root),
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn json_escaped_string_contents(value: &str) -> Result<String, String> {
    let serialized = serde_json::to_string(value).map_err(|error| error.to_string())?;
    serialized
        .strip_prefix('"')
        .and_then(|contents| contents.strip_suffix('"'))
        .map(str::to_string)
        .ok_or_else(|| "serialized fixture path is not a JSON string".to_string())
}
