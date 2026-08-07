//! End-to-end lifecycle tests for the third-party app manager, driven through
//! the public `AppManager` API against a real `Kernel`, a scriptable trusted
//! chrome, and package directories on a real filesystem.
//!
//! Covers: inspect, install, deny, enable, disable, uninstall, malformed
//! package, duplicate id, and failed startup.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use app_host_kernel::ids::{AppId, CapabilityName, SurfaceName};
use app_host_kernel::invocation::{CapabilityHandler, CapabilityOutcome, InvocationResult};
use app_host_kernel::kernel::Kernel;
use app_host_kernel::manifest::{seal, AppManifest};
use app_host_kernel::primitives::capability::{
    CapabilityDeclaration, CapabilityEffect, CapabilityRef,
};
use app_host_kernel::primitives::grant::DataScope;
use app_host_kernel::primitives::run::Initiator;
use app_host_kernel::primitives::surface::ActionIntent;
use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, TrustedChrome,
};

use host_lib::app_manager::{AppManager, AppStatusView};
use host_lib::config::HostConfigService;
use host_lib::surface_ui::SurfaceUiRegistry;

// -- scriptable chrome --------------------------------------------------------

/// Approves or denies every grant prompt based on a shared flag, and records
/// how many grant prompts it saw.
struct ScriptedChrome {
    approve: AtomicBool,
    grant_prompts: AtomicUsize,
}

impl ScriptedChrome {
    fn new(approve: bool) -> Arc<Self> {
        Arc::new(Self {
            approve: AtomicBool::new(approve),
            grant_prompts: AtomicUsize::new(0),
        })
    }
}

impl TrustedChrome for ScriptedChrome {
    fn confirm_grant(&self, _prompt: GrantIssuancePrompt) -> ApprovalDecision {
        self.grant_prompts.fetch_add(1, Ordering::SeqCst);
        if self.approve.load(Ordering::SeqCst) {
            ApprovalDecision::Approved
        } else {
            ApprovalDecision::Denied
        }
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

// -- fixtures -----------------------------------------------------------------

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh, isolated temp directory that is cleaned up when `Scratch` drops.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("ahpkg-{label}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }
    fn path(&self) -> &Path {
        &self.root
    }
    fn sub(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write_package(dir: &Path, document: Value) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("app.json"),
        serde_json::to_vec_pretty(&document).unwrap(),
    )
    .unwrap();
}

/// A minimal, installable `none`-backend package that requests one grant
/// (so install exercises trusted chrome).
fn none_package(id: &str, version: &str) -> Value {
    json!({
        "format_version": 1,
        "id": id,
        "version": version,
        "display_name": "Test App",
        "description": "A test app.",
        "min_host_version": "0.0.1",
        "manifest": {
            "grant_requests": [{
                "scope": {"kind": "exact-capability", "provider": "notes", "capability": "create"},
                "data_scope": {"kind": "none"},
                "condition": "requires-approval",
                "reason": "Create notes on your behalf.",
                "duration": {"kind": "non-expiring"}
            }]
        },
        "backend": {"kind": "none"},
        "data": {"kind": "none"},
        "integrity": {"algorithm": "sha256", "assets": {}}
    })
}

struct Harness {
    kernel: Kernel,
    manager: AppManager,
    surface_ui: SurfaceUiRegistry,
    config: HostConfigService,
    chrome: Arc<ScriptedChrome>,
    _scratch: Scratch,
}

impl Harness {
    fn new(label: &str, approve_grants: bool) -> Self {
        let scratch = Scratch::new(label);
        let apps_root = scratch.sub("apps");
        let chrome = ScriptedChrome::new(approve_grants);
        let mut kernel = Kernel::new(chrome.clone());
        install_notes_provider(&mut kernel);
        Harness {
            kernel,
            manager: AppManager::in_memory(apps_root),
            surface_ui: SurfaceUiRegistry::new(),
            config: HostConfigService::default(),
            chrome,
            _scratch: scratch,
        }
    }

    fn scratch_root(&self) -> &Path {
        self._scratch.path()
    }

    fn install(&mut self, package_dir: &Path) -> Result<Vec<AppStatusView>, String> {
        let inspection = self.manager.inspect(package_dir)?;
        self.manager.install(
            &mut self.kernel,
            &mut self.surface_ui,
            &inspection.staged_id,
            &inspection.package_digest,
            "2026-07-10T00:00:00Z",
        )?;
        Ok(self.status())
    }

    fn status(&self) -> Vec<AppStatusView> {
        self.manager.status_views(&self.kernel, &self.surface_ui)
    }

    fn view(&self, id: &str) -> Option<AppStatusView> {
        self.status().into_iter().find(|view| view.id == id)
    }
}

fn install_notes_provider(kernel: &mut Kernel) {
    let manifest = AppManifest {
        app_id: AppId::new("notes"),
        version: "0.1.0".into(),
        display_name: "Notes".into(),
        description: "Test provider".into(),
        capabilities: vec![CapabilityDeclaration {
            name: CapabilityName::new("create"),
            description: "Create a note".into(),
            input_schema: json!({"type": "object"}).as_object().unwrap().clone(),
            effect: CapabilityEffect::LocalWrite,
            output_schema: None,
        }],
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
    };
    let mut handlers = std::collections::BTreeMap::new();
    let handler: CapabilityHandler = Box::new(|_, _| {
        Ok(CapabilityOutcome {
            result: json!({"ok": true}),
            artifacts: vec![],
        })
    });
    handlers.insert(CapabilityName::new("create"), handler);
    kernel.install(seal(manifest), handlers).unwrap();
}

// -- tests --------------------------------------------------------------------

#[test]
fn inspect_reports_summary_without_installing() {
    let mut harness = Harness::new("inspect", true);
    let dir = harness.scratch_root().join("pkg");
    write_package(&dir, none_package("com.example.inspect", "1.0.0"));

    let inspection = harness.manager.inspect(&dir).unwrap();
    assert_eq!(inspection.id, "com.example.inspect");
    assert_eq!(inspection.backend_kind, "none");
    assert!(inspection.installable);
    assert!(inspection.integrity_ok);
    assert_eq!(inspection.grant_requests.len(), 1);
    assert_eq!(inspection.grant_requests[0].scope_label, "notes/create");
    // Unsigned local package.
    assert!(matches!(
        inspection.signature,
        host_lib::publisher_trust::SignatureState::Unsigned
    ));
    // Inspection installs nothing.
    assert!(harness
        .status()
        .iter()
        .all(|view| view.id != "com.example.inspect"));
}

#[test]
fn install_activates_and_issues_the_confirmed_grant() {
    let mut harness = Harness::new("install", true);
    let dir = harness.scratch_root().join("pkg");
    write_package(&dir, none_package("com.example.installed", "1.0.0"));

    harness.install(&dir).unwrap();

    let app_id = AppId::new("com.example.installed");
    assert!(harness.kernel.installed_app(&app_id).is_ok());
    // The grant was prompted through trusted chrome and issued.
    assert_eq!(harness.chrome.grant_prompts.load(Ordering::SeqCst), 1);
    assert_eq!(harness.kernel.grants_for(&app_id).len(), 1);

    let view = harness.view("com.example.installed").unwrap();
    assert_eq!(view.status, "active");
    assert!(!view.bundled);
    assert!(view.removable);
    assert_eq!(view.signature, "unsigned");
    assert_eq!(view.missing_permissions, 0);
}

#[test]
fn install_issues_an_all_resources_manifest_grant() {
    let mut harness = Harness::new("install-all-resources", true);
    let dir = harness.scratch_root().join("pkg");
    let mut package = none_package("com.example.all-resources", "1.0.0");
    package["manifest"]["grant_requests"][0]["data_scope"] = json!({"kind": "all-resources"});
    write_package(&dir, package);

    let inspection = harness.manager.inspect(&dir).unwrap();
    assert_eq!(
        inspection.grant_requests[0].data_scope_label,
        "all current and future resources"
    );
    harness.install(&dir).unwrap();

    let grants = harness
        .kernel
        .grants_for(&AppId::new("com.example.all-resources"));
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].data_scope, DataScope::AllResources);
}

#[test]
fn package_capability_is_available_to_chat_under_an_explicit_consumer_grant() {
    let mut harness = Harness::new("consumer-grant", true);
    let chat_manifest = host_lib::chat_app::chat_manifest_for_kernel(&harness.kernel);
    let chat_store = Arc::new(Mutex::new(
        host_lib::chat_store::ChatStore::new(harness.scratch_root().join("chat-threads.json"))
            .unwrap(),
    ));
    install_parts(
        &mut harness.kernel,
        chat_manifest,
        host_lib::chat_app::chat_handlers(chat_store),
        app_host_kernel::primitives::grant::GrantOrigin::SystemBundled,
    )
    .unwrap();
    let package = harness.scratch_root().join("consumer-package");
    write_mcp_lifecycle_package(&package, Some("chat"));

    harness.install(&package).unwrap();

    let provider = AppId::new(LIFECYCLE_FIXTURE_ID);
    let status = harness.view(provider.as_str()).unwrap();
    assert_eq!(
        status.status, "active",
        "detail: {:?}",
        status.status_detail
    );
    let binding = harness
        .kernel
        .open_surface(&provider, &SurfaceName::new("inventory"))
        .unwrap();
    let added = harness
        .kernel
        .submit_action(
            &binding,
            ActionIntent {
                capability: CapabilityRef {
                    provider: provider.clone(),
                    capability: CapabilityName::new("add_item"),
                },
                input: json!({"title": "Shared item"}).as_object().unwrap().clone(),
                data_scope: app_host_kernel::primitives::grant::DataScope::None,
                goal: "add fixture item".into(),
            },
        )
        .unwrap();
    assert!(matches!(added.result, InvocationResult::Completed { .. }));

    let chat = AppId::new("chat");
    assert!(harness
        .kernel
        .available_capabilities_for(&chat)
        .unwrap()
        .iter()
        .any(|capability| capability.capability == CapabilityName::new("list_items")));
    let run = harness
        .kernel
        .start_run(
            Initiator::App {
                app_id: chat,
                reason: "read package data".into(),
            },
            "list fixture items",
        )
        .unwrap();
    let listed = harness
        .kernel
        .invoke(
            &run,
            &CapabilityRef {
                provider,
                capability: CapabilityName::new("list_items"),
            },
            json!({}).as_object().unwrap().clone(),
        )
        .unwrap();
    match listed {
        InvocationResult::Completed { result, .. } => {
            assert_eq!(result["items"][0]["title"], "Shared item");
        }
        other => panic!("list_items did not complete: {other:?}"),
    }
}

#[test]
fn install_consumes_staged_bytes_after_source_mutation() {
    let mut harness = Harness::new("source-mutation", true);
    let dir = harness.scratch_root().join("pkg");
    write_package(&dir, none_package("com.example.immutable", "1.0.0"));
    let inspection = harness.manager.inspect(&dir).unwrap();

    write_package(&dir, none_package("com.example.replaced", "9.9.9"));
    harness
        .manager
        .install(
            &mut harness.kernel,
            &mut harness.surface_ui,
            &inspection.staged_id,
            &inspection.package_digest,
            "2026-07-10T00:00:00Z",
        )
        .unwrap();

    assert!(harness.view("com.example.immutable").is_some());
    assert!(harness.view("com.example.replaced").is_none());
}

#[test]
fn install_rejects_an_unapproved_staged_digest() {
    let mut harness = Harness::new("digest-mismatch", true);
    let dir = harness.scratch_root().join("pkg");
    write_package(&dir, none_package("com.example.digest", "1.0.0"));
    let inspection = harness.manager.inspect(&dir).unwrap();

    let error = harness
        .manager
        .install(
            &mut harness.kernel,
            &mut harness.surface_ui,
            &inspection.staged_id,
            &format!("sha256-{}", "0".repeat(64)),
            "2026-07-10T00:00:00Z",
        )
        .unwrap_err();
    assert!(error.contains("approved digest"), "{error}");
    assert!(harness.view("com.example.digest").is_none());
}

#[test]
fn deny_installs_the_app_but_issues_no_grant() {
    let mut harness = Harness::new("deny", false); // chrome denies every grant
    let dir = harness.scratch_root().join("pkg");
    write_package(&dir, none_package("com.example.denied", "1.0.0"));

    harness.install(&dir).unwrap();

    let app_id = AppId::new("com.example.denied");
    // The app installed, but the denied grant was not issued.
    assert!(harness.kernel.installed_app(&app_id).is_ok());
    assert_eq!(harness.kernel.grants_for(&app_id).len(), 0);
    let view = harness.view("com.example.denied").unwrap();
    assert_eq!(view.status, "needs-permissions");
    assert_eq!(view.missing_permissions, 1);
}

#[test]
fn disable_removes_all_active_authority_and_enable_restores_it() {
    let mut harness = Harness::new("toggle", true);
    let dir = harness.scratch_root().join("pkg");
    write_package(&dir, none_package("com.example.toggle", "1.0.0"));
    harness.install(&dir).unwrap();
    let app_id = AppId::new("com.example.toggle");
    assert_eq!(harness.chrome.grant_prompts.load(Ordering::SeqCst), 1);

    // Disable: the app leaves the kernel entirely — no grants, no handlers.
    harness
        .manager
        .set_enabled(
            &mut harness.kernel,
            &mut harness.surface_ui,
            "com.example.toggle",
            false,
        )
        .unwrap();
    assert!(harness.kernel.installed_app(&app_id).is_err());
    assert_eq!(harness.kernel.grants_for(&app_id).len(), 0);
    assert_eq!(
        harness.view("com.example.toggle").unwrap().status,
        "disabled"
    );

    // Enable: back in the kernel with its grant re-issued.
    harness
        .manager
        .set_enabled(
            &mut harness.kernel,
            &mut harness.surface_ui,
            "com.example.toggle",
            true,
        )
        .unwrap();
    assert!(harness.kernel.installed_app(&app_id).is_ok());
    assert_eq!(harness.kernel.grants_for(&app_id).len(), 1);
    assert_eq!(harness.chrome.grant_prompts.load(Ordering::SeqCst), 2);
    assert_eq!(harness.view("com.example.toggle").unwrap().status, "active");
}

#[test]
fn uninstall_removes_the_app_and_purges_secrets_and_data_on_request() {
    let mut harness = Harness::new("uninstall", true);
    let dir = harness.scratch_root().join("pkg");
    // A package with a connector (declares a secret) and a config section.
    let mut document = none_package("com.example.removeme", "1.0.0");
    document["manifest"]["connectors"] = json!([{
        "name": "api",
        "description": "External API.",
        "secret_names": ["api_key"]
    }]);
    document["manifest"]["config_declarations"] = json!([{
        "name": "settings",
        "title": "Settings",
        "description": "App settings.",
        "json_schema": {"type": "object"}
    }]);
    write_package(&dir, document);
    harness.install(&dir).unwrap();

    let app_id = AppId::new("com.example.removeme");
    // Seed a secret and app data.
    harness
        .config
        .put_secret(&mut harness.kernel, &app_id, "api_key", "s3cr3t".into())
        .unwrap();
    let manifest = harness
        .kernel
        .installed_app(&app_id)
        .unwrap()
        .manifest
        .clone();
    harness
        .config
        .update_app_config(
            "com.example.removeme",
            &manifest,
            json!({"k": "v"}).as_object().unwrap().clone(),
        )
        .unwrap();
    assert!(harness.config.has_secret(&app_id, "api_key").unwrap());
    assert!(!harness
        .config
        .get_app_config("com.example.removeme")
        .is_empty());

    harness
        .manager
        .uninstall(
            &mut harness.kernel,
            &mut harness.surface_ui,
            &mut harness.config,
            "com.example.removeme",
            true, // purge secrets
            true, // purge data
        )
        .unwrap();

    // Gone from kernel and manager; secret and data purged per choice.
    assert!(harness.kernel.installed_app(&app_id).is_err());
    assert!(harness.view("com.example.removeme").is_none());
    assert!(!harness.config.has_secret(&app_id, "api_key").unwrap());
    assert!(harness
        .config
        .get_app_config("com.example.removeme")
        .is_empty());
    assert_eq!(harness.manager.records().count(), 0);
}

#[test]
fn uninstall_preserves_data_when_the_user_declines_to_purge() {
    let mut harness = Harness::new("uninstall-keep", true);
    let dir = harness.scratch_root().join("pkg");
    let mut document = none_package("com.example.keepdata", "1.0.0");
    document["manifest"]["config_declarations"] = json!([{
        "name": "settings", "title": "Settings", "description": "d",
        "json_schema": {"type": "object"}
    }]);
    write_package(&dir, document);
    harness.install(&dir).unwrap();
    let app_id = AppId::new("com.example.keepdata");
    let manifest = harness
        .kernel
        .installed_app(&app_id)
        .unwrap()
        .manifest
        .clone();
    harness
        .config
        .update_app_config(
            "com.example.keepdata",
            &manifest,
            json!({"k": "v"}).as_object().unwrap().clone(),
        )
        .unwrap();

    harness
        .manager
        .uninstall(
            &mut harness.kernel,
            &mut harness.surface_ui,
            &mut harness.config,
            "com.example.keepdata",
            false,
            false,
        )
        .unwrap();

    // Data preserved because the user declined to purge.
    assert!(!harness
        .config
        .get_app_config("com.example.keepdata")
        .is_empty());
}

#[test]
fn malformed_package_is_rejected_at_inspect_and_install() {
    let mut harness = Harness::new("malformed", true);
    let dir = harness.scratch_root().join("pkg");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("app.json"), b"{ this is not valid json ]").unwrap();

    assert!(harness.manager.inspect(&dir).is_err());
    assert!(harness.install(&dir).is_err());
    assert_eq!(harness.manager.records().count(), 0);
}

#[test]
fn a_package_with_an_unknown_field_is_rejected() {
    let mut harness = Harness::new("unknown-field", true);
    let dir = harness.scratch_root().join("pkg");
    let mut document = none_package("com.example.strict", "1.0.0");
    document["surprise"] = json!("not allowed");
    write_package(&dir, document);

    assert!(harness.install(&dir).is_err());
}

#[test]
fn duplicate_id_is_refused() {
    let mut harness = Harness::new("dup", true);
    let first = harness.scratch_root().join("a");
    let second = harness.scratch_root().join("b");
    write_package(&first, none_package("com.example.dup", "1.0.0"));
    write_package(&second, none_package("com.example.dup", "2.0.0"));

    harness.install(&first).unwrap();
    let error = harness.install(&second).unwrap_err();
    assert!(error.contains("already installed"), "unexpected: {error}");
    // Only the first install remains.
    assert_eq!(harness.view("com.example.dup").unwrap().version, "1.0.0");
}

#[test]
fn a_package_cannot_impersonate_a_bundled_app_id() {
    let mut harness = Harness::new("impersonate", true);
    let dir = harness.scratch_root().join("pkg");
    // Bare id (no dot) is reserved for bundled apps; the schema/id rule rejects it.
    write_package(&dir, none_package("notes", "1.0.0"));
    let error = harness.install(&dir).unwrap_err();
    assert!(
        error.contains("/id") || error.contains("invalid app id"),
        "unexpected: {error}"
    );
}

#[test]
fn failed_backend_startup_leaves_the_app_installed_but_failed() {
    let mut harness = Harness::new("failed-startup", true);
    let dir = harness.scratch_root().join("pkg");
    let mut document = none_package("com.example.badbackend", "1.0.0");
    // An MCP stdio backend whose command cannot be spawned.
    document["backend"] = json!({
        "kind": "mcp-stdio",
        "authority_mode": "unsandboxed",
        "command": "this-command-does-not-exist-abcdef",
        "args": []
    });
    // MCP backends may declare capabilities; keep grant request too.
    write_package(&dir, document);

    // Install succeeds (record created) even though the backend won't start.
    harness.install(&dir).unwrap();

    let app_id = AppId::new("com.example.badbackend");
    assert!(
        harness.kernel.installed_app(&app_id).is_err(),
        "must not be active"
    );
    let view = harness.view("com.example.badbackend").unwrap();
    assert_eq!(view.status, "failed");
    assert!(view.status_detail.is_some());
    assert!(harness.manager.records().count() == 1);
}

#[test]
fn bundled_apps_are_marked_and_not_removable() {
    // A bundled-looking app installed directly into the kernel appears in the
    // manager as bundled + non-removable, and the manager refuses to touch it.
    let mut harness = Harness::new("bundled", true);
    // Simulate a bundled app via the manager-agnostic kernel install path.
    let manifest = app_host_kernel::manifest::seal(app_host_kernel::manifest::AppManifest {
        app_id: AppId::new("bundled-test"),
        version: "0.1.0".into(),
        display_name: "Notes".into(),
        description: "Bundled".into(),
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
    });
    harness
        .kernel
        .install(manifest, std::collections::BTreeMap::new())
        .unwrap();

    let view = harness.view("bundled-test").unwrap();
    assert!(view.bundled);
    assert!(!view.removable);

    // The manager refuses lifecycle actions on a bundled app.
    assert!(harness
        .manager
        .set_enabled(
            &mut harness.kernel,
            &mut harness.surface_ui,
            "bundled-test",
            false
        )
        .is_err());
    assert!(harness
        .manager
        .uninstall(
            &mut harness.kernel,
            &mut harness.surface_ui,
            &mut harness.config,
            "bundled-test",
            false,
            false
        )
        .is_err());
}
mod support;
use support::*;
