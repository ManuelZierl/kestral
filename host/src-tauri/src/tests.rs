use super::*;
use crate::chat_runtime::tool_feedback;
use crate::chat_store::{ChatMessageRole, ChatMessageStatus};
use app_host_kernel::ids::{ArtifactId, ArtifactTypeName, CapabilityName, ResourceId, RunId};
use app_host_kernel::invocation::{CapabilityHandler, CapabilityOutcome, InvocationRequest};
use app_host_kernel::manifest::{seal, AppManifest};
use app_host_kernel::primitives::capability::{
    CapabilityDeclaration, CapabilityEffect, CapabilityRef,
};
use app_host_kernel::primitives::grant::DenialReason;
use app_host_kernel::primitives::run::{Initiator, RunTerminalState};
use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, TrustedChrome,
};
use app_host_kernel::services::ledger::{LedgerEvent, LedgerRecord};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use uuid::Uuid;

struct StartupTestChrome;

impl TrustedChrome for StartupTestChrome {
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

fn startup_test_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("host-startup-{label}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn startup_failure_message_includes_the_cause_and_recovery() {
    let message = startup_failure_message("read host config failed: permission denied");

    assert!(message.starts_with("Kestral could not start:"));
    assert!(message.contains("read host config failed: permission denied"));
    assert!(message.contains("Delete the named file (or the profile data directory)"));
}

#[test]
fn failed_startup_claim_is_retryable() {
    let installed = Mutex::new(false);

    {
        let _claim = StartupClaim::acquire(&installed).unwrap().unwrap();
        assert!(StartupClaim::acquire(&installed).unwrap().is_none());
    }

    assert!(StartupClaim::acquire(&installed).unwrap().is_some());
}

#[test]
fn completed_startup_claim_stays_claimed() {
    let installed = Mutex::new(false);

    let mut claim = StartupClaim::acquire(&installed).unwrap().unwrap();
    claim.complete();
    drop(claim);

    assert!(StartupClaim::acquire(&installed).unwrap().is_none());
}

fn write_agent_worker_package(path: &std::path::Path) {
    const APP_ID: &str = "com.example.agent-worker";
    let worker = b"// activation-only agent worker fixture\n";
    std::fs::create_dir_all(path.join("backend")).unwrap();
    std::fs::write(path.join("backend/worker.mjs"), worker).unwrap();
    let document = serde_json::json!({
        "format_version": 1,
        "id": APP_ID,
        "version": "1.0.0",
        "display_name": "Agent worker fixture",
        "description": "Host-owned activation fixture.",
        "min_host_version": "0.0.1",
        "manifest": {
            "capabilities": [{
                "name": "agent.run",
                "description": "Run the fixture agent.",
                "input_schema": {
                    "type": "object",
                    "properties": {"messages": {"type": "array", "items": {"type": "object"}}},
                    "required": ["messages"],
                    "additionalProperties": false
                },
                "effect": "external-write"
            }],
            "artifact_types": [{
                "name": "agent-transcript",
                "description": "Fixture agent transcript.",
                "json_schema": {"type": "array", "items": {"type": "object"}}
            }]
        },
        "consumer_grant_requests": [{
            "holder": "chat",
            "request": {
                "scope": {"kind": "exact-capability", "provider": APP_ID, "capability": "agent.run"},
                "data_scope": {"kind": "none"},
                "condition": "silent",
                "reason": "Let Chat use the test agent adapter.",
                "duration": {"kind": "non-expiring"}
            }
        }],
        "backend": {
            "kind": "agent-worker",
            "authority_mode": "unsandboxed",
            "protocol_version": 1,
            "entry": "backend/worker.mjs"
        },
        "data": {"kind": "none"},
        "integrity": {
            "algorithm": "sha256",
            "assets": {"backend/worker.mjs": format!("sha256-{:x}", Sha256::digest(worker))}
        }
    });
    std::fs::write(
        path.join("app.json"),
        serde_json::to_vec_pretty(&document).unwrap(),
    )
    .unwrap();
}

fn seed_stale_mcp_app(path: &std::path::Path) -> Vec<u8> {
    HostPaths::resolve_startup_from(
        path.to_path_buf(),
        std::iter::empty::<std::ffi::OsString>(),
        |_| None,
    )
    .unwrap();
    let state_path = path.join("kernel-state-v1.json");
    let store = kernel_state::FileKernelStateStore::open(state_path.clone()).unwrap();
    let mut kernel = Kernel::with_state_store(Arc::new(StartupTestChrome), store).unwrap();
    let prepared = kernel
        .prepare_install(
            seal(AppManifest {
                app_id: AppId::new("mcp-stale"),
                version: "0.1.0".into(),
                display_name: "Stale MCP".into(),
                description: "Startup reconciliation fixture".into(),
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
            }),
            std::collections::BTreeMap::new(),
        )
        .unwrap();
    kernel.commit_install(prepared.await_approval()).unwrap();
    drop(kernel);
    std::fs::read(state_path).unwrap()
}

fn build_test_host(path: PathBuf) -> Result<Arc<Host>, String> {
    let notices_path = path.join("trusted-notices.json");
    let paths =
        HostPaths::resolve_startup_from(path, std::iter::empty::<std::ffi::OsString>(), |_| None)?;
    build_host(
        paths,
        Arc::new(StartupTestChrome),
        Arc::new(PendingApprovals::default()),
        Arc::new(Mutex::new(TrustedNoticeStore::new(notices_path).unwrap())),
    )
}

#[test]
fn app_status_waits_for_manager_without_owning_kernel() {
    let path = startup_test_dir("app-status-lock-order");
    let host = build_test_host(path.clone()).unwrap();
    let manager = host.app_manager.lock().unwrap();
    let reader_host = host.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        list_installed_apps(HostState::direct(&reader_host))
    });

    started_rx.recv().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(host.kernel.try_lock().is_ok());

    drop(manager);
    reader.join().unwrap().unwrap();
    drop(host);
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn chat_choice_read_waits_for_a_kernel_transition() {
    let path = startup_test_dir("chat-choice-kernel-wait");
    let host = build_test_host(path.clone()).unwrap();
    let lock_host = host.clone();
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let lock_owner = std::thread::spawn(move || {
        let _kernel = lock_host.kernel.lock().unwrap();
        locked_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    locked_rx.recv().unwrap();

    let reader_host = host.clone();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let result =
            tauri::async_runtime::block_on(list_chat_profiles(HostState::direct(&reader_host)));
        result_tx.send(result).unwrap();
    });

    assert!(result_rx
        .recv_timeout(std::time::Duration::from_millis(50))
        .is_err());
    release_tx.send(()).unwrap();
    assert!(result_rx.recv().unwrap().is_ok());

    reader.join().unwrap();
    lock_owner.join().unwrap();
    drop(host);
    let _ = std::fs::remove_dir_all(path);
}

fn record(event: LedgerEvent, sequence: u64) -> LedgerRecord {
    LedgerRecord {
        sequence,
        recorded_at: Utc::now(),
        event,
    }
}

fn tool_capability() -> CapabilityRef {
    CapabilityRef {
        provider: AppId::new("notes"),
        capability: CapabilityName::new("create"),
    }
}

#[test]
fn editor_grant_uses_an_audit_reason_when_the_optional_reason_is_blank() {
    let request = GrantEditorRequest {
        holder: AppId::new("chat"),
        scope: GrantScope::ExactCapability {
            provider: AppId::new("llm-provider"),
            capability: CapabilityName::new("llm.generate"),
        },
        data_scope: DataScope::None,
        condition: GrantCondition::Silent,
        duration: GrantDuration::NonExpiring,
        reason: "  ".into(),
        allow_all_provider_scope: false,
        acknowledge_less_interactive_mcp: false,
    };

    assert_eq!(
        request.grant_request().unwrap().reason,
        "Added from the permissions page"
    );
}

#[test]
fn editor_grant_requires_acknowledgement_for_less_interactive_mcp_access() {
    let mut request = GrantEditorRequest {
        holder: AppId::new("chat"),
        scope: GrantScope::ExactCapability {
            provider: AppId::new("mcp-calendar"),
            capability: CapabilityName::new("create_event"),
        },
        data_scope: DataScope::None,
        condition: GrantCondition::Silent,
        duration: GrantDuration::NonExpiring,
        reason: "User chose silent access".into(),
        allow_all_provider_scope: false,
        acknowledge_less_interactive_mcp: false,
    };

    assert!(request.grant_request().is_err());
    request.acknowledge_less_interactive_mcp = true;
    assert!(request.grant_request().is_ok());
}

#[test]
fn editor_rejects_an_equivalent_active_non_expiring_permission() {
    let path = startup_test_dir("duplicate-editor-grant");
    let host = build_test_host(path.clone()).unwrap();
    tauri::async_runtime::block_on(install_bundled_apps_phased(
        host.clone(),
        host.config.clone(),
        host.file_resources.clone(),
    ))
    .unwrap();
    let request = GrantEditorRequest {
        holder: AppId::new("chat"),
        scope: GrantScope::ExactCapability {
            provider: permissions_app::permissions_app_id(),
            capability: CapabilityName::new(permissions_app::PROPOSE_GRANT),
        },
        data_scope: DataScope::None,
        condition: GrantCondition::Silent,
        duration: GrantDuration::NonExpiring,
        reason: "Duplicate permission".into(),
        allow_all_provider_scope: false,
        acknowledge_less_interactive_mcp: false,
    };

    let error =
        tauri::async_runtime::block_on(issue_editor_grant(HostState::direct(&host), request))
            .unwrap_err();

    assert_eq!(error, "an equivalent active permission already exists");
    let matching = host
        .kernel
        .lock()
        .unwrap()
        .grants_for(&AppId::new("chat"))
        .into_iter()
        .filter(|grant| {
            grant.scope
                == (GrantScope::ExactCapability {
                    provider: permissions_app::permissions_app_id(),
                    capability: CapabilityName::new(permissions_app::PROPOSE_GRANT),
                })
        })
        .count();
    assert_eq!(matching, 1);
    drop(host);
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn editor_rejects_artifact_capabilities_without_artifact_access() {
    let path = startup_test_dir("unscoped-artifact-editor-grant");
    let host = build_test_host(path.clone()).unwrap();
    tauri::async_runtime::block_on(install_bundled_apps_phased(
        host.clone(),
        host.config.clone(),
        host.file_resources.clone(),
    ))
    .unwrap();
    let request = GrantEditorRequest {
        holder: AppId::new("chat"),
        scope: GrantScope::ExactCapability {
            provider: artifacts_app::artifacts_app_id(),
            capability: CapabilityName::new(artifacts_app::ARTIFACTS_READ),
        },
        data_scope: DataScope::None,
        condition: GrantCondition::RequiresApproval,
        duration: GrantDuration::NonExpiring,
        reason: "Broken artifact access".into(),
        allow_all_provider_scope: false,
        acknowledge_less_interactive_mcp: false,
    };

    let error =
        tauri::async_runtime::block_on(issue_editor_grant(HostState::direct(&host), request))
            .unwrap_err();

    assert_eq!(
        error,
        "Artifact permissions must allow selected artifacts or all current and future artifacts"
    );
    drop(host);
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn artifact_access_command_groups_exact_query_and_read_permissions() {
    let path = startup_test_dir("grant-all-artifact-access");
    let host = build_test_host(path.clone()).unwrap();
    tauri::async_runtime::block_on(install_bundled_apps_phased(
        host.clone(),
        host.config.clone(),
        host.file_resources.clone(),
    ))
    .unwrap();

    tauri::async_runtime::block_on(grant_artifact_access(
        HostState::direct(&host),
        AppId::new("chat"),
        artifacts_app::ArtifactAccessTarget::AllArtifacts,
    ))
    .unwrap();

    let kernel = host.kernel.lock().unwrap();
    let artifact_grants = kernel
        .grants_for(&AppId::new("chat"))
        .into_iter()
        .filter(|grant| grant.scope.provider() == &artifacts_app::artifacts_app_id())
        .collect::<Vec<_>>();
    assert_eq!(artifact_grants.len(), 2);
    assert!(artifact_grants
        .iter()
        .all(|grant| grant.data_scope == DataScope::AllResources));
    assert!(artifact_grants
        .iter()
        .all(|grant| matches!(grant.scope, GrantScope::ExactCapability { .. })));
    drop(kernel);
    drop(host);
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn submitted_permission_proposal_issues_an_exact_approval_required_grant() {
    let path = startup_test_dir("permission-proposal");
    let host = build_test_host(path.clone()).unwrap();
    tauri::async_runtime::block_on(install_bundled_apps_phased(
        host.clone(),
        host.config.clone(),
        host.file_resources.clone(),
    ))
    .unwrap();

    let artifact_id = {
        let mut kernel = host.kernel.lock().unwrap();
        let notes_capability = CapabilityName::new("notes.create");
        let notes_handler: CapabilityHandler = Box::new(|_, _| {
            Ok(CapabilityOutcome {
                result: serde_json::json!({"created": true}),
                artifacts: vec![],
            })
        });
        let prepared = kernel
            .prepare_install(
                seal(AppManifest {
                    app_id: AppId::new("com.example.notes"),
                    version: "0.1.0".into(),
                    display_name: "Notes".into(),
                    description: "Permission proposal fixture".into(),
                    capabilities: vec![CapabilityDeclaration {
                        name: notes_capability.clone(),
                        description: "Create a note".into(),
                        input_schema: serde_json::json!({
                            "type": "object",
                            "additionalProperties": false
                        })
                        .as_object()
                        .unwrap()
                        .clone(),
                        output_schema: None,
                        effect: CapabilityEffect::LocalWrite,
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
                }),
                std::collections::BTreeMap::from([(notes_capability, notes_handler)]),
            )
            .unwrap();
        kernel.commit_install(prepared.await_approval()).unwrap();

        let mut available = kernel
            .available_capabilities_for(&AppId::new("chat"))
            .unwrap();
        permissions_app::contextualize_tools(&kernel, &AppId::new("chat"), &mut available).unwrap();
        let list_tool = available
            .iter()
            .find(|view| {
                view.provider_app_id == permissions_app::permissions_app_id()
                    && view.capability == CapabilityName::new(permissions_app::LIST_ACTIVE)
            })
            .unwrap();
        assert!(list_tool
            .description
            .contains("standing active capability grants"));
        assert!(list_tool
            .description
            .contains("not necessarily a tool supplied to the model"));
        let snapshot = list_tool.input_schema["properties"]["snapshot"]["const"].clone();
        assert_eq!(snapshot["holder"], "chat");
        assert!(snapshot["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|permission| {
                permission["provider"] == "llm-provider"
                    && permission["capability"] == "llm.generate"
            }));

        let list_run_id = kernel
            .start_run(
                Initiator::App {
                    app_id: AppId::new("chat"),
                    reason: "test active permission inspection".into(),
                },
                "List active permissions",
            )
            .unwrap();
        let prepared = match kernel
            .prepare_invocation(
                &list_run_id,
                &app_host_kernel::primitives::capability::CapabilityRef {
                    provider: permissions_app::permissions_app_id(),
                    capability: CapabilityName::new(permissions_app::LIST_ACTIVE),
                },
                InvocationRequest {
                    input: serde_json::json!({"snapshot": snapshot})
                        .as_object()
                        .unwrap()
                        .clone(),
                    data_scope: DataScope::None,
                },
            )
            .unwrap()
        {
            PrepareInvocation::Prepared(prepared) => prepared,
            PrepareInvocation::Refused(result) => panic!("permission read refused: {result:?}"),
        };
        let authorized = match kernel
            .authorize_invocation(prepared.await_approval())
            .unwrap()
        {
            app_host_kernel::kernel::AuthorizeInvocation::Authorized(authorized) => authorized,
            app_host_kernel::kernel::AuthorizeInvocation::Refused(result) => {
                panic!("permission read authorization refused: {result:?}")
            }
        };
        let result = kernel.finalize_invocation(authorized.execute()).unwrap();
        kernel
            .end_run(&list_run_id, RunTerminalState::Completed)
            .unwrap();
        assert!(matches!(
            result,
            app_host_kernel::invocation::InvocationResult::Completed { ref result, .. }
                if result["holder"] == "chat"
        ));

        let requestable_tool = available
            .iter()
            .find(|view| {
                view.provider_app_id == permissions_app::permissions_app_id()
                    && view.capability == CapabilityName::new(permissions_app::LIST_REQUESTABLE)
            })
            .unwrap();
        let requestable_snapshot =
            requestable_tool.input_schema["properties"]["snapshot"]["const"].clone();
        assert!(requestable_snapshot["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|permission| permission
                == &serde_json::json!({
                    "provider": "com.example.notes",
                    "provider_display_name": "Notes",
                    "capability": "notes.create",
                    "description": "Create a note",
                    "effect": "local-write"
                })));

        let requestable_run_id = kernel
            .start_run(
                Initiator::App {
                    app_id: AppId::new("chat"),
                    reason: "test requestable permission inspection".into(),
                },
                "List requestable permissions",
            )
            .unwrap();
        let prepared = match kernel
            .prepare_invocation(
                &requestable_run_id,
                &CapabilityRef {
                    provider: permissions_app::permissions_app_id(),
                    capability: CapabilityName::new(permissions_app::LIST_REQUESTABLE),
                },
                InvocationRequest {
                    input: serde_json::json!({"snapshot": requestable_snapshot})
                        .as_object()
                        .unwrap()
                        .clone(),
                    data_scope: DataScope::None,
                },
            )
            .unwrap()
        {
            PrepareInvocation::Prepared(prepared) => prepared,
            PrepareInvocation::Refused(result) => {
                panic!("requestable permission read refused: {result:?}")
            }
        };
        let authorized = match kernel
            .authorize_invocation(prepared.await_approval())
            .unwrap()
        {
            app_host_kernel::kernel::AuthorizeInvocation::Authorized(authorized) => authorized,
            app_host_kernel::kernel::AuthorizeInvocation::Refused(result) => {
                panic!("requestable permission read authorization refused: {result:?}")
            }
        };
        let result = kernel.finalize_invocation(authorized.execute()).unwrap();
        kernel
            .end_run(&requestable_run_id, RunTerminalState::Completed)
            .unwrap();
        assert!(matches!(
            result,
            app_host_kernel::invocation::InvocationResult::Completed { ref result, .. }
                if result["permissions"].as_array().is_some_and(|permissions| {
                    permissions.iter().any(|permission| {
                        permission["provider"] == "com.example.notes"
                            && permission["capability"] == "notes.create"
                    })
                })
        ));

        let proposal_tool = available
            .iter()
            .find(|view| {
                view.provider_app_id == permissions_app::permissions_app_id()
                    && view.capability == CapabilityName::new(permissions_app::PROPOSE_GRANT)
            })
            .unwrap();
        assert!(proposal_tool.input_schema["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .any(|choice| {
                choice["properties"]["provider"]["const"] == "com.example.notes"
                    && choice["properties"]["capability"]["const"] == "notes.create"
            }));
        assert!(proposal_tool
            .description
            .contains("exact requestable choice"));
        let proposal_snapshot =
            proposal_tool.input_schema["properties"]["snapshot"]["const"].clone();

        let run_id = kernel
            .start_run(
                Initiator::App {
                    app_id: AppId::new("chat"),
                    reason: "test permission proposal".into(),
                },
                "Propose permission",
            )
            .unwrap();
        let prepared = match kernel
            .prepare_invocation(
                &run_id,
                &permissions_app::propose_grant_ref(),
                InvocationRequest {
                    input: serde_json::json!({
                        "provider": "com.example.notes",
                        "capability": "notes.create",
                        "reason": "Create the note requested by the user",
                        "snapshot": proposal_snapshot
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                    data_scope: DataScope::None,
                },
            )
            .unwrap()
        {
            PrepareInvocation::Prepared(prepared) => prepared,
            PrepareInvocation::Refused(result) => panic!("proposal refused: {result:?}"),
        };
        let authorized = match kernel
            .authorize_invocation(prepared.await_approval())
            .unwrap()
        {
            app_host_kernel::kernel::AuthorizeInvocation::Authorized(authorized) => authorized,
            app_host_kernel::kernel::AuthorizeInvocation::Refused(result) => {
                panic!("proposal authorization refused: {result:?}")
            }
        };
        let result = kernel.finalize_invocation(authorized.execute()).unwrap();
        kernel
            .end_run(&run_id, RunTerminalState::Completed)
            .unwrap();
        match result {
            app_host_kernel::invocation::InvocationResult::Completed { artifacts, .. } => {
                artifacts[0].artifact_id.clone()
            }
            other => panic!("proposal failed: {other:?}"),
        }
    };

    let submission = tauri::async_runtime::block_on(submit_permission_proposal(
        HostState::direct(&host),
        artifact_id.clone(),
    ))
    .unwrap();
    let PermissionProposalSubmission::Issued {
        effective_condition,
        ..
    } = submission
    else {
        panic!("proposal was not issued")
    };
    assert_eq!(effective_condition, GrantCondition::RequiresApproval);
    let kernel = host.kernel.lock().unwrap();
    assert!(kernel.grants_for(&AppId::new("chat")).iter().any(|grant| {
        grant.condition == GrantCondition::RequiresApproval
            && grant.scope
                == (GrantScope::ExactCapability {
                    provider: AppId::new("com.example.notes"),
                    capability: CapabilityName::new("notes.create"),
                })
    }));
    let mut available_after_grant = kernel
        .available_capabilities_for(&AppId::new("chat"))
        .unwrap();
    // Simulate a later caller narrowing the turn's tool list. Permission state
    // must still come from the broker, not from this transient presentation.
    available_after_grant.retain(|view| view.provider_app_id != AppId::new("com.example.notes"));
    permissions_app::contextualize_tools(&kernel, &AppId::new("chat"), &mut available_after_grant)
        .unwrap();
    let active_after_grant = available_after_grant
        .iter()
        .find(|view| {
            view.provider_app_id == permissions_app::permissions_app_id()
                && view.capability == CapabilityName::new(permissions_app::LIST_ACTIVE)
        })
        .unwrap();
    assert!(
        active_after_grant.input_schema["properties"]["snapshot"]["const"]["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|permission| {
                permission["provider"] == "com.example.notes"
                    && permission["capability"] == "notes.create"
            })
    );
    let requestable_after_grant = available_after_grant
        .iter()
        .find(|view| {
            view.provider_app_id == permissions_app::permissions_app_id()
                && view.capability == CapabilityName::new(permissions_app::LIST_REQUESTABLE)
        })
        .unwrap();
    assert!(
        !requestable_after_grant.input_schema["properties"]["snapshot"]["const"]["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|permission| {
                permission["provider"] == "com.example.notes"
                    && permission["capability"] == "notes.create"
            })
    );
    assert!(available_after_grant.iter().any(|view| {
        view.provider_app_id == permissions_app::permissions_app_id()
            && view.capability == CapabilityName::new(permissions_app::PROPOSE_GRANT)
    }));
    drop(kernel);

    let repeated = tauri::async_runtime::block_on(submit_permission_proposal(
        HostState::direct(&host),
        artifact_id.clone(),
    ))
    .unwrap();
    assert!(matches!(
        repeated,
        PermissionProposalSubmission::AlreadyActive {
            effective_condition: GrantCondition::RequiresApproval,
            ..
        }
    ));

    host.kernel
        .lock()
        .unwrap()
        .issue_grant(
            &AppId::new("chat"),
            &GrantRequest {
                scope: GrantScope::ExactCapability {
                    provider: AppId::new("com.example.notes"),
                    capability: CapabilityName::new("notes.create"),
                },
                data_scope: DataScope::None,
                condition: GrantCondition::Silent,
                duration: GrantDuration::NonExpiring,
                reason: "User deliberately relaxed the permission policy".into(),
            },
        )
        .unwrap();
    let less_interactive = tauri::async_runtime::block_on(submit_permission_proposal(
        HostState::direct(&host),
        artifact_id,
    ))
    .unwrap();
    assert!(matches!(
        less_interactive,
        PermissionProposalSubmission::AlreadyActive {
            effective_condition: GrantCondition::Silent,
            ..
        }
    ));

    drop(host);
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn permission_proposal_is_hidden_when_every_installed_capability_is_granted() {
    let path = startup_test_dir("no-requestable-permissions");
    let host = build_test_host(path.clone()).unwrap();
    tauri::async_runtime::block_on(install_bundled_apps_phased(
        host.clone(),
        host.config.clone(),
        host.file_resources.clone(),
    ))
    .unwrap();

    let mut kernel = host.kernel.lock().unwrap();
    let providers = kernel
        .installed_apps()
        .filter(|app| !app.manifest.capabilities.is_empty())
        .map(|app| app.manifest.app_id.clone())
        .collect::<Vec<_>>();
    for provider in providers {
        kernel
            .issue_grant(
                &AppId::new("chat"),
                &GrantRequest {
                    scope: GrantScope::AllProviderCapabilities { provider },
                    data_scope: DataScope::None,
                    condition: GrantCondition::Silent,
                    duration: GrantDuration::NonExpiring,
                    reason: "Test complete permission catalog".into(),
                },
            )
            .unwrap();
    }

    let mut available = kernel
        .available_capabilities_for(&AppId::new("chat"))
        .unwrap();
    permissions_app::contextualize_tools(&kernel, &AppId::new("chat"), &mut available).unwrap();
    let requestable = available
        .iter()
        .find(|view| {
            view.provider_app_id == permissions_app::permissions_app_id()
                && view.capability == CapabilityName::new(permissions_app::LIST_REQUESTABLE)
        })
        .unwrap();
    assert!(
        requestable.input_schema["properties"]["snapshot"]["const"]["permissions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(!available.iter().any(|view| {
        view.provider_app_id == permissions_app::permissions_app_id()
            && view.capability == CapabilityName::new(permissions_app::PROPOSE_GRANT)
    }));

    drop(kernel);
    drop(host);
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn invalid_host_config_does_not_mutate_durable_kernel_state() {
    let path = startup_test_dir("invalid-config");
    let before = seed_stale_mcp_app(&path);
    std::fs::write(path.join("host-config.json"), b"{").unwrap();

    let error = match build_test_host(path.clone()) {
        Ok(_) => panic!("invalid host config must fail startup"),
        Err(error) => error,
    };

    assert!(error.contains("parse host config failed"));
    assert_eq!(
        std::fs::read(path.join("kernel-state-v1.json")).unwrap(),
        before
    );
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn successful_startup_reconciles_stale_mcp_apps() {
    let path = startup_test_dir("stale-mcp");
    seed_stale_mcp_app(&path);

    let host = build_test_host(path.clone()).unwrap();

    assert!(host
        .kernel
        .lock()
        .unwrap()
        .installed_app(&AppId::new("mcp-stale"))
        .is_err());
    drop(host);

    let store =
        kernel_state::FileKernelStateStore::open(path.join("kernel-state-v1.json")).unwrap();
    let kernel = Kernel::with_state_store(Arc::new(StartupTestChrome), store).unwrap();
    assert!(kernel.installed_app(&AppId::new("mcp-stale")).is_err());
    drop(kernel);
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn harmless_bundled_app_manifest_upgrade_preserves_authority() {
    let path = startup_test_dir("llm-provider-drift");
    let host = build_test_host(path.clone()).unwrap();
    {
        let mut kernel = host.kernel.lock().unwrap();
        let mut old_provider = llm_provider::llm_provider_manifest();
        old_provider.version = "0.0.9".into();
        let prepared = kernel
            .prepare_install(
                seal(old_provider),
                llm_provider::llm_provider_handlers(host.config.clone()),
            )
            .unwrap();
        kernel.commit_install(prepared.await_approval()).unwrap();
    }

    tauri::async_runtime::block_on(install_bundled_apps_phased(
        host.clone(),
        host.config.clone(),
        host.file_resources.clone(),
    ))
    .unwrap();

    let kernel = host.kernel.lock().unwrap();
    assert_eq!(
        kernel
            .installed_app(&llm_provider::llm_provider_app_id())
            .unwrap()
            .manifest
            .version,
        llm_provider::llm_provider_manifest().version
    );
    drop(kernel);
    drop(host);
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn agent_worker_package_installs_and_grants_chat() {
    let path = startup_test_dir("agent-worker-package");
    let host = build_test_host(path.clone()).unwrap();
    tauri::async_runtime::block_on(install_bundled_apps_phased(
        host.clone(),
        host.config.clone(),
        host.file_resources.clone(),
    ))
    .unwrap();
    let package_dir = path.join("agent-package");
    write_agent_worker_package(&package_dir);
    let (record, prepared) = {
        let mut manager = host.app_manager.lock().unwrap();
        let inspection = manager.inspect(&package_dir).unwrap();
        assert!(inspection.installable);
        assert_eq!(inspection.signature.label(), "unsigned");
        let record = manager
            .install_record(
                &inspection.staged_id,
                &inspection.package_digest,
                "2026-07-17T00:00:00Z",
            )
            .unwrap();
        assert_eq!(record.revisions[0].signature_verdict, "unsigned");
        assert!(record.revisions[0].signature_key_id.is_none());
        let prepared = manager
            .prepare_activation_with_invoker(&record.id, host.kernel_invoker.clone())
            .unwrap();
        (record, prepared)
    };

    tauri::async_runtime::block_on(activate_managed_app(
        host.clone(),
        record.id.clone(),
        prepared,
    ))
    .unwrap();

    let kernel = host.kernel.lock().unwrap();
    assert!(kernel
        .installed_app(&AppId::new("com.example.agent-worker"))
        .is_ok());
    assert!(kernel.grants_for(&AppId::new("chat")).iter().any(|grant| {
        grant.scope
            == GrantScope::ExactCapability {
                provider: AppId::new("com.example.agent-worker"),
                capability: CapabilityName::new("agent.run"),
            }
    }));
    drop(kernel);
    drop(host);
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn tool_feedback_surfaces_completed_tool_use_as_compact_status() {
    let run_id = RunId::new("run-parent");
    let tool_run = RunId::new("run-tool");
    let records = vec![
        record(
            LedgerEvent::RunStarted {
                run_id: run_id.clone(),
                initiator: app_host_kernel::primitives::run::Initiator::App {
                    app_id: AppId::new("chat"),
                    reason: "chat".into(),
                },
                goal: "chat".into(),
            },
            0,
        ),
        record(
            LedgerEvent::RunStarted {
                run_id: tool_run.clone(),
                initiator: app_host_kernel::primitives::run::Initiator::Run {
                    app_id: AppId::new("chat"),
                    parent_run_id: run_id.clone(),
                },
                goal: "tool: notes__create".into(),
            },
            1,
        ),
        record(
            LedgerEvent::CapabilityCompleted {
                run_id: tool_run.clone(),
                capability: tool_capability(),
                grant_id: GrantId::new("grant-1"),
                result_sha256: "digest".into(),
                data_scope: DataScope::None,
            },
            2,
        ),
        record(
            LedgerEvent::ArtifactProduced {
                run_id: tool_run.clone(),
                artifact_id: ArtifactId::new("artifact-1"),
                artifact_type: ArtifactTypeName::new("note"),
            },
            3,
        ),
    ];

    let feedback = tool_feedback(&records, run_id.as_str());

    assert_eq!(feedback.len(), 1);
    assert_eq!(feedback[0].role, ChatMessageRole::ToolStatus);
    assert_eq!(feedback[0].status, Some(ChatMessageStatus::Completed));
    assert_eq!(
        feedback[0].text,
        "Used notes / create and produced 1 artifact."
    );
}

#[test]
fn tool_feedback_excludes_agent_runtime_but_keeps_descendants() {
    let run_id = RunId::new("run-parent");
    let agent_run = RunId::new("run-agent");
    let note_run = RunId::new("run-note");
    let records = vec![
        record(
            LedgerEvent::RunStarted {
                run_id: run_id.clone(),
                initiator: app_host_kernel::primitives::run::Initiator::App {
                    app_id: AppId::new("chat"),
                    reason: "chat".into(),
                },
                goal: "chat".into(),
            },
            0,
        ),
        record(
            LedgerEvent::RunStarted {
                run_id: agent_run.clone(),
                initiator: app_host_kernel::primitives::run::Initiator::Run {
                    app_id: AppId::new("chat"),
                    parent_run_id: run_id.clone(),
                },
                goal: "agent.run".into(),
            },
            1,
        ),
        record(
            LedgerEvent::RunStarted {
                run_id: note_run.clone(),
                initiator: app_host_kernel::primitives::run::Initiator::Run {
                    app_id: AppId::new("com.example.agent-worker"),
                    parent_run_id: agent_run.clone(),
                },
                goal: "tool: notes__create".into(),
            },
            2,
        ),
        record(
            LedgerEvent::CapabilityCompleted {
                run_id: agent_run,
                capability: CapabilityRef {
                    provider: AppId::new("com.example.agent-worker"),
                    capability: CapabilityName::new("agent.run"),
                },
                grant_id: GrantId::new("grant-agent"),
                result_sha256: "digest".into(),
                data_scope: DataScope::None,
            },
            3,
        ),
        record(
            LedgerEvent::CapabilityCompleted {
                run_id: note_run.clone(),
                capability: tool_capability(),
                grant_id: GrantId::new("grant-note"),
                result_sha256: "digest".into(),
                data_scope: DataScope::None,
            },
            4,
        ),
        record(
            LedgerEvent::ArtifactProduced {
                run_id: note_run.clone(),
                artifact_id: ArtifactId::new("artifact-note"),
                artifact_type: ArtifactTypeName::new("note"),
            },
            5,
        ),
    ];

    let feedback = tool_feedback(&records, run_id.as_str());

    assert_eq!(feedback.len(), 1);
    assert_eq!(feedback[0].run_id.as_deref(), Some(note_run.as_str()));
    assert_eq!(
        feedback[0].text,
        "Used notes / create and produced 1 artifact."
    );
}

#[test]
fn build_assistant_messages_prepends_tool_status_hint() {
    let reply = crate::chat_app::ChatReply {
        text: "delegated answer".into(),
        reasoning: None,
        run_id: Some(RunId::new("run-parent")),
        artifacts: vec![],
    };
    let hint = crate::chat_store::ChatMessage {
        message_id: String::new(),
        role: ChatMessageRole::ToolStatus,
        text: "restore permission".into(),
        reasoning: None,
        run_id: None,
        artifact_ids: vec![],
        status: Some(ChatMessageStatus::Completed),
        client_request_id: None,
        created_at: String::new(),
        completed_at: None,
    };

    let messages = crate::chat_runtime::build_assistant_messages(&[], reply, Some(hint));

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, ChatMessageRole::ToolStatus);
    assert_eq!(messages[0].text, "restore permission");
    assert_eq!(messages[1].role, ChatMessageRole::Assistant);
}

#[test]
fn tool_feedback_surfaces_refused_tool_calls() {
    let run_id = RunId::new("run-parent");
    let tool_run = RunId::new("run-tool");
    let records = vec![
        record(
            LedgerEvent::RunStarted {
                run_id: run_id.clone(),
                initiator: app_host_kernel::primitives::run::Initiator::App {
                    app_id: AppId::new("chat"),
                    reason: "chat".into(),
                },
                goal: "chat".into(),
            },
            0,
        ),
        record(
            LedgerEvent::RunStarted {
                run_id: tool_run.clone(),
                initiator: app_host_kernel::primitives::run::Initiator::Run {
                    app_id: AppId::new("chat"),
                    parent_run_id: run_id.clone(),
                },
                goal: "tool: notes__create".into(),
            },
            1,
        ),
        record(
            LedgerEvent::InvocationRefused {
                run_id: tool_run,
                capability: tool_capability(),
                reason: DenialReason::NoGrant,
                data_scope: DataScope::None,
            },
            2,
        ),
    ];

    let feedback = tool_feedback(&records, run_id.as_str());

    assert_eq!(feedback.len(), 1);
    assert_eq!(feedback[0].status, Some(ChatMessageStatus::Failed));
    assert!(feedback[0].text.contains("Open Settings -> Permissions"));
}

#[test]
fn tool_feedback_points_file_scope_refusals_to_file_resources() {
    let run_id = RunId::new("run-parent");
    let tool_run = RunId::new("run-tool");
    let records = vec![
        record(
            LedgerEvent::RunStarted {
                run_id: run_id.clone(),
                initiator: app_host_kernel::primitives::run::Initiator::App {
                    app_id: AppId::new("chat"),
                    reason: "chat".into(),
                },
                goal: "chat".into(),
            },
            0,
        ),
        record(
            LedgerEvent::RunStarted {
                run_id: tool_run.clone(),
                initiator: app_host_kernel::primitives::run::Initiator::Run {
                    app_id: AppId::new("chat"),
                    parent_run_id: run_id.clone(),
                },
                goal: "tool: file.read".into(),
            },
            1,
        ),
        record(
            LedgerEvent::InvocationRefused {
                run_id: tool_run,
                capability: CapabilityRef {
                    provider: crate::file_resources::file_broker_app_id(),
                    capability: CapabilityName::new("file.read"),
                },
                reason: DenialReason::Revoked,
                data_scope: DataScope::resources(vec![ResourceId::new("resource-missing")])
                    .unwrap(),
            },
            2,
        ),
    ];

    let feedback = tool_feedback(&records, run_id.as_str());

    assert_eq!(feedback.len(), 1);
    assert!(feedback[0].text.contains("Settings -> File resources"));
    assert!(!feedback[0].text.contains("not permitted right now"));
}

#[test]
fn tool_feedback_distinguishes_declined_approval_from_missing_permission() {
    let run_id = RunId::new("run-parent");
    let tool_run = RunId::new("run-tool");
    let records = vec![
        record(
            LedgerEvent::RunStarted {
                run_id: run_id.clone(),
                initiator: app_host_kernel::primitives::run::Initiator::App {
                    app_id: AppId::new("chat"),
                    reason: "chat".into(),
                },
                goal: "chat".into(),
            },
            0,
        ),
        record(
            LedgerEvent::RunStarted {
                run_id: tool_run.clone(),
                initiator: app_host_kernel::primitives::run::Initiator::Run {
                    app_id: AppId::new("chat"),
                    parent_run_id: run_id.clone(),
                },
                goal: "tool: notes.create".into(),
            },
            1,
        ),
        record(
            LedgerEvent::ApprovalDenied {
                run_id: tool_run,
                capability: tool_capability(),
                grant_id: GrantId::new("grant-1"),
                data_scope: DataScope::None,
            },
            2,
        ),
    ];

    let feedback = tool_feedback(&records, run_id.as_str());

    assert_eq!(feedback.len(), 1);
    assert_eq!(feedback[0].status, Some(ChatMessageStatus::Failed));
    assert!(feedback[0].text.contains("declined approval"));
    assert!(!feedback[0].text.contains("Settings -> Permissions"));
}

#[test]
fn failed_tool_call_is_scoped_to_tool_feedback() {
    let run_id = RunId::new("run-parent");
    let tool_run = RunId::new("run-tool");
    let records = vec![
        record(
            LedgerEvent::RunStarted {
                run_id: run_id.clone(),
                initiator: app_host_kernel::primitives::run::Initiator::App {
                    app_id: AppId::new("chat"),
                    reason: "chat".into(),
                },
                goal: "chat".into(),
            },
            0,
        ),
        record(
            LedgerEvent::RunStarted {
                run_id: tool_run.clone(),
                initiator: app_host_kernel::primitives::run::Initiator::Run {
                    app_id: AppId::new("chat"),
                    parent_run_id: run_id.clone(),
                },
                goal: "tool: notes__create".into(),
            },
            1,
        ),
        record(
            LedgerEvent::CapabilityFailed {
                run_id: tool_run,
                capability: tool_capability(),
                grant_id: GrantId::new("grant-1"),
                error: "disk full".into(),
                data_scope: DataScope::None,
            },
            2,
        ),
        record(
            LedgerEvent::RunEnded {
                run_id: run_id.clone(),
                terminal_state: app_host_kernel::primitives::run::RunTerminalState::Completed,
            },
            3,
        ),
    ];

    let feedback = tool_feedback(&records, run_id.as_str());

    assert_eq!(feedback.len(), 1);
    assert_eq!(feedback[0].status, Some(ChatMessageStatus::Failed));
    assert_eq!(feedback[0].text, "notes / create failed: disk full");
    let reply = crate::chat_app::ChatReply {
        text: "I could not save the note.".into(),
        reasoning: None,
        run_id: Some(run_id),
        artifacts: vec![],
    };

    let messages = crate::chat_runtime::build_assistant_messages(&records, reply, None);

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, ChatMessageRole::ToolStatus);
    assert_eq!(messages[0].status, Some(ChatMessageStatus::Failed));
    assert_eq!(messages[1].role, ChatMessageRole::Assistant);
    assert_eq!(messages[1].status, Some(ChatMessageStatus::Completed));
}

#[test]
fn failed_chat_run_still_marks_its_assistant_reply_failed() {
    let run_id = RunId::new("run-parent");
    let records = vec![record(
        LedgerEvent::RunEnded {
            run_id: run_id.clone(),
            terminal_state: app_host_kernel::primitives::run::RunTerminalState::Failed,
        },
        0,
    )];
    let reply = crate::chat_app::ChatReply {
        text: "Sorry, something went wrong.".into(),
        reasoning: None,
        run_id: Some(run_id),
        artifacts: vec![],
    };

    let messages = crate::chat_runtime::build_assistant_messages(&records, reply, None);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, ChatMessageRole::Assistant);
    assert_eq!(messages[0].status, Some(ChatMessageStatus::Failed));
}
