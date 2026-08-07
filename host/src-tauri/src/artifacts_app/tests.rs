use super::*;
use std::sync::{Arc, Mutex};

use app_host_kernel::invocation::{InvocationRequest, InvocationResult};
use app_host_kernel::kernel::{AuthorizeInvocation, Kernel, PrepareInvocation};
use app_host_kernel::manifest::{seal, GrantRequest};
use app_host_kernel::primitives::grant::{GrantCondition, GrantDuration, GrantOrigin, GrantScope};
use app_host_kernel::primitives::run::Initiator;
use app_host_kernel::services::broker::IssueResult;
use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, TrustedChrome,
};
use app_host_kernel::services::ledger::LedgerEvent;

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

fn consumer_manifest() -> AppManifest {
    AppManifest {
        app_id: AppId::new("com.example.artifact-consumer"),
        version: "1.0.0".into(),
        display_name: "Artifact consumer".into(),
        description: "Artifact query test consumer".into(),
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

fn install(
    kernel: &mut Kernel,
    manifest: AppManifest,
    handlers: BTreeMap<CapabilityName, CapabilityHandler>,
) {
    let prepared = kernel
        .prepare_install_with_grant_origin(seal(manifest), handlers, GrantOrigin::SystemBundled)
        .unwrap();
    kernel.commit_install(prepared.await_approval()).unwrap();
}

fn object(value: serde_json::Value) -> JsonObject {
    value.as_object().cloned().expect("test input is an object")
}

fn invoke(
    kernel: &mut Kernel,
    consumer: &AppId,
    capability: &app_host_kernel::primitives::capability::CapabilityRef,
    input: JsonObject,
    data_scope: DataScope,
) -> InvocationResult {
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: consumer.clone(),
                reason: "artifact access test".into(),
            },
            "artifact access test",
        )
        .unwrap();
    let prepared = match kernel
        .prepare_invocation(&run_id, capability, InvocationRequest { input, data_scope })
        .unwrap()
    {
        PrepareInvocation::Prepared(prepared) => prepared,
        PrepareInvocation::Refused(result) => return result,
    };
    let authorized = match kernel
        .authorize_invocation(prepared.await_approval())
        .unwrap()
    {
        AuthorizeInvocation::Authorized(authorized) => authorized,
        AuthorizeInvocation::Refused(result) => return result,
    };
    kernel.finalize_invocation(authorized.execute()).unwrap()
}

fn create_test_artifact(kernel: &mut Kernel, consumer: &AppId) -> ArtifactId {
    crate::test_app::install_test_app(
        kernel,
        Arc::new(Mutex::new(crate::test_app::TestAppStore::default())),
    )
    .unwrap();
    let capability = crate::test_app::test_capability_ref("create");
    kernel
        .issue_grant(
            consumer,
            &GrantRequest {
                scope: GrantScope::ExactCapability {
                    provider: capability.provider.clone(),
                    capability: capability.capability.clone(),
                },
                data_scope: DataScope::None,
                condition: GrantCondition::Silent,
                reason: "create artifact fixture".into(),
                duration: GrantDuration::NonExpiring,
            },
        )
        .unwrap();
    let result = invoke(
        kernel,
        consumer,
        &capability,
        object(serde_json::json!({"title": "Shared", "body": "artifact body"})),
        DataScope::None,
    );
    let InvocationResult::Completed { artifacts, .. } = result else {
        panic!("artifact fixture should complete: {result:?}");
    };
    artifacts
        .into_iter()
        .next()
        .expect("fixture should produce an artifact")
        .artifact_id
}

#[test]
fn bundled_manifest_identity_is_pinned_for_durable_installs() {
    assert_eq!(
        app_host_kernel::manifest::seal(artifacts_manifest()).content_hash,
        "965ca5472bc37a573cb04727398e0c8860ae7e639a4a2746874dc1709cb62fb2"
    );
}

#[test]
fn manifest_declares_strict_artifact_capabilities() {
    let manifest = artifacts_manifest();
    assert_eq!(manifest.app_id, artifacts_app_id());
    assert_eq!(manifest.capabilities.len(), 2);
    assert_eq!(
        manifest.capabilities[0].name,
        CapabilityName::new("artifacts.query")
    );
    assert_eq!(
        manifest.capabilities[1].name,
        CapabilityName::new("artifacts.read")
    );
}

#[test]
fn handlers_compile_with_strict_snapshots() {
    let handlers = artifacts_handlers();
    assert!(handlers.contains_key(&CapabilityName::new("artifacts.query")));
    assert!(handlers.contains_key(&CapabilityName::new("artifacts.read")));
}

#[test]
fn access_target_requests_query_and_read_without_capability_wildcards() {
    let requests =
        artifact_access_grant_requests(&AppId::new("chat"), &ArtifactAccessTarget::AllArtifacts);
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].data_scope, DataScope::AllResources);
    assert_eq!(requests[1].data_scope, DataScope::AllResources);
    assert!(requests
        .iter()
        .all(|request| matches!(request.scope, GrantScope::ExactCapability { .. })));
}

#[test]
fn exact_access_target_must_name_an_existing_artifact() {
    let kernel = Kernel::new(Arc::new(AllowChrome));
    let error = validate_access_target(
        &kernel,
        &ArtifactAccessTarget::Artifact {
            artifact_id: ArtifactId::new("artifact-missing"),
        },
    )
    .unwrap_err();

    assert_eq!(error, "unknown artifact 'artifact-missing'");
}

#[test]
fn artifact_tools_use_exact_ids_from_active_resource_grants() {
    let mut kernel = Kernel::new(Arc::new(AllowChrome));
    install(&mut kernel, artifacts_manifest(), artifacts_handlers());
    let consumer = consumer_manifest();
    let consumer_id = consumer.app_id.clone();
    install(&mut kernel, consumer, BTreeMap::new());
    let artifact_id = create_test_artifact(&mut kernel, &consumer_id);
    let target = ArtifactAccessTarget::Artifact {
        artifact_id: artifact_id.clone(),
    };
    for request in artifact_access_grant_requests(&consumer_id, &target) {
        kernel.issue_grant(&consumer_id, &request).unwrap();
    }

    let mut available = kernel.available_capabilities_for(&consumer_id).unwrap();
    contextualize_tools(&kernel, &mut available);
    let read_view = available
        .iter()
        .find(|view| view.capability == CapabilityName::new(ARTIFACTS_READ))
        .expect("read tool should be available");
    assert_eq!(
        read_view.input_schema["properties"]["artifact_id"]["enum"],
        serde_json::json!([artifact_id.to_string()])
    );

    let query = app_host_kernel::primitives::capability::CapabilityRef {
        provider: artifacts_app_id(),
        capability: CapabilityName::new(ARTIFACTS_QUERY),
    };
    let query_scope = crate::tool_mapping::invocation_data_scope(
        &kernel,
        &consumer_id,
        &query,
        &JsonObject::new(),
    );
    assert_eq!(
        query_scope,
        DataScope::resources(vec![ResourceId::new(artifact_id.to_string())]).unwrap()
    );
    let query_result = invoke(
        &mut kernel,
        &consumer_id,
        &query,
        JsonObject::new(),
        query_scope,
    );
    assert!(matches!(
        query_result,
        InvocationResult::Completed { result, .. }
            if result["items"][0]["artifact_id"] == artifact_id.to_string()
    ));

    let read = app_host_kernel::primitives::capability::CapabilityRef {
        provider: artifacts_app_id(),
        capability: CapabilityName::new(ARTIFACTS_READ),
    };
    let read_input = object(serde_json::json!({"artifact_id": artifact_id}));
    let read_scope =
        crate::tool_mapping::invocation_data_scope(&kernel, &consumer_id, &read, &read_input);
    let read_result = invoke(&mut kernel, &consumer_id, &read, read_input, read_scope);
    assert!(matches!(
        read_result,
        InvocationResult::Completed { result, .. }
            if result["content"]["body"] == "artifact body"
    ));
}

#[test]
fn artifact_access_rejects_unscoped_requests_instead_of_returning_empty_results() {
    let mut kernel = Kernel::new(Arc::new(AllowChrome));
    install(&mut kernel, artifacts_manifest(), artifacts_handlers());
    let consumer = consumer_manifest();
    let consumer_id = consumer.app_id.clone();
    install(&mut kernel, consumer, BTreeMap::new());
    let capability = app_host_kernel::primitives::capability::CapabilityRef {
        provider: artifacts_app_id(),
        capability: CapabilityName::new("artifacts.query"),
    };
    let grant = match kernel
        .issue_grant(
            &consumer_id,
            &GrantRequest {
                scope: GrantScope::ExactCapability {
                    provider: capability.provider.clone(),
                    capability: capability.capability.clone(),
                },
                data_scope: DataScope::None,
                condition: GrantCondition::Silent,
                reason: "Reproduce unscoped artifact query".into(),
                duration: GrantDuration::NonExpiring,
            },
        )
        .unwrap()
    {
        IssueResult::Issued(grant) => grant,
        IssueResult::Refused => panic!("test grant was refused"),
    };
    let mut available = kernel.available_capabilities_for(&consumer_id).unwrap();
    contextualize_tools(&kernel, &mut available);
    assert!(available
        .iter()
        .all(|view| view.provider_app_id != artifacts_app_id()));
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: consumer_id.clone(),
                reason: "artifact query regression".into(),
            },
            "Query artifacts without resource scope",
        )
        .unwrap();
    let request = InvocationRequest {
        input: JsonObject::new(),
        data_scope: DataScope::None,
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

    let result = kernel.finalize_invocation(authorized.execute()).unwrap();
    assert!(matches!(
        result,
        InvocationResult::Failed { error }
            if error == "artifact access requires exact artifact resource IDs"
    ));
    assert!(kernel.records().iter().any(|record| matches!(
        &record.event,
        LedgerEvent::RunStarted { initiator: Initiator::App { app_id, .. }, .. }
            if app_id == &consumer_id
    )));
    assert!(kernel.records().iter().any(|record| matches!(
        &record.event,
        LedgerEvent::CapabilityInvoked { grant_id, data_scope: DataScope::None, .. }
            if grant_id == &grant.grant_id
    )));
    assert!(kernel.records().iter().any(|record| matches!(
        &record.event,
        LedgerEvent::CapabilityFailed { grant_id, data_scope: DataScope::None, .. }
            if grant_id == &grant.grant_id
    )));
}
