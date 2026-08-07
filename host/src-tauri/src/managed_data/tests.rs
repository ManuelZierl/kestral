use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;

use app_host_kernel::ids::{AppId, ArtifactTypeName};
use app_host_kernel::invocation::{InvocationRequest, InvocationResult};
use app_host_kernel::kernel::{AuthorizeInvocation, Kernel, PrepareInvocation};
use app_host_kernel::manifest::ArtifactTypeDeclaration;
use app_host_kernel::manifest::{seal, AppManifest, GrantRequest};
use app_host_kernel::primitives::capability::{
    CapabilityDeclaration, CapabilityEffect, CapabilityRef,
};
use app_host_kernel::primitives::grant::{DataScope, GrantCondition, GrantDuration, GrantScope};
use app_host_kernel::primitives::run::Initiator;
use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, TrustedChrome,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde_json::{json, Value};
use sha2::Digest;

use crate::atomic_json::AtomicFileWriter;
use crate::package::{
    AppData, ManagedDataCollection, ManagedDataCollectionLimits, ManagedDataExport,
    ManagedDataExportOperation, ManagedDataIndex, ManagedDataOperation, ManagedDataProposal,
    ManagedDataProposalTarget, ManagedDataStoreLimits, ManagedDocumentCollection,
    ManagedDocumentCollectionLimits, ManagedDocumentOperation,
};

use super::{
    data_root, handlers_for_proposals, proposal_scope_from_input, record_resource_id, resource_id,
    ManagedDataCommand, ManagedDataListResult, ManagedDataMutation, ManagedDataRecord,
    ManagedDataRequest, ManagedDataStage, ManagedDataStageDocument, ManagedDataStore,
    ManagedDataV2DocumentOperation, ManagedDataV2Envelope, ManagedDataV2Read, ManagedDataV2Request,
    ManagedDocumentRecord, CONTRACT_BACKUP_FILE, V2_STORE_VERSION,
};

struct AllowChrome;

struct PartialBlobWriter;

impl AtomicFileWriter for PartialBlobWriter {
    fn write_and_sync(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        fs::write(path, &bytes[..bytes.len().min(2)])?;
        Err(io::Error::other("injected partial blob write failure"))
    }

    fn rename(&self, _from: &Path, _to: &Path) -> io::Result<()> {
        Err(io::Error::other("rename is not used by blob writes"))
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
}

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

fn object(value: Value) -> app_host_kernel::JsonObject {
    value.as_object().unwrap().clone()
}

fn public_v2_request(value: Value) -> ManagedDataV2Request {
    let ManagedDataCommand::V2 {
        contract_version: 2,
        request,
    } = serde_json::from_value(json!({
        "contractVersion": 2,
        "request": value,
    }))
    .unwrap()
    else {
        unreachable!()
    };
    request
}

fn contract(record_limit: u32) -> AppData {
    AppData::HostManaged {
        contract_version: 1,
        collections: BTreeMap::from([(
            "items".into(),
            ManagedDataCollection {
                schema: object(json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["group", "title"],
                    "properties": {
                        "group": {"type": "string"},
                        "title": {"type": "string"}
                    }
                })),
                indexes: vec![ManagedDataIndex {
                    name: "group".into(),
                    field: "group".into(),
                    value_schema: object(json!({"type": "string"})),
                    unique: false,
                }],
                operations: BTreeSet::from([
                    ManagedDataOperation::Get,
                    ManagedDataOperation::List,
                    ManagedDataOperation::Create,
                    ManagedDataOperation::Replace,
                    ManagedDataOperation::Delete,
                    ManagedDataOperation::Transaction,
                ]),
                limits: ManagedDataCollectionLimits {
                    records: record_limit,
                    record_bytes: 4096,
                    query_results: 10,
                },
            },
        )]),
        documents: BTreeMap::new(),
        limits: ManagedDataStoreLimits {
            total_bytes: 1024 * 1024,
            transaction_operations: 8,
            batch_operations: None,
        },
        exports: Vec::new(),
        proposals: Vec::new(),
    }
}

fn contract_v2() -> AppData {
    let mut data = contract(2048);
    let AppData::HostManaged { documents, .. } = &mut data else {
        unreachable!()
    };
    *documents = BTreeMap::from([(
        "scenes".into(),
        ManagedDocumentCollection {
            metadata_schema: object(json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["title"],
                "properties": {"title": {"type": "string"}}
            })),
            operations: BTreeSet::from([
                ManagedDocumentOperation::Get,
                ManagedDocumentOperation::List,
                ManagedDocumentOperation::Create,
                ManagedDocumentOperation::Replace,
                ManagedDocumentOperation::UpdateMetadata,
                ManagedDocumentOperation::Delete,
            ]),
            limits: ManagedDocumentCollectionLimits {
                documents: 10,
                metadata_bytes: 4096,
                content_bytes: 8 * 1024 * 1024,
            },
        },
    )]);
    if let AppData::HostManaged {
        contract_version,
        limits,
        ..
    } = &mut data
    {
        *contract_version = 2;
        limits.total_bytes = 64 * 1024 * 1024;
        limits.batch_operations = Some(2048);
    }
    data
}

fn proposal_contract() -> (
    AppData,
    ManagedDataProposal,
    CapabilityDeclaration,
    ArtifactTypeDeclaration,
) {
    let mut data = contract_v2();
    let proposal = ManagedDataProposal {
        capability: app_host_kernel::ids::CapabilityName::new("propose_item"),
        artifact_type: ArtifactTypeName::new("item-proposal"),
        title: "Propose item".into(),
        description: "Create a reviewable item proposal".into(),
        target: ManagedDataProposalTarget::Record {
            collection: "items".into(),
        },
        payload_schema: object(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["title"],
            "properties": {"title": {"type": "string", "maxLength": 120}}
        })),
        max_payload_bytes: 4096,
    };
    let capability = CapabilityDeclaration {
        name: proposal.capability.clone(),
        description: proposal.description.clone(),
        input_schema: crate::package::managed_proposal_input_schema(&proposal),
        effect: CapabilityEffect::LocalWrite,
        output_schema: Some(crate::package::managed_proposal_artifact_schema(
            &AppId::new("com.example.data"),
            &proposal,
        )),
    };
    let artifact = ArtifactTypeDeclaration {
        name: proposal.artifact_type.clone(),
        description: proposal.description.clone(),
        json_schema: crate::package::managed_proposal_artifact_schema(
            &AppId::new("com.example.data"),
            &proposal,
        ),
    };
    if let AppData::HostManaged { proposals, .. } = &mut data {
        proposals.push(proposal.clone());
    }
    (data, proposal, capability, artifact)
}

fn write_proposal_package(
    root: &std::path::Path,
    data: &AppData,
    capability: &CapabilityDeclaration,
    artifact: &ArtifactTypeDeclaration,
) -> String {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("app.json"),
        serde_json::to_vec_pretty(&json!({
            "format_version": 1,
            "id": "com.example.data",
            "version": "1.0.0",
            "display_name": "Managed data",
            "description": "Managed data proposal fixture",
            "min_host_version": "0.1.0-alpha.1",
            "manifest": {
                "capabilities": [capability],
                "artifact_types": [artifact]
            },
            "backend": {"kind": "none"},
            "data": data,
            "integrity": {"algorithm": "sha256", "assets": {}}
        }))
        .unwrap(),
    )
    .unwrap();
    crate::package::package_digest(root).unwrap()
}

#[test]
fn proposal_target_scopes_are_exact_for_collection_record_and_document_targets() {
    let app = AppId::new("com.example.data");
    let id = "11111111-1111-4111-8111-111111111111";
    let collection_scope = DataScope::resources(vec![app_host_kernel::ids::ResourceId::new(
        resource_id(&app, "items"),
    )])
    .unwrap();
    let record_scope = DataScope::resources(vec![app_host_kernel::ids::ResourceId::new(
        record_resource_id(&app, "items", id),
    )])
    .unwrap();
    let document_scope = DataScope::resources(vec![app_host_kernel::ids::ResourceId::new(
        super::document_resource_id(&app, "scenes", id),
    )])
    .unwrap();

    assert_eq!(
        proposal_scope_from_input(
            &app,
            &ManagedDataProposal {
                capability: app_host_kernel::ids::CapabilityName::new("propose_collection"),
                artifact_type: ArtifactTypeName::new("collection-proposal"),
                title: "Collection proposal".into(),
                description: "Collection proposal".into(),
                target: ManagedDataProposalTarget::Collection {
                    collection: "items".into(),
                },
                payload_schema: object(json!({"type": "object"})),
                max_payload_bytes: 1024,
            },
            &object(json!({})),
        )
        .unwrap(),
        collection_scope
    );
    assert_eq!(
        proposal_scope_from_input(
            &app,
            &ManagedDataProposal {
                capability: app_host_kernel::ids::CapabilityName::new("propose_record"),
                artifact_type: ArtifactTypeName::new("record-proposal"),
                title: "Record proposal".into(),
                description: "Record proposal".into(),
                target: ManagedDataProposalTarget::Record {
                    collection: "items".into(),
                },
                payload_schema: object(json!({"type": "object"})),
                max_payload_bytes: 1024,
            },
            &object(json!({"targetId": id})),
        )
        .unwrap(),
        record_scope
    );
    assert_eq!(
        proposal_scope_from_input(
            &app,
            &ManagedDataProposal {
                capability: app_host_kernel::ids::CapabilityName::new("propose_document"),
                artifact_type: ArtifactTypeName::new("document-proposal"),
                title: "Document proposal".into(),
                description: "Document proposal".into(),
                target: ManagedDataProposalTarget::Document {
                    document_collection: "scenes".into(),
                },
                payload_schema: object(json!({"type": "object"})),
                max_payload_bytes: 1024,
            },
            &object(json!({"targetId": id})),
        )
        .unwrap(),
        document_scope
    );
}

fn temp_root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("kestral-managed-data-{}", uuid::Uuid::new_v4()))
}

fn create(
    store: &ManagedDataStore,
    app: &AppId,
    data: &AppData,
    group: &str,
    title: &str,
) -> ManagedDataRecord {
    serde_json::from_value(
        store
            .request(
                app,
                data.host_managed().unwrap(),
                ManagedDataRequest::Create {
                    collection: "items".into(),
                    value: object(json!({"group": group, "title": title})),
                },
            )
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn crud_query_cas_and_restart_are_persistent() {
    let root = temp_root();
    let app = AppId::new("com.example.data");
    let data = contract(10);
    let store = ManagedDataStore::new(root.clone());
    let first = create(&store, &app, &data, "one", "First");
    create(&store, &app, &data, "two", "Second");

    let listed: ManagedDataListResult = serde_json::from_value(
        store
            .request(
                &app,
                data.host_managed().unwrap(),
                ManagedDataRequest::List {
                    collection: "items".into(),
                    query: Some(super::ManagedDataQuery {
                        index: Some("group".into()),
                        equals: Some(json!("one")),
                        after: None,
                        limit: Some(5),
                    }),
                },
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(listed.records, vec![first.clone()]);
    assert_eq!(listed.next_after, None);

    let restarted = ManagedDataStore::new(root.clone());
    let replaced: ManagedDataRecord = serde_json::from_value(
        restarted
            .request(
                &app,
                data.host_managed().unwrap(),
                ManagedDataRequest::Replace {
                    collection: "items".into(),
                    id: first.id.clone(),
                    expected_revision: 1,
                    value: object(json!({"group": "one", "title": "Updated"})),
                },
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(replaced.revision, 2);
    assert!(restarted
        .request(
            &app,
            data.host_managed().unwrap(),
            ManagedDataRequest::Replace {
                collection: "items".into(),
                id: first.id,
                expected_revision: 1,
                value: object(json!({"group": "one", "title": "Stale"})),
            },
        )
        .unwrap_err()
        .contains("expected revision 1, found 2"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unique_indexes_refuse_duplicate_creates_without_changing_the_store() {
    let root = temp_root();
    let app = AppId::new("com.example.data");
    let mut data = contract(10);
    let AppData::HostManaged { collections, .. } = &mut data else {
        unreachable!()
    };
    collections.get_mut("items").unwrap().indexes[0].unique = true;
    let store = ManagedDataStore::new(root.clone());
    let first = create(&store, &app, &data, "one", "First");

    let error = store
        .request(
            &app,
            data.host_managed().unwrap(),
            ManagedDataRequest::Create {
                collection: "items".into(),
                value: object(json!({"group": "one", "title": "Duplicate"})),
            },
        )
        .unwrap_err();
    assert!(error.contains("unique index 'group' already contains that value"));
    let second = create(&store, &app, &data, "two", "Second");
    let replace_error = store
        .request(
            &app,
            data.host_managed().unwrap(),
            ManagedDataRequest::Replace {
                collection: "items".into(),
                id: second.id.clone(),
                expected_revision: second.revision,
                value: object(json!({"group": "one", "title": "Duplicate"})),
            },
        )
        .unwrap_err();
    assert!(replace_error.contains("unique index 'group' already contains that value"));

    let listed: ManagedDataListResult = serde_json::from_value(
        store
            .request(
                &app,
                data.host_managed().unwrap(),
                ManagedDataRequest::List {
                    collection: "items".into(),
                    query: Some(super::ManagedDataQuery {
                        index: Some("group".into()),
                        equals: Some(json!("one")),
                        after: None,
                        limit: Some(10),
                    }),
                },
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(listed.records, vec![first]);

    let unchanged: Option<ManagedDataRecord> = serde_json::from_value(
        store
            .request(
                &app,
                data.host_managed().unwrap(),
                ManagedDataRequest::Get {
                    collection: "items".into(),
                    id: second.id,
                },
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(unchanged.unwrap().value["group"], "two");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn transactions_are_atomic_when_a_later_operation_fails() {
    let root = temp_root();
    let app = AppId::new("com.example.data");
    let data = contract(10);
    let store = ManagedDataStore::new(root.clone());
    let first = create(&store, &app, &data, "one", "Before");

    let error = store
        .request(
            &app,
            data.host_managed().unwrap(),
            ManagedDataRequest::Transaction {
                operations: vec![
                    ManagedDataMutation::Replace {
                        collection: "items".into(),
                        id: first.id.clone(),
                        expected_revision: 1,
                        value: object(json!({"group": "one", "title": "After"})),
                    },
                    ManagedDataMutation::Delete {
                        collection: "items".into(),
                        id: first.id.clone(),
                        expected_revision: 1,
                    },
                ],
            },
        )
        .unwrap_err();
    assert!(error.contains("expected revision 1, found 2"));
    let stored: Option<ManagedDataRecord> = serde_json::from_value(
        store
            .request(
                &app,
                data.host_managed().unwrap(),
                ManagedDataRequest::Get {
                    collection: "items".into(),
                    id: first.id,
                },
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(stored.unwrap().value["title"], "Before");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn schemas_and_collection_quotas_refuse_without_deleting_data() {
    let root = temp_root();
    let app = AppId::new("com.example.data");
    let data = contract(1);
    let store = ManagedDataStore::new(root.clone());
    let first = create(&store, &app, &data, "one", "First");
    assert!(store
        .request(
            &app,
            data.host_managed().unwrap(),
            ManagedDataRequest::Create {
                collection: "items".into(),
                value: object(json!({"group": "two", "title": "Second"})),
            },
        )
        .unwrap_err()
        .contains("1-record limit"));
    assert!(store
        .request(
            &app,
            data.host_managed().unwrap(),
            ManagedDataRequest::Replace {
                collection: "items".into(),
                id: first.id.clone(),
                expected_revision: 1,
                value: object(json!({"group": "one", "unknown": true})),
            },
        )
        .unwrap_err()
        .contains("does not match its schema"));
    let stored: Option<ManagedDataRecord> = serde_json::from_value(
        store
            .request(
                &app,
                data.host_managed().unwrap(),
                ManagedDataRequest::Get {
                    collection: "items".into(),
                    id: first.id,
                },
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(stored.unwrap().value["title"], "First");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn compatible_contract_changes_are_backed_up_before_the_first_write() {
    let root = temp_root();
    let app = AppId::new("com.example.data");
    let original = contract(10);
    let store = ManagedDataStore::new(root.clone());
    let first = create(&store, &app, &original, "one", "First");
    let changed = contract(20);
    let changed_contract = changed.host_managed().unwrap();
    store.validate_contract(&app, changed_contract).unwrap();
    store
        .request(
            &app,
            changed.host_managed().unwrap(),
            ManagedDataRequest::Replace {
                collection: "items".into(),
                id: first.id,
                expected_revision: 1,
                value: object(json!({"group": "one", "title": "Updated"})),
            },
        )
        .unwrap();
    assert!(root.join(app.as_str()).join(CONTRACT_BACKUP_FILE).is_file());

    // Keep the binding immutable after use; this also confirms the declaration
    // remains ordinary package data rather than live mutable registry state.
    assert!(matches!(changed, AppData::HostManaged { .. }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn independent_store_instances_serialize_one_apps_mutations() {
    let root = temp_root();
    let app = AppId::new("com.example.data");
    let data = contract(32);
    let workers: Vec<_> = (0..16)
        .map(|index| {
            let root = if index % 2 == 0 {
                root.clone()
            } else {
                root.parent()
                    .unwrap()
                    .join(".")
                    .join(root.file_name().unwrap())
            };
            let app = app.clone();
            let data = data.clone();
            std::thread::spawn(move || {
                create(
                    &ManagedDataStore::new(root),
                    &app,
                    &data,
                    "shared",
                    &format!("Item {index}"),
                )
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let store = ManagedDataStore::new(root.clone());
    let mut after = None;
    let mut count = 0;
    loop {
        let listed: ManagedDataListResult = serde_json::from_value(
            store
                .request(
                    &app,
                    data.host_managed().unwrap(),
                    ManagedDataRequest::List {
                        collection: "items".into(),
                        query: Some(super::ManagedDataQuery {
                            index: None,
                            equals: None,
                            after,
                            limit: Some(10),
                        }),
                    },
                )
                .unwrap(),
        )
        .unwrap();
        count += listed.records.len();
        let Some(cursor) = listed.next_after else {
            break;
        };
        after = Some(cursor);
    }
    assert_eq!(count, 16);

    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn linked_app_directories_are_refused() {
    use std::os::unix::fs::symlink;

    let root = temp_root();
    let outside = temp_root();
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(&root).unwrap();
    let app = AppId::new("com.example.data");
    symlink(&outside, root.join(app.as_str())).unwrap();

    let error = ManagedDataStore::new(root.clone())
        .request(
            &app,
            contract(10).host_managed().unwrap(),
            ManagedDataRequest::Create {
                collection: "items".into(),
                value: object(json!({"group": "one", "title": "Redirected"})),
            },
        )
        .unwrap_err();

    assert!(error.contains("is a symlink"), "{error}");
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn startup_refuses_oversized_stores_before_parsing() {
    let root = temp_root();
    let app = AppId::new("com.example.data");
    let app_root = root.join(app.as_str());
    fs::create_dir_all(&app_root).unwrap();
    let file = fs::File::create(app_root.join(super::STORE_FILE)).unwrap();
    file.set_len((super::MAX_DOCUMENT_BYTES + 1) as u64)
        .unwrap();

    let error = ManagedDataStore::validate_all(&root).unwrap_err();

    assert!(
        error.contains("exceeds the 67108864-byte host limit"),
        "{error}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn generated_reads_use_exact_resource_grants_and_the_kernel_action_path() {
    let apps_root = temp_root();
    let root = super::data_root(&apps_root);
    let app = AppId::new("com.example.data");
    let consumer = AppId::new("com.example.consumer");
    let mut data = contract(10);
    let collection = match &mut data {
        AppData::HostManaged {
            collections,
            documents: _,
            exports,
            ..
        } => {
            exports.push(ManagedDataExport {
                capability: app_host_kernel::ids::CapabilityName::new("list_items"),
                operation: ManagedDataExportOperation::List,
                collection: "items".into(),
                index: Some("group".into()),
                equals_host_input: None,
            });
            collections.get("items").unwrap().clone()
        }
        _ => unreachable!(),
    };
    let capability = CapabilityDeclaration {
        name: app_host_kernel::ids::CapabilityName::new("list_items"),
        description: "List items by group".into(),
        input_schema: crate::package::managed_export_input_schema(
            ManagedDataExportOperation::List,
            &collection,
            collection.indexes.first(),
            None,
        ),
        effect: CapabilityEffect::ReadOnly,
        output_schema: Some(crate::package::managed_export_output_schema(
            ManagedDataExportOperation::List,
            &collection,
        )),
    };
    let store = ManagedDataStore::new(root.clone());
    create(&store, &app, &data, "one", "First");

    let handlers = super::handlers_for_exports(&apps_root, &app, &data).unwrap();
    let mut kernel = Kernel::new(Arc::new(AllowChrome));
    install(
        &mut kernel,
        manifest(app.clone(), vec![capability]),
        handlers,
    );
    install(
        &mut kernel,
        manifest(consumer.clone(), Vec::new()),
        BTreeMap::new(),
    );
    let reference = CapabilityRef {
        provider: app.clone(),
        capability: app_host_kernel::ids::CapabilityName::new("list_items"),
    };
    let resource = app_host_kernel::ids::ResourceId::new(super::resource_id(&app, "items"));
    kernel
        .issue_grant(
            &consumer,
            &GrantRequest {
                scope: GrantScope::ExactCapability {
                    provider: app,
                    capability: reference.capability.clone(),
                },
                data_scope: DataScope::resources(vec![resource.clone()]).unwrap(),
                condition: GrantCondition::Silent,
                reason: "Read the selected app collection".into(),
                duration: GrantDuration::NonExpiring,
            },
        )
        .unwrap();
    let result = invoke(
        &mut kernel,
        &consumer,
        &reference,
        object(json!({"equals": "one"})),
        DataScope::resources(vec![resource]).unwrap(),
    );
    let InvocationResult::Completed { result, .. } = result else {
        panic!("generated managed-data read should complete: {result:?}");
    };
    assert_eq!(result["records"][0]["value"]["title"], "First");

    fs::remove_dir_all(apps_root).unwrap();
}

#[test]
fn proposal_handler_is_read_only_over_managed_data_and_produces_provenance_artifacts() {
    let apps_root = temp_root();
    let package_root = temp_root();
    let app = AppId::new("com.example.data");
    let consumer = AppId::new("com.example.consumer");
    let (data, proposal, capability, artifact) = proposal_contract();
    let digest = write_proposal_package(&package_root, &data, &capability, &artifact);
    let store = ManagedDataStore::new(data_root(&apps_root));
    let created: Value = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::Create {
                mutation_id: "create-item".into(),
                expected_generation: 0,
                collection: "items".into(),
                value: object(json!({"group": "one", "title": "Original"})),
            },
        )
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let revision = created["revision"].as_u64().unwrap();
    let generation_before = created["revision"].as_u64().unwrap();

    let handlers = handlers_for_proposals(&apps_root, &package_root, &app, &data, &digest).unwrap();
    let mut provider_manifest = manifest(app.clone(), vec![capability.clone()]);
    provider_manifest.artifact_types = vec![artifact];
    let mut kernel = Kernel::new(Arc::new(AllowChrome));
    install(&mut kernel, provider_manifest, handlers);
    install(
        &mut kernel,
        manifest(consumer.clone(), Vec::new()),
        BTreeMap::new(),
    );
    let reference = CapabilityRef {
        provider: app.clone(),
        capability: proposal.capability.clone(),
    };
    let scope = DataScope::resources(vec![app_host_kernel::ids::ResourceId::new(
        record_resource_id(&app, "items", &id),
    )])
    .unwrap();
    kernel
        .issue_grant(
            &consumer,
            &GrantRequest {
                scope: GrantScope::ExactCapability {
                    provider: app.clone(),
                    capability: proposal.capability.clone(),
                },
                data_scope: DataScope::AllResources,
                condition: GrantCondition::Silent,
                reason: "Create a reviewable proposal".into(),
                duration: GrantDuration::NonExpiring,
            },
        )
        .unwrap();
    let result = invoke(
        &mut kernel,
        &consumer,
        &reference,
        object(json!({
            "targetId": id,
            "targetRevision": revision,
            "payload": {"title": "Suggested"}
        })),
        scope.clone(),
    );
    let InvocationResult::Completed { artifacts, result } = result else {
        panic!("proposal should complete: {result:?}");
    };
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].artifact_type, proposal.artifact_type);
    assert_eq!(artifacts[0].content["targetAppId"], "com.example.data");
    assert_eq!(artifacts[0].content["targetKind"], "record");
    assert_eq!(artifacts[0].content["payload"]["title"], "Suggested");
    assert_eq!(result, artifacts[0].content);
    assert_eq!(artifacts[0].provenance.produced_by, app);
    assert_eq!(
        artifacts[0].provenance.capability.capability,
        proposal.capability
    );
    assert_eq!(
        artifacts[0].provenance.grant_id,
        kernel.grants_for(&consumer)[0].grant_id
    );

    let unchanged = store
        .request_v2(
            &AppId::new("com.example.data"),
            data.host_managed().unwrap(),
            ManagedDataV2Request::Get {
                collection: "items".into(),
                id: artifacts[0].content["resourceId"]
                    .as_str()
                    .unwrap()
                    .rsplit(':')
                    .next()
                    .unwrap()
                    .into(),
                expected_generation: None,
            },
        )
        .unwrap();
    assert_eq!(unchanged["record"]["value"]["title"], "Original");
    assert_eq!(generation_before, 1);
    assert_eq!(
        artifacts[0].provenance.run_id,
        artifacts[0].provenance.run_id
    );

    let wrong_scope = invoke(
        &mut kernel,
        &consumer,
        &reference,
        object(json!({
            "targetId": artifacts[0].content["resourceId"].as_str().unwrap().rsplit(':').next().unwrap(),
            "targetRevision": revision,
            "payload": {"title": "Wrong scope"}
        })),
        DataScope::resources(vec![app_host_kernel::ids::ResourceId::new(resource_id(
            &app, "items",
        ))])
        .unwrap(),
    );
    assert!(matches!(wrong_scope, InvocationResult::Failed { .. }));

    let _ = fs::remove_dir_all(apps_root);
    let _ = fs::remove_dir_all(package_root);
}

#[test]
fn proposal_handler_rejects_stale_missing_malformed_and_changed_contract_targets() {
    let apps_root = temp_root();
    let package_root = temp_root();
    let app = AppId::new("com.example.data");
    let consumer = AppId::new("com.example.consumer");
    let (data, proposal, capability, artifact) = proposal_contract();
    let digest = write_proposal_package(&package_root, &data, &capability, &artifact);
    let store = ManagedDataStore::new(data_root(&apps_root));
    let created: Value = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::Create {
                mutation_id: "create-item".into(),
                expected_generation: 0,
                collection: "items".into(),
                value: object(json!({"group": "one", "title": "Original"})),
            },
        )
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let resource = record_resource_id(&app, "items", &id);
    let handlers = handlers_for_proposals(&apps_root, &package_root, &app, &data, &digest).unwrap();
    let mut provider_manifest = manifest(app.clone(), vec![capability.clone()]);
    provider_manifest.artifact_types = vec![artifact];
    let mut kernel = Kernel::new(Arc::new(AllowChrome));
    install(&mut kernel, provider_manifest, handlers);
    install(
        &mut kernel,
        manifest(consumer.clone(), Vec::new()),
        BTreeMap::new(),
    );
    let reference = CapabilityRef {
        provider: app.clone(),
        capability: proposal.capability.clone(),
    };
    let scope =
        DataScope::resources(vec![app_host_kernel::ids::ResourceId::new(resource)]).unwrap();
    kernel
        .issue_grant(
            &consumer,
            &GrantRequest {
                scope: GrantScope::ExactCapability {
                    provider: app.clone(),
                    capability: proposal.capability.clone(),
                },
                data_scope: scope.clone(),
                condition: GrantCondition::Silent,
                reason: "proposal test".into(),
                duration: GrantDuration::NonExpiring,
            },
        )
        .unwrap();
    let stale = invoke(
        &mut kernel,
        &consumer,
        &reference,
        object(json!({
            "targetId": id, "targetRevision": 99, "payload": {"title": "stale"}
        })),
        scope.clone(),
    );
    assert!(matches!(stale, InvocationResult::Failed { .. }));

    let missing_id = uuid::Uuid::new_v4().to_string();
    let missing = invoke(
        &mut kernel,
        &consumer,
        &reference,
        object(json!({
            "targetId": missing_id, "targetRevision": 1, "payload": {"title": "missing"}
        })),
        scope.clone(),
    );
    assert!(matches!(missing, InvocationResult::Failed { .. }));

    let malformed = kernel
        .start_run(
            Initiator::App {
                app_id: consumer.clone(),
                reason: "malformed proposal".into(),
            },
            "malformed",
        )
        .unwrap();
    assert!(kernel
        .prepare_invocation(
            &malformed,
            &reference,
            InvocationRequest {
                input: object(
                    json!({"targetId": id, "targetRevision": 1, "payload": {"title": 7}})
                ),
                data_scope: scope.clone(),
            }
        )
        .is_err());
    kernel
        .end_run(
            &malformed,
            app_host_kernel::primitives::run::RunTerminalState::Failed,
        )
        .unwrap();

    let mut package_value: Value =
        serde_json::from_slice(&fs::read(package_root.join("app.json")).unwrap()).unwrap();
    package_value["description"] = json!("changed contract");
    fs::write(
        package_root.join("app.json"),
        serde_json::to_vec(&package_value).unwrap(),
    )
    .unwrap();
    let changed = invoke(
        &mut kernel,
        &consumer,
        &reference,
        object(json!({
            "targetId": id, "targetRevision": 1, "payload": {"title": "changed"}
        })),
        scope,
    );
    assert!(matches!(changed, InvocationResult::Failed { .. }));

    let _ = fs::remove_dir_all(apps_root);
    let _ = fs::remove_dir_all(package_root);
}

#[test]
fn v2_generation_idempotency_and_atomic_document_batches() {
    let root = temp_root();
    let app = AppId::new("com.example.data");
    let data = contract_v2();
    let store = ManagedDataStore::new(root.clone());
    let create = ManagedDataV2Request::Create {
        mutation_id: "record-create".into(),
        expected_generation: 0,
        collection: "items".into(),
        value: object(json!({"group": "one", "title": "First"})),
    };
    let first = store
        .request_v2(&app, data.host_managed().unwrap(), create.clone())
        .unwrap();
    let restarted = ManagedDataStore::new(root.clone());
    assert_eq!(
        restarted
            .request_v2(&app, data.host_managed().unwrap(), create)
            .unwrap(),
        first
    );
    assert!(store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::Create {
                mutation_id: "record-create".into(),
                expected_generation: 0,
                collection: "items".into(),
                value: object(json!({"group": "one", "title": "Different"})),
            },
        )
        .unwrap_err()
        .contains("reused with a different request"));
    assert!(store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::Create {
                mutation_id: "record-create-2".into(),
                expected_generation: 0,
                collection: "items".into(),
                value: object(json!({"group": "one", "title": "Second"})),
            },
        )
        .unwrap_err()
        .contains("generation conflict"));

    let content = b"hello";
    let content_hash = format!("sha256-{:x}", sha2::Sha256::digest(content));
    let batch: Value = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::BeginBatch {
                mutation_id: "batch-begin".into(),
                expected_generation: 1,
                operations: Vec::new(),
                documents: vec![ManagedDataV2DocumentOperation::Create {
                    stage_id: "scene".into(),
                    collection: "scenes".into(),
                    metadata: object(json!({"title": "Scene"})),
                    content_length: content.len() as u64,
                    content_sha256: content_hash.clone(),
                }],
            },
        )
        .unwrap();
    let batch_id = batch["batchId"].as_str().unwrap().to_string();
    assert_eq!(batch["documents"][0]["stageId"], "scene");
    let document_id = batch["documents"][0]["documentId"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::GetDocument {
                collection: "scenes".into(),
                id: document_id.clone(),
                offset: 0,
                length: 5,
                expected_generation: Some(1),
            },
        )
        .unwrap_err()
        .contains("does not exist"));
    store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::AppendDocumentChunk {
                mutation_id: "chunk-1".into(),
                batch_id: batch_id.clone(),
                document_id: document_id.clone(),
                chunk_index: 0,
                content_base64: BASE64.encode(content),
            },
        )
        .unwrap();
    let committed = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::CommitBatch {
                mutation_id: "batch-commit".into(),
                batch_id: batch_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(committed["generation"], 2);
    assert!(store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::GetDocument {
                collection: "scenes".into(),
                id: document_id.clone(),
                offset: 0,
                length: 5,
                expected_generation: Some(1),
            },
        )
        .unwrap_err()
        .contains("generation conflict"));
    assert_eq!(
        store
            .request_v2(
                &app,
                data.host_managed().unwrap(),
                ManagedDataV2Request::CommitBatch {
                    mutation_id: "batch-commit".into(),
                    batch_id,
                },
            )
            .unwrap(),
        committed
    );
    let chunk = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::GetDocument {
                collection: "scenes".into(),
                id: document_id.clone(),
                offset: 0,
                length: 5,
                expected_generation: Some(2),
            },
        )
        .unwrap();
    assert_eq!(chunk["contentBase64"], BASE64.encode(content));
    let snapshot = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::ReadSnapshot {
                expected_generation: Some(2),
                reads: vec![
                    ManagedDataV2Read::RecordList {
                        collection: "items".into(),
                        query: None,
                    },
                    ManagedDataV2Read::DocumentGet {
                        collection: "scenes".into(),
                        id: document_id,
                    },
                ],
            },
        )
        .unwrap();
    assert_eq!(snapshot["generation"], 2);
    assert_eq!(snapshot["results"][0]["kind"], "record-list");
    assert_eq!(snapshot["results"][1]["kind"], "document-get");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn v2_commit_removes_only_new_blobs_after_a_later_stale_document_failure() {
    let root = temp_root();
    let app = AppId::new("com.example.blob-rollback");
    let data = contract_v2();
    let contract = data.host_managed().unwrap();
    let store = ManagedDataStore::new(root.clone());
    let existing_id = "00000000-0000-4000-8000-000000000002".to_string();
    let new_id = "00000000-0000-4000-8000-000000000001".to_string();
    let existing_content = b"old";
    let existing_hash = format!("sha256-{:x}", sha2::Sha256::digest(existing_content));
    assert!(super::write_blob(&store, &app, &existing_hash, existing_content).unwrap());
    let existing = ManagedDocumentRecord {
        id: existing_id.clone(),
        revision: 2,
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:01Z".into(),
        metadata: object(json!({"title": "Existing"})),
        content_sha256: existing_hash.clone(),
        content_length: existing_content.len() as u64,
    };
    let new_content = b"new";
    let new_hash = format!("sha256-{:x}", sha2::Sha256::digest(new_content));
    let digest = super::contract_digest(contract).unwrap();
    let mut state = ManagedDataV2Envelope {
        version: V2_STORE_VERSION,
        generation: 0,
        contract_digest: digest.clone(),
        collections: BTreeMap::new(),
        documents: BTreeMap::from([(
            "scenes".into(),
            BTreeMap::from([(existing_id.clone(), existing)]),
        )]),
        receipts: BTreeMap::new(),
    };
    let stage = ManagedDataStage {
        version: V2_STORE_VERSION,
        batch_id: "rollback-batch".into(),
        expected_generation: 0,
        contract_digest: digest,
        records: Vec::new(),
        documents: BTreeMap::from([
            (
                new_id.clone(),
                ManagedDataStageDocument {
                    stage_id: "new".into(),
                    operation: ManagedDataV2DocumentOperation::Create {
                        stage_id: "new".into(),
                        collection: "scenes".into(),
                        metadata: object(json!({"title": "New"})),
                        content_length: new_content.len() as u64,
                        content_sha256: new_hash.clone(),
                    },
                    chunks: BTreeMap::from([(0, BASE64.encode(new_content))]),
                },
            ),
            (
                existing_id.clone(),
                ManagedDataStageDocument {
                    stage_id: "stale".into(),
                    operation: ManagedDataV2DocumentOperation::Replace {
                        stage_id: "stale".into(),
                        collection: "scenes".into(),
                        id: existing_id.clone(),
                        expected_revision: 1,
                        metadata: object(json!({"title": "Stale"})),
                        content_length: new_content.len() as u64,
                        content_sha256: new_hash.clone(),
                    },
                    chunks: BTreeMap::from([(0, BASE64.encode(new_content))]),
                },
            ),
        ]),
        receipts: BTreeMap::new(),
        expires_at: u64::MAX,
    };
    let error = store
        .commit_stage(
            &app,
            contract,
            "sha256-rollback",
            &mut state,
            stage,
            "rollback-commit".into(),
        )
        .unwrap_err();
    assert!(error.contains("expected revision 1"));
    assert!(!store.blob_path(&app, &new_hash).exists());
    assert_eq!(
        fs::read(store.blob_path(&app, &existing_hash)).unwrap(),
        existing_content
    );
    assert_eq!(state.generation, 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn v2_failed_commit_does_not_remove_a_reused_identical_blob() {
    let root = temp_root();
    let app = AppId::new("com.example.reused-blob");
    let mut data = contract_v2();
    if let AppData::HostManaged { documents, .. } = &mut data {
        documents.get_mut("scenes").unwrap().limits.content_bytes = 5;
    }
    let contract = data.host_managed().unwrap();
    let store = ManagedDataStore::new(root.clone());
    let existing_id = "00000000-0000-4000-8000-000000000002".to_string();
    let new_id = "00000000-0000-4000-8000-000000000001".to_string();
    let content = b"same";
    let hash = format!("sha256-{:x}", sha2::Sha256::digest(content));
    assert!(super::write_blob(&store, &app, &hash, content).unwrap());
    let digest = super::contract_digest(contract).unwrap();
    let existing = ManagedDocumentRecord {
        id: existing_id.clone(),
        revision: 1,
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:01Z".into(),
        metadata: object(json!({"title": "Existing"})),
        content_sha256: hash.clone(),
        content_length: content.len() as u64,
    };
    let mut state = ManagedDataV2Envelope {
        version: V2_STORE_VERSION,
        generation: 0,
        contract_digest: digest.clone(),
        collections: BTreeMap::new(),
        documents: BTreeMap::from([("scenes".into(), BTreeMap::from([(existing_id, existing)]))]),
        receipts: BTreeMap::new(),
    };
    let stage = ManagedDataStage {
        version: V2_STORE_VERSION,
        batch_id: "reuse-batch".into(),
        expected_generation: 0,
        contract_digest: digest,
        records: Vec::new(),
        documents: BTreeMap::from([(
            new_id,
            ManagedDataStageDocument {
                stage_id: "new".into(),
                operation: ManagedDataV2DocumentOperation::Create {
                    stage_id: "new".into(),
                    collection: "scenes".into(),
                    metadata: object(json!({"title": "Duplicate content"})),
                    content_length: content.len() as u64,
                    content_sha256: hash.clone(),
                },
                chunks: BTreeMap::from([(0, BASE64.encode(content))]),
            },
        )]),
        receipts: BTreeMap::new(),
        expires_at: u64::MAX,
    };
    let error = store
        .commit_stage(
            &app,
            contract,
            "sha256-reuse",
            &mut state,
            stage,
            "reuse-commit".into(),
        )
        .unwrap_err();
    assert!(error.contains("content quota"));
    assert_eq!(fs::read(store.blob_path(&app, &hash)).unwrap(), content);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn blob_write_failure_removes_a_partial_file() {
    let root = temp_root();
    let app = AppId::new("com.example.partial-blob");
    let store = ManagedDataStore::with_writer(root.clone(), Arc::new(PartialBlobWriter));
    let content = b"partial content";
    let hash = format!("sha256-{:x}", sha2::Sha256::digest(content));
    assert!(super::write_blob(&store, &app, &hash, content).is_err());
    assert!(!store.blob_path(&app, &hash).exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn v2_metadata_only_document_update_preserves_content_and_updates_snapshot_metadata() {
    let root = temp_root();
    let app = AppId::new("com.example.metadata-document");
    let data = contract_v2();
    let store = ManagedDataStore::new(root.clone());
    let content = b"hello";
    let content_hash = format!("sha256-{:x}", sha2::Sha256::digest(content));
    let batch: Value = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::BeginBatch {
                mutation_id: "create-document".into(),
                expected_generation: 0,
                operations: Vec::new(),
                documents: vec![ManagedDataV2DocumentOperation::Create {
                    stage_id: "scene".into(),
                    collection: "scenes".into(),
                    metadata: object(json!({"title": "Before"})),
                    content_length: content.len() as u64,
                    content_sha256: content_hash.clone(),
                }],
            },
        )
        .unwrap();
    let batch_id = batch["batchId"].as_str().unwrap().to_string();
    let document_id = batch["documents"][0]["documentId"]
        .as_str()
        .unwrap()
        .to_string();
    store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::AppendDocumentChunk {
                mutation_id: "create-chunk".into(),
                batch_id: batch_id.clone(),
                document_id: document_id.clone(),
                chunk_index: 0,
                content_base64: BASE64.encode(content),
            },
        )
        .unwrap();
    store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::CommitBatch {
                mutation_id: "create-commit".into(),
                batch_id,
            },
        )
        .unwrap();

    let before = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::GetDocument {
                collection: "scenes".into(),
                id: document_id.clone(),
                offset: 0,
                length: content.len() as u32,
                expected_generation: Some(1),
            },
        )
        .unwrap();
    let before_updated_at = before["document"]["updatedAt"]
        .as_str()
        .unwrap()
        .to_string();

    let update = ManagedDataV2Request::BeginBatch {
        mutation_id: "metadata-update".into(),
        expected_generation: 1,
        operations: Vec::new(),
        documents: vec![ManagedDataV2DocumentOperation::UpdateMetadata {
            collection: "scenes".into(),
            id: document_id.clone(),
            expected_revision: 1,
            metadata: object(json!({"title": "After"})),
        }],
    };
    let update_batch: Value = store
        .request_v2(&app, data.host_managed().unwrap(), update.clone())
        .unwrap();
    assert!(update_batch["documents"].as_array().unwrap().is_empty());
    let update_batch_id = update_batch["batchId"].as_str().unwrap();
    assert!(store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::AppendDocumentChunk {
                mutation_id: "metadata-chunk".into(),
                batch_id: update_batch_id.into(),
                document_id: document_id.clone(),
                chunk_index: 0,
                content_base64: BASE64.encode(content),
            },
        )
        .unwrap_err()
        .contains("metadata-only"));
    let updated = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::CommitBatch {
                mutation_id: "metadata-commit".into(),
                batch_id: update_batch_id.into(),
            },
        )
        .unwrap();
    assert_eq!(updated["generation"], 2);
    assert_eq!(updated["documents"][0]["revision"], 2);
    assert_eq!(updated["documents"][0]["metadata"]["title"], "After");
    assert_eq!(updated["documents"][0]["contentSha256"], content_hash);
    assert_eq!(updated["documents"][0]["contentLength"], content.len());
    assert_ne!(updated["documents"][0]["updatedAt"], before_updated_at);
    let content_after = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::GetDocument {
                collection: "scenes".into(),
                id: document_id.clone(),
                offset: 0,
                length: content.len() as u32,
                expected_generation: Some(2),
            },
        )
        .unwrap();
    assert_eq!(content_after["contentBase64"], BASE64.encode(content));

    let snapshot = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::ReadSnapshot {
                expected_generation: Some(2),
                reads: vec![ManagedDataV2Read::DocumentList {
                    collection: "scenes".into(),
                    after: None,
                    limit: None,
                }],
            },
        )
        .unwrap();
    assert_eq!(
        snapshot["results"][0]["documents"][0]["metadata"]["title"],
        "After"
    );
    assert_eq!(snapshot["results"][0]["documents"][0]["revision"], 2);

    assert!(store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::BeginBatch {
                mutation_id: "stale-metadata".into(),
                expected_generation: 2,
                operations: Vec::new(),
                documents: vec![ManagedDataV2DocumentOperation::UpdateMetadata {
                    collection: "scenes".into(),
                    id: document_id.clone(),
                    expected_revision: 1,
                    metadata: object(json!({"title": "Stale"})),
                }],
            },
        )
        .unwrap_err()
        .contains("expected revision 1"));
    assert!(store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::BeginBatch {
                mutation_id: "invalid-metadata".into(),
                expected_generation: 2,
                operations: Vec::new(),
                documents: vec![ManagedDataV2DocumentOperation::UpdateMetadata {
                    collection: "scenes".into(),
                    id: document_id.clone(),
                    expected_revision: 2,
                    metadata: object(json!({"title": 7})),
                }],
            },
        )
        .unwrap_err()
        .contains("does not match its schema"));

    let proposal = ManagedDataProposal {
        capability: app_host_kernel::ids::CapabilityName::new("propose-scene"),
        artifact_type: ArtifactTypeName::new("scene-proposal"),
        title: "Propose scene".into(),
        description: "Propose a scene change".into(),
        target: ManagedDataProposalTarget::Document {
            document_collection: "scenes".into(),
        },
        payload_schema: object(json!({"type": "object", "additionalProperties": false})),
        max_payload_bytes: 1,
    };
    let target = store
        .proposal_target(
            &app,
            data.host_managed().unwrap(),
            &proposal,
            &object(json!({"targetId": document_id})),
        )
        .unwrap();
    assert_eq!(target.revision, Some(2));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn v2_appends_record_operations_in_bounded_chunks_and_commits_once() {
    let root = temp_root();
    let app = AppId::new("com.example.large-batch");
    let mut data = contract_v2();
    if let AppData::HostManaged { limits, .. } = &mut data {
        limits.batch_operations = Some(1000);
    }
    let store = ManagedDataStore::new(root.clone());
    let operations = (0..1000)
        .map(|index| ManagedDataMutation::Create {
            collection: "items".into(),
            value: object(json!({
                "group": "import",
                "title": format!("Imported {index}")
            })),
        })
        .collect::<Vec<_>>();
    let batch: Value = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::BeginBatch {
                mutation_id: "large-begin".into(),
                expected_generation: 0,
                operations: operations[..64].to_vec(),
                documents: Vec::new(),
            },
        )
        .unwrap();
    let batch_id = batch["batchId"].as_str().unwrap().to_string();
    let first_append = ManagedDataV2Request::AppendBatchOperations {
        mutation_id: "large-append-0".into(),
        batch_id: batch_id.clone(),
        operations: operations[64..128].to_vec(),
    };
    let first_append_result = store
        .request_v2(&app, data.host_managed().unwrap(), first_append.clone())
        .unwrap();
    assert_eq!(
        store
            .request_v2(&app, data.host_managed().unwrap(), first_append)
            .unwrap(),
        first_append_result
    );
    assert!(store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::AppendBatchOperations {
                mutation_id: "large-append-0".into(),
                batch_id: batch_id.clone(),
                operations: operations[128..192].to_vec(),
            },
        )
        .unwrap_err()
        .contains("reused with a different request"));
    for (chunk_index, chunk) in operations[128..].chunks(64).enumerate() {
        store
            .request_v2(
                &app,
                data.host_managed().unwrap(),
                ManagedDataV2Request::AppendBatchOperations {
                    mutation_id: format!("large-append-{}", chunk_index + 1),
                    batch_id: batch_id.clone(),
                    operations: chunk.to_vec(),
                },
            )
            .unwrap();
    }
    assert!(store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::AppendBatchOperations {
                mutation_id: "over-limit".into(),
                batch_id: batch_id.clone(),
                operations: vec![ManagedDataMutation::Create {
                    collection: "items".into(),
                    value: object(json!({"group": "import", "title": "Too many"})),
                }],
            },
        )
        .unwrap_err()
        .contains("1000-operation limit"));

    let committed = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::CommitBatch {
                mutation_id: "large-commit".into(),
                batch_id,
            },
        )
        .unwrap();
    assert_eq!(committed["generation"], 1);
    assert_eq!(committed["records"].as_array().unwrap().len(), 1000);

    let first_id = committed["records"][0]["id"].as_str().unwrap().to_string();
    let failed_batch: Value = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::BeginBatch {
                mutation_id: "failed-begin".into(),
                expected_generation: 1,
                operations: vec![
                    ManagedDataMutation::Create {
                        collection: "items".into(),
                        value: object(json!({"group": "import", "title": "Invisible"})),
                    },
                    ManagedDataMutation::Replace {
                        collection: "items".into(),
                        id: uuid::Uuid::new_v4().to_string(),
                        expected_revision: 1,
                        value: object(json!({"group": "import", "title": "Missing"})),
                    },
                ],
                documents: Vec::new(),
            },
        )
        .unwrap();
    assert!(store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::CommitBatch {
                mutation_id: "failed-commit".into(),
                batch_id: failed_batch["batchId"].as_str().unwrap().into(),
            },
        )
        .unwrap_err()
        .contains("does not exist"));
    let unchanged = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            ManagedDataV2Request::Get {
                collection: "items".into(),
                id: first_id,
                expected_generation: Some(1),
            },
        )
        .unwrap();
    assert_eq!(unchanged["generation"], 1);
    assert_eq!(unchanged["record"]["revision"], 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn v2_stages_and_commits_a_seven_mib_document_with_bounded_chunks() {
    let root = temp_root();
    let app = AppId::new("com.example.large-document");
    let data = contract_v2();
    let store = ManagedDataStore::new(root.clone());
    let content = vec![b'x'; 7 * 1024 * 1024];
    let content_hash = format!("sha256-{:x}", sha2::Sha256::digest(&content));
    let batch: Value = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            public_v2_request(json!({
                "kind": "begin-batch",
                "mutationId": "large-batch",
                "expectedGeneration": 0,
                "operations": [],
                "documents": [{
                    "kind": "create",
                    "stageId": "large-scene",
                    "collection": "scenes",
                    "metadata": {"title": "Large scene"},
                    "contentLength": content.len(),
                    "contentSha256": content_hash
                }]
            })),
        )
        .unwrap();
    let batch_id = batch["batchId"].as_str().unwrap().to_string();
    let document_id = batch["documents"][0]["documentId"]
        .as_str()
        .unwrap()
        .to_string();
    for (chunk_index, chunk) in content.chunks(super::V2_MAX_CHUNK_BYTES).enumerate() {
        assert!(chunk.len() <= super::V2_MAX_CHUNK_BYTES);
        store
            .request_v2(
                &app,
                data.host_managed().unwrap(),
                public_v2_request(json!({
                    "kind": "append-document-chunk",
                    "mutationId": format!("large-chunk-{chunk_index}"),
                    "batchId": batch_id.clone(),
                    "documentId": document_id.clone(),
                    "chunkIndex": chunk_index,
                    "contentBase64": BASE64.encode(chunk)
                })),
            )
            .unwrap();
    }
    let committed = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            public_v2_request(json!({
                "kind": "commit-batch",
                "mutationId": "large-commit",
                "batchId": batch_id.clone()
            })),
        )
        .unwrap();
    assert_eq!(committed["generation"], 1);
    let chunk = store
        .request_v2(
            &app,
            data.host_managed().unwrap(),
            public_v2_request(json!({
                "kind": "read-snapshot",
                "expectedGeneration": 1,
                "reads": [{
                    "kind": "document-content",
                    "collection": "scenes",
                    "id": document_id,
                    "offset": content.len() - 16,
                    "length": 16
                }]
            })),
        )
        .unwrap();
    assert_eq!(chunk["generation"], 1);
    assert_eq!(chunk["results"][0]["contentLength"], content.len());
    fs::remove_dir_all(root).unwrap();
}

fn manifest(app_id: AppId, capabilities: Vec<CapabilityDeclaration>) -> AppManifest {
    AppManifest {
        app_id,
        version: "1.0.0".into(),
        display_name: "Managed data fixture".into(),
        description: "Managed data fixture".into(),
        capabilities,
        surfaces: Vec::new(),
        agents: Vec::new(),
        skills: Vec::new(),
        assistant_profiles: Vec::new(),
        automations: Vec::new(),
        connectors: Vec::new(),
        config_declarations: Vec::new(),
        artifact_types: Vec::new(),
        extension_points: Vec::new(),
        extension_contributions: Vec::new(),
        grant_requests: Vec::new(),
        event_subscriptions: Vec::new(),
    }
}

fn install(
    kernel: &mut Kernel,
    manifest: AppManifest,
    handlers: BTreeMap<
        app_host_kernel::ids::CapabilityName,
        app_host_kernel::invocation::CapabilityHandler,
    >,
) {
    let prepared = kernel.prepare_install(seal(manifest), handlers).unwrap();
    kernel.commit_install(prepared.await_approval()).unwrap();
}

fn invoke(
    kernel: &mut Kernel,
    consumer: &AppId,
    capability: &CapabilityRef,
    input: app_host_kernel::JsonObject,
    data_scope: DataScope,
) -> InvocationResult {
    let run_id = kernel
        .start_run(
            Initiator::App {
                app_id: consumer.clone(),
                reason: "managed data test".into(),
            },
            "managed data test",
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
