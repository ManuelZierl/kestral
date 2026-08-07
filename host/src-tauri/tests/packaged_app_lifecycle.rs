//! End-to-end proof of the packaged MCP app path using a host-owned fixture.
//!
//! - Inspection (no node) asserts the declared capabilities, permissions,
//!   config, custom surface, artifact type, and unsigned signature.
//! - The full lifecycle (node-gated) installs the package, invokes a backend
//!   capability, produces an artifact, survives a simulated restart, and
//!   disables + uninstalls cleanly.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::{json, Map, Value};

use app_host_kernel::ids::{AppId, CapabilityName, SurfaceName};
use app_host_kernel::invocation::InvocationResult;
use app_host_kernel::kernel::Kernel;
use app_host_kernel::primitives::capability::CapabilityRef;
use app_host_kernel::primitives::surface::ActionIntent;
use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, TrustedChrome,
};
use app_host_kernel::JsonObject;

use host_lib::app_manager::AppManager;
use host_lib::config::HostConfigService;
use host_lib::publisher_trust::SignatureState;
use host_lib::surface_ui::SurfaceUiRegistry;

// -- helpers ------------------------------------------------------------------

struct ApproveAll;
impl TrustedChrome for ApproveAll {
    fn confirm_grant(&self, _p: GrantIssuancePrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }
    fn approve_capability(&self, _p: CapabilityApprovalPrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }
    fn confirm_event_subscriptions(&self, _p: EventSubscriptionPrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }
    fn show_notice(&self, _n: ChromeNotice) -> Result<(), ChromeNoticeError> {
        Ok(())
    }
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Scratch {
    root: PathBuf,
}
impl Scratch {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("packaged-app-e2e-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn obj(value: Value) -> JsonObject {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn lifecycle_data_file(apps_root: &Path) -> PathBuf {
    let app_root = apps_root.join(".data").join(LIFECYCLE_FIXTURE_ID);
    let state: Value =
        serde_json::from_slice(&std::fs::read(app_root.join("app-data-state-v1.json")).unwrap())
            .unwrap();
    let revision = state["active_revision_id"].as_str().unwrap();
    app_root
        .join("app-data-revisions")
        .join(revision)
        .join("items.json")
}

/// Drive one surface action through the kernel's full public action path.
fn submit(
    kernel: &mut Kernel,
    surface_app: &str,
    surface: &str,
    provider: &str,
    capability: &str,
    input: JsonObject,
) -> InvocationResult {
    let binding = kernel
        .open_surface(&AppId::new(surface_app), &SurfaceName::new(surface))
        .expect("open surface");
    let intent = ActionIntent {
        capability: CapabilityRef {
            provider: AppId::new(provider),
            capability: CapabilityName::new(capability),
        },
        input,
        data_scope: app_host_kernel::primitives::grant::DataScope::None,
        goal: capability.to_string(),
    };
    kernel
        .submit_action(&binding, intent)
        .expect("submit")
        .result
}

// -- tests --------------------------------------------------------------------

#[test]
fn packaged_mcp_app_inspects_without_running_code() {
    let scratch = Scratch::new();
    let package = scratch.root.join("package");
    write_mcp_lifecycle_package(&package, None);
    let mut manager = AppManager::in_memory(scratch.root.join("apps"));
    let inspection = manager.inspect(&package).unwrap();

    assert_eq!(inspection.id, LIFECYCLE_FIXTURE_ID);
    let caps: Vec<&str> = inspection
        .capabilities
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(caps.len(), 2);
    for expected in ["add_item", "list_items"] {
        assert!(caps.contains(&expected), "missing capability {expected}");
    }
    assert_eq!(inspection.grant_requests.len(), 2);
    assert_eq!(inspection.config.len(), 1);
    assert!(inspection.surfaces.iter().any(|s| s.has_custom_ui));
    assert!(inspection
        .artifact_types
        .iter()
        .any(|t| t == "item-snapshot"));
    assert!(matches!(inspection.signature, SignatureState::Unsigned));
    assert_eq!(inspection.backend_kind, "mcp-stdio");
    assert!(
        inspection.integrity_ok,
        "integrity: {:?}",
        inspection.integrity_error
    );
    assert!(inspection.installable);
}

#[test]
fn packaged_mcp_app_full_lifecycle() {
    let node = std::process::Command::new("node")
        .arg("--version")
        .output()
        .expect("packaged MCP app lifecycle requires `node` on PATH");
    assert!(node.status.success(), "`node --version` failed");

    let scratch = Scratch::new();
    let package = scratch.root.join("package");
    write_mcp_lifecycle_package(&package, None);
    let store = scratch.root.join("installed-apps.json");
    let apps_root = scratch.root.join("apps");
    let chrome: Arc<dyn TrustedChrome> = Arc::new(ApproveAll);
    let fixture_id = AppId::new(LIFECYCLE_FIXTURE_ID);

    // --- session 1: install, use, and produce an artifact ------------------
    {
        let mut kernel = Kernel::new(chrome.clone());
        let mut surface_ui = SurfaceUiRegistry::new();
        let mut manager = AppManager::new(
            store.clone(),
            scratch.root.join("trust-store.json"),
            apps_root.clone(),
            scratch.root.join("update-journal.json"),
            false,
        )
        .unwrap();

        let inspection = manager.inspect(&package).unwrap();
        manager
            .install(
                &mut kernel,
                &mut surface_ui,
                &inspection.staged_id,
                &inspection.package_digest,
                "2026-07-10T00:00:00Z",
            )
            .unwrap();

        let view = manager
            .status_views(&kernel, &surface_ui)
            .into_iter()
            .find(|v| v.id == LIFECYCLE_FIXTURE_ID)
            .expect("fixture app listed");
        assert_eq!(view.status, "active", "detail: {:?}", view.status_detail);
        assert!(view.surfaces.iter().any(|s| s.has_custom_ui));

        // Invoke a backend capability → completes and produces an artifact.
        let added = submit(
            &mut kernel,
            LIFECYCLE_FIXTURE_ID,
            "inventory",
            LIFECYCLE_FIXTURE_ID,
            "add_item",
            obj(json!({"title": "Buy milk"})),
        );
        assert!(
            matches!(added, InvocationResult::Completed { .. }),
            "add_item: {added:?}"
        );
        assert!(
            kernel
                .artifacts()
                .any(|a| a.artifact_type.as_str() == "item-snapshot"
                    && a.provenance.produced_by == fixture_id),
            "add_item should produce an item-snapshot artifact"
        );

        // The backend persisted its own data.
        assert!(
            lifecycle_data_file(&apps_root).is_file(),
            "backend must persist items.json in its data directory"
        );
    } // drop → node backend killed (simulated shutdown)

    // --- session 2: restart, verify data survived, then disable/uninstall --
    {
        let mut kernel = Kernel::new(chrome.clone());
        let mut surface_ui = SurfaceUiRegistry::new();
        let mut manager = AppManager::new(
            store.clone(),
            scratch.root.join("trust-store.json"),
            apps_root.clone(),
            scratch.root.join("update-journal.json"),
            false,
        )
        .unwrap();

        // Records persisted → the app re-activates on boot.
        let failures = manager.reactivate_enabled(&mut kernel, &mut surface_ui);
        assert!(failures.is_empty(), "reactivate failed: {failures:?}");
        assert!(kernel.installed_app(&fixture_id).is_ok());

        // The task added before the restart is still there.
        let listed = submit(
            &mut kernel,
            LIFECYCLE_FIXTURE_ID,
            "inventory",
            LIFECYCLE_FIXTURE_ID,
            "list_items",
            Map::new(),
        );
        match listed {
            InvocationResult::Completed { result, .. } => {
                assert!(
                    result.to_string().contains("Buy milk"),
                    "task did not survive restart: {result}"
                );
            }
            other => panic!("list_items not completed: {other:?}"),
        }

        // Disable: the app leaves the kernel; its data file is retained.
        manager
            .set_enabled(&mut kernel, &mut surface_ui, LIFECYCLE_FIXTURE_ID, false)
            .unwrap();
        assert!(kernel.installed_app(&fixture_id).is_err());
        assert!(
            lifecycle_data_file(&apps_root).is_file(),
            "disabled app keeps its data for re-enable"
        );

        // Re-enable, then uninstall (no purge) → gone, payload removed.
        manager
            .set_enabled(&mut kernel, &mut surface_ui, LIFECYCLE_FIXTURE_ID, true)
            .unwrap();
        let mut config = HostConfigService::default();
        manager
            .uninstall(
                &mut kernel,
                &mut surface_ui,
                &mut config,
                LIFECYCLE_FIXTURE_ID,
                false,
                false,
            )
            .unwrap();
        assert!(kernel.installed_app(&fixture_id).is_err());
        assert!(
            !apps_root.join(LIFECYCLE_FIXTURE_ID).exists(),
            "uninstall removes the payload directory"
        );
        assert_eq!(manager.records().count(), 0);
    }
}
mod support;
use support::*;
