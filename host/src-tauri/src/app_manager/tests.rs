use super::update_journal::{UpdateJournal, UpdatePhase};
use super::*;
use crate::package;
use app_host_kernel::ids::ExtensionPointName;
use app_host_kernel::kernel::Kernel;
use app_host_kernel::manifest::{AppManifest, ExtensionPointDeclaration};
use app_host_kernel::services::chrome::{
    ApprovalDecision, ChromeNotice, ChromeNoticeError, TrustedChrome,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct TestChrome;

impl TrustedChrome for TestChrome {
    fn confirm_grant(
        &self,
        _prompt: app_host_kernel::services::chrome::GrantIssuancePrompt,
    ) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn approve_capability(
        &self,
        _prompt: app_host_kernel::services::chrome::CapabilityApprovalPrompt,
    ) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn confirm_event_subscriptions(
        &self,
        _prompt: app_host_kernel::services::chrome::EventSubscriptionPrompt,
    ) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn show_notice(&self, _notice: ChromeNotice) -> Result<(), ChromeNoticeError> {
        Ok(())
    }
}

fn workspace(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("app-manager-{label}-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn package_dir(root: &Path) -> PathBuf {
    root.join("package")
}

fn write_package(
    root: &Path,
    id: &str,
    version: &str,
    display_name: &str,
    description: &str,
    publisher_key_id: Option<&str>,
    grant_requests: Vec<serde_json::Value>,
) {
    fs::create_dir_all(root).unwrap();
    let mut publisher = json!({"name": "Publisher"});
    if let Some(key_id) = publisher_key_id {
        publisher["key_id"] = json!(key_id);
    }
    let document = json!({
        "format_version": 1,
        "id": id,
        "version": version,
        "display_name": display_name,
        "description": description,
        "publisher": publisher,
        "min_host_version": "0.0.1",
        "manifest": {
            "grant_requests": grant_requests,
        },
        "backend": {"kind": "none"},
        "data": {"kind": "none"},
        "integrity": {"algorithm": "sha256", "assets": {}},
    });
    fs::write(
        root.join("app.json"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
}

fn write_host_managed_package(root: &Path, id: &str, version: &str, require_group: bool) {
    fs::create_dir_all(root).unwrap();
    let required = if require_group {
        json!(["title", "group"])
    } else {
        json!(["title"])
    };
    let document = json!({
        "format_version": 1,
        "id": id,
        "version": version,
        "display_name": "Managed fixture",
        "description": "Managed fixture",
        "min_host_version": "0.0.1",
        "manifest": {},
        "backend": {"kind": "none"},
        "data": {
            "kind": "host-managed",
            "contract_version": 1,
            "collections": {
                "items": {
                    "schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": required,
                        "properties": {
                            "title": {"type": "string"},
                            "group": {"type": "string"}
                        }
                    },
                    "indexes": [],
                    "operations": ["get", "list", "create"],
                    "limits": {"records": 10, "record_bytes": 4096, "query_results": 10}
                }
            },
            "limits": {"total_bytes": 1048576, "transaction_operations": 8},
            "exports": []
        },
        "integrity": {"algorithm": "sha256", "assets": {}}
    });
    fs::write(
        root.join("app.json"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
}

fn extension_manifest(
    id: &str,
    points: Vec<ExtensionPointDeclaration>,
    contributions: Vec<ExtensionContribution>,
) -> AppManifest {
    AppManifest {
        app_id: AppId::new(id),
        version: "1.0.0".into(),
        display_name: id.into(),
        description: "Extension contract fixture".into(),
        capabilities: vec![],
        surfaces: vec![],
        agents: vec![],
        skills: vec![],
        assistant_profiles: vec![],
        automations: vec![],
        connectors: vec![],
        config_declarations: vec![],
        artifact_types: vec![],
        extension_points: points,
        extension_contributions: contributions,
        grant_requests: vec![],
        event_subscriptions: vec![],
    }
}

fn package_document_with_extension_version(version: Option<u32>) -> package::PackageDocument {
    let extension_points = version
        .map(|contract_version| {
            json!([{
                "name": "message-actions",
                "contract_version": contract_version,
                "context_schema": {"type": "object"}
            }])
        })
        .unwrap_or_else(|| json!([]));
    serde_json::from_value(json!({
        "format_version": 1,
        "id": "com.example.target",
        "version": "1.0.0",
        "display_name": "Target",
        "description": "Target fixture",
        "min_host_version": "0.0.1",
        "manifest": {"extension_points": extension_points},
        "backend": {"kind": "none"},
        "data": {"kind": "none"},
        "integrity": {"algorithm": "sha256", "assets": {}}
    }))
    .unwrap()
}

#[test]
fn extension_contribution_status_distinguishes_exact_missing_and_mismatched_targets() {
    let contribution = ExtensionContribution {
        target_app: AppId::new("com.example.target"),
        extension_point: ExtensionPointName::new("message-actions"),
        contract_version: 6,
        surface: SurfaceName::new("annotation"),
    };

    let missing =
        extension_contribution_views(std::slice::from_ref(&contribution), &BTreeMap::new());
    assert_eq!(
        missing[0].compatibility,
        AppExtensionCompatibility::TargetMissing
    );

    let point_missing = extension_contribution_views(
        std::slice::from_ref(&contribution),
        &BTreeMap::from([("com.example.target".into(), BTreeMap::new())]),
    );
    assert_eq!(
        point_missing[0].compatibility,
        AppExtensionCompatibility::PointMissing
    );

    let mismatch = extension_contribution_views(
        std::slice::from_ref(&contribution),
        &BTreeMap::from([(
            "com.example.target".into(),
            BTreeMap::from([("message-actions".into(), 7)]),
        )]),
    );
    assert_eq!(
        mismatch[0].compatibility,
        AppExtensionCompatibility::ContractMismatch
    );
    assert_eq!(mismatch[0].target_contract_version, Some(7));

    let exact = extension_contribution_views(
        &[contribution],
        &BTreeMap::from([(
            "com.example.target".into(),
            BTreeMap::from([("message-actions".into(), 6)]),
        )]),
    );
    assert_eq!(exact[0].compatibility, AppExtensionCompatibility::Exact);
}

#[test]
fn breaking_target_contract_warns_only_for_currently_compatible_contributions() {
    let root = workspace("extension-update-warning");
    let manager = AppManager::in_memory(root.clone());
    let current = package_document_with_extension_version(Some(6));
    let target = package_document_with_extension_version(Some(7));
    let compatible = extension_manifest(
        "com.example.compatible",
        vec![],
        vec![ExtensionContribution {
            target_app: AppId::new("com.example.target"),
            extension_point: ExtensionPointName::new("message-actions"),
            contract_version: 6,
            surface: SurfaceName::new("annotation"),
        }],
    );
    let already_dormant = extension_manifest(
        "com.example.old",
        vec![],
        vec![ExtensionContribution {
            target_app: AppId::new("com.example.target"),
            extension_point: ExtensionPointName::new("message-actions"),
            contract_version: 5,
            surface: SurfaceName::new("old-annotation"),
        }],
    );

    let diff = manager.diff_documents(Some(&current), &target, &[already_dormant, compatible]);

    assert_eq!(diff.extension_warnings.len(), 1);
    assert_eq!(
        diff.extension_warnings[0].contributor_app_id,
        "com.example.compatible"
    );
    assert_eq!(diff.extension_warnings[0].target_contract_version, Some(7));
    fs::remove_dir_all(root).unwrap();
}

const DATA_BACKEND: &str = r#"import { createInterface } from "node:readline";
const lines = createInterface({ input: process.stdin });
const send = (id, result) => process.stdout.write(`${JSON.stringify({jsonrpc:"2.0",id,result})}\n`);
lines.on("line", (line) => {
  const request = JSON.parse(line);
  if (request.method === "initialize") send(request.id, {protocolVersion:request.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:"data-test",version:"1.0.0"}});
  else if (request.method === "tools/list") send(request.id, {tools:[]});
  else if (request.id !== undefined) process.stdout.write(`${JSON.stringify({jsonrpc:"2.0",id:request.id,error:{code:-32601,message:"unknown method"}})}\n`);
});
"#;

fn write_versioned_package(
    root: &Path,
    id: &str,
    app_version: &str,
    data_format: u32,
    transitions: &[(u32, u32, bool)],
) {
    let migration = r#"import { createInterface } from "node:readline";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
const lines = createInterface({ input: process.stdin });
lines.on("line", (line) => {
  const request = JSON.parse(line);
  if (request.method !== "kestral/app-data/migrate") {
    process.stdout.write(`${JSON.stringify({jsonrpc:"2.0",id:request.id,error:{code:-32601,message:"unknown method"}})}\n`);
    return;
  }
  const path = join(process.env.APP_HOST_DATA_DIR, "record.json");
  const record = JSON.parse(readFileSync(path, "utf8"));
  record.version = request.params.to_format_version;
  writeFileSync(path, JSON.stringify(record));
  process.stdout.write(`${JSON.stringify({jsonrpc:"2.0",id:request.id,result:{protocol_version:1,format_version:record.version}})}\n`);
});
"#;
    fs::create_dir_all(root.join("backend")).unwrap();
    fs::write(root.join("backend/server.mjs"), DATA_BACKEND).unwrap();
    fs::write(root.join("backend/migrate.mjs"), migration).unwrap();
    let transitions: Vec<_> = transitions
        .iter()
        .map(|(from, to, destructive)| json!({"from": from, "to": to, "destructive": destructive}))
        .collect();
    let digest = |bytes: &[u8]| format!("sha256-{:x}", Sha256::digest(bytes));
    let document = json!({
        "format_version": 1,
        "id": id,
        "version": app_version,
        "display_name": "Versioned app",
        "description": "Versioned app-data fixture",
        "min_host_version": "0.0.1",
        "manifest": {},
        "backend": {
            "kind": "mcp-stdio",
            "authority_mode": "unsandboxed",
            "command": "node",
            "args": ["backend/server.mjs"]
        },
        "data": {
            "kind": "versioned",
            "format_version": data_format,
            "migration": {
                "protocol_version": 1,
                "command": "node",
                "entry": "backend/migrate.mjs",
                "args": ["backend/migrate.mjs"],
                "transitions": transitions
            }
        },
        "integrity": {
            "algorithm": "sha256",
            "assets": {
                "backend/server.mjs": digest(DATA_BACKEND.as_bytes()),
                "backend/migrate.mjs": digest(migration.as_bytes())
            }
        }
    });
    fs::write(
        root.join("app.json"),
        serde_json::to_vec_pretty(&document).unwrap(),
    )
    .unwrap();
}

fn open_manager(root: &Path) -> AppManager {
    AppManager::new(
        root.join("installed-apps.json"),
        root.join("trust-store.json"),
        root.join("apps"),
        root.join("update-journal.json"),
        false,
    )
    .unwrap()
}

fn kernel_and_invoker() -> (Arc<Mutex<Kernel>>, crate::agent_worker::KernelInvokerClient) {
    let kernel = Arc::new(Mutex::new(Kernel::new(Arc::new(TestChrome))));
    let invoker = crate::agent_worker::KernelInvokerClient::spawn(kernel.clone());
    (kernel, invoker)
}

fn activate_prepared(
    manager: &mut AppManager,
    kernel: &mut Kernel,
    surface_ui: &mut SurfaceUiRegistry,
    app_id: &str,
    prepared: PreparedActivation,
) {
    let prepared = manager
        .prepare_kernel_activation(kernel, app_id, prepared)
        .map_err(|failure| failure.reason)
        .unwrap();
    let continuation = manager
        .commit_kernel_activation(
            kernel,
            app_id,
            prepared.install.await_approval(),
            prepared.continuation,
        )
        .map_err(|failure| failure.reason)
        .unwrap();
    let approvals = manager
        .prepare_consumer_grants(kernel, continuation.consumer_grant_requests.clone())
        .unwrap();
    manager
        .finish_kernel_activation(
            kernel,
            surface_ui,
            app_id,
            continuation,
            PreparedGrant::await_grouped_approvals(approvals).unwrap(),
        )
        .map_err(|failure| failure.reason)
        .unwrap();
}

fn simple_grant(scope: serde_json::Value) -> serde_json::Value {
    json!({
        "scope": scope,
        "data_scope": {"kind": "none"},
        "condition": "requires-approval",
        "reason": "test",
        "duration": {"kind": "non-expiring"}
    })
}

fn install_revision(manager: &mut AppManager, package_root: &Path) -> InstallRecord {
    let inspection = manager.inspect(package_root).unwrap();
    manager
        .install_record(
            &inspection.staged_id,
            &inspection.package_digest,
            "2026-07-18T00:00:00Z",
        )
        .unwrap()
}

#[test]
fn off_lock_activation_is_revalidated_before_kernel_install() {
    let root = workspace("activation-revalidation");
    let package = package_dir(&root);
    write_package(
        &package,
        "com.example.activation",
        "1.0.0",
        "Activation",
        "Activation test",
        None,
        vec![],
    );
    let mut manager = open_manager(&root);
    let record = install_revision(&mut manager, &package);
    let (kernel_state, invoker) = kernel_and_invoker();
    let preparation = manager
        .activation_preparation_with_invoker(&record.id, invoker)
        .unwrap();
    let prepared = preparation.prepare().unwrap();

    manager.set_enabled_state(&record.id, false).unwrap();
    let kernel = kernel_state.lock().unwrap();
    let failure = manager
        .prepare_kernel_activation(&kernel, &record.id, prepared)
        .err()
        .unwrap();

    assert!(failure.reason.contains("no longer enabled"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn transition_plan_refuses_incompatible_host_managed_data_while_disabled() {
    let root = workspace("managed-data-update-preflight");
    let source = root.join("source-package");
    let target = root.join("target-package");
    let apps_root = root.join("apps");
    let app_id = "com.example.managed";
    write_host_managed_package(&source, app_id, "1.0.0", false);
    write_host_managed_package(&target, app_id, "2.0.0", true);
    let mut manager = AppManager::in_memory(apps_root.clone());
    install_revision(&mut manager, &source);
    let app = AppId::new(app_id);
    let source_document = package::read_document(&source).unwrap();
    crate::managed_data::ManagedDataStore::new(crate::managed_data::data_root(&apps_root))
        .request(
            &app,
            source_document.data.host_managed().unwrap(),
            crate::managed_data::ManagedDataRequest::Create {
                collection: "items".into(),
                value: json!({"title": "Existing"}).as_object().unwrap().clone(),
            },
        )
        .unwrap();
    manager.set_enabled_state(app_id, false).unwrap();

    let inspection = manager.inspect(&target).unwrap();
    let error = manager
        .plan_managed_app_transition(ManagedAppTransitionRequest {
            operation: ManagedAppOperation::Update,
            staged_id: Some(inspection.staged_id),
            package_digest: Some(inspection.package_digest),
            app_id: None,
            revision_id: None,
            acknowledge_downgrade: false,
            acknowledge_revert_data_caveat: false,
        })
        .unwrap_err();

    assert!(
        error.contains("target host-managed data contract is incompatible"),
        "{error}"
    );
    assert_eq!(manager.record(app_id).unwrap().revisions.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn managed_data_access_revalidates_installed_package_digest() {
    let root = workspace("managed-data-runtime-digest");
    let package = package_dir(&root);
    let app_id = "com.example.managed";
    write_host_managed_package(&package, app_id, "1.0.0", false);
    let mut manager = open_manager(&root);
    let record = install_revision(&mut manager, &package);
    let revision = manager.active_revision(&record).unwrap();
    let manifest_path = Path::new(&revision.payload_dir).join("app.json");
    let mut document: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    document["description"] = json!("Modified after install");
    fs::write(&manifest_path, serde_json::to_vec(&document).unwrap()).unwrap();

    let error = manager.active_host_managed_data(app_id).unwrap_err();

    assert!(error.contains("package digest mismatch"), "{error}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn journaled_update_migrates_a_candidate_and_retains_the_source_backup() {
    let node = std::process::Command::new("node")
        .arg("--version")
        .output()
        .expect("app-data update test requires node on PATH");
    assert!(node.status.success());
    let root = workspace("app-data-update");
    let source_package = root.join("source-package");
    let target_package = root.join("target-package");
    let apps_root = root.join("apps");
    let app_id = "com.example.versioned";
    write_versioned_package(&source_package, app_id, "1.0.0", 1, &[]);
    write_versioned_package(&target_package, app_id, "2.0.0", 2, &[(1, 2, false)]);
    let mut manager = AppManager::in_memory(apps_root.clone());
    install_revision(&mut manager, &source_package);
    let (kernel, invoker) = kernel_and_invoker();
    let mut kernel = kernel.lock().unwrap();
    let mut surface_ui = SurfaceUiRegistry::new();
    let prepared = manager
        .prepare_activation_with_invoker(app_id, invoker.clone())
        .unwrap();
    activate_prepared(&mut manager, &mut kernel, &mut surface_ui, app_id, prepared);
    let source_data = crate::app_data::current_revision(&apps_root, app_id)
        .unwrap()
        .unwrap();
    let source_dir = apps_root
        .join(".data")
        .join(app_id)
        .join("app-data-revisions")
        .join(&source_data.revision_id);
    fs::write(
        source_dir.join("record.json"),
        br#"{"version":1,"value":"kept"}"#,
    )
    .unwrap();

    let inspection = manager.inspect(&target_package).unwrap();
    let plan = manager
        .plan_managed_app_transition(ManagedAppTransitionRequest {
            operation: ManagedAppOperation::Update,
            staged_id: Some(inspection.staged_id),
            package_digest: Some(inspection.package_digest),
            app_id: None,
            revision_id: None,
            acknowledge_downgrade: false,
            acknowledge_revert_data_caveat: false,
        })
        .unwrap();
    assert_eq!(
        plan.data_transition.as_ref().map(|data| (
            data.source_format_version,
            data.target_format_version,
            data.destructive,
        )),
        Some((Some(1), 2, false))
    );
    let mut journal = manager.begin_journaled_transition(plan).unwrap().unwrap();
    let client = manager
        .deactivate_journaled_transition(&mut kernel, &mut surface_ui, &mut journal)
        .unwrap();
    if let Some(client) = client {
        client.shutdown();
    }
    let _ = manager
        .data_migration_preparation(&journal)
        .unwrap()
        .unwrap()
        .execute()
        .unwrap();
    // Re-running after a crash before the validated phase recreates the same
    // candidate from the untouched source and remains safe.
    let (source_digest, candidate_digest) = manager
        .data_migration_preparation(&journal)
        .unwrap()
        .unwrap()
        .execute()
        .unwrap();
    manager
        .mark_data_candidate_validated(&mut journal, source_digest, candidate_digest)
        .unwrap();
    fs::write(
        source_dir.join("record.json"),
        br#"{"version":1,"value":"changed-after-validation"}"#,
    )
    .unwrap();
    let error = manager.commit_data_candidate(&mut journal).unwrap_err();
    assert!(error.contains("changed after candidate validation"));
    fs::write(
        source_dir.join("record.json"),
        br#"{"version":1,"value":"kept"}"#,
    )
    .unwrap();
    let candidate_id = journal
        .data_transition
        .as_ref()
        .unwrap()
        .candidate
        .revision_id
        .clone();
    let candidate_record = apps_root
        .join(".data")
        .join(app_id)
        .join("app-data-revisions")
        .join(candidate_id)
        .join("record.json");
    fs::write(
        &candidate_record,
        br#"{"version":2,"value":"changed-after-validation"}"#,
    )
    .unwrap();
    let error = manager.commit_data_candidate(&mut journal).unwrap_err();
    assert!(error.contains("candidate changed after validation"));
    fs::write(&candidate_record, br#"{"version":2,"value":"kept"}"#).unwrap();
    manager.commit_data_candidate(&mut journal).unwrap();
    manager.commit_data_candidate(&mut journal).unwrap();
    let prepared = manager
        .transition_activation_preparation(&journal, &journal.target_revision.revision_id, invoker)
        .unwrap()
        .prepare()
        .unwrap();
    activate_prepared(&mut manager, &mut kernel, &mut surface_ui, app_id, prepared);
    manager
        .commit_journaled_transition(&mut journal, 1)
        .unwrap();

    let active = crate::app_data::current_revision(&apps_root, app_id)
        .unwrap()
        .unwrap();
    assert_eq!(active.format_version, 2);
    let active_record: serde_json::Value = serde_json::from_slice(
        &fs::read(
            apps_root
                .join(".data")
                .join(app_id)
                .join("app-data-revisions")
                .join(&active.revision_id)
                .join("record.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(active_record, json!({"version": 2, "value": "kept"}));
    assert!(source_dir.join("record.json").is_file());

    let old = manager.inspect(&source_package).unwrap();
    let error = manager
        .plan_managed_app_transition(ManagedAppTransitionRequest {
            operation: ManagedAppOperation::Downgrade,
            staged_id: Some(old.staged_id),
            package_digest: Some(old.package_digest),
            app_id: None,
            revision_id: None,
            acknowledge_downgrade: true,
            acknowledge_revert_data_caveat: false,
        })
        .unwrap_err();
    assert!(error.contains("no tested migration from active format 2"));
}

#[test]
fn phased_journal_transition_commits_target_revision() {
    let root = workspace("phased-transition");
    let current = package_dir(&root).join("current");
    let target = package_dir(&root).join("target");
    write_package(
        &current,
        "com.example.phased",
        "1.0.0",
        "Phased",
        "Current",
        None,
        vec![],
    );
    write_package(
        &target,
        "com.example.phased",
        "1.1.0",
        "Phased",
        "Target",
        None,
        vec![],
    );
    let mut manager = open_manager(&root);
    let record = install_revision(&mut manager, &current);
    let (kernel_state, invoker) = kernel_and_invoker();
    let mut surface_ui = SurfaceUiRegistry::new();
    let current_prepared = manager
        .activation_preparation_with_invoker(&record.id, invoker.clone())
        .unwrap()
        .prepare()
        .unwrap();
    let mut kernel = kernel_state.lock().unwrap();
    activate_prepared(
        &mut manager,
        &mut kernel,
        &mut surface_ui,
        &record.id,
        current_prepared,
    );

    let inspection = manager.inspect(&target).unwrap();
    let plan = manager
        .plan_managed_app_transition(ManagedAppTransitionRequest {
            operation: ManagedAppOperation::Update,
            staged_id: Some(inspection.staged_id),
            package_digest: Some(inspection.package_digest),
            app_id: None,
            revision_id: None,
            acknowledge_downgrade: false,
            acknowledge_revert_data_caveat: false,
        })
        .unwrap();
    let mut journal = manager.begin_journaled_transition(plan).unwrap().unwrap();
    let client = manager
        .deactivate_journaled_transition(&mut kernel, &mut surface_ui, &mut journal)
        .unwrap();
    assert!(client.is_none());
    drop(kernel);

    let prepared = manager
        .transition_activation_preparation(&journal, &journal.target_revision.revision_id, invoker)
        .unwrap()
        .prepare()
        .unwrap();
    let mut kernel = kernel_state.lock().unwrap();
    activate_prepared(
        &mut manager,
        &mut kernel,
        &mut surface_ui,
        &journal.app_id,
        prepared,
    );
    manager
        .commit_journaled_transition(&mut journal, 1)
        .unwrap();

    assert_eq!(
        manager
            .record("com.example.phased")
            .unwrap()
            .active_revision_id,
        journal.target_revision.revision_id
    );
    assert_eq!(
        kernel
            .installed_app(&AppId::new("com.example.phased"))
            .unwrap()
            .manifest
            .version,
        "1.1.0"
    );
    assert!(!root.join("update-journal.json").exists());
    drop(kernel);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn transition_application_uses_the_server_owned_plan() {
    let root = workspace("server-owned-plan");
    let current = package_dir(&root).join("current");
    let target = package_dir(&root).join("target");
    write_package(
        &current,
        "com.example.owned-plan",
        "1.0.0",
        "Owned plan",
        "Current",
        None,
        vec![],
    );
    write_package(
        &target,
        "com.example.owned-plan",
        "1.1.0",
        "Owned plan",
        "Target",
        None,
        vec![],
    );
    let mut manager = open_manager(&root);
    install_revision(&mut manager, &current);
    let inspection = manager.inspect(&target).unwrap();
    let mut presented = manager
        .plan_managed_app_transition(ManagedAppTransitionRequest {
            operation: ManagedAppOperation::Update,
            staged_id: Some(inspection.staged_id),
            package_digest: Some(inspection.package_digest),
            app_id: None,
            revision_id: None,
            acknowledge_downgrade: false,
            acknowledge_revert_data_caveat: false,
        })
        .unwrap();
    let transition_id = presented.transition_id.clone();
    presented.target_revision_id = "../outside".into();
    presented.app_id = "com.example.other".into();

    let trusted = manager
        .take_managed_app_transition_plan(&transition_id)
        .unwrap();

    assert_eq!(trusted.app_id, "com.example.owned-plan");
    assert!(Uuid::parse_str(&trusted.target_revision_id).is_ok());
    assert!(manager
        .take_managed_app_transition_plan(&transition_id)
        .is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn transition_persistence_failure_removes_the_copied_revision() {
    let root = workspace("transition-persist-rollback");
    let current = package_dir(&root).join("current");
    let target = package_dir(&root).join("target");
    write_package(
        &current,
        "com.example.persist-rollback",
        "1.0.0",
        "Rollback",
        "Current",
        None,
        vec![],
    );
    write_package(
        &target,
        "com.example.persist-rollback",
        "1.1.0",
        "Rollback",
        "Target",
        None,
        vec![],
    );
    let mut manager = open_manager(&root);
    install_revision(&mut manager, &current);
    let inspection = manager.inspect(&target).unwrap();
    let plan = manager
        .plan_managed_app_transition(ManagedAppTransitionRequest {
            operation: ManagedAppOperation::Update,
            staged_id: Some(inspection.staged_id),
            package_digest: Some(inspection.package_digest),
            app_id: None,
            revision_id: None,
            acknowledge_downgrade: false,
            acknowledge_revert_data_caveat: false,
        })
        .unwrap();
    let revisions = root
        .join("apps")
        .join("com.example.persist-rollback")
        .join("revisions");
    let before = fs::read_dir(&revisions).unwrap().count();
    fs::remove_file(root.join("installed-apps.json")).unwrap();
    fs::create_dir(root.join("installed-apps.json")).unwrap();

    let error = manager.begin_journaled_transition(plan).unwrap_err();

    assert!(error.contains("replace installed apps"), "{error}");
    assert_eq!(
        manager
            .record("com.example.persist-rollback")
            .unwrap()
            .revisions
            .len(),
        1
    );
    assert_eq!(fs::read_dir(revisions).unwrap().count(), before);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn store_requires_revision_fields_and_rejects_old_version() {
    let root = workspace("store-shape");
    let record = json!({
        "id": "com.example.strict",
        "enabled": true,
        "uninstalling": false,
        "lifecycle_generation": 0,
        "purge_secrets": false,
        "purge_data": false,
        "purge_secret_names": [],
        "active_revision_id": "revision-1",
        "revisions": [],
    });
    fs::write(
        root.join("installed-apps.json"),
        serde_json::to_vec(&json!({"version": 4, "apps": [record]})).unwrap(),
    )
    .unwrap();
    assert!(AppManager::new(
        root.join("installed-apps.json"),
        root.join("trust-store.json"),
        root.join("apps"),
        root.join("update-journal.json"),
        false,
    )
    .is_ok());

    fs::write(
        root.join("installed-apps.json"),
        serde_json::to_vec(&json!({"version": 3, "apps": []})).unwrap(),
    )
    .unwrap();
    let error = AppManager::new(
        root.join("installed-apps.json"),
        root.join("trust-store.json"),
        root.join("apps"),
        root.join("update-journal.json"),
        false,
    )
    .err()
    .unwrap();
    assert!(error.contains("unsupported installed-apps store version"));
}

#[test]
fn same_version_same_app_different_digest_is_a_conflict() {
    let root = workspace("same-version-conflict");
    let current = package_dir(&root).join("current");
    let target = package_dir(&root).join("target");
    write_package(
        &current,
        "com.example.conflict",
        "1.0.0",
        "Conflict",
        "current",
        None,
        vec![],
    );
    write_package(
        &target,
        "com.example.conflict",
        "1.0.0",
        "Conflict",
        "target",
        None,
        vec![],
    );

    let mut manager = open_manager(&root);
    let _installed = install_revision(&mut manager, &current);
    let inspection = manager.inspect(&target).unwrap();
    let error = manager
        .plan_managed_app_transition(ManagedAppTransitionRequest {
            operation: ManagedAppOperation::Update,
            staged_id: Some(inspection.staged_id.clone()),
            package_digest: Some(inspection.package_digest.clone()),
            app_id: None,
            revision_id: None,
            acknowledge_downgrade: false,
            acknowledge_revert_data_caveat: false,
        })
        .unwrap_err();
    assert!(error.contains("version conflict"));
}

#[test]
fn downgrade_requires_explicit_acknowledgement() {
    let root = workspace("downgrade-intent");
    let current = package_dir(&root).join("current");
    let target = package_dir(&root).join("target");
    write_package(
        &current,
        "com.example.down",
        "2.0.0",
        "Down",
        "current",
        None,
        vec![],
    );
    write_package(
        &target,
        "com.example.down",
        "1.5.0",
        "Down",
        "target",
        None,
        vec![],
    );

    let mut manager = open_manager(&root);
    let _installed = install_revision(&mut manager, &current);
    let inspection = manager.inspect(&target).unwrap();

    let error = manager
        .plan_managed_app_transition(ManagedAppTransitionRequest {
            operation: ManagedAppOperation::Downgrade,
            staged_id: Some(inspection.staged_id.clone()),
            package_digest: Some(inspection.package_digest.clone()),
            app_id: None,
            revision_id: None,
            acknowledge_downgrade: false,
            acknowledge_revert_data_caveat: false,
        })
        .unwrap_err();
    assert!(error.contains("explicit acknowledgement"));

    let plan = manager
        .plan_managed_app_transition(ManagedAppTransitionRequest {
            operation: ManagedAppOperation::Downgrade,
            staged_id: Some(inspection.staged_id.clone()),
            package_digest: Some(inspection.package_digest.clone()),
            app_id: None,
            revision_id: None,
            acknowledge_downgrade: true,
            acknowledge_revert_data_caveat: false,
        })
        .unwrap();
    assert_eq!(plan.operation, ManagedAppOperation::Downgrade);
    assert_eq!(plan.diff.version_relation, ManagedAppVersionRelation::Lower);
}

#[test]
fn permission_and_publisher_diffs_are_typed() {
    let root = workspace("diffs");
    let current = package_dir(&root).join("current");
    let target = package_dir(&root).join("target");
    write_package(
        &current,
        "com.example.diff",
        "1.0.0",
        "Diff",
        "current",
        Some("ed25519:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        vec![simple_grant(
            json!({"kind": "exact-capability", "provider": "notes", "capability": "read"}),
        )],
    );
    write_package(
        &target,
        "com.example.diff",
        "1.1.0",
        "Diff",
        "target",
        Some("ed25519:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        vec![
            simple_grant(
                json!({"kind": "exact-capability", "provider": "notes", "capability": "read"}),
            ),
            simple_grant(
                json!({"kind": "exact-capability", "provider": "notes", "capability": "write"}),
            ),
        ],
    );

    let mut manager = open_manager(&root);
    let _installed = install_revision(&mut manager, &current);
    let inspection = manager.inspect(&target).unwrap();
    let plan = manager
        .plan_managed_app_transition(ManagedAppTransitionRequest {
            operation: ManagedAppOperation::Update,
            staged_id: Some(inspection.staged_id.clone()),
            package_digest: Some(inspection.package_digest.clone()),
            app_id: None,
            revision_id: None,
            acknowledge_downgrade: false,
            acknowledge_revert_data_caveat: false,
        })
        .unwrap();

    assert_eq!(
        plan.diff.publisher_key_continuity,
        ManagedAppPublisherContinuity::Changed
    );
    assert_eq!(plan.diff.permissions.added.len(), 1);
    assert_eq!(plan.diff.permissions.unchanged.len(), 1);
    assert_eq!(plan.diff.capabilities_added.len(), 0);
}

#[test]
fn retain_recent_revisions_keeps_active_and_newest_two() {
    let mut revisions = vec![
        AppRevision {
            revision_id: "r1".into(),
            version: "1.0.0".into(),
            display_name: "Example".into(),
            description: "old".into(),
            backend_kind: "none".into(),
            publisher: None,
            signature_verdict: "unsigned".into(),
            signature_key_id: None,
            min_host_version: "0.0.1".into(),
            installed_at: "2026-07-18T00:00:00Z".into(),
            payload_dir: "apps/com.example/revisions/r1".into(),
            package_digest: "sha256-1".into(),
        },
        AppRevision {
            revision_id: "r2".into(),
            version: "1.1.0".into(),
            display_name: "Example".into(),
            description: "middle".into(),
            backend_kind: "none".into(),
            publisher: None,
            signature_verdict: "unsigned".into(),
            signature_key_id: None,
            min_host_version: "0.0.1".into(),
            installed_at: "2026-07-18T00:00:01Z".into(),
            payload_dir: "apps/com.example/revisions/r2".into(),
            package_digest: "sha256-2".into(),
        },
        AppRevision {
            revision_id: "r3".into(),
            version: "1.2.0".into(),
            display_name: "Example".into(),
            description: "new".into(),
            backend_kind: "none".into(),
            publisher: None,
            signature_verdict: "unsigned".into(),
            signature_key_id: None,
            min_host_version: "0.0.1".into(),
            installed_at: "2026-07-18T00:00:02Z".into(),
            payload_dir: "apps/com.example/revisions/r3".into(),
            package_digest: "sha256-3".into(),
        },
    ];

    AppManager::retain_recent_revisions(&mut revisions, "r2");

    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0].revision_id, "r2");
    assert_eq!(revisions[1].revision_id, "r3");
}

#[test]
fn prepared_journal_recovery_completes_a_transition() {
    let root = workspace("journal-recovery");
    let current = package_dir(&root).join("current");
    let target = package_dir(&root).join("target");
    write_package(
        &current,
        "com.example.recover",
        "1.0.0",
        "Recover",
        "current",
        None,
        vec![],
    );
    write_package(
        &target,
        "com.example.recover",
        "1.1.0",
        "Recover",
        "target",
        None,
        vec![],
    );

    let mut manager = open_manager(&root);
    let _installed = install_revision(&mut manager, &current);
    let target_inspection = manager.inspect(&target).unwrap();
    let staged_dir =
        package::staged_dir(manager.staging_root(), &target_inspection.staged_id).unwrap();
    let app_root = manager
        .apps_root
        .join("com.example.recover")
        .join("revisions");
    fs::create_dir_all(&app_root).unwrap();
    let revision_id = "revision-target".to_string();
    let payload_dir = app_root.join(&revision_id);
    package::copy_verified_package(&staged_dir, &payload_dir, &target_inspection.package_digest)
        .unwrap();

    let mut stored = manager.record("com.example.recover").unwrap().clone();
    let target_revision = AppRevision {
        revision_id: revision_id.clone(),
        version: "1.1.0".into(),
        display_name: "Recover".into(),
        description: "target".into(),
        backend_kind: "none".into(),
        publisher: None,
        signature_verdict: target_inspection.signature.label().to_string(),
        signature_key_id: target_inspection
            .signature
            .key_id()
            .map(|value| value.to_string()),
        min_host_version: "0.0.1".into(),
        installed_at: "2026-07-18T00:00:03Z".into(),
        payload_dir: payload_dir.to_string_lossy().to_string(),
        package_digest: target_inspection.package_digest.clone(),
    };
    stored.revisions.push(target_revision.clone());
    fs::write(
        root.join("installed-apps.json"),
        serde_json::to_vec(&json!({
            "version": 4,
            "apps": [stored],
        }))
        .unwrap(),
    )
    .unwrap();
    let journal = UpdateJournal::new(
        "transition-recover".into(),
        "com.example.recover".into(),
        ManagedAppOperation::Update,
        Some("revision-current".into()),
        target_revision,
        vec![],
        true,
    );
    let journal = UpdateJournal {
        phase: UpdatePhase::Prepared,
        ..journal
    };
    fs::write(
        root.join("update-journal.json"),
        serde_json::to_vec(&journal).unwrap(),
    )
    .unwrap();

    let mut reloaded = open_manager(&root);
    let (kernel, invoker) = kernel_and_invoker();
    let mut surface_ui = crate::surface_ui::SurfaceUiRegistry::new();
    {
        let mut kernel = kernel.lock().unwrap();
        reloaded
            .recover_managed_app_transition(&mut kernel, &mut surface_ui, invoker)
            .unwrap();
    }

    assert!(!root.join("update-journal.json").exists());
    let recovered = reloaded.record("com.example.recover").unwrap();
    assert_eq!(recovered.active_revision_id, "revision-target");
    assert_eq!(recovered.revisions.len(), 2);
}

#[test]
fn loading_an_unsupported_update_journal_version_fails_fast() {
    let root = workspace("unsupported-journal-version");
    let journal = UpdateJournal::new(
        "transition-invalid-version".into(),
        "com.example.invalid".into(),
        ManagedAppOperation::Update,
        None,
        AppRevision {
            revision_id: "revision-target".into(),
            version: "1.0.0".into(),
            display_name: "Invalid".into(),
            description: "Invalid".into(),
            backend_kind: "none".into(),
            publisher: None,
            signature_verdict: "unsigned".into(),
            signature_key_id: None,
            min_host_version: "0.0.1".into(),
            installed_at: "2026-07-18T00:00:00Z".into(),
            payload_dir: "apps/com.example.invalid/revisions/revision-target".into(),
            package_digest: "sha256-test".into(),
        },
        Vec::new(),
        true,
    );
    let mut document = serde_json::to_value(journal).unwrap();
    document["version"] = json!(3);
    fs::write(
        root.join("update-journal.json"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();

    let error = match AppManager::new(
        root.join("installed-apps.json"),
        root.join("trust-store.json"),
        root.join("apps"),
        root.join("update-journal.json"),
        false,
    ) {
        Ok(_) => panic!("unsupported update journal version must be refused"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        "unsupported app update journal version 3; expected 2"
    );
}
