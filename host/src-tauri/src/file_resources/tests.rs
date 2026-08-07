use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use app_host_kernel::clock::FixedClock;
use app_host_kernel::ids::{AppId, CapabilityName, ResourceId};
use app_host_kernel::invocation::InvocationRequest;
use app_host_kernel::kernel::{AuthorizeInvocation, Kernel, PrepareInvocation};
use app_host_kernel::manifest::seal;
use app_host_kernel::primitives::grant::{DataScope, GrantOrigin, GrantScope, GrantStatus};
use app_host_kernel::primitives::run::Initiator;
use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, TrustedChrome,
};
use base64::Engine;
use chrono::{TimeZone, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::atomic_json::{AtomicFileWriter, FailingAtomicFileWriter, FailingFileOperation};

use super::{
    file_broker_app_id, file_broker_handlers, file_broker_manifest, file_resource_grant_request,
    file_resource_registry_path, FileResourceGrantOperation, FileResourceRegistryService,
};

#[test]
fn bundled_manifest_identity_is_pinned_for_durable_installs() {
    assert_eq!(
        seal(file_broker_manifest()).content_hash,
        "e1b9eb4163898ef0ff66a7c00b77e9e3f3de0bd6f29bf54a13b4b5e82c1f19f8"
    );
}

struct AllowChrome;

impl TrustedChrome for AllowChrome {
    fn confirm_grant(&self, _prompt: GrantIssuancePrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn approve_capability(&self, _prompt: CapabilityApprovalPrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn confirm_event_subscriptions(&self, _prompt: EventSubscriptionPrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn show_notice(&self, _notice: ChromeNotice) -> Result<(), ChromeNoticeError> {
        Ok(())
    }
}

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("host-file-resources-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct NthRenameFailWriter {
    fail_on: usize,
    renames: Mutex<usize>,
}

impl NthRenameFailWriter {
    fn new(fail_on: usize) -> Self {
        Self {
            fail_on,
            renames: Mutex::new(0),
        }
    }
}

impl AtomicFileWriter for NthRenameFailWriter {
    fn write_and_sync(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = fs::File::create(path)?;
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all()
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mut renames = self.renames.lock().unwrap();
        *renames += 1;
        if *renames == self.fail_on {
            return Err(io::Error::other("injected rename failure"));
        }
        fs::rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
}

fn temp_dir(label: &str) -> TempDirGuard {
    TempDirGuard::new(label)
}

fn registry_service(root: &Path) -> FileResourceRegistryService {
    FileResourceRegistryService::new(file_resource_registry_path(root)).unwrap()
}

fn registry_service_with_writer(
    root: &Path,
    writer: Arc<dyn AtomicFileWriter>,
) -> FileResourceRegistryService {
    FileResourceRegistryService::with_writer(file_resource_registry_path(root), writer).unwrap()
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn new_kernel() -> Kernel {
    let chrome = Arc::new(AllowChrome);
    let clock = FixedClock::new(Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap());
    Kernel::with_clock(chrome, clock)
}

fn install_app(kernel: &mut Kernel, app_id: &str) {
    let prepared = kernel
        .prepare_install_with_grant_origin(
            seal(app_manifest(app_id)),
            BTreeMap::new(),
            GrantOrigin::SystemBundled,
        )
        .unwrap();
    kernel.commit_install(prepared.await_approval()).unwrap();
}

fn install_file_broker(kernel: &mut Kernel, registry: Arc<Mutex<FileResourceRegistryService>>) {
    let prepared = kernel
        .prepare_install_with_grant_origin(
            seal(file_broker_manifest()),
            file_broker_handlers(registry),
            GrantOrigin::SystemBundled,
        )
        .unwrap();
    kernel.commit_install(prepared.await_approval()).unwrap();
}

fn app_manifest(app_id: &str) -> app_host_kernel::manifest::AppManifest {
    app_host_kernel::manifest::AppManifest {
        app_id: AppId::new(app_id),
        version: "0.1.0".into(),
        display_name: app_id.into(),
        description: "test holder".into(),
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
        grant_requests: vec![],
        event_subscriptions: vec![],
    }
}

fn grant_read(kernel: &mut Kernel, holder: &AppId, resource_id: ResourceId) {
    let request = file_resource_grant_request(
        holder.clone(),
        resource_id,
        FileResourceGrantOperation::Read,
    );
    let result = kernel.issue_grant(holder, &request).unwrap();
    assert!(matches!(
        result,
        app_host_kernel::services::broker::IssueResult::Issued(_)
    ));
}

#[test]
fn file_broker_invocation_scope_comes_from_requested_resource() {
    let capability = app_host_kernel::primitives::capability::CapabilityRef {
        provider: file_broker_app_id(),
        capability: CapabilityName::new("file.read"),
    };
    let input = serde_json::from_value(json!({"resource_id": "resource-1"})).unwrap();

    assert_eq!(
        super::invocation_data_scope(&capability, &input),
        DataScope::Resources {
            resource_ids: vec![ResourceId::new("resource-1")],
        }
    );
}

#[test]
fn non_file_broker_invocations_keep_unscoped_authority() {
    let capability = app_host_kernel::primitives::capability::CapabilityRef {
        provider: AppId::new("notes"),
        capability: CapabilityName::new("read"),
    };
    let input = serde_json::from_value(json!({"resource_id": "resource-1"})).unwrap();

    assert_eq!(
        super::invocation_data_scope(&capability, &input),
        DataScope::None
    );
}

fn invoke_file_read_with_data_scope(
    kernel: &mut Kernel,
    holder: &AppId,
    granted_resource: ResourceId,
    requested_resource: ResourceId,
) -> app_host_kernel::invocation::InvocationResult {
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: holder.clone(),
                reason: "test".into(),
            },
            "read resource",
        )
        .unwrap();
    let capability = app_host_kernel::primitives::capability::CapabilityRef {
        provider: file_broker_app_id(),
        capability: CapabilityName::new("file.read"),
    };
    let request = InvocationRequest {
        input: serde_json::from_value(json!({"resource_id": requested_resource})).unwrap(),
        data_scope: DataScope::resources(vec![granted_resource]).unwrap(),
    };
    let prepared = match kernel
        .prepare_invocation(&run_id, &capability, request)
        .unwrap()
    {
        PrepareInvocation::Prepared(prepared) => prepared,
        PrepareInvocation::Refused(result) => panic!("unexpected refusal: {result:?}"),
    };
    let authorized = match kernel
        .authorize_invocation(prepared.await_approval())
        .unwrap()
    {
        AuthorizeInvocation::Authorized(authorized) => authorized,
        AuthorizeInvocation::Refused(result) => panic!("unexpected refusal: {result:?}"),
    };
    kernel.finalize_invocation(authorized.execute()).unwrap()
}

#[test]
fn file_broker_registry_restart_survives_reload() {
    let temp = temp_dir("restart");
    let file_path = temp.path().join("note.txt");
    write_file(&file_path, "hello");

    let resource_id = {
        let mut service = registry_service(temp.path());
        let view = service.register_resource(&file_path).unwrap();
        view.resource.resource_id
    };

    let service = registry_service(temp.path());
    let resources = service.list_resources();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].resource_id, resource_id);
    assert_eq!(resources[0].status, super::FileResourceStatus::Active);
}

#[test]
fn file_broker_registry_load_rejects_corruption() {
    let temp = temp_dir("corruption");
    fs::write(file_resource_registry_path(temp.path()), b"{").unwrap();

    let error = match FileResourceRegistryService::new(file_resource_registry_path(temp.path())) {
        Ok(_) => panic!("corrupt registry must fail to load"),
        Err(error) => error,
    };
    assert!(
        error.contains("parse file resource registry failed"),
        "{error}"
    );
}

#[test]
fn file_broker_registration_failure_rolls_back_memory_and_disk() {
    let temp = temp_dir("register-failure");
    let file_path = temp.path().join("note.txt");
    write_file(&file_path, "hello");
    let _initial = registry_service(temp.path());
    let before = fs::read(file_resource_registry_path(temp.path())).unwrap();

    let writer = Arc::new(FailingAtomicFileWriter::new(FailingFileOperation::Rename));
    let mut service = registry_service_with_writer(temp.path(), writer);

    let error = service.register_resource(&file_path).unwrap_err();
    assert!(
        error.contains("injected rename failure") || error.contains("replace file"),
        "{error}"
    );
    assert!(service.list_resources().is_empty());
    assert_eq!(
        fs::read(file_resource_registry_path(temp.path())).unwrap(),
        before
    );
}

#[test]
fn file_broker_registration_indeterminate_keeps_committed_candidate() {
    let temp = temp_dir("register-indeterminate");
    let file_path = temp.path().join("note.txt");
    write_file(&file_path, "hello");
    let _initial = registry_service(temp.path());

    let writer = Arc::new(FailingAtomicFileWriter::new(
        FailingFileOperation::SyncParent,
    ));
    let mut service = registry_service_with_writer(temp.path(), writer);

    let error = service.register_resource(&file_path).unwrap_err();

    assert!(error.contains("candidate was committed"), "{error}");
    assert_eq!(service.list_resources().len(), 1);
    let raw = fs::read_to_string(file_resource_registry_path(temp.path())).unwrap();
    assert!(raw.contains("note.txt"), "{raw}");
}

#[test]
fn file_broker_registration_rejects_symlink_root() {
    let temp = temp_dir("symlink-root");
    let target = temp.path().join("target.txt");
    write_file(&target, "hello");
    let link = temp.path().join("link.txt");
    if create_symlink_file(&target, &link).is_err() {
        return;
    }

    let mut service = registry_service(temp.path());
    let error = service.register_resource(&link).unwrap_err();
    assert!(error.contains("must not be a symlink"), "{error}");
    assert!(service.list_resources().is_empty());
}

#[test]
fn file_broker_file_and_directory_semantics_are_distinct() {
    let temp = temp_dir("semantics");
    let file_root = temp.path().join("root.txt");
    write_file(&file_root, "root");
    let dir_root = temp.path().join("folder");
    fs::create_dir_all(&dir_root).unwrap();
    write_file(&dir_root.join("child.txt"), "child");

    let mut service = registry_service(temp.path());
    let file_id = service
        .register_resource(&file_root)
        .unwrap()
        .resource
        .resource_id;
    let dir_id = service
        .register_resource(&dir_root)
        .unwrap()
        .resource
        .resource_id;

    let file_entries = service.list_entries(&file_id, None).unwrap();
    assert_eq!(file_entries.entries.len(), 1);
    assert_eq!(file_entries.entries[0].kind, super::FileEntryKind::File);
    assert!(service.list_entries(&file_id, Some("child.txt")).is_err());
    assert!(service
        .read_file(&file_id, Some("child.txt"), None)
        .is_err());

    let dir_entries = service.list_entries(&dir_id, None).unwrap();
    assert_eq!(dir_entries.entries.len(), 1);
    assert_eq!(dir_entries.entries[0].display_name, "child.txt");
    let read = service.read_file(&dir_id, Some("child.txt"), None).unwrap();
    assert_eq!(
        read.content_base64,
        base64::engine::general_purpose::STANDARD.encode("child")
    );
}

#[test]
fn file_broker_write_replaces_existing_file_on_windows_safe_path() {
    let temp = temp_dir("replace");
    let file_root = temp.path().join("root.txt");
    write_file(&file_root, "old");
    let mut service = registry_service(temp.path());
    let resource_id = service
        .register_resource(&file_root)
        .unwrap()
        .resource
        .resource_id;

    let view = service
        .write_file(
            &resource_id,
            None,
            &base64::engine::general_purpose::STANDARD.encode("new"),
            None,
        )
        .unwrap();

    assert!(view.replaced);
    assert_eq!(fs::read_to_string(&file_root).unwrap(), "new");
}

#[test]
fn file_broker_write_conflict_guard_rejects_wrong_sha() {
    let temp = temp_dir("write-conflict");
    let file_root = temp.path().join("root.txt");
    write_file(&file_root, "old");
    let mut service = registry_service(temp.path());
    let resource_id = service
        .register_resource(&file_root)
        .unwrap()
        .resource
        .resource_id;

    let error = service
        .write_file(
            &resource_id,
            None,
            &base64::engine::general_purpose::STANDARD.encode("new"),
            Some("deadbeef"),
        )
        .unwrap_err();

    assert!(error.contains("SHA-256 conflict"), "{error}");
    assert_eq!(fs::read_to_string(&file_root).unwrap(), "old");
}

#[test]
fn file_broker_delete_conflict_guard_rejects_wrong_sha() {
    let temp = temp_dir("delete-conflict");
    let file_root = temp.path().join("root.txt");
    write_file(&file_root, "old");
    let mut service = registry_service(temp.path());
    let resource_id = service
        .register_resource(&file_root)
        .unwrap()
        .resource
        .resource_id;

    let error = service
        .delete_file(&resource_id, None, Some("deadbeef"))
        .unwrap_err();

    assert!(error.contains("SHA-256 conflict"), "{error}");
    assert!(file_root.exists());
}

#[test]
fn file_broker_bounded_read_truncates_large_file() {
    let temp = temp_dir("bounded-read");
    let file_root = temp.path().join("root.txt");
    write_file(&file_root, &"abc".repeat(100));
    let mut service = registry_service(temp.path());
    let resource_id = service
        .register_resource(&file_root)
        .unwrap()
        .resource
        .resource_id;

    let read = service.read_file(&resource_id, None, Some(16)).unwrap();
    assert_eq!(read.bytes_read, 16);
    assert!(read.truncated);
    assert_eq!(read.total_bytes, 300);
    let expected = "abc".repeat(100);
    assert_eq!(
        read.sha256,
        format!("{:x}", Sha256::digest(&expected.as_bytes()[..16]))
    );
}

#[test]
fn file_broker_rejects_oversized_writes_before_decoding() {
    let temp = temp_dir("bounded-write");
    let dir_root = temp.path().join("folder");
    fs::create_dir_all(&dir_root).unwrap();
    let mut service = registry_service(temp.path());
    let resource_id = service
        .register_resource(&dir_root)
        .unwrap()
        .resource
        .resource_id;
    let oversized =
        base64::engine::general_purpose::STANDARD.encode(vec![0_u8; super::MAX_WRITE_BYTES + 1]);

    let error = service
        .write_file(&resource_id, Some("large.bin"), &oversized, None)
        .unwrap_err();

    assert!(error.contains("write limit"), "{error}");
    assert!(!dir_root.join("large.bin").exists());
}

#[test]
fn file_broker_rejects_unbounded_directory_listings() {
    let temp = temp_dir("bounded-list");
    let dir_root = temp.path().join("folder");
    fs::create_dir_all(&dir_root).unwrap();
    for index in 0..=super::MAX_DIRECTORY_ENTRIES {
        write_file(&dir_root.join(format!("entry-{index}.txt")), "x");
    }
    let mut service = registry_service(temp.path());
    let resource_id = service
        .register_resource(&dir_root)
        .unwrap()
        .resource
        .resource_id;

    let error = service.list_entries(&resource_id, None).unwrap_err();

    assert!(error.contains("more than"), "{error}");
}

#[test]
fn file_broker_stale_target_is_rejected_after_move() {
    let temp = temp_dir("stale-target");
    let file_root = temp.path().join("root.txt");
    let moved = temp.path().join("moved.txt");
    write_file(&file_root, "old");
    let mut service = registry_service(temp.path());
    let resource_id = service
        .register_resource(&file_root)
        .unwrap()
        .resource
        .resource_id;
    fs::rename(&file_root, &moved).unwrap();

    let error = service.read_file(&resource_id, None, None).unwrap_err();
    assert!(
        error.contains("canonicalize")
            || error.contains("open file")
            || error.contains("inspect file"),
        "{error}"
    );
}

#[test]
fn file_broker_traversal_and_absolute_paths_are_rejected() {
    let temp = temp_dir("traversal");
    let dir_root = temp.path().join("folder");
    fs::create_dir_all(&dir_root).unwrap();
    write_file(&dir_root.join("child.txt"), "child");
    let mut service = registry_service(temp.path());
    let resource_id = service
        .register_resource(&dir_root)
        .unwrap()
        .resource
        .resource_id;

    assert!(service
        .list_entries(&resource_id, Some("../escape"))
        .is_err());
    assert!(service
        .read_file(&resource_id, Some("../escape"), None)
        .is_err());
    assert!(service
        .write_file(&resource_id, Some("../escape"), "Y2hpbGQ=", None)
        .is_err());
    assert!(service
        .delete_file(&resource_id, Some("../escape"), None)
        .is_err());

    let absolute = if cfg!(windows) {
        "C:\\escape.txt"
    } else {
        "/escape.txt"
    };
    assert!(service.list_entries(&resource_id, Some(absolute)).is_err());
}

#[test]
fn file_broker_symlink_escape_is_rejected_when_supported() {
    let temp = temp_dir("symlink-escape");
    let root = temp.path().join("folder");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    write_file(&outside.join("secret.txt"), "secret");
    let link = root.join("escape.txt");
    if create_symlink_file(&outside.join("secret.txt"), &link).is_err() {
        return;
    }
    let mut service = registry_service(temp.path());
    let resource_id = service
        .register_resource(&root)
        .unwrap()
        .resource
        .resource_id;

    let error = service
        .read_file(&resource_id, Some("escape.txt"), None)
        .unwrap_err();
    assert!(
        error.contains("escapes resource root") || error.contains("canonicalize"),
        "{error}"
    );
}

#[test]
fn file_broker_wrong_authorized_data_scope_fails_in_handler() {
    let temp = temp_dir("scope-failure");
    let registry = Arc::new(Mutex::new(registry_service(temp.path())));
    let file_a = temp.path().join("a.txt");
    let file_b = temp.path().join("b.txt");
    write_file(&file_a, "a");
    write_file(&file_b, "b");

    let resource_a = registry
        .lock()
        .unwrap()
        .register_resource(&file_a)
        .unwrap()
        .resource
        .resource_id;
    let resource_b = registry
        .lock()
        .unwrap()
        .register_resource(&file_b)
        .unwrap()
        .resource
        .resource_id;

    let mut kernel = new_kernel();
    install_file_broker(&mut kernel, registry.clone());
    let holder = AppId::new("consumer");
    install_app(&mut kernel, "consumer");
    grant_read(&mut kernel, &holder, resource_a.clone());

    let result = invoke_file_read_with_data_scope(&mut kernel, &holder, resource_a, resource_b);
    match result {
        app_host_kernel::invocation::InvocationResult::Failed { error } => {
            assert!(error.contains("resource scope mismatch"), "{error}");
        }
        other => panic!("expected handler failure, got {other:?}"),
    }
}

#[test]
fn file_broker_pending_removal_reconciliation_revokes_grants() {
    let temp = temp_dir("reconcile");
    let registry = Arc::new(Mutex::new(registry_service(temp.path())));
    let file_path = temp.path().join("resource.txt");
    write_file(&file_path, "hello");
    let resource_id = registry
        .lock()
        .unwrap()
        .register_resource(&file_path)
        .unwrap()
        .resource
        .resource_id;

    let mut kernel = new_kernel();
    install_file_broker(&mut kernel, registry.clone());
    let holder = AppId::new("consumer");
    install_app(&mut kernel, holder.as_str());
    grant_read(&mut kernel, &holder, resource_id.clone());
    assert!(!kernel.grants_for(&holder).is_empty());

    registry
        .lock()
        .unwrap()
        .begin_removal(&resource_id)
        .unwrap();
    registry
        .lock()
        .unwrap()
        .reconcile_with_kernel(&mut kernel)
        .unwrap();

    assert!(registry.lock().unwrap().list_resources().is_empty());
    assert!(kernel.grants_for(&holder).is_empty());
}

#[test]
fn file_broker_removal_finalize_failure_keeps_pending_state() {
    let temp = temp_dir("removal-failure");
    let file_path = temp.path().join("resource.txt");
    write_file(&file_path, "hello");
    let mut kernel = new_kernel();
    let registry = Arc::new(Mutex::new(registry_service(temp.path())));
    let resource_id = registry
        .lock()
        .unwrap()
        .register_resource(&file_path)
        .unwrap()
        .resource
        .resource_id;
    registry
        .lock()
        .unwrap()
        .begin_removal(&resource_id)
        .unwrap();

    let registry = Arc::new(Mutex::new(registry_service_with_writer(
        temp.path(),
        Arc::new(NthRenameFailWriter::new(1)),
    )));
    install_file_broker(&mut kernel, registry.clone());
    install_app(&mut kernel, "consumer");
    let grant = file_resource_grant_request(
        AppId::new("consumer"),
        resource_id.clone(),
        FileResourceGrantOperation::Read,
    );
    let holder = AppId::new("consumer");
    let result = kernel.issue_grant(&holder, &grant).unwrap();
    assert!(matches!(
        result,
        app_host_kernel::services::broker::IssueResult::Issued(_)
    ));

    let error = registry
        .lock()
        .unwrap()
        .reconcile_with_kernel(&mut kernel)
        .unwrap_err();
    assert!(
        error.contains("injected rename failure") || error.contains("replace file"),
        "{error}"
    );
    assert_eq!(
        registry.lock().unwrap().list_resources()[0].status,
        super::FileResourceStatus::Removing
    );
    assert!(kernel.grants_for(&holder).is_empty());
}

#[test]
fn file_broker_startup_reconciliation_revokes_orphaned_grants() {
    let temp = temp_dir("orphaned-grant");
    let mut registry = registry_service(temp.path());
    let mut kernel = new_kernel();
    install_file_broker(
        &mut kernel,
        Arc::new(Mutex::new(registry_service(temp.path()))),
    );
    let holder = AppId::new("consumer");
    install_app(&mut kernel, holder.as_str());
    let resource_id = ResourceId::new("removed-resource");
    grant_read(&mut kernel, &holder, resource_id);

    registry.reconcile_with_kernel(&mut kernel).unwrap();

    assert_eq!(
        kernel.grant_statuses_for(&holder)[0].status,
        GrantStatus::Revoked
    );
}

#[test]
fn file_broker_grant_validation_rejects_unknown_resources() {
    let temp = temp_dir("unknown-grant-resource");
    let registry = registry_service(temp.path());
    let scope = GrantScope::ExactCapability {
        provider: file_broker_app_id(),
        capability: CapabilityName::new("file.list"),
    };
    let data_scope = DataScope::resources(vec![ResourceId::new("removed-resource")]).unwrap();

    let error = registry
        .validate_grant_data_scope(&scope, &data_scope)
        .unwrap_err();

    assert_eq!(error, "unknown file resource: removed-resource");
}

fn create_symlink_file(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)
    }
}
