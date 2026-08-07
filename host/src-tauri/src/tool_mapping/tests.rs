use super::*;
use app_host_kernel::ids::{AppId, CapabilityName, ResourceId};
use app_host_kernel::invocation::CapabilityHandler;
use app_host_kernel::kernel::Kernel;
use app_host_kernel::manifest::{seal, AppManifest, GrantRequest};
use app_host_kernel::primitives::capability::{CapabilityDeclaration, CapabilityEffect};
use app_host_kernel::primitives::grant::{DataScope, GrantCondition};
use app_host_kernel::CapabilityAuthorizationView;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

struct TestChrome;

impl app_host_kernel::services::chrome::TrustedChrome for TestChrome {
    fn confirm_grant(
        &self,
        _prompt: app_host_kernel::services::chrome::GrantIssuancePrompt,
    ) -> app_host_kernel::services::chrome::ApprovalDecision {
        app_host_kernel::services::chrome::ApprovalDecision::Approved
    }
    fn approve_capability(
        &self,
        _prompt: app_host_kernel::services::chrome::CapabilityApprovalPrompt,
    ) -> app_host_kernel::services::chrome::ApprovalDecision {
        app_host_kernel::services::chrome::ApprovalDecision::Approved
    }
    fn confirm_event_subscriptions(
        &self,
        _prompt: app_host_kernel::services::chrome::EventSubscriptionPrompt,
    ) -> app_host_kernel::services::chrome::ApprovalDecision {
        app_host_kernel::services::chrome::ApprovalDecision::Approved
    }
    fn show_notice(
        &self,
        _notice: app_host_kernel::services::chrome::ChromeNotice,
    ) -> Result<(), app_host_kernel::services::chrome::ChromeNoticeError> {
        Ok(())
    }
}

#[test]
fn names_are_sanitized_correctly() {
    assert_eq!(
        cap_ref_to_tool_name(&CapabilityRef {
            provider: AppId::new("mcp-weather"),
            capability: CapabilityName::new("get_forecast"),
        }),
        "mcp_weather__get_forecast"
    );
    assert_eq!(
        cap_ref_to_tool_name(&CapabilityRef {
            provider: AppId::new("notes"),
            capability: CapabilityName::new("create_note"),
        }),
        "notes__create_note"
    );
}

#[test]
fn managed_proposal_scope_is_exact_under_an_all_resources_grant() {
    let proposal = crate::package::ManagedDataProposal {
        capability: CapabilityName::new("propose_item"),
        artifact_type: app_host_kernel::ids::ArtifactTypeName::new("item-proposal"),
        title: "Propose item".into(),
        description: "Reviewable item proposal".into(),
        target: crate::package::ManagedDataProposalTarget::Record {
            collection: "items".into(),
        },
        payload_schema: json!({"type": "object", "additionalProperties": false})
            .as_object()
            .unwrap()
            .clone(),
        max_payload_bytes: 1024,
    };
    let provider = AppId::new("com.example.data");
    let consumer = AppId::new("chat");
    let capability = CapabilityDeclaration {
        name: proposal.capability.clone(),
        description: proposal.description.clone(),
        input_schema: crate::package::managed_proposal_input_schema(&proposal),
        effect: CapabilityEffect::LocalWrite,
        output_schema: Some(crate::package::managed_proposal_artifact_schema(
            &provider, &proposal,
        )),
    };
    let capability_name = capability.name.clone();
    let manifest = AppManifest {
        app_id: provider.clone(),
        version: "1.0.0".into(),
        display_name: "Data".into(),
        description: "Data".into(),
        capabilities: vec![capability],
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
    let mut kernel = Kernel::new(Arc::new(TestChrome));
    let handler: CapabilityHandler = Box::new(|_, _| {
        Ok(app_host_kernel::invocation::CapabilityOutcome {
            result: Value::Null,
            artifacts: vec![],
        })
    });
    let prepared = kernel
        .prepare_install(seal(manifest), BTreeMap::from([(capability_name, handler)]))
        .unwrap();
    kernel.commit_install(prepared.await_approval()).unwrap();
    let consumer_manifest = AppManifest {
        app_id: consumer.clone(),
        version: "1.0.0".into(),
        display_name: "Chat".into(),
        description: "Chat".into(),
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
    };
    let prepared = kernel
        .prepare_install(seal(consumer_manifest), BTreeMap::new())
        .unwrap();
    kernel.commit_install(prepared.await_approval()).unwrap();
    kernel
        .issue_grant(
            &consumer,
            &GrantRequest {
                scope: app_host_kernel::primitives::grant::GrantScope::ExactCapability {
                    provider: provider.clone(),
                    capability: proposal.capability.clone(),
                },
                data_scope: DataScope::AllResources,
                condition: GrantCondition::Silent,
                reason: "proposal test".into(),
                duration: app_host_kernel::primitives::grant::GrantDuration::NonExpiring,
            },
        )
        .unwrap();
    let capability_ref = CapabilityRef {
        provider: provider.clone(),
        capability: proposal.capability,
    };
    let input = json!({"targetId": "11111111-1111-4111-8111-111111111111", "targetRevision": 1, "payload": {}})
        .as_object()
        .unwrap()
        .clone();
    assert_eq!(
        managed_data_invocation_data_scope(&kernel, &capability_ref, &input),
        Some(DataScope::Resources {
            resource_ids: vec![ResourceId::new(
                "app-data:com.example.data:items:record:11111111-1111-4111-8111-111111111111",
            )]
        })
    );
}

#[test]
fn file_broker_tools_expose_only_granted_resource_ids() {
    let view = CapabilityUseView {
        provider_app_id: AppId::new("com.ma-zierl.host.file-broker"),
        provider_display_name: "File Broker".into(),
        capability: CapabilityName::new("file.read"),
        description: "Read a registered file resource".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"resource_id": {"type": "string"}},
            "required": ["resource_id"]
        })
        .as_object()
        .unwrap()
        .clone(),
        authorizations: vec![
            CapabilityAuthorizationView {
                condition: GrantCondition::RequiresApproval,
                data_scope: DataScope::Resources {
                    resource_ids: vec![app_host_kernel::ids::ResourceId::new("resource-b")],
                },
            },
            CapabilityAuthorizationView {
                condition: GrantCondition::RequiresApproval,
                data_scope: DataScope::Resources {
                    resource_ids: vec![app_host_kernel::ids::ResourceId::new("resource-a")],
                },
            },
        ],
    };

    let tool = capability_view_to_tool_def_named(&view, "file_broker__file_read".into());

    assert_eq!(
        tool.function.parameters["properties"]["resource_id"]["enum"],
        json!(["resource-a", "resource-b"])
    );
}

#[test]
fn chat_hides_and_overrides_host_bound_thread_input() {
    let view = CapabilityUseView {
        provider_app_id: AppId::new("reading"),
        provider_display_name: "Reading".into(),
        capability: CapabilityName::new("list"),
        description: "List marks".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "thread_id": {
                    "type": "string",
                    "x-kestral-host-input": "current-chat-thread-id"
                },
                "query": {"type": "string"}
            },
            "required": ["thread_id", "query"],
            "additionalProperties": false
        })
        .as_object()
        .unwrap()
        .clone(),
        authorizations: vec![CapabilityAuthorizationView {
            condition: GrantCondition::Silent,
            data_scope: DataScope::None,
        }],
    };

    let tool = capability_view_to_chat_tool(&view, "reading__list".into(), Some("thread-7"))
        .unwrap()
        .unwrap();

    assert!(tool.definition.function.parameters["properties"]
        .get("thread_id")
        .is_none());
    assert_eq!(
        tool.definition.function.parameters["required"],
        json!(["query"])
    );
    assert_eq!(
        tool.binding.bind(JsonObject::from_iter([
            ("thread_id".into(), json!("forged")),
            ("query".into(), json!("read")),
        ])),
        JsonObject::from_iter([
            ("thread_id".into(), json!("thread-7")),
            ("query".into(), json!("read")),
        ])
    );
    assert!(
        capability_view_to_chat_tool(&view, "reading__list".into(), None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn chat_hides_and_injects_active_permission_snapshot() {
    let snapshot = json!({
        "holder": "chat",
        "permissions": [{
            "provider": "notes",
            "provider_display_name": "Notes",
            "capability": "read",
            "authorizations": [{
                "data_scope": {"kind": "none"},
                "condition": "silent"
            }]
        }],
        "omitted_count": 0
    });
    let view = CapabilityUseView {
        provider_app_id: crate::permissions_app::permissions_app_id(),
        provider_display_name: "Permissions".into(),
        capability: CapabilityName::new(crate::permissions_app::LIST_ACTIVE),
        description: "List active permissions".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "snapshot": {
                    "const": snapshot,
                    "x-kestral-host-input": "active-permissions"
                }
            },
            "required": ["snapshot"],
            "additionalProperties": false
        })
        .as_object()
        .unwrap()
        .clone(),
        authorizations: vec![CapabilityAuthorizationView {
            condition: GrantCondition::Silent,
            data_scope: DataScope::None,
        }],
    };

    let tool = capability_view_to_chat_tool(&view, "permissions__list_active".into(), None)
        .unwrap()
        .unwrap();

    assert!(tool.definition.function.parameters["properties"]
        .get("snapshot")
        .is_none());
    assert_eq!(tool.definition.function.parameters["required"], json!([]));
    assert_eq!(tool.binding.bind(JsonObject::new())["snapshot"], snapshot);
}

#[test]
fn chat_hides_and_injects_requestable_permission_snapshot() {
    let snapshot = json!({
        "holder": "chat",
        "permissions": [{
            "provider": "notes",
            "provider_display_name": "Notes",
            "capability": "notes.create",
            "description": "Create a note",
            "effect": "local-write"
        }],
        "omitted_count": 0
    });
    let view = CapabilityUseView {
        provider_app_id: crate::permissions_app::permissions_app_id(),
        provider_display_name: "Permissions".into(),
        capability: CapabilityName::new(crate::permissions_app::LIST_REQUESTABLE),
        description: "List requestable permissions".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "snapshot": {
                    "const": snapshot,
                    "x-kestral-host-input": "requestable-permissions"
                }
            },
            "required": ["snapshot"],
            "additionalProperties": false
        })
        .as_object()
        .unwrap()
        .clone(),
        authorizations: vec![CapabilityAuthorizationView {
            condition: GrantCondition::Silent,
            data_scope: DataScope::None,
        }],
    };

    let tool = capability_view_to_chat_tool(&view, "permissions__list_requestable".into(), None)
        .unwrap()
        .unwrap();

    assert!(tool.definition.function.parameters["properties"]
        .get("snapshot")
        .is_none());
    assert_eq!(tool.definition.function.parameters["required"], json!([]));
    assert_eq!(tool.binding.bind(JsonObject::new())["snapshot"], snapshot);
}

#[test]
fn chat_hides_and_injects_permission_proposal_candidates() {
    let snapshot = json!({
        "holder": "chat",
        "permissions": [{
            "provider": "notes",
            "provider_display_name": "Notes",
            "capability": "notes.create",
            "description": "Create a note",
            "effect": "local-write"
        }],
        "omitted_count": 0
    });
    let view = CapabilityUseView {
        provider_app_id: crate::permissions_app::permissions_app_id(),
        provider_display_name: "Permissions".into(),
        capability: CapabilityName::new(crate::permissions_app::PROPOSE_GRANT),
        description: "Propose a permission".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "provider": {"const": "notes"},
                "capability": {"const": "notes.create"},
                "reason": {"type": "string"},
                "snapshot": {
                    "const": snapshot,
                    "x-kestral-host-input": "requestable-permissions"
                }
            },
            "required": ["provider", "capability", "reason", "snapshot"],
            "additionalProperties": false
        })
        .as_object()
        .unwrap()
        .clone(),
        authorizations: vec![CapabilityAuthorizationView {
            condition: GrantCondition::Silent,
            data_scope: DataScope::None,
        }],
    };

    let tool = capability_view_to_chat_tool(&view, "permissions__propose_grant".into(), None)
        .unwrap()
        .unwrap();

    assert!(tool.definition.function.parameters["properties"]
        .get("snapshot")
        .is_none());
    assert_eq!(
        tool.binding.bind(JsonObject::from_iter([(
            "snapshot".into(),
            json!({"holder": "forged", "permissions": [], "omitted_count": 0}),
        )]))["snapshot"],
        snapshot
    );
}
