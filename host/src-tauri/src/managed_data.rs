use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use app_host_kernel::ids::{AppId, CapabilityName};
use app_host_kernel::invocation::{CapabilityHandler, CapabilityOutcome, HandlerFailure};
use app_host_kernel::primitives::artifact::ArtifactDraft;
use app_host_kernel::primitives::grant::DataScope;
use app_host_kernel::JsonObject;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::atomic_json::{
    load_json_document, persist_json_document, standard_writer, AtomicFileWriter, AtomicJsonError,
};
use crate::package::{
    AppData, HostManagedDataRef, ManagedDataCollection, ManagedDataExportOperation,
    ManagedDataOperation, ManagedDataProposal, ManagedDataProposalTarget,
    ManagedDocumentCollection, ManagedDocumentOperation, MAX_MANAGED_DATA_BATCH_OPERATIONS,
};

const STORE_VERSION: u32 = 1;
const STORE_FILE: &str = "managed-data-v1.json";
const CONTRACT_BACKUP_FILE: &str = "managed-data-contract-backup-v1.json";
const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_ID_LENGTH: usize = 64;
const V2_STORE_VERSION: u32 = 2;
const V2_STORE_FILE: &str = "managed-data-v2.json";
const V2_STAGE_FILE: &str = "managed-data-stage-v2.json";
const V2_BLOB_DIR: &str = "managed-data-blobs-v2";
const V2_MAX_RECEIPTS: usize = 1024;
// Staged JSON contains base64 transport overhead; the authoritative quota is
// the decoded bound below, while this envelope bound prevents unbounded input.
const V2_MAX_STAGE_BYTES: usize = 96 * 1024 * 1024;
const V2_MAX_STAGE_DECODED_BYTES: usize = 64 * 1024 * 1024;
const V2_MAX_BATCH_APPEND_OPERATIONS: usize = 64;
const V2_MAX_CHUNK_BYTES: usize = 384 * 1024;
const V2_MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
const V2_STAGE_TTL_SECONDS: u64 = 15 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDataRecord {
    pub id: String,
    pub revision: u64,
    pub created_at: String,
    pub updated_at: String,
    pub value: JsonObject,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDataListResult {
    pub records: Vec<ManagedDataRecord>,
    pub next_after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedDataDeleteResult {
    pub id: String,
    pub deleted: bool,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedDataQuery {
    #[serde(default)]
    pub index: Option<String>,
    #[serde(default)]
    pub equals: Option<Value>,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ManagedDataRequest {
    Get {
        collection: String,
        id: String,
    },
    List {
        collection: String,
        #[serde(default)]
        query: Option<ManagedDataQuery>,
    },
    Create {
        collection: String,
        value: JsonObject,
    },
    Replace {
        collection: String,
        id: String,
        #[serde(rename = "expectedRevision")]
        expected_revision: u64,
        value: JsonObject,
    },
    Delete {
        collection: String,
        id: String,
        #[serde(rename = "expectedRevision")]
        expected_revision: u64,
    },
    Transaction {
        operations: Vec<ManagedDataMutation>,
    },
}

/// The command argument remains backward compatible with the v1 request shape.
/// Version 2 is explicitly wrapped so a v1 operation can never be interpreted
/// as a v2 mutation by accident.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ManagedDataCommand {
    V1(ManagedDataRequest),
    V1Tagged {
        #[serde(rename = "contractVersion")]
        contract_version: u32,
        request: ManagedDataRequest,
    },
    V2 {
        #[serde(rename = "contractVersion")]
        contract_version: u32,
        request: ManagedDataV2Request,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ManagedDataV2Request {
    ReadSnapshot {
        #[serde(rename = "expectedGeneration")]
        expected_generation: Option<u64>,
        reads: Vec<ManagedDataV2Read>,
    },
    Get {
        collection: String,
        id: String,
        #[serde(rename = "expectedGeneration")]
        expected_generation: Option<u64>,
    },
    List {
        collection: String,
        #[serde(default)]
        query: Option<ManagedDataQuery>,
        #[serde(rename = "expectedGeneration")]
        expected_generation: Option<u64>,
    },
    GetDocument {
        collection: String,
        id: String,
        #[serde(rename = "offset")]
        offset: u64,
        #[serde(rename = "length")]
        length: u32,
        #[serde(rename = "expectedGeneration")]
        expected_generation: Option<u64>,
    },
    ListDocuments {
        collection: String,
        #[serde(default)]
        after: Option<String>,
        #[serde(default)]
        limit: Option<u32>,
        #[serde(rename = "expectedGeneration")]
        expected_generation: Option<u64>,
    },
    Create {
        #[serde(rename = "mutationId")]
        mutation_id: String,
        #[serde(rename = "expectedGeneration")]
        expected_generation: u64,
        collection: String,
        value: JsonObject,
    },
    Replace {
        #[serde(rename = "mutationId")]
        mutation_id: String,
        #[serde(rename = "expectedGeneration")]
        expected_generation: u64,
        collection: String,
        id: String,
        #[serde(rename = "expectedRevision")]
        expected_revision: u64,
        value: JsonObject,
    },
    Delete {
        #[serde(rename = "mutationId")]
        mutation_id: String,
        #[serde(rename = "expectedGeneration")]
        expected_generation: u64,
        collection: String,
        id: String,
        #[serde(rename = "expectedRevision")]
        expected_revision: u64,
    },
    BeginBatch {
        #[serde(rename = "mutationId")]
        mutation_id: String,
        #[serde(rename = "expectedGeneration")]
        expected_generation: u64,
        operations: Vec<ManagedDataMutation>,
        documents: Vec<ManagedDataV2DocumentOperation>,
    },
    AppendBatchOperations {
        #[serde(rename = "mutationId")]
        mutation_id: String,
        #[serde(rename = "batchId")]
        batch_id: String,
        operations: Vec<ManagedDataMutation>,
    },
    AppendDocumentChunk {
        #[serde(rename = "mutationId")]
        mutation_id: String,
        #[serde(rename = "batchId")]
        batch_id: String,
        #[serde(rename = "documentId")]
        document_id: String,
        #[serde(rename = "chunkIndex")]
        chunk_index: u32,
        #[serde(rename = "contentBase64")]
        content_base64: String,
    },
    CommitBatch {
        #[serde(rename = "mutationId")]
        mutation_id: String,
        #[serde(rename = "batchId")]
        batch_id: String,
    },
    AbortBatch {
        #[serde(rename = "mutationId")]
        mutation_id: String,
        #[serde(rename = "batchId")]
        batch_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedDataV2RecordView {
    pub id: String,
    pub revision: u64,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    pub value: JsonObject,
}

impl From<&ManagedDataRecord> for ManagedDataV2RecordView {
    fn from(record: &ManagedDataRecord) -> Self {
        Self {
            id: record.id.clone(),
            revision: record.revision,
            created_at: record.created_at.clone(),
            updated_at: record.updated_at.clone(),
            value: record.value.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ManagedDataV2Read {
    RecordGet {
        collection: String,
        id: String,
    },
    RecordList {
        collection: String,
        #[serde(default)]
        query: Option<ManagedDataQuery>,
    },
    DocumentGet {
        collection: String,
        id: String,
    },
    DocumentList {
        collection: String,
        #[serde(default)]
        after: Option<String>,
        #[serde(default)]
        limit: Option<u32>,
    },
    DocumentContent {
        collection: String,
        id: String,
        offset: u64,
        length: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ManagedDataV2ReadResult {
    RecordGet {
        record: Option<ManagedDataV2RecordView>,
    },
    RecordList {
        records: Vec<ManagedDataV2RecordView>,
        #[serde(rename = "nextAfter")]
        next_after: Option<String>,
    },
    DocumentGet {
        document: Option<ManagedDocumentRecordView>,
    },
    DocumentList {
        documents: Vec<ManagedDocumentRecordView>,
        #[serde(rename = "nextAfter")]
        next_after: Option<String>,
    },
    DocumentContent {
        document: ManagedDocumentRecordView,
        offset: u64,
        #[serde(rename = "contentBase64")]
        content_base64: String,
        #[serde(rename = "contentLength")]
        content_length: u64,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ManagedDataV2DocumentOperation {
    Create {
        #[serde(rename = "stageId")]
        stage_id: String,
        collection: String,
        metadata: JsonObject,
        #[serde(rename = "contentLength")]
        content_length: u64,
        #[serde(rename = "contentSha256")]
        content_sha256: String,
    },
    Replace {
        #[serde(rename = "stageId")]
        stage_id: String,
        collection: String,
        id: String,
        #[serde(rename = "expectedRevision")]
        expected_revision: u64,
        metadata: JsonObject,
        #[serde(rename = "contentLength")]
        content_length: u64,
        #[serde(rename = "contentSha256")]
        content_sha256: String,
    },
    UpdateMetadata {
        collection: String,
        id: String,
        #[serde(rename = "expectedRevision")]
        expected_revision: u64,
        metadata: JsonObject,
    },
    Delete {
        collection: String,
        id: String,
        #[serde(rename = "expectedRevision")]
        expected_revision: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ManagedDataMutation {
    Create {
        collection: String,
        value: JsonObject,
    },
    Replace {
        collection: String,
        id: String,
        #[serde(rename = "expectedRevision")]
        expected_revision: u64,
        value: JsonObject,
    },
    Delete {
        collection: String,
        id: String,
        #[serde(rename = "expectedRevision")]
        expected_revision: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedDataDocument {
    version: u32,
    generation: u64,
    contract_digest: String,
    collections: BTreeMap<String, BTreeMap<String, ManagedDataRecord>>,
}

pub struct ManagedDataStore {
    data_root: PathBuf,
    lock_root: PathBuf,
    writer: Arc<dyn AtomicFileWriter>,
}

impl ManagedDataStore {
    pub fn new(data_root: PathBuf) -> Self {
        Self::with_writer(data_root, standard_writer())
    }

    pub(crate) fn with_writer(data_root: PathBuf, writer: Arc<dyn AtomicFileWriter>) -> Self {
        let lock_root = stable_path_identity(&data_root);
        Self {
            data_root,
            lock_root,
            writer,
        }
    }

    pub fn request(
        &self,
        app_id: &AppId,
        contract: HostManagedDataRef<'_>,
        request: ManagedDataRequest,
    ) -> Result<Value, String> {
        let operation_lock = self.operation_lock(app_id)?;
        let _operation = operation_lock
            .lock()
            .map_err(|_| "managed-data operation lock poisoned".to_string())?;
        self.validate_contract_version(contract)?;
        if self.v2_path(app_id).is_file() {
            return Err("managed-data v2 state exists; a v1 contract cannot read it".into());
        }
        let digest = contract_digest(contract)?;
        let mut document = self.load(app_id, contract, &digest)?;
        match request {
            ManagedDataRequest::Get { collection, id } => {
                let declaration =
                    require_operation(contract, &collection, ManagedDataOperation::Get)?;
                validate_id(&id)?;
                let value = document
                    .collections
                    .get(&collection)
                    .and_then(|records| records.get(&id))
                    .cloned();
                validate_optional_record(value.as_ref(), declaration)?;
                serde_json::to_value(value)
                    .map_err(|error| format!("serialize managed-data record failed: {error}"))
            }
            ManagedDataRequest::List { collection, query } => {
                require_operation(contract, &collection, ManagedDataOperation::List)?;
                let result = list_records(&document, contract, &collection, query)?;
                serde_json::to_value(result)
                    .map_err(|error| format!("serialize managed-data list failed: {error}"))
            }
            ManagedDataRequest::Create { collection, value } => {
                require_operation(contract, &collection, ManagedDataOperation::Create)?;
                let result = apply_create(&mut document, contract, &collection, value)?;
                self.persist_mutation(app_id, contract, &digest, &mut document)?;
                serde_json::to_value(result)
                    .map_err(|error| format!("serialize managed-data record failed: {error}"))
            }
            ManagedDataRequest::Replace {
                collection,
                id,
                expected_revision,
                value,
            } => {
                require_operation(contract, &collection, ManagedDataOperation::Replace)?;
                let result = apply_replace(
                    &mut document,
                    contract,
                    &collection,
                    &id,
                    expected_revision,
                    value,
                )?;
                self.persist_mutation(app_id, contract, &digest, &mut document)?;
                serde_json::to_value(result)
                    .map_err(|error| format!("serialize managed-data record failed: {error}"))
            }
            ManagedDataRequest::Delete {
                collection,
                id,
                expected_revision,
            } => {
                require_operation(contract, &collection, ManagedDataOperation::Delete)?;
                let result =
                    apply_delete(&mut document, contract, &collection, &id, expected_revision)?;
                self.persist_mutation(app_id, contract, &digest, &mut document)?;
                serde_json::to_value(result)
                    .map_err(|error| format!("serialize managed-data delete failed: {error}"))
            }
            ManagedDataRequest::Transaction { operations } => {
                if operations.is_empty()
                    || operations.len() > contract.limits.transaction_operations as usize
                {
                    return Err(format!(
                        "managed-data transaction must contain 1-{} operations",
                        contract.limits.transaction_operations
                    ));
                }
                let mut candidate = document.clone();
                let mut results = Vec::with_capacity(operations.len());
                for operation in operations {
                    let (collection, required) = mutation_operation(&operation);
                    let declaration =
                        require_operation(contract, collection, ManagedDataOperation::Transaction)?;
                    if !declaration.operations.contains(&required) {
                        return Err(format!(
                            "managed-data collection '{collection}' does not permit {required:?}"
                        ));
                    }
                    results.push(apply_mutation(&mut candidate, contract, operation)?);
                }
                validate_document_limits(&candidate, contract)?;
                document = candidate;
                self.persist_mutation(app_id, contract, &digest, &mut document)?;
                Ok(Value::Array(results))
            }
        }
    }

    pub fn validate_contract(
        &self,
        app_id: &AppId,
        contract: HostManagedDataRef<'_>,
    ) -> Result<(), String> {
        let operation_lock = self.operation_lock(app_id)?;
        let _operation = operation_lock
            .lock()
            .map_err(|_| "managed-data operation lock poisoned".to_string())?;
        self.validate_contract_version(contract)?;
        if contract.contract_version == 1 && self.v2_path(app_id).is_file() {
            return Err("managed-data v2 state exists; a v1 contract cannot reuse it".into());
        }
        let digest = contract_digest(contract)?;
        if contract.contract_version == 2 {
            self.load_v2(app_id, contract, &digest).map(|_| ())
        } else {
            self.load(app_id, contract, &digest).map(|_| ())
        }
    }

    fn proposal_target(
        &self,
        app_id: &AppId,
        contract: HostManagedDataRef<'_>,
        proposal: &ManagedDataProposal,
        input: &JsonObject,
    ) -> Result<ProposalTargetState, String> {
        if contract.contract_version != 2 {
            return Err("managed-data proposals require contract version 2".into());
        }
        let operation_lock = self.operation_lock(app_id)?;
        let _operation = operation_lock
            .lock()
            .map_err(|_| "managed-data operation lock poisoned".to_string())?;
        self.reject_v2_links(app_id)?;
        self.cleanup_expired_stage(app_id)?;
        let digest = contract_digest(contract)?;
        let state = self.load_v2(app_id, contract, &digest)?;
        let (kind, collection, resource_id, revision) = match &proposal.target {
            ManagedDataProposalTarget::Collection { collection } => {
                if !contract.collections.contains_key(collection) {
                    return Err(format!("unknown managed-data collection '{collection}'"));
                }
                (
                    "collection",
                    collection.clone(),
                    resource_id(app_id, collection),
                    None,
                )
            }
            ManagedDataProposalTarget::Record { collection } => {
                let id = required_string(input, "targetId").map_err(|error| error.0)?;
                validate_id(&id)?;
                let record = state
                    .collections
                    .get(collection)
                    .and_then(|records| records.get(&id))
                    .ok_or_else(|| format!("managed-data record '{id}' does not exist"))?;
                (
                    "record",
                    collection.clone(),
                    record_resource_id(app_id, collection, &id),
                    Some(record.revision),
                )
            }
            ManagedDataProposalTarget::Document {
                document_collection,
            } => {
                let id = required_string(input, "targetId").map_err(|error| error.0)?;
                validate_id(&id)?;
                let document = state
                    .documents
                    .get(document_collection)
                    .and_then(|documents| documents.get(&id))
                    .ok_or_else(|| format!("managed-data document '{id}' does not exist"))?;
                (
                    "document",
                    document_collection.clone(),
                    document_resource_id(app_id, document_collection, &id),
                    Some(document.revision),
                )
            }
        };
        Ok(ProposalTargetState {
            kind,
            collection,
            resource_id,
            generation: state.generation,
            revision,
        })
    }

    pub(crate) fn validate_all(data_root: &Path) -> Result<(), String> {
        if !data_root.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(data_root).map_err(|error| {
            format!(
                "read app data directory '{}' failed: {error}",
                data_root.display()
            )
        })? {
            let entry = entry.map_err(|error| format!("read app data entry failed: {error}"))?;
            let file_type = entry
                .file_type()
                .map_err(|error| format!("inspect app data entry failed: {error}"))?;
            if file_type.is_symlink() {
                return Err(format!(
                    "managed-data root contains a symlink: {}",
                    entry.path().display()
                ));
            }
            if !file_type.is_dir() {
                continue;
            }
            let app_id = AppId::new(entry.file_name().to_string_lossy().into_owned());
            let store = Self::new(data_root.to_path_buf());
            let operation_lock = store.operation_lock(&app_id)?;
            let _operation = operation_lock
                .lock()
                .map_err(|_| "managed-data operation lock poisoned".to_string())?;
            if store.v2_path(&app_id).is_file() {
                store.reject_v2_links(&app_id)?;
                let state: ManagedDataV2Envelope =
                    load_json_document(&store.v2_path(&app_id), "managed data v2")?.ok_or_else(
                        || "managed data v2 disappeared while validating".to_string(),
                    )?;
                validate_v2_envelope(&state, &store, &app_id)?;
            }
            if store.stage_path(&app_id).is_file() {
                let stage = store.load_stage(&app_id)?.ok_or_else(|| {
                    "managed data v2 batch disappeared while validating".to_string()
                })?;
                store.validate_stage_size(&stage)?;
            }
            let Some(document) = store.load_document(&store.path(&app_id), "managed data")? else {
                if store
                    .load_document(
                        &store.contract_backup_path(&app_id),
                        "managed data contract backup",
                    )?
                    .is_some()
                {
                    return Err(format!(
                        "managed-data backup for app '{}' exists without an active store",
                        app_id
                    ));
                }
                continue;
            };
            validate_envelope(&document)?;
            if let Some(backup) = store.load_document(
                &store.contract_backup_path(&app_id),
                "managed data contract backup",
            )? {
                validate_envelope(&backup)?;
            }
        }
        Ok(())
    }

    pub(crate) fn exists(&self, app_id: &AppId) -> bool {
        (self.path(app_id).is_file() && !self.path(app_id).is_symlink())
            || (self.v2_path(app_id).is_file() && !self.v2_path(app_id).is_symlink())
    }

    fn validate_contract_version(&self, contract: HostManagedDataRef<'_>) -> Result<(), String> {
        if !matches!(contract.contract_version, 1 | 2) {
            return Err(format!(
                "unsupported managed-data contract version {}",
                contract.contract_version
            ));
        }
        Ok(())
    }

    fn load(
        &self,
        app_id: &AppId,
        contract: HostManagedDataRef<'_>,
        digest: &str,
    ) -> Result<ManagedDataDocument, String> {
        let Some(document) = self.load_document(&self.path(app_id), "managed data")? else {
            return Ok(ManagedDataDocument {
                version: STORE_VERSION,
                generation: 0,
                contract_digest: digest.to_string(),
                collections: BTreeMap::new(),
            });
        };
        validate_envelope(&document)?;
        validate_document(&document, contract)?;
        Ok(document)
    }

    fn persist_mutation(
        &self,
        app_id: &AppId,
        contract: HostManagedDataRef<'_>,
        digest: &str,
        document: &mut ManagedDataDocument,
    ) -> Result<(), String> {
        self.reject_linked_app_path(app_id)?;
        if document.generation > 0 && document.contract_digest != digest {
            persist_json_document(
                &self.contract_backup_path(app_id),
                document,
                "managed data contract backup",
                self.writer.as_ref(),
            )
            .map_err(AtomicJsonError::into_message)?;
        }
        document.contract_digest = digest.to_string();
        document.generation = document
            .generation
            .checked_add(1)
            .ok_or_else(|| "managed-data generation overflow".to_string())?;
        validate_document(document, contract)?;
        persist_json_document(
            &self.path(app_id),
            document,
            "managed data",
            self.writer.as_ref(),
        )
        .map_err(AtomicJsonError::into_message)
    }

    fn path(&self, app_id: &AppId) -> PathBuf {
        self.data_root.join(app_id.as_str()).join(STORE_FILE)
    }

    fn contract_backup_path(&self, app_id: &AppId) -> PathBuf {
        self.data_root
            .join(app_id.as_str())
            .join(CONTRACT_BACKUP_FILE)
    }

    fn load_document(
        &self,
        path: &Path,
        label: &str,
    ) -> Result<Option<ManagedDataDocument>, String> {
        self.reject_linked_path(path, label)?;
        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.len() > MAX_DOCUMENT_BYTES as u64 {
                return Err(format!(
                    "{label} exceeds the {MAX_DOCUMENT_BYTES}-byte host limit"
                ));
            }
        }
        load_json_document(path, label)
    }

    fn reject_linked_app_path(&self, app_id: &AppId) -> Result<(), String> {
        self.reject_linked_path(
            &self.data_root.join(app_id.as_str()),
            "managed data directory",
        )?;
        self.reject_linked_path(&self.path(app_id), "managed data")?;
        self.reject_linked_path(
            &self.contract_backup_path(app_id),
            "managed data contract backup",
        )
    }

    fn reject_linked_path(&self, path: &Path, label: &str) -> Result<(), String> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(format!("{label} path '{}' is a symlink", path.display()))
            }
            Ok(_) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("inspect {label} path failed: {error}")),
        }
    }

    fn operation_lock(&self, app_id: &AppId) -> Result<Arc<Mutex<()>>, String> {
        static LOCKS: OnceLock<Mutex<BTreeMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
        let key = self.lock_root.join(app_id.as_str()).join(STORE_FILE);
        let mut locks = LOCKS
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .map_err(|_| "managed-data lock registry poisoned".to_string())?;
        Ok(locks
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone())
    }
}

fn read_snapshot_result(
    store: &ManagedDataStore,
    app_id: &AppId,
    contract: HostManagedDataRef<'_>,
    state: &ManagedDataV2Envelope,
    read: &ManagedDataV2Read,
) -> Result<ManagedDataV2ReadResult, String> {
    match read {
        ManagedDataV2Read::RecordGet { collection, id } => {
            let declaration = require_operation(contract, collection, ManagedDataOperation::Get)?;
            validate_id(id)?;
            let record = state
                .collections
                .get(collection)
                .and_then(|records| records.get(id))
                .cloned();
            validate_optional_record(record.as_ref(), declaration)?;
            Ok(ManagedDataV2ReadResult::RecordGet {
                record: record.as_ref().map(ManagedDataV2RecordView::from),
            })
        }
        ManagedDataV2Read::RecordList { collection, query } => {
            require_operation(contract, collection, ManagedDataOperation::List)?;
            let result = list_records(
                &ManagedDataDocument {
                    version: STORE_VERSION,
                    generation: state.generation.max(1),
                    contract_digest: state.contract_digest.clone(),
                    collections: state.collections.clone(),
                },
                contract,
                collection,
                query.clone(),
            )?;
            Ok(ManagedDataV2ReadResult::RecordList {
                records: result
                    .records
                    .iter()
                    .map(ManagedDataV2RecordView::from)
                    .collect(),
                next_after: result.next_after,
            })
        }
        ManagedDataV2Read::DocumentGet { collection, id } => {
            require_document_operation(contract, collection, ManagedDocumentOperation::Get)?;
            validate_id(id)?;
            let document = state
                .documents
                .get(collection)
                .and_then(|documents| documents.get(id))
                .map(ManagedDocumentRecordView::from);
            Ok(ManagedDataV2ReadResult::DocumentGet { document })
        }
        ManagedDataV2Read::DocumentList {
            collection,
            after,
            limit,
        } => {
            require_document_operation(contract, collection, ManagedDocumentOperation::List)?;
            let after = after
                .as_ref()
                .map(|id| validate_id(id).map(|_| id.clone()))
                .transpose()?;
            let limit = limit.unwrap_or(100).min(100) as usize;
            let empty = BTreeMap::new();
            let documents = state.documents.get(collection).unwrap_or(&empty);
            let mut listed: Vec<ManagedDocumentRecordView> = documents
                .iter()
                .filter(|(id, _)| after.as_ref().is_none_or(|cursor| *id > cursor))
                .map(|(_, document)| ManagedDocumentRecordView::from(document))
                .take(limit + 1)
                .collect();
            let has_more = listed.len() > limit;
            if has_more {
                listed.pop();
            }
            let next_after = has_more
                .then(|| listed.last().map(|document| document.id.clone()))
                .flatten();
            Ok(ManagedDataV2ReadResult::DocumentList {
                documents: listed,
                next_after,
            })
        }
        ManagedDataV2Read::DocumentContent {
            collection,
            id,
            offset,
            length,
        } => {
            require_document_operation(contract, collection, ManagedDocumentOperation::Get)?;
            validate_id(id)?;
            let document = state
                .documents
                .get(collection)
                .and_then(|documents| documents.get(id))
                .ok_or_else(|| format!("managed-data document '{id}' does not exist"))?;
            let content = store.read_blob(app_id, document)?;
            let end = offset
                .checked_add(u64::from(*length))
                .ok_or_else(|| "managed-data document chunk range overflow".to_string())?;
            if *length as usize > V2_MAX_CHUNK_BYTES || end > document.content_length {
                return Err(format!(
                    "managed-data document chunk must be at most {V2_MAX_CHUNK_BYTES} bytes and within the document"
                ));
            }
            let start = usize::try_from(*offset).map_err(|_| "document offset is too large")?;
            let end = usize::try_from(end).map_err(|_| "document end is too large")?;
            Ok(ManagedDataV2ReadResult::DocumentContent {
                document: ManagedDocumentRecordView::from(document),
                offset: *offset,
                content_base64: BASE64.encode(&content[start..end]),
                content_length: document.content_length,
            })
        }
    }
}

fn stable_path_identity(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut missing = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            return path.to_path_buf();
        };
        missing.push(name.to_os_string());
        let Some(parent) = existing.parent() else {
            return path.to_path_buf();
        };
        existing = parent;
    }
    let Ok(mut identity) = std::fs::canonicalize(existing) else {
        return path.to_path_buf();
    };
    for component in missing.into_iter().rev() {
        identity.push(component);
    }
    identity
}

pub fn data_root(app_records_root: &Path) -> PathBuf {
    app_records_root.join(".data")
}

pub fn resource_id(app_id: &AppId, collection: &str) -> String {
    format!("app-data:{app_id}:{collection}")
}

pub fn record_resource_id(app_id: &AppId, collection: &str, id: &str) -> String {
    format!("app-data:{app_id}:{collection}:record:{id}")
}

pub fn document_resource_id(app_id: &AppId, collection: &str, id: &str) -> String {
    format!("app-data:{app_id}:{collection}:document:{id}")
}

pub fn handlers_for_exports(
    apps_root: &Path,
    app_id: &AppId,
    data: &AppData,
) -> Result<BTreeMap<CapabilityName, CapabilityHandler>, String> {
    let Some(contract) = data.host_managed() else {
        return Ok(BTreeMap::new());
    };
    let store = ManagedDataStore::new(data_root(apps_root));
    store.validate_contract(app_id, contract)?;
    let mut handlers = BTreeMap::new();
    for export in contract.exports {
        let app_id = app_id.clone();
        let data = data.clone();
        let export = export.clone();
        let data_root = data_root(apps_root);
        let handler: CapabilityHandler = Box::new(move |input, context| {
            if context.cancellation.is_cancelled() {
                return Err(HandlerFailure("managed-data read cancelled".into()));
            }
            let expected_resource = resource_id(&app_id, &export.collection);
            match &context.authorized_data_scope {
                DataScope::Resources { resource_ids }
                    if resource_ids.len() == 1 && resource_ids[0].as_str() == expected_resource => {
                }
                _ => {
                    return Err(HandlerFailure(format!(
                        "managed-data export requires exact resource scope '{expected_resource}'"
                    )))
                }
            }
            let contract = data
                .host_managed()
                .ok_or_else(|| HandlerFailure("managed-data contract is unavailable".into()))?;
            let request = match export.operation {
                ManagedDataExportOperation::Get => ManagedDataRequest::Get {
                    collection: export.collection.clone(),
                    id: required_string(input, "id")?,
                },
                ManagedDataExportOperation::List => ManagedDataRequest::List {
                    collection: export.collection.clone(),
                    query: Some(ManagedDataQuery {
                        index: export.index.clone(),
                        equals: input.get("equals").cloned(),
                        after: optional_string(input, "after")?,
                        limit: optional_u32(input, "limit")?,
                    }),
                },
            };
            let result = ManagedDataStore::new(data_root.clone())
                .request(&app_id, contract, request)
                .map_err(HandlerFailure)?;
            if context.cancellation.is_cancelled() {
                return Err(HandlerFailure("managed-data read cancelled".into()));
            }
            Ok(CapabilityOutcome {
                result,
                artifacts: Vec::new(),
            })
        });
        handlers.insert(export.capability.clone(), handler);
    }
    Ok(handlers)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProposalTargetState {
    kind: &'static str,
    collection: String,
    resource_id: String,
    generation: u64,
    revision: Option<u64>,
}

pub fn handlers_for_proposals(
    apps_root: &Path,
    package_dir: &Path,
    app_id: &AppId,
    data: &AppData,
    expected_package_digest: &str,
) -> Result<BTreeMap<CapabilityName, CapabilityHandler>, String> {
    let Some(contract) = data.host_managed() else {
        return Ok(BTreeMap::new());
    };
    if contract.proposals.is_empty() {
        return Ok(BTreeMap::new());
    }
    let store = ManagedDataStore::new(data_root(apps_root));
    store.validate_contract(app_id, contract)?;
    let mut handlers = BTreeMap::new();
    for proposal in contract.proposals {
        let app_id = app_id.clone();
        let data = data.clone();
        let proposal = proposal.clone();
        let package_dir = package_dir.to_path_buf();
        let expected_package_digest = expected_package_digest.to_string();
        let data_root = data_root(apps_root);
        let capability = proposal.capability.clone();
        let handler: CapabilityHandler = Box::new(move |input, context| {
            if context.cancellation.is_cancelled() {
                return Err(HandlerFailure("managed-data proposal cancelled".into()));
            }
            revalidate_proposal_contract(&package_dir, &app_id, &data, &expected_package_digest)
                .map_err(HandlerFailure)?;
            let expected_scope =
                proposal_scope_from_input(&app_id, &proposal, input).map_err(HandlerFailure)?;
            if context.authorized_data_scope != expected_scope {
                return Err(HandlerFailure(format!(
                    "managed-data proposal requires exact target resource scope '{}', found {:?}",
                    expected_scope_label(&expected_scope),
                    context.authorized_data_scope
                )));
            }
            let payload = input
                .get("payload")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    HandlerFailure("managed-data proposal payload must be an object".into())
                })?
                .clone();
            let payload_bytes = serde_json::to_vec(&payload).map_err(|error| {
                HandlerFailure(format!("serialize proposal payload failed: {error}"))
            })?;
            if payload_bytes.len() > proposal.max_payload_bytes as usize {
                return Err(HandlerFailure(format!(
                    "managed-data proposal payload exceeds the {}-byte limit",
                    proposal.max_payload_bytes
                )));
            }
            validate_schema_value(
                &Value::Object(payload.clone()),
                &proposal.payload_schema,
                "managed-data proposal payload",
            )
            .map_err(HandlerFailure)?;

            let contract = data.host_managed().ok_or_else(|| {
                HandlerFailure("managed-data proposal contract is unavailable".into())
            })?;
            let target = ManagedDataStore::new(data_root.clone())
                .proposal_target(&app_id, contract, &proposal, input)
                .map_err(HandlerFailure)?;
            match target.revision {
                None => {
                    let expected_generation = input
                        .get("targetGeneration")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| {
                            HandlerFailure(
                                "managed-data proposal targetGeneration must be an integer".into(),
                            )
                        })?;
                    if expected_generation != target.generation {
                        return Err(HandlerFailure(format!(
                            "managed-data collection generation is stale; expected {expected_generation}, found {}",
                            target.generation
                        )));
                    }
                }
                Some(revision) => {
                    let expected_revision = input
                        .get("targetRevision")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| {
                            HandlerFailure(
                                "managed-data proposal targetRevision must be an integer".into(),
                            )
                        })?;
                    if expected_revision != revision {
                        return Err(HandlerFailure(format!(
                            "managed-data target revision is stale; expected {expected_revision}, found {revision}"
                        )));
                    }
                }
            }
            let content = serde_json::json!({
                "targetAppId": app_id.as_str(),
                "targetKind": target.kind,
                "collection": target.collection,
                "resourceId": target.resource_id,
                "targetGeneration": target.generation,
                "targetRevision": target.revision,
                "payload": payload,
            });
            if context.cancellation.is_cancelled() {
                return Err(HandlerFailure("managed-data proposal cancelled".into()));
            }
            Ok(CapabilityOutcome {
                result: content.clone(),
                artifacts: vec![ArtifactDraft {
                    artifact_type: proposal.artifact_type.clone(),
                    title: proposal.title.clone(),
                    content,
                }],
            })
        });
        handlers.insert(capability, handler);
    }
    Ok(handlers)
}

fn revalidate_proposal_contract(
    package_dir: &Path,
    app_id: &AppId,
    expected_data: &AppData,
    expected_package_digest: &str,
) -> Result<(), String> {
    let actual_digest = crate::package::package_digest(package_dir)?;
    if actual_digest != expected_package_digest {
        return Err("installed package changed while proposal was executing".into());
    }
    let document = crate::package::read_document(package_dir)?;
    if document.id != app_id.as_str() {
        return Err("installed proposal package identity changed".into());
    }
    if let Some(error) = crate::package::structural_error(&document) {
        return Err(format!(
            "installed proposal package is no longer valid: {error}"
        ));
    }
    if &document.data != expected_data {
        return Err("installed proposal data contract changed while proposal was executing".into());
    }
    Ok(())
}

fn proposal_scope_from_input(
    app_id: &AppId,
    proposal: &ManagedDataProposal,
    input: &JsonObject,
) -> Result<DataScope, String> {
    let resource_id = match &proposal.target {
        ManagedDataProposalTarget::Collection { collection } => resource_id(app_id, collection),
        ManagedDataProposalTarget::Record { collection } => {
            let id = required_string(input, "targetId").map_err(|error| error.0)?;
            validate_id(&id)?;
            record_resource_id(app_id, collection, &id)
        }
        ManagedDataProposalTarget::Document {
            document_collection,
        } => {
            let id = required_string(input, "targetId").map_err(|error| error.0)?;
            validate_id(&id)?;
            document_resource_id(app_id, document_collection, &id)
        }
    };
    DataScope::resources(vec![app_host_kernel::ids::ResourceId::new(resource_id)])
        .map_err(|error| error.to_string())
}

fn expected_scope_label(scope: &DataScope) -> String {
    match scope {
        DataScope::Resources { resource_ids } => resource_ids
            .first()
            .map(ToString::to_string)
            .unwrap_or_default(),
        DataScope::None => "none".into(),
        DataScope::AllResources => "all-resources".into(),
    }
}

fn required_string(input: &JsonObject, field: &str) -> Result<String, HandlerFailure> {
    input
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| HandlerFailure(format!("managed-data input requires string '{field}'")))
}

fn optional_string(input: &JsonObject, field: &str) -> Result<Option<String>, HandlerFailure> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(HandlerFailure(format!(
            "managed-data input '{field}' must be a string or null"
        ))),
    }
}

fn optional_u32(input: &JsonObject, field: &str) -> Result<Option<u32>, HandlerFailure> {
    match input.get(field) {
        None => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| {
                HandlerFailure(format!("managed-data input '{field}' must be an integer"))
            }),
    }
}

fn mutation_operation(operation: &ManagedDataMutation) -> (&str, ManagedDataOperation) {
    match operation {
        ManagedDataMutation::Create { collection, .. } => {
            (collection, ManagedDataOperation::Create)
        }
        ManagedDataMutation::Replace { collection, .. } => {
            (collection, ManagedDataOperation::Replace)
        }
        ManagedDataMutation::Delete { collection, .. } => {
            (collection, ManagedDataOperation::Delete)
        }
    }
}

fn apply_mutation(
    document: &mut ManagedDataDocument,
    contract: HostManagedDataRef<'_>,
    operation: ManagedDataMutation,
) -> Result<Value, String> {
    match operation {
        ManagedDataMutation::Create { collection, value } => {
            serde_json::to_value(apply_create(document, contract, &collection, value)?)
                .map_err(|error| format!("serialize managed-data record failed: {error}"))
        }
        ManagedDataMutation::Replace {
            collection,
            id,
            expected_revision,
            value,
        } => serde_json::to_value(apply_replace(
            document,
            contract,
            &collection,
            &id,
            expected_revision,
            value,
        )?)
        .map_err(|error| format!("serialize managed-data record failed: {error}")),
        ManagedDataMutation::Delete {
            collection,
            id,
            expected_revision,
        } => serde_json::to_value(apply_delete(
            document,
            contract,
            &collection,
            &id,
            expected_revision,
        )?)
        .map_err(|error| format!("serialize managed-data delete failed: {error}")),
    }
}

fn apply_mutation_v2(
    document: &mut ManagedDataDocument,
    contract: HostManagedDataRef<'_>,
    operation: ManagedDataMutation,
) -> Result<Value, String> {
    match operation {
        ManagedDataMutation::Create { collection, value } => {
            let record = apply_create(document, contract, &collection, value)?;
            serde_json::to_value(ManagedDataV2RecordView::from(&record))
                .map_err(|error| format!("serialize managed-data v2 record failed: {error}"))
        }
        ManagedDataMutation::Replace {
            collection,
            id,
            expected_revision,
            value,
        } => {
            let record = apply_replace(
                document,
                contract,
                &collection,
                &id,
                expected_revision,
                value,
            )?;
            serde_json::to_value(ManagedDataV2RecordView::from(&record))
                .map_err(|error| format!("serialize managed-data v2 record failed: {error}"))
        }
        ManagedDataMutation::Delete {
            collection,
            id,
            expected_revision,
        } => serde_json::to_value(apply_delete(
            document,
            contract,
            &collection,
            &id,
            expected_revision,
        )?)
        .map_err(|error| format!("serialize managed-data v2 delete failed: {error}")),
    }
}

fn apply_create(
    document: &mut ManagedDataDocument,
    contract: HostManagedDataRef<'_>,
    collection: &str,
    value: JsonObject,
) -> Result<ManagedDataRecord, String> {
    let declaration = contract
        .collections
        .get(collection)
        .ok_or_else(|| format!("unknown managed-data collection '{collection}'"))?;
    validate_value(&value, declaration, collection)?;
    let records = document
        .collections
        .entry(collection.to_string())
        .or_default();
    if records.len() >= declaration.limits.records as usize {
        return Err(format!(
            "managed-data collection '{collection}' reached its {}-record limit",
            declaration.limits.records
        ));
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let record = ManagedDataRecord {
        id: id.clone(),
        revision: 1,
        created_at: now.clone(),
        updated_at: now,
        value,
    };
    records.insert(id, record.clone());
    validate_document_limits(document, contract)?;
    Ok(record)
}

fn apply_replace(
    document: &mut ManagedDataDocument,
    contract: HostManagedDataRef<'_>,
    collection: &str,
    id: &str,
    expected_revision: u64,
    value: JsonObject,
) -> Result<ManagedDataRecord, String> {
    validate_id(id)?;
    let declaration = contract
        .collections
        .get(collection)
        .ok_or_else(|| format!("unknown managed-data collection '{collection}'"))?;
    validate_value(&value, declaration, collection)?;
    let current = document
        .collections
        .get_mut(collection)
        .and_then(|records| records.get_mut(id))
        .ok_or_else(|| format!("managed-data record '{id}' does not exist"))?;
    if current.revision != expected_revision {
        return Err(format!(
            "managed-data record changed; expected revision {expected_revision}, found {}",
            current.revision
        ));
    }
    current.revision = current
        .revision
        .checked_add(1)
        .ok_or_else(|| "managed-data record revision overflow".to_string())?;
    current.updated_at = Utc::now().to_rfc3339();
    current.value = value;
    let result = current.clone();
    validate_document_limits(document, contract)?;
    Ok(result)
}

fn apply_delete(
    document: &mut ManagedDataDocument,
    contract: HostManagedDataRef<'_>,
    collection: &str,
    id: &str,
    expected_revision: u64,
) -> Result<ManagedDataDeleteResult, String> {
    validate_id(id)?;
    if !contract.collections.contains_key(collection) {
        return Err(format!("unknown managed-data collection '{collection}'"));
    }
    let records = document
        .collections
        .get_mut(collection)
        .ok_or_else(|| format!("managed-data record '{id}' does not exist"))?;
    let current = records
        .get(id)
        .ok_or_else(|| format!("managed-data record '{id}' does not exist"))?;
    if current.revision != expected_revision {
        return Err(format!(
            "managed-data record changed; expected revision {expected_revision}, found {}",
            current.revision
        ));
    }
    let revision = current
        .revision
        .checked_add(1)
        .ok_or_else(|| "managed-data record revision overflow".to_string())?;
    records.remove(id);
    Ok(ManagedDataDeleteResult {
        id: id.to_string(),
        deleted: true,
        revision,
    })
}

fn list_records(
    document: &ManagedDataDocument,
    contract: HostManagedDataRef<'_>,
    collection: &str,
    query: Option<ManagedDataQuery>,
) -> Result<ManagedDataListResult, String> {
    let declaration = contract
        .collections
        .get(collection)
        .ok_or_else(|| format!("unknown managed-data collection '{collection}'"))?;
    let query = query.unwrap_or(ManagedDataQuery {
        index: None,
        equals: None,
        after: None,
        limit: None,
    });
    if let Some(after) = &query.after {
        validate_id(after)?;
    }
    let index = match (&query.index, &query.equals) {
        (None, None) => None,
        (Some(name), Some(equals)) => {
            let index = declaration
                .indexes
                .iter()
                .find(|index| index.name == *name)
                .ok_or_else(|| format!("unknown managed-data index '{name}'"))?;
            validate_schema_value(
                equals,
                &index.value_schema,
                &format!("managed-data index '{name}' query"),
            )?;
            Some((index, equals))
        }
        _ => {
            return Err(
                "managed-data equality query requires both index and equals, or neither".into(),
            )
        }
    };
    let limit = query
        .limit
        .unwrap_or(declaration.limits.query_results)
        .min(declaration.limits.query_results) as usize;
    if limit == 0 {
        return Err("managed-data query limit must be positive".into());
    }
    let empty = BTreeMap::new();
    let records = document.collections.get(collection).unwrap_or(&empty);
    let mut matches = records
        .iter()
        .filter(|(id, _)| query.after.as_ref().is_none_or(|after| *id > after))
        .filter(|(_, record)| match index {
            Some((index, equals)) => record.value.get(&index.field) == Some(equals),
            None => true,
        })
        .map(|(_, record)| record.clone());
    let mut result: Vec<ManagedDataRecord> = matches.by_ref().take(limit + 1).collect();
    let has_more = result.len() > limit;
    if has_more {
        result.pop();
    }
    let next_after = has_more
        .then(|| result.last().map(|record| record.id.clone()))
        .flatten();
    Ok(ManagedDataListResult {
        records: result,
        next_after,
    })
}

fn validate_envelope(document: &ManagedDataDocument) -> Result<(), String> {
    if document.version != STORE_VERSION {
        return Err(format!(
            "unsupported managed-data store version {}; expected {STORE_VERSION}",
            document.version
        ));
    }
    if document.generation == 0 {
        return Err("managed-data store contains a zero generation".into());
    }
    if document.contract_digest.is_empty() {
        return Err("managed-data store contains an empty contract digest".into());
    }
    for (collection, records) in &document.collections {
        if collection.is_empty() || collection.len() > 64 {
            return Err("managed-data store contains an invalid collection name".into());
        }
        for (id, record) in records {
            if id != &record.id {
                return Err(format!(
                    "managed-data record key '{id}' does not match its record id"
                ));
            }
            validate_id(id)?;
            if record.revision == 0 {
                return Err(format!("managed-data record '{id}' has a zero revision"));
            }
            let created_at = chrono::DateTime::parse_from_rfc3339(&record.created_at)
                .map_err(|_| format!("managed-data record '{id}' has an invalid created_at"))?;
            let updated_at = chrono::DateTime::parse_from_rfc3339(&record.updated_at)
                .map_err(|_| format!("managed-data record '{id}' has an invalid updated_at"))?;
            if created_at > updated_at {
                return Err(format!(
                    "managed-data record '{id}' was updated before it was created"
                ));
            }
        }
    }
    let bytes = serde_json::to_vec_pretty(document)
        .map_err(|error| format!("serialize managed-data store failed: {error}"))?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(format!(
            "managed-data store exceeds the {MAX_DOCUMENT_BYTES}-byte host limit"
        ));
    }
    Ok(())
}

fn validate_document(
    document: &ManagedDataDocument,
    contract: HostManagedDataRef<'_>,
) -> Result<(), String> {
    validate_envelope(document)?;
    for (collection, records) in &document.collections {
        let declaration = contract.collections.get(collection).ok_or_else(|| {
            format!("managed-data store contains undeclared collection '{collection}'")
        })?;
        if records.len() > declaration.limits.records as usize {
            return Err(format!(
                "managed-data collection '{collection}' exceeds its record limit"
            ));
        }
        for record in records.values() {
            validate_value(&record.value, declaration, collection)?;
        }
    }
    validate_document_limits(document, contract)
}

fn validate_document_limits(
    document: &ManagedDataDocument,
    contract: HostManagedDataRef<'_>,
) -> Result<(), String> {
    for (collection, records) in &document.collections {
        let declaration = contract.collections.get(collection).ok_or_else(|| {
            format!("managed-data store contains undeclared collection '{collection}'")
        })?;
        for index in declaration.indexes.iter().filter(|index| index.unique) {
            let mut values = BTreeSet::new();
            for record in records.values() {
                let Some(value) = record.value.get(&index.field) else {
                    continue;
                };
                if value.is_null() {
                    continue;
                }
                let canonical = serde_json::to_string(value).map_err(|error| {
                    format!(
                        "serialize managed-data unique index '{}' failed: {error}",
                        index.name
                    )
                })?;
                if !values.insert(canonical) {
                    return Err(format!(
                        "managed-data collection '{collection}' unique index '{}' already contains that value",
                        index.name
                    ));
                }
            }
        }
    }
    let bytes = serde_json::to_vec_pretty(document)
        .map_err(|error| format!("serialize managed-data store failed: {error}"))?;
    let limit = usize::try_from(contract.limits.total_bytes)
        .unwrap_or(usize::MAX)
        .min(MAX_DOCUMENT_BYTES);
    if bytes.len() > limit {
        return Err(format!("managed-data store exceeds its {limit}-byte limit"));
    }
    Ok(())
}

fn validate_optional_record(
    record: Option<&ManagedDataRecord>,
    declaration: &ManagedDataCollection,
) -> Result<(), String> {
    if let Some(record) = record {
        validate_value(&record.value, declaration, "record")?;
    }
    Ok(())
}

fn validate_value(
    value: &JsonObject,
    declaration: &ManagedDataCollection,
    collection: &str,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("serialize managed-data value failed: {error}"))?;
    if bytes.len() > declaration.limits.record_bytes as usize {
        return Err(format!(
            "managed-data value for collection '{collection}' exceeds its {}-byte limit",
            declaration.limits.record_bytes
        ));
    }
    validate_schema_value(
        &Value::Object(value.clone()),
        &declaration.schema,
        &format!("managed-data collection '{collection}' value"),
    )
}

fn validate_schema_value(value: &Value, schema: &JsonObject, label: &str) -> Result<(), String> {
    let validator = jsonschema::validator_for(&Value::Object(schema.clone()))
        .map_err(|error| format!("{label} schema is invalid: {error}"))?;
    let mut errors: Vec<String> = validator
        .iter_errors(value)
        .map(|error| format!("{}: {error}", error.instance_path))
        .collect();
    if errors.is_empty() {
        return Ok(());
    }
    errors.sort();
    Err(format!(
        "{label} does not match its schema: {}",
        errors.join("; ")
    ))
}

fn require_operation<'a>(
    contract: HostManagedDataRef<'a>,
    collection: &str,
    operation: ManagedDataOperation,
) -> Result<&'a ManagedDataCollection, String> {
    let declaration = contract
        .collections
        .get(collection)
        .ok_or_else(|| format!("unknown managed-data collection '{collection}'"))?;
    if !declaration.operations.contains(&operation) {
        return Err(format!(
            "managed-data collection '{collection}' does not permit {operation:?}"
        ));
    }
    Ok(declaration)
}

fn validate_id(id: &str) -> Result<(), String> {
    let canonical = Uuid::parse_str(id)
        .map(|uuid| uuid.get_version_num() == 4 && uuid.hyphenated().to_string() == id)
        .unwrap_or(false);
    if id.len() > MAX_ID_LENGTH || !canonical {
        return Err("managed-data id must be a canonical version-4 UUID".into());
    }
    Ok(())
}

fn contract_digest(contract: HostManagedDataRef<'_>) -> Result<String, String> {
    let bytes = if contract.contract_version == 1 {
        serde_json::to_vec(&(
            contract.contract_version,
            contract.collections,
            contract.limits,
            contract.exports,
        ))
    } else {
        serde_json::to_vec(&(
            contract.contract_version,
            contract.collections,
            contract.documents,
            contract.limits,
            contract.exports,
            contract.proposals,
        ))
    }
    .map_err(|error| format!("serialize managed-data contract failed: {error}"))?;
    Ok(format!("sha256-{:x}", Sha256::digest(bytes)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedDataV2Envelope {
    version: u32,
    generation: u64,
    contract_digest: String,
    collections: BTreeMap<String, BTreeMap<String, ManagedDataRecord>>,
    documents: BTreeMap<String, BTreeMap<String, ManagedDocumentRecord>>,
    receipts: BTreeMap<String, ManagedDataReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedDocumentRecord {
    id: String,
    revision: u64,
    created_at: String,
    updated_at: String,
    metadata: JsonObject,
    content_sha256: String,
    content_length: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedDocumentRecordView {
    pub id: String,
    pub revision: u64,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    pub metadata: JsonObject,
    #[serde(rename = "contentSha256")]
    pub content_sha256: String,
    #[serde(rename = "contentLength")]
    pub content_length: u64,
}

impl From<&ManagedDocumentRecord> for ManagedDocumentRecordView {
    fn from(document: &ManagedDocumentRecord) -> Self {
        Self {
            id: document.id.clone(),
            revision: document.revision,
            created_at: document.created_at.clone(),
            updated_at: document.updated_at.clone(),
            metadata: document.metadata.clone(),
            content_sha256: document.content_sha256.clone(),
            content_length: document.content_length,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedDataReceipt {
    digest: String,
    result: Value,
    generation: u64,
    recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedDataStage {
    version: u32,
    batch_id: String,
    expected_generation: u64,
    contract_digest: String,
    records: Vec<ManagedDataMutation>,
    documents: BTreeMap<String, ManagedDataStageDocument>,
    receipts: BTreeMap<String, ManagedDataReceipt>,
    expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedDataStageDocument {
    #[serde(rename = "stageId")]
    stage_id: String,
    operation: ManagedDataV2DocumentOperation,
    chunks: BTreeMap<u32, String>,
}

impl ManagedDataStore {
    /// Execute a contract-v2 request. This deliberately has its own state and
    /// persistence path: legacy v1 reads and mutations retain their exact
    /// request and result shapes.
    pub fn request_v2(
        &self,
        app_id: &AppId,
        contract: HostManagedDataRef<'_>,
        request: ManagedDataV2Request,
    ) -> Result<Value, String> {
        if contract.contract_version != 2 {
            return Err(format!(
                "managed-data v2 requires contract version 2, found {}",
                contract.contract_version
            ));
        }
        let operation_lock = self.operation_lock(app_id)?;
        let _operation = operation_lock
            .lock()
            .map_err(|_| "managed-data operation lock poisoned".to_string())?;
        self.reject_v2_links(app_id)?;
        self.cleanup_expired_stage(app_id)?;
        let digest = contract_digest(contract)?;
        let mut state = self.load_v2(app_id, contract, &digest)?;
        match request {
            ManagedDataV2Request::ReadSnapshot {
                expected_generation,
                reads,
            } => {
                check_generation(state.generation, expected_generation)?;
                if reads.is_empty() || reads.len() > 64 {
                    return Err("managed-data readSnapshot requires 1-64 reads".into());
                }
                let results = reads
                    .iter()
                    .map(|read| read_snapshot_result(self, app_id, contract, &state, read))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(serde_json::json!({
                    "generation": state.generation,
                    "results": results,
                }))
            }
            ManagedDataV2Request::Get {
                collection,
                id,
                expected_generation,
            } => {
                check_generation(state.generation, expected_generation)?;
                let declaration =
                    require_operation(contract, &collection, ManagedDataOperation::Get)?;
                validate_id(&id)?;
                let record = state
                    .collections
                    .get(&collection)
                    .and_then(|records| records.get(&id))
                    .cloned();
                validate_optional_record(record.as_ref(), declaration)?;
                serde_json::to_value(serde_json::json!({
                    "generation": state.generation,
                    "record": record.as_ref().map(ManagedDataV2RecordView::from),
                }))
                .map_err(|error| format!("serialize managed-data v2 record failed: {error}"))
            }
            ManagedDataV2Request::List {
                collection,
                query,
                expected_generation,
            } => {
                check_generation(state.generation, expected_generation)?;
                require_operation(contract, &collection, ManagedDataOperation::List)?;
                let result = list_records(
                    &ManagedDataDocument {
                        version: STORE_VERSION,
                        generation: state.generation.max(1),
                        contract_digest: digest.clone(),
                        collections: state.collections.clone(),
                    },
                    contract,
                    &collection,
                    query,
                )?;
                serde_json::to_value(serde_json::json!({
                    "generation": state.generation,
                    "records": result
                        .records
                        .iter()
                        .map(ManagedDataV2RecordView::from)
                        .collect::<Vec<_>>(),
                    "nextAfter": result.next_after,
                }))
                .map_err(|error| format!("serialize managed-data v2 list failed: {error}"))
            }
            ManagedDataV2Request::GetDocument {
                collection,
                id,
                offset,
                length,
                expected_generation,
            } => {
                check_generation(state.generation, expected_generation)?;
                require_document_operation(contract, &collection, ManagedDocumentOperation::Get)?;
                validate_id(&id)?;
                let document = state
                    .documents
                    .get(&collection)
                    .and_then(|documents| documents.get(&id))
                    .ok_or_else(|| format!("managed-data document '{id}' does not exist"))?;
                let content = self.read_blob(app_id, document)?;
                let end = offset
                    .checked_add(u64::from(length))
                    .ok_or_else(|| "managed-data document chunk range overflow".to_string())?;
                if length as usize > V2_MAX_CHUNK_BYTES || end > document.content_length {
                    return Err(format!(
                        "managed-data document chunk must be at most {V2_MAX_CHUNK_BYTES} bytes and within the document"
                    ));
                }
                let start = usize::try_from(offset).map_err(|_| "document offset is too large")?;
                let end = usize::try_from(end).map_err(|_| "document end is too large")?;
                let chunk = &content[start..end];
                Ok(serde_json::json!({
                    "generation": state.generation,
                    "document": ManagedDocumentRecordView::from(document),
                    "offset": offset,
                    "contentBase64": BASE64.encode(chunk),
                    "contentLength": document.content_length,
                }))
            }
            ManagedDataV2Request::ListDocuments {
                collection,
                after,
                limit,
                expected_generation,
            } => {
                check_generation(state.generation, expected_generation)?;
                let declaration = require_document_operation(
                    contract,
                    &collection,
                    ManagedDocumentOperation::List,
                )?;
                let after = after.map(|id| validate_id(&id).map(|_| id)).transpose()?;
                let limit = limit.unwrap_or(100).min(100) as usize;
                let documents = state.documents.get(&collection);
                let empty = BTreeMap::new();
                let documents = documents.unwrap_or(&empty);
                let mut listed: Vec<ManagedDocumentRecordView> = documents
                    .iter()
                    .filter(|(id, _)| after.as_ref().is_none_or(|cursor| *id > cursor))
                    .map(|(_, document)| ManagedDocumentRecordView::from(document))
                    .take(limit + 1)
                    .collect();
                if listed.len() > limit {
                    listed.pop();
                }
                let next_after = listed
                    .last()
                    .map(|document| document.id.clone())
                    .filter(|id| documents.keys().any(|candidate| candidate > id));
                let _ = declaration;
                Ok(serde_json::json!({
                    "generation": state.generation,
                    "documents": listed,
                    "nextAfter": next_after,
                }))
            }
            mutation @ ManagedDataV2Request::Create { .. }
            | mutation @ ManagedDataV2Request::Replace { .. }
            | mutation @ ManagedDataV2Request::Delete { .. } => {
                self.execute_v2_record_mutation(app_id, contract, &digest, state, mutation)
            }
            ManagedDataV2Request::BeginBatch {
                mutation_id,
                expected_generation,
                operations,
                documents,
            } => {
                let digest = request_digest(&ManagedDataV2Request::BeginBatch {
                    mutation_id: mutation_id.clone(),
                    expected_generation,
                    operations: operations.clone(),
                    documents: documents.clone(),
                })?;
                validate_mutation_id(&mutation_id)?;
                if let Some(result) = receipt_result(&state.receipts, &mutation_id, &digest)? {
                    return Ok(result);
                }
                if state.generation != expected_generation {
                    return Err(generation_conflict(expected_generation, state.generation));
                }
                if let Some(existing) = self.load_stage(app_id)? {
                    if let Some(result) = receipt_result(&existing.receipts, &mutation_id, &digest)?
                    {
                        return Ok(result);
                    }
                    return Err(
                        "managed-data already has an active batch; commit or abort it first".into(),
                    );
                }
                validate_batch_declarations(contract, &operations, &documents)?;
                let batch_id = Uuid::new_v4().to_string();
                let mut staged_documents = BTreeMap::new();
                let mut document_ids = Vec::new();
                let mut stage_ids = BTreeSet::new();
                let mut document_targets = BTreeSet::new();
                for operation in documents {
                    let (id, stage_id) = match &operation {
                        ManagedDataV2DocumentOperation::Create { stage_id, .. }
                        | ManagedDataV2DocumentOperation::Replace { stage_id, .. } => {
                            validate_stage_id(stage_id)?;
                            if !stage_ids.insert(stage_id.clone()) {
                                return Err(format!(
                                    "managed-data batch contains duplicate stageId '{stage_id}'"
                                ));
                            }
                            let id = match &operation {
                                ManagedDataV2DocumentOperation::Create { .. } => {
                                    Uuid::new_v4().to_string()
                                }
                                ManagedDataV2DocumentOperation::Replace { id, .. } => {
                                    validate_id(id)?;
                                    id.clone()
                                }
                                ManagedDataV2DocumentOperation::UpdateMetadata { .. }
                                | ManagedDataV2DocumentOperation::Delete { .. } => unreachable!(),
                            };
                            (id, stage_id.clone())
                        }
                        ManagedDataV2DocumentOperation::UpdateMetadata { id, .. }
                        | ManagedDataV2DocumentOperation::Delete { id, .. } => {
                            validate_id(id)?;
                            (id.clone(), String::new())
                        }
                    };
                    if !document_targets.insert(id.clone()) {
                        return Err(format!(
                            "managed-data batch targets document '{id}' more than once"
                        ));
                    }
                    validate_document_operation_input(contract, &operation, &state)?;
                    staged_documents.insert(
                        id.clone(),
                        ManagedDataStageDocument {
                            stage_id: stage_id.clone(),
                            operation,
                            chunks: BTreeMap::new(),
                        },
                    );
                    if !stage_id.is_empty() {
                        document_ids.push(serde_json::json!({
                            "stageId": stage_id,
                            "documentId": id,
                        }));
                    }
                }
                let stage = ManagedDataStage {
                    version: V2_STORE_VERSION,
                    batch_id: batch_id.clone(),
                    expected_generation,
                    contract_digest: digest.clone(),
                    records: operations,
                    documents: staged_documents,
                    receipts: BTreeMap::new(),
                    expires_at: now_epoch_seconds().saturating_add(V2_STAGE_TTL_SECONDS),
                };
                self.persist_stage(app_id, &stage)?;
                let result = serde_json::json!({
                    "batchId": batch_id,
                    "generation": state.generation,
                    "documents": document_ids,
                });
                // The begin receipt belongs to the durable stage, not the
                // published state. Incomplete work therefore remains invisible.
                let mut stage = stage;
                stage.receipts.insert(
                    mutation_id,
                    ManagedDataReceipt {
                        digest,
                        result: result.clone(),
                        generation: state.generation,
                        recorded_at: Utc::now().to_rfc3339(),
                    },
                );
                self.persist_stage(app_id, &stage)?;
                Ok(result)
            }
            ManagedDataV2Request::AppendBatchOperations {
                mutation_id,
                batch_id,
                operations,
            } => {
                validate_mutation_id(&mutation_id)?;
                if operations.is_empty() || operations.len() > V2_MAX_BATCH_APPEND_OPERATIONS {
                    return Err(format!(
                        "managed-data batch operation appends must contain 1-{V2_MAX_BATCH_APPEND_OPERATIONS} operations"
                    ));
                }
                let mut stage = self
                    .load_stage(app_id)?
                    .ok_or_else(|| "managed-data batch does not exist".to_string())?;
                if stage.batch_id != batch_id || stage.expires_at <= now_epoch_seconds() {
                    return Err("managed-data batch is expired or unknown".into());
                }
                let digest = request_digest(&ManagedDataV2Request::AppendBatchOperations {
                    mutation_id: mutation_id.clone(),
                    batch_id: batch_id.clone(),
                    operations: operations.clone(),
                })?;
                if let Some(result) = receipt_result(&stage.receipts, &mutation_id, &digest)? {
                    return Ok(result);
                }
                validate_batch_record_operations(contract, &operations)?;
                let limit = contract.limits.batch_operations.ok_or_else(|| {
                    "managed-data contract is missing batch_operations".to_string()
                })? as usize;
                let staged_operations = stage.records.len() + stage.documents.len();
                if staged_operations.saturating_add(operations.len()) > limit {
                    return Err(format!(
                        "managed-data batch exceeds its {limit}-operation limit"
                    ));
                }
                stage.records.extend(operations);
                let result = serde_json::json!({
                    "batchId": batch_id,
                    "appended": stage.records.len() + stage.documents.len(),
                });
                stage.receipts.insert(
                    mutation_id,
                    ManagedDataReceipt {
                        digest,
                        result: result.clone(),
                        generation: state.generation,
                        recorded_at: Utc::now().to_rfc3339(),
                    },
                );
                trim_receipts(&mut stage.receipts)?;
                self.persist_stage(app_id, &stage)?;
                Ok(result)
            }
            ManagedDataV2Request::AppendDocumentChunk {
                mutation_id,
                batch_id,
                document_id,
                chunk_index,
                content_base64,
            } => {
                validate_mutation_id(&mutation_id)?;
                let mut stage = self
                    .load_stage(app_id)?
                    .ok_or_else(|| "managed-data batch does not exist".to_string())?;
                if stage.batch_id != batch_id || stage.expires_at <= now_epoch_seconds() {
                    return Err("managed-data batch is expired or unknown".into());
                }
                let digest = request_digest(&ManagedDataV2Request::AppendDocumentChunk {
                    mutation_id: mutation_id.clone(),
                    batch_id: batch_id.clone(),
                    document_id: document_id.clone(),
                    chunk_index,
                    content_base64: content_base64.clone(),
                })?;
                if let Some(result) = receipt_result(&stage.receipts, &mutation_id, &digest)? {
                    return Ok(result);
                }
                let bytes = BASE64
                    .decode(content_base64.as_bytes())
                    .map_err(|_| "managed-data document chunk is not valid base64".to_string())?;
                if bytes.is_empty() || bytes.len() > V2_MAX_CHUNK_BYTES {
                    return Err(format!(
                        "managed-data document chunks must be 1-{V2_MAX_CHUNK_BYTES} bytes"
                    ));
                }
                let document = stage.documents.get_mut(&document_id).ok_or_else(|| {
                    "managed-data batch does not contain that document".to_string()
                })?;
                if matches!(
                    document.operation,
                    ManagedDataV2DocumentOperation::UpdateMetadata { .. }
                        | ManagedDataV2DocumentOperation::Delete { .. }
                ) {
                    return Err(
                        "managed-data metadata-only and delete operations do not accept content chunks".into(),
                    );
                }
                if document.chunks.contains_key(&chunk_index) {
                    return Err("managed-data document chunk index was already written".into());
                }
                document.chunks.insert(chunk_index, BASE64.encode(bytes));
                self.validate_stage_size(&stage)?;
                let result = serde_json::json!({ "batchId": batch_id, "documentId": document_id, "chunkIndex": chunk_index });
                stage.receipts.insert(
                    mutation_id,
                    ManagedDataReceipt {
                        digest,
                        result: result.clone(),
                        generation: state.generation,
                        recorded_at: Utc::now().to_rfc3339(),
                    },
                );
                trim_receipts(&mut stage.receipts)?;
                self.persist_stage(app_id, &stage)?;
                Ok(result)
            }
            ManagedDataV2Request::CommitBatch {
                mutation_id,
                batch_id,
            } => {
                validate_mutation_id(&mutation_id)?;
                let digest = request_digest(&ManagedDataV2Request::CommitBatch {
                    mutation_id: mutation_id.clone(),
                    batch_id: batch_id.clone(),
                })?;
                if let Some(result) = receipt_result(&state.receipts, &mutation_id, &digest)? {
                    return Ok(result);
                }
                let stage = self
                    .load_stage(app_id)?
                    .ok_or_else(|| "managed-data batch does not exist".to_string())?;
                if stage.batch_id != batch_id || stage.expires_at <= now_epoch_seconds() {
                    return Err("managed-data batch is expired or unknown".into());
                }
                if state.generation != stage.expected_generation {
                    return Err(generation_conflict(
                        stage.expected_generation,
                        state.generation,
                    ));
                }
                let result =
                    self.commit_stage(app_id, contract, &digest, &mut state, stage, mutation_id)?;
                Ok(result)
            }
            ManagedDataV2Request::AbortBatch {
                mutation_id,
                batch_id,
            } => {
                validate_mutation_id(&mutation_id)?;
                let digest = request_digest(&ManagedDataV2Request::AbortBatch {
                    mutation_id: mutation_id.clone(),
                    batch_id: batch_id.clone(),
                })?;
                if let Some(result) = receipt_result(&state.receipts, &mutation_id, &digest)? {
                    return Ok(result);
                }
                let stage = self
                    .load_stage(app_id)?
                    .ok_or_else(|| "managed-data batch does not exist".to_string())?;
                if stage.batch_id != batch_id {
                    return Err("managed-data batch id does not match".into());
                }
                let result = serde_json::json!({ "batchId": batch_id, "aborted": true, "generation": state.generation });
                state.receipts.insert(
                    mutation_id,
                    ManagedDataReceipt {
                        digest,
                        result: result.clone(),
                        generation: state.generation,
                        recorded_at: Utc::now().to_rfc3339(),
                    },
                );
                self.persist_v2_current(app_id, contract, &contract_digest(contract)?, &mut state)?;
                let _ = std::fs::remove_file(self.stage_path(app_id));
                Ok(result)
            }
        }
    }

    fn execute_v2_record_mutation(
        &self,
        app_id: &AppId,
        contract: HostManagedDataRef<'_>,
        contract_digest: &str,
        mut state: ManagedDataV2Envelope,
        request: ManagedDataV2Request,
    ) -> Result<Value, String> {
        let (mutation_id, expected_generation) = match &request {
            ManagedDataV2Request::Create {
                mutation_id,
                expected_generation,
                ..
            }
            | ManagedDataV2Request::Replace {
                mutation_id,
                expected_generation,
                ..
            }
            | ManagedDataV2Request::Delete {
                mutation_id,
                expected_generation,
                ..
            } => (mutation_id.clone(), *expected_generation),
            _ => unreachable!(),
        };
        validate_mutation_id(&mutation_id)?;
        let digest = request_digest(&request)?;
        if let Some(result) = receipt_result(&state.receipts, &mutation_id, &digest)? {
            return Ok(result);
        }
        if state.generation != expected_generation {
            return Err(generation_conflict(expected_generation, state.generation));
        }
        let mut document = ManagedDataDocument {
            version: STORE_VERSION,
            generation: state.generation.max(1),
            contract_digest: contract_digest.to_string(),
            collections: state.collections,
        };
        let result = match request {
            ManagedDataV2Request::Create {
                collection, value, ..
            } => {
                require_operation(contract, &collection, ManagedDataOperation::Create)?;
                let record = apply_create(&mut document, contract, &collection, value)?;
                serde_json::to_value(ManagedDataV2RecordView::from(&record))
                    .map_err(|error| format!("serialize managed-data v2 record failed: {error}"))?
            }
            ManagedDataV2Request::Replace {
                collection,
                id,
                expected_revision,
                value,
                ..
            } => {
                require_operation(contract, &collection, ManagedDataOperation::Replace)?;
                let record = apply_replace(
                    &mut document,
                    contract,
                    &collection,
                    &id,
                    expected_revision,
                    value,
                )?;
                serde_json::to_value(ManagedDataV2RecordView::from(&record))
                    .map_err(|error| format!("serialize managed-data v2 record failed: {error}"))?
            }
            ManagedDataV2Request::Delete {
                collection,
                id,
                expected_revision,
                ..
            } => {
                require_operation(contract, &collection, ManagedDataOperation::Delete)?;
                serde_json::to_value(apply_delete(
                    &mut document,
                    contract,
                    &collection,
                    &id,
                    expected_revision,
                )?)
                .map_err(|error| format!("serialize managed-data v2 delete failed: {error}"))?
            }
            _ => unreachable!(),
        };
        state.collections = document.collections;
        state.receipts.insert(
            mutation_id.clone(),
            ManagedDataReceipt {
                digest,
                result: result.clone(),
                generation: state.generation.saturating_add(1),
                recorded_at: Utc::now().to_rfc3339(),
            },
        );
        trim_receipts(&mut state.receipts)?;
        self.persist_v2(app_id, contract, contract_digest, &mut state)?;
        Ok(result)
    }

    fn commit_stage(
        &self,
        app_id: &AppId,
        contract: HostManagedDataRef<'_>,
        request_digest: &str,
        state: &mut ManagedDataV2Envelope,
        stage: ManagedDataStage,
        mutation_id: String,
    ) -> Result<Value, String> {
        let contract_digest = contract_digest(contract)?;
        let mut candidate = state.clone();
        let mut new_blobs = Vec::new();
        let mut state_persisted = false;
        let outcome = (|| -> Result<Value, String> {
            let mut record_results = Vec::new();
            let mut record_document = ManagedDataDocument {
                version: STORE_VERSION,
                generation: candidate.generation.max(1),
                contract_digest: contract_digest.to_string(),
                collections: candidate.collections.clone(),
            };
            for operation in stage.records {
                let (collection, required) = mutation_operation(&operation);
                let declaration =
                    require_operation(contract, collection, ManagedDataOperation::Transaction)?;
                if !declaration.operations.contains(&required) {
                    return Err(format!(
                        "managed-data collection '{collection}' does not permit {required:?}"
                    ));
                }
                record_results.push(apply_mutation_v2(
                    &mut record_document,
                    contract,
                    operation,
                )?);
            }
            candidate.collections = record_document.collections;
            let mut document_results = Vec::new();
            for (id, staged) in &stage.documents {
                match &staged.operation {
                    ManagedDataV2DocumentOperation::Delete {
                        collection,
                        expected_revision,
                        ..
                    } => {
                        require_document_operation(
                            contract,
                            collection,
                            ManagedDocumentOperation::Delete,
                        )?;
                        let documents =
                            candidate.documents.get_mut(collection).ok_or_else(|| {
                                format!("managed-data document '{id}' does not exist")
                            })?;
                        let current = documents.get(id).ok_or_else(|| {
                            format!("managed-data document '{id}' does not exist")
                        })?;
                        if current.revision != *expected_revision {
                            return Err(format!("managed-data document changed; expected revision {expected_revision}, found {}", current.revision));
                        }
                        let revision = current.revision;
                        documents.remove(id);
                        document_results.push(
                        serde_json::json!({ "id": id, "deleted": true, "revision": revision + 1 }),
                    );
                    }
                    ManagedDataV2DocumentOperation::Create {
                        collection,
                        metadata,
                        content_length,
                        content_sha256,
                        ..
                    } => {
                        require_document_operation(
                            contract,
                            collection,
                            ManagedDocumentOperation::Create,
                        )?;
                        let declaration = contract.documents.get(collection).ok_or_else(|| {
                            format!("unknown managed-data document collection '{collection}'")
                        })?;
                        validate_document_metadata(metadata, declaration, collection)?;
                        let content = staged_content(staged, *content_length, content_sha256)?;
                        if write_blob(self, app_id, content_sha256, &content)? {
                            new_blobs.push(content_sha256.clone());
                        }
                        let now = Utc::now().to_rfc3339();
                        let document = ManagedDocumentRecord {
                            id: id.clone(),
                            revision: 1,
                            created_at: now.clone(),
                            updated_at: now,
                            metadata: metadata.clone(),
                            content_sha256: content_sha256.clone(),
                            content_length: *content_length,
                        };
                        candidate
                            .documents
                            .entry(collection.clone())
                            .or_default()
                            .insert(id.clone(), document.clone());
                        document_results.push(
                            serde_json::to_value(ManagedDocumentRecordView::from(&document))
                                .map_err(|error| error.to_string())?,
                        );
                    }
                    ManagedDataV2DocumentOperation::Replace {
                        collection,
                        expected_revision,
                        metadata,
                        content_length,
                        content_sha256,
                        ..
                    } => {
                        require_document_operation(
                            contract,
                            collection,
                            ManagedDocumentOperation::Replace,
                        )?;
                        let declaration = contract.documents.get(collection).ok_or_else(|| {
                            format!("unknown managed-data document collection '{collection}'")
                        })?;
                        validate_document_metadata(metadata, declaration, collection)?;
                        let content = staged_content(staged, *content_length, content_sha256)?;
                        let documents =
                            candidate.documents.get_mut(collection).ok_or_else(|| {
                                format!("managed-data document '{id}' does not exist")
                            })?;
                        let current = documents.get(id).ok_or_else(|| {
                            format!("managed-data document '{id}' does not exist")
                        })?;
                        if current.revision != *expected_revision {
                            return Err(format!("managed-data document changed; expected revision {expected_revision}, found {}", current.revision));
                        }
                        if write_blob(self, app_id, content_sha256, &content)? {
                            new_blobs.push(content_sha256.clone());
                        }
                        let current = documents.get_mut(id).ok_or_else(|| {
                            format!("managed-data document '{id}' does not exist")
                        })?;
                        current.revision = current
                            .revision
                            .checked_add(1)
                            .ok_or_else(|| "managed-data document revision overflow".to_string())?;
                        current.updated_at = Utc::now().to_rfc3339();
                        current.metadata = metadata.clone();
                        current.content_sha256 = content_sha256.clone();
                        current.content_length = *content_length;
                        document_results.push(
                            serde_json::to_value(ManagedDocumentRecordView::from(&*current))
                                .map_err(|error| error.to_string())?,
                        );
                    }
                    ManagedDataV2DocumentOperation::UpdateMetadata {
                        collection,
                        id,
                        expected_revision,
                        metadata,
                    } => {
                        require_document_operation(
                            contract,
                            collection,
                            ManagedDocumentOperation::UpdateMetadata,
                        )?;
                        let declaration = contract.documents.get(collection).ok_or_else(|| {
                            format!("unknown managed-data document collection '{collection}'")
                        })?;
                        validate_document_metadata(metadata, declaration, collection)?;
                        let documents =
                            candidate.documents.get_mut(collection).ok_or_else(|| {
                                format!("managed-data document '{id}' does not exist")
                            })?;
                        let current = documents.get_mut(id).ok_or_else(|| {
                            format!("managed-data document '{id}' does not exist")
                        })?;
                        if current.revision != *expected_revision {
                            return Err(format!(
                            "managed-data document changed; expected revision {expected_revision}, found {}",
                            current.revision
                        ));
                        }
                        current.revision = current
                            .revision
                            .checked_add(1)
                            .ok_or_else(|| "managed-data document revision overflow".to_string())?;
                        current.updated_at = Utc::now().to_rfc3339();
                        current.metadata = metadata.clone();
                        document_results.push(
                            serde_json::to_value(ManagedDocumentRecordView::from(&*current))
                                .map_err(|error| error.to_string())?,
                        );
                    }
                }
            }
            validate_v2_state(&candidate, contract, self, app_id)?;
            let result = serde_json::json!({ "generation": candidate.generation + 1, "records": record_results, "documents": document_results });
            candidate.receipts.insert(
                mutation_id,
                ManagedDataReceipt {
                    digest: request_digest.to_string(),
                    result: result.clone(),
                    generation: candidate.generation + 1,
                    recorded_at: Utc::now().to_rfc3339(),
                },
            );
            trim_receipts(&mut candidate.receipts)?;
            match self.persist_v2_atomic(app_id, contract, &contract_digest, &mut candidate) {
                Ok(()) => {
                    state_persisted = true;
                    Ok(result)
                }
                Err(error) => {
                    if error.is_indeterminate() {
                        state_persisted = true;
                    }
                    Err(error.into_message())
                }
            }
        })();
        let result = match outcome {
            Ok(result) => result,
            Err(error) => {
                if !state_persisted {
                    if let Err(cleanup) = self.remove_created_blobs(app_id, &new_blobs) {
                        return Err(format!("{error}; {cleanup}"));
                    }
                }
                return Err(error);
            }
        };
        *state = candidate;
        let _ = std::fs::remove_file(self.stage_path(app_id));
        Ok(result)
    }

    fn load_v2(
        &self,
        app_id: &AppId,
        contract: HostManagedDataRef<'_>,
        digest: &str,
    ) -> Result<ManagedDataV2Envelope, String> {
        if self.v2_path(app_id).is_file() {
            let state: ManagedDataV2Envelope =
                load_json_document(&self.v2_path(app_id), "managed data v2")?
                    .ok_or_else(|| "managed data v2 disappeared while loading".to_string())?;
            validate_v2_state(&state, contract, self, app_id)?;
            return Ok(state);
        }
        let legacy = self.load(
            app_id,
            HostManagedDataRef {
                contract_version: 1,
                collections: contract.collections,
                documents: &BTreeMap::new(),
                limits: contract.limits,
                exports: contract.exports,
                proposals: &[],
            },
            digest,
        )?;
        let state = ManagedDataV2Envelope {
            version: V2_STORE_VERSION,
            generation: legacy.generation,
            contract_digest: digest.to_string(),
            collections: legacy.collections,
            documents: BTreeMap::new(),
            receipts: BTreeMap::new(),
        };
        validate_v2_state(&state, contract, self, app_id)?;
        Ok(state)
    }

    fn persist_v2(
        &self,
        app_id: &AppId,
        contract: HostManagedDataRef<'_>,
        digest: &str,
        state: &mut ManagedDataV2Envelope,
    ) -> Result<(), String> {
        self.persist_v2_atomic(app_id, contract, digest, state)
            .map_err(AtomicJsonError::into_message)
    }

    fn persist_v2_atomic(
        &self,
        app_id: &AppId,
        contract: HostManagedDataRef<'_>,
        digest: &str,
        state: &mut ManagedDataV2Envelope,
    ) -> Result<(), AtomicJsonError> {
        state.version = V2_STORE_VERSION;
        state.contract_digest = digest.to_string();
        state.generation = state.generation.checked_add(1).ok_or_else(|| {
            AtomicJsonError::NotCommitted("managed-data generation overflow".into())
        })?;
        self.persist_v2_current_atomic(app_id, contract, digest, state)
    }

    fn persist_v2_current(
        &self,
        app_id: &AppId,
        contract: HostManagedDataRef<'_>,
        digest: &str,
        state: &mut ManagedDataV2Envelope,
    ) -> Result<(), String> {
        self.persist_v2_current_atomic(app_id, contract, digest, state)
            .map_err(AtomicJsonError::into_message)
    }

    fn persist_v2_current_atomic(
        &self,
        app_id: &AppId,
        contract: HostManagedDataRef<'_>,
        digest: &str,
        state: &mut ManagedDataV2Envelope,
    ) -> Result<(), AtomicJsonError> {
        state.version = V2_STORE_VERSION;
        state.contract_digest = digest.to_string();
        validate_v2_state(state, contract, self, app_id).map_err(AtomicJsonError::NotCommitted)?;
        persist_json_document(
            &self.v2_path(app_id),
            state,
            "managed data v2",
            self.writer.as_ref(),
        )
    }

    fn remove_created_blobs(&self, app_id: &AppId, hashes: &[String]) -> Result<(), String> {
        for hash in hashes {
            match self.writer.remove_file(&self.blob_path(app_id, hash)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "remove managed-data blob '{}' after failed commit failed: {error}",
                        hash
                    ));
                }
            }
        }
        Ok(())
    }

    fn persist_stage(&self, app_id: &AppId, stage: &ManagedDataStage) -> Result<(), String> {
        self.validate_stage_size(stage)?;
        persist_json_document(
            &self.stage_path(app_id),
            stage,
            "managed data v2 batch",
            self.writer.as_ref(),
        )
        .map_err(AtomicJsonError::into_message)
    }

    fn load_stage(&self, app_id: &AppId) -> Result<Option<ManagedDataStage>, String> {
        if !self.stage_path(app_id).exists() {
            return Ok(None);
        }
        let stage: ManagedDataStage =
            load_json_document(&self.stage_path(app_id), "managed data v2 batch")?
                .ok_or_else(|| "managed data v2 batch disappeared".to_string())?;
        if stage.version != V2_STORE_VERSION {
            return Err("unsupported managed-data v2 batch version".into());
        }
        self.validate_stage_size(&stage)?;
        Ok(Some(stage))
    }

    fn cleanup_expired_stage(&self, app_id: &AppId) -> Result<(), String> {
        if let Some(stage) = self.load_stage(app_id)? {
            if stage.expires_at <= now_epoch_seconds() {
                std::fs::remove_file(self.stage_path(app_id)).map_err(|error| {
                    format!("remove expired managed-data batch failed: {error}")
                })?;
            }
        }
        Ok(())
    }

    fn validate_stage_size(&self, stage: &ManagedDataStage) -> Result<(), String> {
        if stage.records.len().saturating_add(stage.documents.len())
            > MAX_MANAGED_DATA_BATCH_OPERATIONS as usize
        {
            return Err(format!(
                "managed-data batch exceeds the {MAX_MANAGED_DATA_BATCH_OPERATIONS}-operation host limit"
            ));
        }
        let bytes = serde_json::to_vec(stage)
            .map_err(|error| format!("serialize managed-data batch failed: {error}"))?;
        if bytes.len() > V2_MAX_STAGE_BYTES {
            return Err(format!(
                "managed-data batch exceeds the {V2_MAX_STAGE_BYTES}-byte limit"
            ));
        }
        let mut decoded_total = 0usize;
        for document in stage.documents.values() {
            if matches!(
                &document.operation,
                ManagedDataV2DocumentOperation::UpdateMetadata { .. }
                    | ManagedDataV2DocumentOperation::Delete { .. }
            ) && !document.chunks.is_empty()
            {
                return Err(
                    "managed-data metadata-only and delete operations cannot contain content chunks"
                        .into(),
                );
            }
            for chunk in document.chunks.values() {
                let decoded = BASE64
                    .decode(chunk.as_bytes())
                    .map_err(|_| "managed-data staged chunk is invalid base64".to_string())?;
                if decoded.is_empty() || decoded.len() > V2_MAX_CHUNK_BYTES {
                    return Err(format!(
                        "managed-data staged chunks must be 1-{V2_MAX_CHUNK_BYTES} decoded bytes"
                    ));
                }
                decoded_total = decoded_total
                    .checked_add(decoded.len())
                    .ok_or_else(|| "managed-data staged content quota overflow".to_string())?;
            }
        }
        if decoded_total > V2_MAX_STAGE_DECODED_BYTES {
            return Err("managed-data staged decoded content exceeds the 64 MiB host limit".into());
        }
        Ok(())
    }

    fn read_blob(
        &self,
        app_id: &AppId,
        document: &ManagedDocumentRecord,
    ) -> Result<Vec<u8>, String> {
        let path = self.blob_path(app_id, &document.content_sha256);
        self.reject_linked_path(&path, "managed data blob")?;
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("read managed-data document blob failed: {error}"))?;
        if bytes.len() as u64 != document.content_length
            || format!("sha256-{:x}", Sha256::digest(&bytes)) != document.content_sha256
        {
            return Err("managed-data document blob failed integrity validation".into());
        }
        Ok(bytes)
    }

    fn reject_v2_links(&self, app_id: &AppId) -> Result<(), String> {
        self.reject_linked_path(&self.v2_path(app_id), "managed data v2")?;
        self.reject_linked_path(&self.stage_path(app_id), "managed data v2 batch")?;
        self.reject_linked_path(&self.blob_dir(app_id), "managed data v2 blob directory")
    }

    fn v2_path(&self, app_id: &AppId) -> PathBuf {
        self.data_root.join(app_id.as_str()).join(V2_STORE_FILE)
    }
    fn stage_path(&self, app_id: &AppId) -> PathBuf {
        self.data_root.join(app_id.as_str()).join(V2_STAGE_FILE)
    }
    fn blob_dir(&self, app_id: &AppId) -> PathBuf {
        self.data_root.join(app_id.as_str()).join(V2_BLOB_DIR)
    }
    fn blob_path(&self, app_id: &AppId, hash: &str) -> PathBuf {
        self.blob_dir(app_id)
            .join(hash.strip_prefix("sha256-").unwrap_or(hash))
    }
}

fn check_generation(actual: u64, expected: Option<u64>) -> Result<(), String> {
    if let Some(expected) = expected {
        if expected != actual {
            return Err(generation_conflict(expected, actual));
        }
    }
    Ok(())
}

fn generation_conflict(expected: u64, found: u64) -> String {
    format!("managed-data generation conflict; expected {expected}, found {found}")
}

fn validate_mutation_id(id: &str) -> Result<(), String> {
    if id.len() > 128
        || id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err("managed-data mutation_id must be a bounded ASCII identifier".into());
    }
    Ok(())
}

fn validate_stage_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(
            "managed-data stageId must be a bounded ASCII identifier (1-64 characters)".into(),
        );
    }
    Ok(())
}

fn request_digest(request: &ManagedDataV2Request) -> Result<String, String> {
    let value = serde_json::to_vec(request)
        .map_err(|error| format!("serialize managed-data request failed: {error}"))?;
    Ok(format!("sha256-{:x}", Sha256::digest(value)))
}

fn receipt_result(
    receipts: &BTreeMap<String, ManagedDataReceipt>,
    id: &str,
    digest: &str,
) -> Result<Option<Value>, String> {
    let Some(receipt) = receipts.get(id) else {
        return Ok(None);
    };
    if receipt.digest != digest {
        return Err(format!(
            "managed-data mutation id '{id}' was reused with a different request"
        ));
    }
    Ok(Some(receipt.result.clone()))
}

fn trim_receipts(receipts: &mut BTreeMap<String, ManagedDataReceipt>) -> Result<(), String> {
    if receipts.len() > V2_MAX_RECEIPTS {
        return Err(
            "managed-data receipt quota exceeded; refusing mutation to preserve idempotency".into(),
        );
    }
    Ok(())
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

fn require_document_operation<'a>(
    contract: HostManagedDataRef<'a>,
    collection: &str,
    operation: ManagedDocumentOperation,
) -> Result<&'a ManagedDocumentCollection, String> {
    let declaration = contract
        .documents
        .get(collection)
        .ok_or_else(|| format!("unknown managed-data document collection '{collection}'"))?;
    if !declaration.operations.contains(&operation) {
        return Err(format!(
            "managed-data document collection '{collection}' does not permit {operation:?}"
        ));
    }
    Ok(declaration)
}

fn validate_document_metadata(
    metadata: &JsonObject,
    declaration: &ManagedDocumentCollection,
    collection: &str,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(metadata)
        .map_err(|error| format!("serialize document metadata failed: {error}"))?;
    if bytes.len() > declaration.limits.metadata_bytes as usize {
        return Err(format!(
            "managed-data document metadata for collection '{collection}' exceeds its limit"
        ));
    }
    validate_schema_value(
        &Value::Object(metadata.clone()),
        &declaration.metadata_schema,
        &format!("managed-data document collection '{collection}' metadata"),
    )
}

fn validate_batch_declarations(
    contract: HostManagedDataRef<'_>,
    records: &[ManagedDataMutation],
    documents: &[ManagedDataV2DocumentOperation],
) -> Result<(), String> {
    if records.len() > V2_MAX_BATCH_APPEND_OPERATIONS
        || documents.len() > V2_MAX_BATCH_APPEND_OPERATIONS
    {
        return Err(format!(
            "managed-data batch initial chunks must contain at most {V2_MAX_BATCH_APPEND_OPERATIONS} operations"
        ));
    }
    let limit = contract
        .limits
        .batch_operations
        .ok_or_else(|| "managed-data contract is missing batch_operations".to_string())?
        as usize;
    if records.len().saturating_add(documents.len()) > limit {
        return Err(format!(
            "managed-data batch exceeds its {limit}-operation limit"
        ));
    }
    validate_batch_record_operations(contract, records)?;
    for operation in documents {
        let (collection, required) = match operation {
            ManagedDataV2DocumentOperation::Create { collection, .. } => {
                (collection, ManagedDocumentOperation::Create)
            }
            ManagedDataV2DocumentOperation::Replace { collection, .. } => {
                (collection, ManagedDocumentOperation::Replace)
            }
            ManagedDataV2DocumentOperation::Delete { collection, .. } => {
                (collection, ManagedDocumentOperation::Delete)
            }
            ManagedDataV2DocumentOperation::UpdateMetadata { collection, .. } => {
                (collection, ManagedDocumentOperation::UpdateMetadata)
            }
        };
        require_document_operation(contract, collection, required)?;
    }
    Ok(())
}

fn validate_batch_record_operations(
    contract: HostManagedDataRef<'_>,
    records: &[ManagedDataMutation],
) -> Result<(), String> {
    for operation in records {
        let (collection, required) = mutation_operation(operation);
        let declaration =
            require_operation(contract, collection, ManagedDataOperation::Transaction)?;
        if !declaration.operations.contains(&required) {
            return Err(format!(
                "managed-data collection '{collection}' does not permit {required:?}"
            ));
        }
    }
    Ok(())
}

fn validate_document_operation_input(
    contract: HostManagedDataRef<'_>,
    operation: &ManagedDataV2DocumentOperation,
    state: &ManagedDataV2Envelope,
) -> Result<(), String> {
    let (collection, metadata) = match operation {
        ManagedDataV2DocumentOperation::Create {
            collection,
            metadata,
            ..
        }
        | ManagedDataV2DocumentOperation::Replace {
            collection,
            metadata,
            ..
        }
        | ManagedDataV2DocumentOperation::UpdateMetadata {
            collection,
            metadata,
            ..
        } => (collection, metadata),
        ManagedDataV2DocumentOperation::Delete { .. } => return Ok(()),
    };
    let declaration = contract
        .documents
        .get(collection)
        .ok_or_else(|| format!("unknown managed-data document collection '{collection}'"))?;
    validate_document_metadata(metadata, declaration, collection)?;
    if let ManagedDataV2DocumentOperation::Create {
        content_length,
        content_sha256,
        ..
    }
    | ManagedDataV2DocumentOperation::Replace {
        content_length,
        content_sha256,
        ..
    } = operation
    {
        if *content_length > V2_MAX_DOCUMENT_BYTES as u64
            || *content_length > declaration.limits.content_bytes
        {
            return Err(
                "managed-data document content exceeds its host or collection limit".into(),
            );
        }
        if !is_sha256(content_sha256) {
            return Err("managed-data document content_sha256 must be sha256- followed by 64 lowercase hex characters".into());
        }
    }
    if let ManagedDataV2DocumentOperation::Replace {
        collection,
        id,
        expected_revision,
        ..
    }
    | ManagedDataV2DocumentOperation::UpdateMetadata {
        collection,
        id,
        expected_revision,
        ..
    } = operation
    {
        let current = state
            .documents
            .get(collection)
            .and_then(|documents| documents.get(id))
            .ok_or_else(|| format!("managed-data document '{id}' does not exist"))?;
        if current.revision != *expected_revision {
            return Err(format!(
                "managed-data document changed; expected revision {expected_revision}, found {}",
                current.revision
            ));
        }
    }
    Ok(())
}

fn staged_content(
    stage: &ManagedDataStageDocument,
    length: u64,
    hash: &str,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    for (expected, (index, value)) in stage.chunks.iter().enumerate() {
        if *index as usize != expected {
            return Err("managed-data document chunks are incomplete or out of order".into());
        }
        bytes.extend(
            BASE64
                .decode(value.as_bytes())
                .map_err(|_| "managed-data staged chunk is invalid base64".to_string())?,
        );
    }
    if bytes.len() as u64 != length || format!("sha256-{:x}", Sha256::digest(&bytes)) != hash {
        return Err(
            "managed-data document content length or digest does not match its declaration".into(),
        );
    }
    Ok(bytes)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256-")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn write_blob(
    store: &ManagedDataStore,
    app_id: &AppId,
    hash: &str,
    content: &[u8],
) -> Result<bool, String> {
    if !is_sha256(hash) {
        return Err("managed-data blob hash is invalid".into());
    }
    let actual = format!("sha256-{:x}", Sha256::digest(content));
    if actual != hash {
        return Err("managed-data document content digest mismatch".into());
    }
    std::fs::create_dir_all(store.blob_dir(app_id))
        .map_err(|error| format!("create managed-data blob directory failed: {error}"))?;
    let path = store.blob_path(app_id, hash);
    store.reject_linked_path(&path, "managed data blob")?;
    if path.is_file() {
        let existing = std::fs::read(&path)
            .map_err(|error| format!("read existing managed-data blob failed: {error}"))?;
        if existing != content {
            return Err(
                "managed-data content-addressed blob already exists with different content".into(),
            );
        }
        return Ok(false);
    }
    if let Err(error) = store.writer.write_and_sync(&path, content) {
        let failure = format!("write managed-data blob failed: {error}");
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            return Err(failure);
        }
        return match store.writer.remove_file(&path) {
            Ok(()) => Err(failure),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(failure),
            Err(error) => Err(format!(
                "{failure}; remove partial managed-data blob failed: {error}"
            )),
        };
    }
    Ok(true)
}

fn validate_v2_state(
    state: &ManagedDataV2Envelope,
    contract: HostManagedDataRef<'_>,
    store: &ManagedDataStore,
    app_id: &AppId,
) -> Result<(), String> {
    if state.version != V2_STORE_VERSION {
        return Err(format!(
            "unsupported managed-data store version {}; expected {V2_STORE_VERSION}",
            state.version
        ));
    }
    if state.contract_digest.is_empty() {
        return Err("managed-data v2 store contains an empty contract digest".into());
    }
    let legacy = ManagedDataDocument {
        version: STORE_VERSION,
        generation: state.generation.max(1),
        contract_digest: state.contract_digest.clone(),
        collections: state.collections.clone(),
    };
    validate_document(&legacy, contract)?;
    let mut content_total = 0u64;
    for (collection, documents) in &state.documents {
        let declaration = contract.documents.get(collection).ok_or_else(|| {
            format!("managed-data v2 store contains undeclared document collection '{collection}'")
        })?;
        if documents.len() > declaration.limits.documents as usize {
            return Err(format!(
                "managed-data document collection '{collection}' exceeds its document limit"
            ));
        }
        let collection_content = documents.values().try_fold(0u64, |total, document| {
            total.checked_add(document.content_length).ok_or_else(|| {
                format!("managed-data document collection '{collection}' content quota overflow")
            })
        })?;
        if collection_content > declaration.limits.content_bytes {
            return Err(format!(
                "managed-data document collection '{collection}' exceeds its content quota"
            ));
        }
        content_total = content_total
            .checked_add(collection_content)
            .ok_or_else(|| "managed-data document content quota overflow".to_string())?;
        for (id, document) in documents {
            if id != &document.id {
                return Err("managed-data document key does not match its id".into());
            }
            validate_id(id)?;
            if document.revision == 0 {
                return Err("managed-data document has a zero revision".into());
            }
            validate_document_metadata(&document.metadata, declaration, collection)?;
            if document.content_length > declaration.limits.content_bytes
                || document.content_length > V2_MAX_DOCUMENT_BYTES as u64
                || !is_sha256(&document.content_sha256)
            {
                return Err("managed-data document has invalid content metadata".into());
            }
            let path = store.blob_path(app_id, &document.content_sha256);
            let length = std::fs::metadata(path)
                .map_err(|error| format!("managed-data document blob is missing: {error}"))?
                .len();
            if length != document.content_length {
                return Err("managed-data document blob length does not match metadata".into());
            }
        }
    }
    if state.receipts.len() > V2_MAX_RECEIPTS {
        return Err("managed-data receipt quota exceeded".into());
    }
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("serialize managed-data v2 store failed: {error}"))?;
    let limit = usize::try_from(contract.limits.total_bytes)
        .unwrap_or(usize::MAX)
        .min(MAX_DOCUMENT_BYTES);
    if bytes.len() > limit {
        return Err(format!(
            "managed-data v2 store exceeds its {limit}-byte limit"
        ));
    }
    if content_total > limit as u64
        || content_total.saturating_add(bytes.len() as u64) > limit as u64
        || content_total > 64 * 1024 * 1024
    {
        return Err("managed-data v2 content and receipt state exceeds its total quota".into());
    }
    Ok(())
}

fn validate_v2_envelope(
    state: &ManagedDataV2Envelope,
    store: &ManagedDataStore,
    app_id: &AppId,
) -> Result<(), String> {
    if state.version != V2_STORE_VERSION
        || state.generation == 0
        || state.contract_digest.is_empty()
    {
        return Err("managed-data v2 state envelope is invalid".into());
    }
    if state.receipts.len() > V2_MAX_RECEIPTS {
        return Err("managed-data v2 state contains too many receipts".into());
    }
    let mut content_total = 0u64;
    for documents in state.documents.values() {
        for (id, document) in documents {
            validate_id(id)?;
            if id != &document.id || document.revision == 0 || !is_sha256(&document.content_sha256)
            {
                return Err("managed-data v2 state contains an invalid document record".into());
            }
            if !document.metadata.is_empty() && document.metadata.len() > 65_536 {
                return Err("managed-data v2 document metadata is too large".into());
            }
            let blob = store.blob_path(app_id, &document.content_sha256);
            store.reject_linked_path(&blob, "managed data blob")?;
            let metadata = std::fs::metadata(&blob)
                .map_err(|error| format!("managed-data document blob is missing: {error}"))?;
            if metadata.len() != document.content_length {
                return Err("managed-data document blob length does not match state".into());
            }
            let bytes = std::fs::read(&blob)
                .map_err(|error| format!("read managed-data document blob failed: {error}"))?;
            if format!("sha256-{:x}", Sha256::digest(bytes)) != document.content_sha256 {
                return Err("managed-data document blob failed integrity validation".into());
            }
            content_total = content_total
                .checked_add(document.content_length)
                .ok_or_else(|| "managed-data v2 content quota overflow".to_string())?;
        }
    }
    let state_bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("serialize managed-data v2 state failed: {error}"))?;
    if state_bytes.len() > MAX_DOCUMENT_BYTES
        || content_total.saturating_add(state_bytes.len() as u64) > 64 * 1024 * 1024
    {
        return Err("managed-data v2 state exceeds the host quota".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
