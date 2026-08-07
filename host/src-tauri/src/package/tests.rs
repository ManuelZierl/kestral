use super::*;

#[test]
fn package_ids_cannot_impersonate_bundled_apps() {
    for id in [
        "chat",
        "llm-provider",
        "com.ma-zierl.kestral-artifacts",
        "com.ma-zierl.host.file-broker",
        "com.ma-zierl.host.permissions",
    ] {
        assert!(!id_is_valid(id), "bundled app id must stay reserved: {id}");
    }
    assert!(id_is_valid("com.example.workspace"));
}

#[test]
fn frozen_alpha_1_format_1_package_remains_readable() {
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/persistence/alpha-1/whole-profile/apps/com.example.fixture/revisions/revision-alpha-1",
    );

    let document = read_document(&package).unwrap();

    assert_eq!(document.format_version, SUPPORTED_FORMAT_VERSION);
    assert!(structural_error(&document).is_none());
    assert!(verify_integrity(&package, &document.integrity).is_ok());
}

#[test]
fn external_app_release_evidence_schema_compiles() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("schemas/external-app-release-evidence.schema.json");
    let schema: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();

    jsonschema::validator_for(&schema).expect("external app release evidence schema must compile");
}

#[test]
fn host_managed_exports_require_exact_read_only_capability_contracts() {
    let collection = ManagedDataCollection {
        schema: serde_json::from_value(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["group"],
            "properties": {"group": {"type": "string"}}
        }))
        .unwrap(),
        indexes: vec![ManagedDataIndex {
            name: "group".into(),
            field: "group".into(),
            value_schema: serde_json::from_value(json!({"type": "string"})).unwrap(),
            unique: false,
        }],
        operations: BTreeSet::from([ManagedDataOperation::List]),
        limits: ManagedDataCollectionLimits {
            records: 100,
            record_bytes: 4096,
            query_results: 20,
        },
    };
    let export = ManagedDataExport {
        capability: CapabilityName::new("list_items"),
        operation: ManagedDataExportOperation::List,
        collection: "items".into(),
        index: Some("group".into()),
        equals_host_input: Some(ManagedDataExportHostInput::CurrentChatThreadId),
    };
    let capability = CapabilityDeclaration {
        name: export.capability.clone(),
        description: "List matching items".into(),
        input_schema: managed_export_input_schema(
            export.operation,
            &collection,
            collection.indexes.first(),
            export.equals_host_input,
        ),
        effect: CapabilityEffect::ReadOnly,
        output_schema: Some(managed_export_output_schema(export.operation, &collection)),
    };
    let collections = BTreeMap::from([("items".into(), collection)]);
    let limits = ManagedDataStoreLimits {
        total_bytes: 1024 * 1024,
        transaction_operations: 8,
        batch_operations: None,
    };

    assert!(validate_host_managed_data(
        1,
        &collections,
        &BTreeMap::new(),
        &limits,
        std::slice::from_ref(&export),
        &[],
        std::slice::from_ref(&capability),
        &[],
        "com.example.managed-data",
    )
    .is_ok());
    assert_eq!(
        capability.input_schema["properties"]["equals"][crate::tool_mapping::HOST_INPUT_ANNOTATION],
        crate::tool_mapping::CURRENT_CHAT_THREAD_ID
    );

    let public_document = json!({
        "format_version": 1,
        "id": "com.example.managed-data",
        "version": "1.0.0",
        "display_name": "Managed data fixture",
        "description": "Backend-free managed data package",
        "min_host_version": "0.1.0-alpha.1",
        "manifest": {"capabilities": [capability.clone()]},
        "backend": {"kind": "none"},
        "data": AppData::HostManaged {
            contract_version: 1,
            collections: collections.clone(),
            documents: BTreeMap::new(),
            limits: limits.clone(),
            exports: vec![export.clone()],
            proposals: Vec::new(),
        },
        "integrity": {"algorithm": "sha256", "assets": {}}
    });
    validate_public_schema(&public_document).unwrap();
    let parsed: PackageDocument = serde_json::from_value(public_document).unwrap();
    assert!(structural_error(&parsed).is_none());

    let mut unsafe_capability = capability;
    unsafe_capability.effect = CapabilityEffect::LocalWrite;
    assert!(validate_host_managed_data(
        1,
        &collections,
        &BTreeMap::new(),
        &limits,
        &[export],
        &[],
        &[unsafe_capability],
        &[],
        "com.example.managed-data",
    )
    .unwrap_err()
    .contains("must declare effect 'read-only'"));
}

#[test]
fn host_managed_proposals_require_exact_generated_contracts() {
    let proposal = ManagedDataProposal {
        capability: CapabilityName::new("propose_item"),
        artifact_type: app_host_kernel::ids::ArtifactTypeName::new("item-proposal"),
        title: "Propose item change".into(),
        description: "Create a reviewable item proposal".into(),
        target: ManagedDataProposalTarget::Record {
            collection: "items".into(),
        },
        payload_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["title"],
            "properties": {"title": {"type": "string", "maxLength": 120}}
        })
        .as_object()
        .unwrap()
        .clone(),
        max_payload_bytes: 4096,
    };
    let collection = ManagedDataCollection {
        schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["title"],
            "properties": {"title": {"type": "string"}}
        })
        .as_object()
        .unwrap()
        .clone(),
        indexes: Vec::new(),
        operations: BTreeSet::from([ManagedDataOperation::Get]),
        limits: ManagedDataCollectionLimits {
            records: 10,
            record_bytes: 4096,
            query_results: 10,
        },
    };
    let capability = CapabilityDeclaration {
        name: proposal.capability.clone(),
        description: proposal.description.clone(),
        input_schema: managed_proposal_input_schema(&proposal),
        effect: CapabilityEffect::LocalWrite,
        output_schema: Some(managed_proposal_artifact_schema(
            &AppId::new("com.example.proposals"),
            &proposal,
        )),
    };
    let artifact = ArtifactTypeDeclaration {
        name: proposal.artifact_type.clone(),
        description: proposal.description.clone(),
        json_schema: managed_proposal_artifact_schema(
            &AppId::new("com.example.proposals"),
            &proposal,
        ),
    };
    let collections = BTreeMap::from([("items".into(), collection)]);
    let limits = ManagedDataStoreLimits {
        total_bytes: 1024 * 1024,
        transaction_operations: 8,
        batch_operations: Some(2048),
    };
    assert!(validate_host_managed_data(
        2,
        &collections,
        &BTreeMap::new(),
        &limits,
        &[],
        std::slice::from_ref(&proposal),
        std::slice::from_ref(&capability),
        std::slice::from_ref(&artifact),
        "com.example.proposals",
    )
    .is_ok());

    let mut wrong = capability.clone();
    wrong.effect = CapabilityEffect::ReadOnly;
    assert!(validate_host_managed_data(
        2,
        &collections,
        &BTreeMap::new(),
        &limits,
        &[],
        std::slice::from_ref(&proposal),
        &[wrong],
        std::slice::from_ref(&artifact),
        "com.example.proposals",
    )
    .unwrap_err()
    .contains("must declare effect 'local-write'"));

    let public = json!({
        "format_version": 1,
        "id": "com.example.proposals",
        "version": "1.0.0",
        "display_name": "Proposal fixture",
        "description": "Backend-free proposal package",
        "min_host_version": "0.1.0-alpha.1",
        "manifest": {"capabilities": [capability], "artifact_types": [artifact]},
        "backend": {"kind": "none"},
        "data": {
            "kind": "host-managed",
            "contract_version": 2,
            "collections": collections,
            "limits": limits,
            "exports": [],
            "proposals": [proposal]
        },
        "integrity": {"algorithm": "sha256", "assets": {}}
    });
    validate_public_schema(&public).unwrap();
    let parsed: PackageDocument = serde_json::from_value(public).unwrap();
    assert!(
        structural_error(&parsed).is_none(),
        "{:?}",
        structural_error(&parsed)
    );

    let mut malformed = parsed;
    if let AppData::HostManaged { proposals, .. } = &mut malformed.data {
        proposals[0]
            .payload_schema
            .insert("additionalProperties".into(), Value::Bool(true));
    }
    assert!(structural_error(&malformed)
        .unwrap()
        .contains("additionalProperties false"));
}

#[test]
fn backend_none_rejects_capabilities_not_bound_to_exports_or_proposals() {
    let mut document: PackageDocument = serde_json::from_value(json!({
        "format_version": 1,
        "id": "com.example.backend-free",
        "version": "1.0.0",
        "display_name": "Backend-free",
        "description": "No executable capability provider",
        "min_host_version": "0.1.0-alpha.1",
        "manifest": {"capabilities": [{
            "name": "unbound",
            "description": "Not host implemented",
            "input_schema": {"type": "object", "additionalProperties": false}
        }]},
        "backend": {"kind": "none"},
        "data": {"kind": "host-managed", "contract_version": 2, "collections": {
            "items": {"schema": {"type": "object", "properties": {}, "additionalProperties": false}, "indexes": [], "operations": ["get"], "limits": {"records": 1, "record_bytes": 4096, "query_results": 1}}
         }, "limits": {"total_bytes": 1024, "transaction_operations": 1, "batch_operations": 2048}, "exports": [], "proposals": []},
        "integrity": {"algorithm": "sha256", "assets": {}}
    })).unwrap();
    assert!(structural_error(&document)
        .unwrap()
        .contains("only capabilities listed in data.exports or data.proposals"));
    if let AppData::HostManaged { proposals, .. } = &mut document.data {
        proposals.push(ManagedDataProposal {
            capability: CapabilityName::new("unbound"),
            artifact_type: app_host_kernel::ids::ArtifactTypeName::new("missing"),
            title: "Proposal".into(),
            description: "Proposal".into(),
            target: ManagedDataProposalTarget::Collection {
                collection: "items".into(),
            },
            payload_schema: json!({"type": "object", "additionalProperties": false})
                .as_object()
                .unwrap()
                .clone(),
            max_payload_bytes: 1,
        });
    }
    assert!(structural_error(&document).is_some());
}

#[test]
fn host_managed_contract_v2_accepts_record_only_and_document_only_but_rejects_empty() {
    let document = json!({
        "format_version": 1,
        "id": "com.example.documents",
        "version": "1.0.0",
        "display_name": "Managed documents",
        "description": "Contract v2 fixture",
        "min_host_version": "0.1.0-alpha.1",
        "manifest": {},
        "backend": {"kind": "none"},
        "data": {
            "kind": "host-managed",
            "contract_version": 2,
            "collections": {
                "records": {
                    "schema": {"type": "object", "additionalProperties": false, "properties": {}},
                    "indexes": [],
                    "operations": ["get", "list"],
                    "limits": {"records": 10, "record_bytes": 4096, "query_results": 10}
                }
            },
            "documents": {
                "scenes": {
                    "metadata_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {"title": {"type": "string"}}
                    },
                    "operations": ["get", "list", "create", "replace", "delete"],
                    "limits": {"documents": 10, "metadata_bytes": 4096, "content_bytes": 8388608}
                }
            },
            "limits": {"total_bytes": 67108864, "transaction_operations": 32, "batch_operations": 2048},
            "exports": []
        },
        "integrity": {"algorithm": "sha256", "assets": {}}
    });
    validate_public_schema(&document).unwrap();
    let parsed: PackageDocument = serde_json::from_value(document).unwrap();
    assert!(
        structural_error(&parsed).is_none(),
        "{:?}",
        structural_error(&parsed)
    );

    let record_only = json!({
        "format_version": 1,
        "id": "com.example.record-only",
        "version": "1.0.0",
        "display_name": "Record-only managed data",
        "description": "Contract v2 record-only fixture",
        "min_host_version": "0.1.0-alpha.1",
        "manifest": {},
        "backend": {"kind": "none"},
        "data": {
            "kind": "host-managed",
            "contract_version": 2,
            "collections": {
                "records": {
                    "schema": {"type": "object", "additionalProperties": false, "properties": {}},
                    "indexes": [],
                    "operations": ["get", "list"],
                    "limits": {"records": 10, "record_bytes": 4096, "query_results": 10}
                }
            },
            "limits": {"total_bytes": 1024, "transaction_operations": 1, "batch_operations": 2048},
            "exports": []
        },
        "integrity": {"algorithm": "sha256", "assets": {}}
    });
    validate_public_schema(&record_only).unwrap();
    let parsed: PackageDocument = serde_json::from_value(record_only).unwrap();
    assert!(structural_error(&parsed).is_none());

    let mut document_only = json!({
        "format_version": 1,
        "id": "com.example.document-only",
        "version": "1.0.0",
        "display_name": "Document-only managed data",
        "description": "Contract v2 document-only fixture",
        "min_host_version": "0.1.0-alpha.1",
        "manifest": {},
        "backend": {"kind": "none"},
        "data": {
            "kind": "host-managed",
            "contract_version": 2,
            "collections": {},
            "documents": {
                "scenes": {
                    "metadata_schema": {"type": "object", "additionalProperties": false, "properties": {}},
                    "operations": ["get", "list", "create", "update-metadata"],
                    "limits": {"documents": 10, "metadata_bytes": 4096, "content_bytes": 8388608}
                }
            },
            "limits": {"total_bytes": 67108864, "transaction_operations": 32, "batch_operations": 2048},
            "exports": []
        },
        "integrity": {"algorithm": "sha256", "assets": {}}
    });
    validate_public_schema(&document_only).unwrap();
    let parsed: PackageDocument = serde_json::from_value(document_only.clone()).unwrap();
    assert!(structural_error(&parsed).is_none());

    document_only["data"]["documents"] = json!({});
    assert!(validate_public_schema(&document_only).is_err());
    let parsed: PackageDocument = serde_json::from_value(document_only).unwrap();
    assert!(structural_error(&parsed)
        .unwrap()
        .contains("at least one record or document collection"));

    let mut v1_empty = json!({
        "format_version": 1,
        "id": "com.example.empty-v1",
        "version": "1.0.0",
        "display_name": "Empty v1 managed data",
        "description": "Contract v1 empty fixture",
        "min_host_version": "0.1.0-alpha.1",
        "manifest": {},
        "backend": {"kind": "none"},
        "data": {
            "kind": "host-managed",
            "contract_version": 1,
            "collections": {},
            "limits": {"total_bytes": 1024, "transaction_operations": 1},
            "exports": []
        },
        "integrity": {"algorithm": "sha256", "assets": {}}
    });
    assert!(validate_public_schema(&v1_empty).is_err());
    v1_empty["data"]["collections"] = json!({});
    let parsed: PackageDocument = serde_json::from_value(v1_empty).unwrap();
    assert!(structural_error(&parsed)
        .unwrap()
        .contains("contract v1 requires 1-64 collections"));
}

#[test]
fn inspection_exposes_native_backend_authority_without_inferring_it_from_text() {
    let cases = [
        (json!({"kind": "none"}), None),
        (
            json!({"kind": "mcp-streamable-http", "url": "https://example.test/mcp"}),
            None,
        ),
        (
            json!({"kind": "mcp-stdio", "authority_mode": "unsandboxed", "command": "node", "args": []}),
            Some(BackendAuthorityMode::Unsandboxed),
        ),
        (
            json!({"kind": "agent-worker", "authority_mode": "sandboxed", "protocol_version": 1, "entry": "worker.mjs"}),
            Some(BackendAuthorityMode::Sandboxed),
        ),
    ];

    for (value, expected) in cases {
        let backend: Backend = serde_json::from_value(value).unwrap();
        assert_eq!(backend.authority_mode(), expected);
    }
}
use crate::publisher_trust::{
    PackageSignatureDocument, PublisherTrustStore, SignatureState, TrustScope,
};
use app_host_kernel::ids::AppId;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use sha2::Digest;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "package-security-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = remove_read_only_tree(&self.0);
    }
}

fn write_package(root: &Path, assets: &[(&str, &[u8])]) {
    let mut integrity = serde_json::Map::new();
    for (path, bytes) in assets {
        let destination = root.join(path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, bytes).unwrap();
        integrity.insert(
            (*path).into(),
            json!(format!("sha256-{:x}", Sha256::digest(bytes))),
        );
    }
    let document = json!({
        "format_version": 1,
        "id": "com.example.security",
        "version": "1.0.0",
        "display_name": "Security fixture",
        "description": "Package boundary fixture",
        "min_host_version": "0.0.1",
        "manifest": {},
        "backend": {"kind": "none"},
        "data": {"kind": "none"},
        "integrity": {"algorithm": "sha256", "assets": integrity}
    });
    fs::write(root.join(APP_JSON), serde_json::to_vec(&document).unwrap()).unwrap();
}

fn write_package_with_meta(
    root: &Path,
    assets: &[(&str, &[u8])],
    app_id: &str,
    version: &str,
    min_host_version: &str,
    publisher_key_id: Option<&str>,
) {
    let mut integrity = serde_json::Map::new();
    for (path, bytes) in assets {
        let destination = root.join(path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, bytes).unwrap();
        integrity.insert(
            (*path).into(),
            json!(format!("sha256-{:x}", Sha256::digest(bytes))),
        );
    }
    let mut publisher = json!({"name": "Publisher Label"});
    if let Some(key_id) = publisher_key_id {
        publisher["key_id"] = json!(key_id);
    }
    let document = json!({
        "format_version": 1,
        "id": app_id,
        "version": version,
        "display_name": "Security fixture",
        "description": "Package boundary fixture",
        "publisher": publisher,
        "min_host_version": min_host_version,
        "manifest": {},
        "backend": {"kind": "none"},
        "data": {"kind": "none"},
        "integrity": {"algorithm": "sha256", "assets": integrity}
    });
    fs::write(root.join(APP_JSON), serde_json::to_vec(&document).unwrap()).unwrap();
}

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn signature_document(
    signing_key: &SigningKey,
    package_digest: &str,
    app_id: &str,
) -> (PackageSignatureDocument, String) {
    let public_key = signing_key.verifying_key();
    let public_key_bytes = public_key.as_bytes();
    let key_id = format!("ed25519:{:x}", Sha256::digest(public_key_bytes));
    let signature = signing_key.sign(&PublisherTrustStore::signing_bytes(package_digest, app_id));
    (
        PackageSignatureDocument {
            algorithm: "ed25519".into(),
            key_id: key_id.clone(),
            public_key: STANDARD.encode(public_key_bytes),
            signature: STANDARD.encode(signature.to_bytes()),
        },
        key_id,
    )
}

fn write_signature(root: &Path, signature: &PackageSignatureDocument) {
    fs::write(
        root.join("app.signature.json"),
        serde_json::to_vec(signature).unwrap(),
    )
    .unwrap();
}

#[test]
fn resolves_built_package_from_project_directory() {
    let project = Scratch::new("project-directory");
    let package = project.0.join("dist");
    fs::create_dir(&package).unwrap();
    write_package(&package, &[]);

    assert_eq!(resolve_package_directory(&project.0).unwrap(), package);
}

#[test]
fn reports_expected_package_locations() {
    let source = Scratch::new("missing-package");

    let error = resolve_package_directory(&source.0).unwrap_err();
    assert!(
        error.contains("expected app.json or dist/app.json"),
        "{error}"
    );
}

#[test]
fn package_data_contract_is_required_and_versioned_migrations_are_explicit() {
    let fixture = Scratch::new("app-data-contract");
    let migration = b"// inspection-only migration fixture\n";
    write_package(&fixture.0, &[("backend/migrate.mjs", migration)]);
    let mut document: Value =
        serde_json::from_slice(&fs::read(fixture.0.join(APP_JSON)).unwrap()).unwrap();
    document.as_object_mut().unwrap().remove("data");
    fs::write(
        fixture.0.join(APP_JSON),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    assert!(read_document(&fixture.0)
        .unwrap_err()
        .contains("\"data\" is a required property"));

    document["backend"] = json!({
        "kind": "mcp-stdio",
        "authority_mode": "unsandboxed",
        "command": "node",
        "args": []
    });
    document["data"] = json!({
        "kind": "versioned",
        "format_version": 2,
        "migration": {
            "protocol_version": 1,
            "command": "node",
            "entry": "backend/migrate.mjs",
            "args": ["backend/migrate.mjs"],
            "transitions": [{"from": 1, "to": 2, "destructive": false}]
        }
    });
    fs::write(
        fixture.0.join(APP_JSON),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();

    let parsed = read_document(&fixture.0).unwrap();
    assert_eq!(parsed.data.format_version(), Some(2));
    assert!(structural_error(&parsed).is_none());
    let inspection = inspect(&fixture.0).unwrap();
    assert_eq!(inspection.data.format_version, Some(2));
    assert_eq!(inspection.data.transitions.len(), 1);
}

#[test]
fn public_schema_accepts_extension_contributions() {
    let fixture = Scratch::new("extension-contribution");
    let package = &fixture.0;
    let ui = b"<!doctype html><html><body>fixture</body></html>";
    let backend = b"// inspection-only fixture\n";
    fs::create_dir_all(package.join("ui")).unwrap();
    fs::create_dir_all(package.join("backend")).unwrap();
    fs::write(package.join("ui/index.html"), ui).unwrap();
    fs::write(package.join("backend/server.mjs"), backend).unwrap();
    let document = json!({
        "format_version": 1,
        "id": "com.example.extension",
        "version": "1.0.0",
        "display_name": "Extension fixture",
        "description": "Package schema fixture.",
        "min_host_version": "0.0.1",
        "manifest": {
            "capabilities": [{
                "name": "list_data",
                "description": "List fixture data",
                "input_schema": {"type": "object", "properties": {}, "additionalProperties": false},
                "effect": "read-only"
            }],
            "surfaces": [{
                "name": "extension-card",
                "kind": "card",
                "title": "Extension fixture",
                "description": "Contributed test surface.",
                "intents": [{"provider": "com.example.extension", "capability": "list_data"}],
                "ui": {"entry": "ui/index.html"}
            }],
            "extension_contributions": [{
                "target_app": "chat",
                "extension_point": "message-actions",
                "contract_version": 1,
                "surface": "extension-card"
            }]
        },
        "consumer_grant_requests": [{
            "holder": "chat",
            "request": {
                "scope": {"kind": "exact-capability", "provider": "com.example.extension", "capability": "list_data"},
                "data_scope": {"kind": "none"},
                "condition": "silent",
                "reason": "Let the fixture consumer read package data.",
                "duration": {"kind": "non-expiring"}
            }
        }],
        "backend": {"kind": "mcp-stdio", "authority_mode": "unsandboxed", "command": "node", "args": ["backend/server.mjs"]},
        "data": {"kind": "none"},
        "integrity": {"algorithm": "sha256", "assets": {
            "ui/index.html": format!("sha256-{:x}", Sha256::digest(ui)),
            "backend/server.mjs": format!("sha256-{:x}", Sha256::digest(backend))
        }}
    });
    fs::write(
        package.join(APP_JSON),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();

    let document = read_document(package).unwrap();

    assert_eq!(document.manifest.extension_contributions.len(), 1);
    assert_eq!(document.manifest.capabilities.len(), 1);
    assert_eq!(document.consumer_grant_requests.len(), 1);
    assert_eq!(
        document.consumer_grant_requests[0].holder,
        AppId::new("chat")
    );
    assert!(verify_integrity(package, &document.integrity).is_ok());
    let inspection = inspect(package).unwrap();
    assert!(inspection
        .grant_requests
        .iter()
        .any(|request| { request.scope_label == "chat -> com.example.extension/list_data" }));
}

#[test]
fn package_theme_colors_are_bounded_unique_and_css_safe() {
    let fixture = Scratch::new("theme-colors");
    write_package(&fixture.0, &[]);
    let mut value: Value =
        serde_json::from_slice(&fs::read(fixture.0.join(APP_JSON)).unwrap()).unwrap();
    value["theme_colors"] = json!([{
        "name": "storm-track",
        "title": "Storm track",
        "description": "Forecast path on the map.",
        "light": "#315ea8",
        "dark": "rgba(141, 177, 255, 0.9)"
    }]);
    fs::write(
        fixture.0.join(APP_JSON),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();

    let document = read_document(&fixture.0).unwrap();
    assert!(structural_error(&document).is_none());
    assert_eq!(document.theme_colors[0].name, "storm-track");

    value["theme_colors"][0]["light"] = json!("rgb(999, 0, 0)");
    fs::write(
        fixture.0.join(APP_JSON),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let document = read_document(&fixture.0).unwrap();
    assert!(structural_error(&document)
        .unwrap()
        .contains("valid HEX, rgb(), or rgba()"));

    value["theme_colors"] = json!([
        {
            "name": "storm-track",
            "title": "Storm track",
            "description": "Forecast path on the map.",
            "light": "#315ea8",
            "dark": "#8db1ff"
        },
        {
            "name": "storm-track",
            "title": "Duplicate",
            "description": "Duplicate token.",
            "light": "#315ea8",
            "dark": "#8db1ff"
        }
    ]);
    fs::write(
        fixture.0.join(APP_JSON),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let document = read_document(&fixture.0).unwrap();
    assert!(structural_error(&document)
        .unwrap()
        .contains("duplicate name 'storm-track'"));
}

#[test]
fn package_icons_support_checked_assets_and_kestral_catalog_names() {
    let asset_fixture = Scratch::new("asset-icon");
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4 4h16v16H4z"/></svg>"#;
    write_package(&asset_fixture.0, &[("ui/icon.svg", svg)]);
    let mut value: Value =
        serde_json::from_slice(&fs::read(asset_fixture.0.join(APP_JSON)).unwrap()).unwrap();
    value["icon"] = json!("ui/icon.svg");
    fs::write(
        asset_fixture.0.join(APP_JSON),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();

    let document = read_document(&asset_fixture.0).unwrap();
    assert!(matches!(document.icon, Some(PackageIcon::Asset(_))));
    assert!(matches!(
        app_icon_view(&asset_fixture.0, document.icon.as_ref()).unwrap(),
        Some(AppIconView::Asset { ref media_type, .. }) if media_type == "image/svg+xml"
    ));

    let catalog_fixture = Scratch::new("catalog-icon");
    write_package(&catalog_fixture.0, &[]);
    let mut value: Value =
        serde_json::from_slice(&fs::read(catalog_fixture.0.join(APP_JSON)).unwrap()).unwrap();
    value["icon"] = json!({"kind": "kestral", "name": "check-square"});
    fs::write(
        catalog_fixture.0.join(APP_JSON),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();

    let document = read_document(&catalog_fixture.0).unwrap();
    assert!(matches!(
        app_icon_view(&catalog_fixture.0, document.icon.as_ref()).unwrap(),
        Some(AppIconView::Kestral {
            name: KestralIconName::CheckSquare
        })
    ));
}

#[test]
fn package_icons_reject_unknown_catalog_names_and_active_svg_content() {
    let catalog_fixture = Scratch::new("unknown-catalog-icon");
    write_package(&catalog_fixture.0, &[]);
    let mut value: Value =
        serde_json::from_slice(&fs::read(catalog_fixture.0.join(APP_JSON)).unwrap()).unwrap();
    value["icon"] = json!({"kind": "kestral", "name": "typo"});
    fs::write(
        catalog_fixture.0.join(APP_JSON),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(read_document(&catalog_fixture.0)
        .unwrap_err()
        .contains("icon"));

    let svg_fixture = Scratch::new("active-svg-icon");
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
    write_package(&svg_fixture.0, &[("ui/icon.svg", svg)]);
    let mut value: Value =
        serde_json::from_slice(&fs::read(svg_fixture.0.join(APP_JSON)).unwrap()).unwrap();
    value["icon"] = json!("ui/icon.svg");
    fs::write(
        svg_fixture.0.join(APP_JSON),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(inspect(&svg_fixture.0)
        .unwrap_err()
        .contains("unsupported active or external SVG content"));
}

#[test]
fn public_schema_accepts_agent_worker_backend() {
    let fixture = Scratch::new("agent-worker");
    let package = &fixture.0;
    let worker = b"// inspection-only agent worker fixture\n";
    fs::create_dir_all(package.join("backend")).unwrap();
    fs::write(package.join("backend/worker.mjs"), worker).unwrap();
    let document = json!({
        "format_version": 1,
        "id": "com.example.agent-worker",
        "version": "1.0.0",
        "display_name": "Agent worker fixture",
        "description": "Package schema fixture.",
        "min_host_version": "0.0.1",
        "manifest": {
            "capabilities": [{
                "name": "agent.run",
                "description": "Run the fixture agent.",
                "input_schema": {"type": "object", "properties": {}, "additionalProperties": false},
                "effect": "external-write"
            }],
            "artifact_types": [{
                "name": "agent-transcript",
                "description": "Fixture agent transcript.",
                "json_schema": {"type": "array", "items": {"type": "object"}}
            }]
        },
        "backend": {"kind": "agent-worker", "authority_mode": "unsandboxed", "protocol_version": 1, "entry": "backend/worker.mjs"},
        "data": {"kind": "none"},
        "integrity": {"algorithm": "sha256", "assets": {
            "backend/worker.mjs": format!("sha256-{:x}", Sha256::digest(worker))
        }}
    });
    fs::write(
        package.join(APP_JSON),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();

    let document = read_document(package).unwrap();

    assert_eq!(document.id, "com.example.agent-worker");
    assert!(matches!(
        document.backend,
        Backend::AgentWorker {
            authority_mode: BackendAuthorityMode::Unsandboxed,
            protocol_version: 1,
            ..
        }
    ));
    assert!(verify_integrity(package, &document.integrity).is_ok());
    assert!(inspect(package).unwrap().installable);
}

#[test]
fn public_schema_rejects_unsupported_backend_lifecycle_fields() {
    for field in ["health", "shutdown"] {
        let source = Scratch::new(field);
        write_package(&source.0, &[]);
        let mut value: Value =
            serde_json::from_slice(&fs::read(source.0.join(APP_JSON)).unwrap()).unwrap();
        value[field] = json!({});
        fs::write(source.0.join(APP_JSON), serde_json::to_vec(&value).unwrap()).unwrap();

        let error = read_document(&source.0).unwrap_err();
        assert!(
            error.contains(field),
            "unexpected error for {field}: {error}"
        );
    }
}

#[test]
fn package_text_inputs_are_size_bounded_before_parsing_or_bundling() {
    let document_source = Scratch::new("oversized-document");
    File::create(document_source.0.join(APP_JSON))
        .unwrap()
        .set_len(MAX_APP_DOCUMENT_BYTES as u64 + 1)
        .unwrap();
    let error = read_document(&document_source.0).unwrap_err();
    assert!(error.contains("app.json"), "{error}");
    assert!(error.contains("maximum"), "{error}");

    let signature_source = Scratch::new("oversized-signature");
    File::create(signature_source.0.join("app.signature.json"))
        .unwrap()
        .set_len(MAX_SIGNATURE_DOCUMENT_BYTES as u64 + 1)
        .unwrap();
    let error = read_signature_document(&signature_source.0).unwrap_err();
    assert!(error.contains("app.signature.json"), "{error}");
    assert!(error.contains("maximum"), "{error}");

    let surface_source = Scratch::new("oversized-surface");
    write_package(&surface_source.0, &[]);
    fs::create_dir_all(surface_source.0.join("ui")).unwrap();
    File::create(surface_source.0.join("ui/index.html"))
        .unwrap()
        .set_len(MAX_SURFACE_UI_BYTES as u64 + 1)
        .unwrap();
    let mut value: Value =
        serde_json::from_slice(&fs::read(surface_source.0.join(APP_JSON)).unwrap()).unwrap();
    value["manifest"]["surfaces"] = json!([{
        "name": "panel",
        "kind": "panel",
        "title": "Panel",
        "description": "Oversized surface fixture.",
        "ui": {"entry": "ui/index.html"}
    }]);
    value["integrity"]["assets"]["ui/index.html"] = json!(format!("sha256-{}", "0".repeat(64)));
    fs::write(
        surface_source.0.join(APP_JSON),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let document = read_document(&surface_source.0).unwrap();
    let error = match translate(&surface_source.0, &document) {
        Ok(_) => panic!("oversized UI entry should be rejected"),
        Err(error) => error,
    };
    assert!(error.contains("ui entry 'ui/index.html'"), "{error}");
    assert!(error.contains("maximum"), "{error}");
}

#[test]
fn consumer_grants_cannot_delegate_an_unrelated_provider() {
    let source = Scratch::new("foreign-consumer-grant");
    write_package(&source.0, &[]);
    let mut value: Value =
        serde_json::from_slice(&fs::read(source.0.join(APP_JSON)).unwrap()).unwrap();
    value["consumer_grant_requests"] = json!([{
        "holder": "chat",
        "request": {
            "scope": {"kind": "exact-capability", "provider": "notes", "capability": "read"},
            "data_scope": {"kind": "none"},
            "condition": "silent",
            "reason": "Not this package's authority",
            "duration": {"kind": "non-expiring"}
        }
    }]);
    fs::write(source.0.join(APP_JSON), serde_json::to_vec(&value).unwrap()).unwrap();

    let inspection = inspect(&source.0).unwrap();
    assert!(!inspection.installable);
    assert!(inspection
        .blocking_error
        .unwrap()
        .contains("only expose capabilities provided by this package"));
}

#[test]
fn rejects_unlisted_executable() {
    let source = Scratch::new("unlisted");
    write_package(&source.0, &[]);
    fs::create_dir_all(source.0.join("backend")).unwrap();
    fs::write(source.0.join("backend/evil.exe"), b"MZ").unwrap();

    let error = stage_and_inspect(&source.0, &source.0.join("staging")).unwrap_err();
    assert!(error.contains("declaration mismatch"), "{error}");
}

#[test]
fn rejects_case_colliding_declared_paths() {
    let source = Scratch::new("case-collision");
    write_package(&source.0, &[("ui/a.js", b"a"), ("ui/A.js", b"b")]);

    let error = package_digest(&source.0).unwrap_err();
    assert!(error.contains("case-colliding"), "{error}");
}

#[test]
fn rejects_checksum_mismatch() {
    let source = Scratch::new("checksum");
    write_package(&source.0, &[("ui/a.js", b"expected")]);
    fs::write(source.0.join("ui/a.js"), b"changed").unwrap();

    let document = read_document(&source.0).unwrap();
    let error = verify_integrity(&source.0, &document.integrity).unwrap_err();
    assert!(error.contains("checksum mismatch"), "{error}");
}

#[test]
fn rejects_missing_declared_file() {
    let source = Scratch::new("missing");
    write_package(&source.0, &[("ui/a.js", b"expected")]);
    fs::remove_file(source.0.join("ui/a.js")).unwrap();

    let error = package_digest(&source.0).unwrap_err();
    assert!(error.contains("missing=[\"ui/a.js\"]"), "{error}");
}

#[test]
fn detects_source_mutation_during_copy() {
    let source = Scratch::new("copy-source");
    let destination = Scratch::new("copy-source-destination");
    fs::remove_dir(&destination.0).unwrap();
    write_package(&source.0, &[("ui/a.js", b"a")]);
    let digest = package_digest(&source.0).unwrap();
    let mut mutated = false;

    let error = copy_verified_package_with_hook(&source.0, &destination.0, &digest, |_| {
        if !mutated {
            fs::write(source.0.join("ui/a.js"), b"mutated").unwrap();
            mutated = true;
        }
        Ok(())
    })
    .unwrap_err();
    assert!(
        error.contains("post-copy package verification failed"),
        "{error}"
    );
    assert!(!destination.0.exists());
}

#[test]
fn detects_destination_mutation_before_post_copy_verification() {
    let source = Scratch::new("copy-destination");
    let destination = Scratch::new("copy-destination-target");
    fs::remove_dir(&destination.0).unwrap();
    write_package(&source.0, &[("ui/a.js", b"a")]);
    let digest = package_digest(&source.0).unwrap();

    let error = copy_verified_package_with_hook(&source.0, &destination.0, &digest, |root| {
        if root.join("ui/a.js").is_file() {
            fs::write(root.join("ui/a.js"), b"mutated destination").unwrap();
        }
        Ok(())
    })
    .unwrap_err();
    assert!(
        error.contains("post-copy package verification failed"),
        "{error}"
    );
    assert!(!destination.0.exists());
}

#[test]
fn strict_semver_values_block_inspection() {
    let document = PackageDocument {
        format_version: 1,
        id: "com.example.security".into(),
        version: "1.0".into(),
        display_name: "Security fixture".into(),
        description: "Package boundary fixture".into(),
        publisher: None,
        license: None,
        icon: None,
        theme_colors: Vec::new(),
        min_host_version: "0.0.1".into(),
        manifest: PackageManifestBody::default(),
        consumer_grant_requests: Vec::new(),
        backend: Backend::None,
        data: AppData::None,
        integrity: Integrity {
            algorithm: "sha256".into(),
            assets: Default::default(),
        },
    };

    let error = structural_error(&document).unwrap();
    assert!(error.contains("strict semver"), "{error}");

    let mut min_host_document = document;
    min_host_document.version = "1.0.0".into();
    min_host_document.min_host_version = "1.0".into();
    let error = structural_error(&min_host_document).unwrap();
    assert!(error.contains("strict semver"), "{error}");
}

#[test]
fn detached_signature_transitions_through_unknown_trusted_and_revoked_states() {
    let source = Scratch::new("signed-package");
    let signer = signing_key(7);
    write_package_with_meta(
        &source.0,
        &[("ui/a.js", b"a")],
        "com.example.security",
        "1.0.0",
        "0.0.1",
        None,
    );
    let digest = package_digest(&source.0).unwrap();
    let (signature, key_id) = signature_document(&signer, &digest, "com.example.security");
    write_signature(&source.0, &signature);

    let mut trust_store = PublisherTrustStore::in_memory();
    let inspection = inspect_with_trust(&source.0, &trust_store).unwrap();
    assert!(matches!(
        inspection.signature,
        SignatureState::ValidUnknownKey { .. }
    ));
    assert!(inspection.installable);

    trust_store
        .trust_key(
            &key_id,
            &signature.public_key,
            TrustScope::AppId {
                app_id: AppId::new("com.example.security"),
            },
        )
        .unwrap();
    let trusted = inspect_with_trust(&source.0, &trust_store).unwrap();
    assert!(matches!(trusted.signature, SignatureState::Trusted { .. }));
    assert!(trusted.installable);

    trust_store
        .revoke_key(
            &key_id,
            &TrustScope::AppId {
                app_id: AppId::new("com.example.security"),
            },
        )
        .unwrap();
    let revoked = inspect_with_trust(&source.0, &trust_store).unwrap();
    assert!(matches!(revoked.signature, SignatureState::Revoked { .. }));
    assert!(!revoked.installable);
    assert!(revoked.blocking_error.unwrap().contains("revoked"));
}

#[test]
fn tampered_signed_package_is_rejected() {
    let source = Scratch::new("tampered-signed-package");
    let signer = signing_key(8);
    write_package_with_meta(
        &source.0,
        &[("ui/a.js", b"a")],
        "com.example.security",
        "1.0.0",
        "0.0.1",
        None,
    );
    let digest = package_digest(&source.0).unwrap();
    let (signature, _key_id) = signature_document(&signer, &digest, "com.example.security");
    write_signature(&source.0, &signature);

    fs::write(source.0.join("ui/a.js"), b"mutated").unwrap();

    let inspection = inspect_with_trust(&source.0, &PublisherTrustStore::in_memory()).unwrap();
    assert!(matches!(
        inspection.signature,
        SignatureState::Invalid { .. }
    ));
    assert!(!inspection.installable);
    assert!(inspection
        .blocking_error
        .unwrap()
        .contains("invalid package signature"));
}

#[test]
fn malformed_signature_carrier_blocks_inspection() {
    let source = Scratch::new("malformed-signature");
    write_package_with_meta(
        &source.0,
        &[("ui/a.js", b"a")],
        "com.example.security",
        "1.0.0",
        "0.0.1",
        None,
    );
    fs::write(
        source.0.join("app.signature.json"),
        br#"{"algorithm":"ed25519","key_id":"ed25519:1234","public_key":"not-base64","signature":"also-not-base64"}"#,
    )
    .unwrap();

    let error = inspect(&source.0).unwrap_err();
    assert!(error.contains("app.signature.json"), "{error}");
}

#[test]
fn same_publisher_label_with_different_keys_stays_unknown_for_the_untrusted_key() {
    let trusted_source = Scratch::new("publisher-label-a");
    let unknown_source = Scratch::new("publisher-label-b");
    let signer_a = signing_key(9);
    let signer_b = signing_key(10);

    write_package_with_meta(
        &trusted_source.0,
        &[("ui/a.js", b"a")],
        "com.example.label-a",
        "1.0.0",
        "0.0.1",
        None,
    );
    write_package_with_meta(
        &unknown_source.0,
        &[("ui/a.js", b"a")],
        "com.example.label-b",
        "1.0.0",
        "0.0.1",
        None,
    );

    let trusted_digest = package_digest(&trusted_source.0).unwrap();
    let (trusted_signature, trusted_key_id) =
        signature_document(&signer_a, &trusted_digest, "com.example.label-a");
    write_signature(&trusted_source.0, &trusted_signature);

    let unknown_digest = package_digest(&unknown_source.0).unwrap();
    let (unknown_signature, _unknown_key_id) =
        signature_document(&signer_b, &unknown_digest, "com.example.label-b");
    write_signature(&unknown_source.0, &unknown_signature);

    let mut trust_store = PublisherTrustStore::in_memory();
    trust_store
        .trust_key(
            &trusted_key_id,
            &trusted_signature.public_key,
            TrustScope::NamespacePrefix {
                namespace_prefix: "com.example".into(),
            },
        )
        .unwrap();

    let trusted = inspect_with_trust(&trusted_source.0, &trust_store).unwrap();
    let unknown = inspect_with_trust(&unknown_source.0, &trust_store).unwrap();

    assert!(matches!(trusted.signature, SignatureState::Trusted { .. }));
    assert!(matches!(
        unknown.signature,
        SignatureState::ValidUnknownKey { .. }
    ));
}

#[cfg(unix)]
#[test]
fn rejects_symlink_payloads() {
    use std::os::unix::fs::symlink;

    let source = Scratch::new("symlink");
    write_package(&source.0, &[("ui/a.js", b"a")]);
    fs::remove_file(source.0.join("ui/a.js")).unwrap();
    symlink(source.0.join(APP_JSON), source.0.join("ui/a.js")).unwrap();

    let error = package_digest(&source.0).unwrap_err();
    assert!(error.contains("symlinks are unsupported"), "{error}");
}
