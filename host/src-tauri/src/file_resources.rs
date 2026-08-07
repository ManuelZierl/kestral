use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use app_host_kernel::ids::{AppId, CapabilityName, ResourceId};
use app_host_kernel::invocation::{CapabilityHandler, CapabilityOutcome, HandlerFailure};
use app_host_kernel::kernel::Kernel;
use app_host_kernel::manifest::{AppManifest, GrantRequest};
use app_host_kernel::primitives::capability::{
    CapabilityDeclaration, CapabilityEffect, CapabilityRef,
};
use app_host_kernel::primitives::grant::{DataScope, GrantCondition, GrantDuration, GrantScope};
use app_host_kernel::JsonObject;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::atomic_json::{
    persist_json_document, standard_writer, AtomicFileWriter, AtomicJsonError,
};

const STORE_VERSION: u32 = 1;
const FILE_BROKER_APP_ID: &str = "com.ma-zierl.host.file-broker";
const MAX_READ_BYTES: usize = 1024 * 1024;
const MAX_WRITE_BYTES: usize = 1024 * 1024;
const MAX_WRITE_BASE64_CHARS: usize = 4 * MAX_WRITE_BYTES.div_ceil(3);
const MAX_DIRECTORY_ENTRIES: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileResourceKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileResourceStatus {
    Active,
    Removing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileResourceRecord {
    pub resource_id: ResourceId,
    pub display_name: String,
    pub kind: FileResourceKind,
    pub canonical_path: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileResourceView {
    pub resource_id: ResourceId,
    pub display_name: String,
    pub kind: FileResourceKind,
    pub created_at: DateTime<Utc>,
    pub status: FileResourceStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedFileResourceView {
    #[serde(flatten)]
    pub resource: FileResourceView,
    pub canonical_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntryView {
    pub path: String,
    pub display_name: String,
    pub kind: FileEntryKind,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileListView {
    pub resource_id: ResourceId,
    pub resource_kind: FileResourceKind,
    pub entries: Vec<FileEntryView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileReadView {
    pub resource_id: ResourceId,
    pub path: String,
    pub bytes_read: usize,
    pub total_bytes: u64,
    pub truncated: bool,
    pub sha256: String,
    pub content_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileWriteView {
    pub resource_id: ResourceId,
    pub path: String,
    pub bytes_written: usize,
    pub replaced: bool,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileDeleteView {
    pub resource_id: ResourceId,
    pub path: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileResourceOperationRequest {
    pub resource_id: ResourceId,
    #[serde(default)]
    pub relative_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileResourceReadRequest {
    pub resource_id: ResourceId,
    #[serde(default)]
    pub relative_path: Option<String>,
    #[serde(default)]
    pub limit_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileResourceWriteRequest {
    pub resource_id: ResourceId,
    #[serde(default)]
    pub relative_path: Option<String>,
    pub content_base64: String,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileResourceDeleteRequest {
    pub resource_id: ResourceId,
    #[serde(default)]
    pub relative_path: Option<String>,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileResourceRegistryDocument {
    version: u32,
    resources: Vec<FileResourceRecord>,
    pending_removals: Vec<ResourceId>,
}

pub struct FileResourceRegistryService {
    path: PathBuf,
    document: FileResourceRegistryDocument,
    writer: Arc<dyn AtomicFileWriter>,
}

impl FileResourceRegistryService {
    pub fn new(path: PathBuf) -> Result<Self, String> {
        Self::with_writer(path, standard_writer())
    }

    pub fn with_writer(path: PathBuf, writer: Arc<dyn AtomicFileWriter>) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("create file resource registry directory failed: {error}")
            })?;
        }
        if !path.exists() {
            let service = Self {
                path: path.clone(),
                document: FileResourceRegistryDocument {
                    version: STORE_VERSION,
                    resources: Vec::new(),
                    pending_removals: Vec::new(),
                },
                writer,
            };
            service.persist().map_err(AtomicJsonError::into_message)?;
            return Ok(service);
        }
        let raw = fs::read_to_string(&path)
            .map_err(|error| format!("read file resource registry failed: {error}"))?;
        let document: FileResourceRegistryDocument =
            serde_json::from_str(&raw).map_err(|error| {
                format!(
                    "parse file resource registry failed; preserved '{}': {error}",
                    path.display()
                )
            })?;
        if document.version != STORE_VERSION {
            return Err(format!(
                "unsupported file resource registry version: {}",
                document.version
            ));
        }
        Ok(Self {
            path,
            document,
            writer,
        })
    }

    fn persist(&self) -> Result<(), AtomicJsonError> {
        persist_json_document(
            &self.path,
            &self.document,
            "file resource registry",
            self.writer.as_ref(),
        )
    }

    pub fn list_resources(&self) -> Vec<FileResourceView> {
        let mut views: Vec<_> = self
            .document
            .resources
            .iter()
            .map(|resource| self.view_for(resource))
            .collect();
        views.sort_by(|left, right| left.display_name.cmp(&right.display_name));
        views
    }

    pub fn list_trusted_resources(&self) -> Vec<TrustedFileResourceView> {
        let mut views: Vec<_> = self
            .document
            .resources
            .iter()
            .map(|resource| TrustedFileResourceView {
                resource: self.view_for(resource),
                canonical_path: resource.canonical_path.clone(),
            })
            .collect();
        views.sort_by(|left, right| left.resource.display_name.cmp(&right.resource.display_name));
        views
    }

    pub fn resource(&self, resource_id: &ResourceId) -> Option<&FileResourceRecord> {
        self.document
            .resources
            .iter()
            .find(|resource| &resource.resource_id == resource_id)
    }

    pub fn active_resource(&self, resource_id: &ResourceId) -> Result<&FileResourceRecord, String> {
        let resource = self
            .resource(resource_id)
            .ok_or_else(|| format!("unknown file resource: {resource_id}"))?;
        if self.document.pending_removals.contains(resource_id) {
            return Err(format!("file resource '{resource_id}' is being removed"));
        }
        Ok(resource)
    }

    pub fn register_resource(&mut self, path: &Path) -> Result<TrustedFileResourceView, String> {
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            format!("inspect selected path '{}' failed: {error}", path.display())
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "selected path '{}' must not be a symlink",
                path.display()
            ));
        }
        let canonical = fs::canonicalize(path).map_err(|error| {
            format!(
                "canonicalize selected path '{}' failed: {error}",
                path.display()
            )
        })?;
        let kind = file_kind(&canonical)?;
        let canonical_path = canonical.to_string_lossy().to_string();
        if self
            .document
            .resources
            .iter()
            .any(|resource| resource.canonical_path == canonical_path)
        {
            return Err(format!(
                "file resource '{}' is already registered",
                canonical.display()
            ));
        }
        let record = FileResourceRecord {
            resource_id: ResourceId::new(format!("resource-{}", Uuid::new_v4())),
            display_name: display_name_for(&canonical, kind),
            kind,
            canonical_path,
            created_at: Utc::now(),
        };
        let previous = self.document.clone();
        self.document.resources.push(record.clone());
        if let Err(error) = self.persist() {
            if !error.is_indeterminate() {
                self.document = previous;
            }
            return Err(error.into_message());
        }
        Ok(TrustedFileResourceView {
            resource: self.view_for(&record),
            canonical_path: record.canonical_path,
        })
    }

    pub fn begin_removal(
        &mut self,
        resource_id: &ResourceId,
    ) -> Result<FileResourceRecord, String> {
        let resource = self
            .resource(resource_id)
            .ok_or_else(|| format!("unknown file resource: {resource_id}"))?
            .clone();
        if self.document.pending_removals.contains(resource_id) {
            return Ok(resource);
        }
        let previous = self.document.clone();
        self.document.pending_removals.push(resource_id.clone());
        if let Err(error) = self.persist() {
            if !error.is_indeterminate() {
                self.document = previous;
            }
            return Err(error.into_message());
        }
        Ok(resource)
    }

    pub fn finalize_removal(&mut self, resource_id: &ResourceId) -> Result<(), String> {
        let previous = self.document.clone();
        self.document
            .resources
            .retain(|resource| &resource.resource_id != resource_id);
        self.document
            .pending_removals
            .retain(|pending| pending != resource_id);
        if let Err(error) = self.persist() {
            if !error.is_indeterminate() {
                self.document = previous;
            }
            return Err(error.into_message());
        }
        Ok(())
    }

    pub fn reconcile_with_kernel(&mut self, kernel: &mut Kernel) -> Result<(), String> {
        let pending = self.document.pending_removals.clone();
        for resource_id in pending {
            kernel
                .revoke_grants_for_resource(&resource_id)
                .map_err(|error| error.to_string())?;
            self.finalize_removal(&resource_id)?;
        }

        let active_resource_ids = self
            .document
            .resources
            .iter()
            .map(|resource| &resource.resource_id)
            .collect::<std::collections::BTreeSet<_>>();
        let holders = kernel
            .installed_apps()
            .map(|app| app.manifest.app_id.clone())
            .collect::<Vec<_>>();
        let orphaned_grants = holders
            .iter()
            .flat_map(|holder| kernel.grant_statuses_for(holder))
            .filter(|view| view.status == app_host_kernel::primitives::grant::GrantStatus::Active)
            .filter(|view| view.grant.scope.provider() == &file_broker_app_id())
            .filter(|view| match &view.grant.data_scope {
                DataScope::Resources { resource_ids } => resource_ids
                    .iter()
                    .any(|resource_id| !active_resource_ids.contains(resource_id)),
                DataScope::None => true,
                DataScope::AllResources => false,
            })
            .map(|view| view.grant.grant_id)
            .collect::<Vec<_>>();
        for grant_id in orphaned_grants {
            kernel
                .revoke_grant(&grant_id)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn validate_grant_data_scope(
        &self,
        scope: &GrantScope,
        data_scope: &DataScope,
    ) -> Result<(), String> {
        if scope.provider() != &file_broker_app_id() {
            return Ok(());
        }
        let DataScope::Resources { resource_ids } = data_scope else {
            return Err("File Broker permissions require a registered resource".into());
        };
        for resource_id in resource_ids {
            self.active_resource(resource_id)?;
        }
        Ok(())
    }

    pub fn list_entries(
        &self,
        resource_id: &ResourceId,
        relative_path: Option<&str>,
    ) -> Result<FileListView, String> {
        let resource = self.active_resource(resource_id)?;
        let root = PathBuf::from(&resource.canonical_path);
        let target = resolve_target(&root, resource.kind, relative_path, false)?;
        let metadata = fs::symlink_metadata(&target).map_err(|error| {
            format!(
                "inspect resource target '{}' failed: {error}",
                target.display()
            )
        })?;
        if metadata.is_file() {
            return Ok(FileListView {
                resource_id: resource.resource_id.clone(),
                resource_kind: resource.kind,
                entries: vec![entry_view(&root, &target)?],
            });
        }
        if !metadata.is_dir() {
            return Err(format!(
                "resource target '{}' must be a file or directory",
                target.display()
            ));
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(&target)
            .map_err(|error| format!("read directory '{}' failed: {error}", target.display()))?
        {
            if entries.len() >= MAX_DIRECTORY_ENTRIES {
                return Err(format!(
                    "directory contains more than {MAX_DIRECTORY_ENTRIES} entries; choose a narrower resource path"
                ));
            }
            let entry = entry.map_err(|error| format!("read directory entry failed: {error}"))?;
            entries.push(entry_view(&root, &entry.path())?);
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(FileListView {
            resource_id: resource.resource_id.clone(),
            resource_kind: resource.kind,
            entries,
        })
    }

    pub fn read_file(
        &self,
        resource_id: &ResourceId,
        relative_path: Option<&str>,
        limit_bytes: Option<u64>,
    ) -> Result<FileReadView, String> {
        let resource = self.active_resource(resource_id)?;
        let root = PathBuf::from(&resource.canonical_path);
        let target = resolve_target(&root, resource.kind, relative_path, false)?;
        let mut file = File::open(&target)
            .map_err(|error| format!("open file '{}' failed: {error}", target.display()))?;
        let metadata = fs::symlink_metadata(&target)
            .map_err(|error| format!("inspect file '{}' failed: {error}", target.display()))?;
        if !metadata.is_file() {
            return Err(format!(
                "resource target '{}' is not a regular file",
                target.display()
            ));
        }
        ensure_same_file(&file, &metadata, &target)?;
        let limit = limit_bytes
            .map(|value| value.min(MAX_READ_BYTES as u64) as usize)
            .unwrap_or(MAX_READ_BYTES);
        let mut buffer = Vec::new();
        std::io::Read::by_ref(&mut file)
            .take((limit + 1) as u64)
            .read_to_end(&mut buffer)
            .map_err(|error| format!("read file '{}' failed: {error}", target.display()))?;
        let truncated = buffer.len() > limit;
        if truncated {
            buffer.truncate(limit);
        }
        Ok(FileReadView {
            resource_id: resource.resource_id.clone(),
            path: relative_path.unwrap_or("").to_string(),
            bytes_read: buffer.len(),
            total_bytes: metadata.len(),
            truncated,
            sha256: format!("{:x}", Sha256::digest(&buffer)),
            content_base64: STANDARD.encode(&buffer),
        })
    }

    pub fn write_file(
        &self,
        resource_id: &ResourceId,
        relative_path: Option<&str>,
        content_base64: &str,
        expected_sha256: Option<&str>,
    ) -> Result<FileWriteView, String> {
        let resource = self.active_resource(resource_id)?;
        let root = PathBuf::from(&resource.canonical_path);
        let target = resolve_target(&root, resource.kind, relative_path, true)?;
        if content_base64.len() > MAX_WRITE_BASE64_CHARS {
            return Err(format!(
                "file content exceeds the {MAX_WRITE_BYTES}-byte write limit"
            ));
        }
        let content = STANDARD
            .decode(content_base64)
            .map_err(|error| format!("decode file content failed: {error}"))?;
        if content.len() > MAX_WRITE_BYTES {
            return Err(format!(
                "file content exceeds the {MAX_WRITE_BYTES}-byte write limit"
            ));
        }
        let replaced = target.exists();
        if replaced {
            let metadata = fs::symlink_metadata(&target)
                .map_err(|error| format!("inspect file '{}' failed: {error}", target.display()))?;
            if !metadata.is_file() {
                return Err(format!(
                    "resource target '{}' is not a regular file",
                    target.display()
                ));
            }
        }
        if let Some(expected_sha256) = expected_sha256 {
            if !replaced {
                return Err("expected SHA-256 conflict guard requires an existing target".into());
            }
            let current = sha256_of_file(&target)?;
            if current != expected_sha256 {
                return Err(format!(
                    "SHA-256 conflict for '{}': expected {}, got {}",
                    target.display(),
                    expected_sha256,
                    current
                ));
            }
        }
        let temp_path = target.with_file_name(format!(
            ".{}.{}.tmp",
            target
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("file"),
            Uuid::new_v4()
        ));
        {
            let mut temp = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp_path)
                .map_err(|error| {
                    format!(
                        "create temporary file '{}' failed: {error}",
                        temp_path.display()
                    )
                })?;
            temp.write_all(&content).map_err(|error| {
                format!(
                    "write temporary file '{}' failed: {error}",
                    temp_path.display()
                )
            })?;
            temp.sync_all().map_err(|error| {
                format!(
                    "sync temporary file '{}' failed: {error}",
                    temp_path.display()
                )
            })?;
        }
        let backup_path = if replaced {
            let backup_path = target.with_file_name(format!(
                ".{}.{}.bak",
                target
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("file"),
                Uuid::new_v4()
            ));
            fs::rename(&target, &backup_path).map_err(|error| {
                let _ = fs::remove_file(&temp_path);
                format!(
                    "prepare replacement for '{}' failed: {error}",
                    target.display()
                )
            })?;
            Some(backup_path)
        } else {
            None
        };
        if let Err(error) = fs::rename(&temp_path, &target) {
            if let Some(backup_path) = &backup_path {
                let _ = fs::rename(backup_path, &target);
                let _ = fs::remove_file(backup_path);
            }
            let _ = fs::remove_file(&temp_path);
            return Err(format!(
                "replace file '{}' failed: {error}",
                target.display()
            ));
        }
        if let Some(backup_path) = &backup_path {
            let _ = fs::remove_file(backup_path);
        }
        Ok(FileWriteView {
            resource_id: resource.resource_id.clone(),
            path: relative_path.unwrap_or("").to_string(),
            bytes_written: content.len(),
            replaced,
            sha256: format!("{:x}", Sha256::digest(&content)),
        })
    }

    pub fn delete_file(
        &self,
        resource_id: &ResourceId,
        relative_path: Option<&str>,
        expected_sha256: Option<&str>,
    ) -> Result<FileDeleteView, String> {
        let resource = self.active_resource(resource_id)?;
        let root = PathBuf::from(&resource.canonical_path);
        let target = resolve_target(&root, resource.kind, relative_path, true)?;
        let metadata = fs::symlink_metadata(&target)
            .map_err(|error| format!("inspect file '{}' failed: {error}", target.display()))?;
        if !metadata.is_file() {
            return Err(format!(
                "resource target '{}' is not a regular file",
                target.display()
            ));
        }
        if let Some(expected_sha256) = expected_sha256 {
            let current = sha256_of_file(&target)?;
            if current != expected_sha256 {
                return Err(format!(
                    "SHA-256 conflict for '{}': expected {}, got {}",
                    target.display(),
                    expected_sha256,
                    current
                ));
            }
        }
        fs::remove_file(&target)
            .map_err(|error| format!("delete file '{}' failed: {error}", target.display()))?;
        Ok(FileDeleteView {
            resource_id: resource.resource_id.clone(),
            path: relative_path.unwrap_or("").to_string(),
            deleted: true,
        })
    }

    fn view_for(&self, resource: &FileResourceRecord) -> FileResourceView {
        FileResourceView {
            resource_id: resource.resource_id.clone(),
            display_name: resource.display_name.clone(),
            kind: resource.kind,
            created_at: resource.created_at,
            status: if self
                .document
                .pending_removals
                .contains(&resource.resource_id)
            {
                FileResourceStatus::Removing
            } else {
                FileResourceStatus::Active
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileResourceGrantOperation {
    List,
    Read,
    CreateOrReplace,
    Delete,
}

impl FileResourceGrantOperation {
    pub fn capability_name(self) -> &'static str {
        match self {
            Self::List => "file.list",
            Self::Read => "file.read",
            Self::CreateOrReplace => "file.create-or-replace",
            Self::Delete => "file.delete",
        }
    }
}

pub fn invocation_data_scope(capability: &CapabilityRef, input: &JsonObject) -> DataScope {
    if capability.provider != file_broker_app_id() {
        return DataScope::None;
    }
    input
        .get("resource_id")
        .and_then(serde_json::Value::as_str)
        .map(|resource_id| DataScope::Resources {
            resource_ids: vec![ResourceId::new(resource_id)],
        })
        .unwrap_or(DataScope::None)
}

pub fn constrain_tool_schema_to_grants(
    capability: &CapabilityRef,
    input_schema: &JsonObject,
    data_scopes: &[DataScope],
) -> JsonObject {
    if capability.provider != file_broker_app_id() {
        return input_schema.clone();
    }
    let mut resource_ids = data_scopes
        .iter()
        .flat_map(|scope| match scope {
            DataScope::Resources { resource_ids } => resource_ids.as_slice(),
            DataScope::None | DataScope::AllResources => &[],
        })
        .map(|resource_id| serde_json::Value::String(resource_id.to_string()))
        .collect::<Vec<_>>();
    resource_ids.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    resource_ids.dedup();

    let mut schema = input_schema.clone();
    if let Some(resource_id_schema) = schema
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|properties| properties.get_mut("resource_id"))
        .and_then(serde_json::Value::as_object_mut)
    {
        resource_id_schema.insert("enum".into(), serde_json::Value::Array(resource_ids));
    }
    schema
}

pub fn file_broker_app_id() -> AppId {
    AppId::new(FILE_BROKER_APP_ID)
}

pub fn file_broker_manifest() -> AppManifest {
    AppManifest {
        app_id: file_broker_app_id(),
        version: "0.1.0".into(),
        display_name: "File Broker".into(),
        description: "Host-owned brokered file access for trusted resources.".into(),
        capabilities: vec![
            capability(
                "file.list",
                "List the contents of a registered file resource",
                CapabilityEffect::ReadOnly,
            ),
            capability(
                "file.read",
                "Read a registered file resource",
                CapabilityEffect::ReadOnly,
            ),
            capability(
                "file.create-or-replace",
                "Create or replace a file inside a registered directory resource",
                CapabilityEffect::LocalWrite,
            ),
            capability(
                "file.delete",
                "Delete a file inside a registered resource",
                CapabilityEffect::Destructive,
            ),
        ],
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

fn capability(name: &str, description: &str, effect: CapabilityEffect) -> CapabilityDeclaration {
    CapabilityDeclaration {
        name: CapabilityName::new(name),
        description: description.into(),
        input_schema: capability_input_schema(name),
        output_schema: Some(capability_output_schema(name)),
        effect,
    }
}

fn capability_input_schema(name: &str) -> JsonObject {
    schema_object(match name {
        "file.list" => json!({
            "type": "object",
            "properties": {
                "resource_id": {"type": "string"},
                "relative_path": {"type": ["string", "null"]}
            },
            "required": ["resource_id"],
            "additionalProperties": false
        }),
        "file.read" => json!({
            "type": "object",
            "properties": {
                "resource_id": {"type": "string"},
                "relative_path": {"type": ["string", "null"]},
                "limit_bytes": {"type": ["integer", "null"], "minimum": 1}
            },
            "required": ["resource_id"],
            "additionalProperties": false
        }),
        "file.create-or-replace" => json!({
            "type": "object",
            "properties": {
                "resource_id": {"type": "string"},
                "relative_path": {"type": ["string", "null"]},
                "content_base64": {"type": "string", "maxLength": MAX_WRITE_BASE64_CHARS},
                "expected_sha256": {"type": ["string", "null"]}
            },
            "required": ["resource_id", "content_base64"],
            "additionalProperties": false
        }),
        "file.delete" => json!({
            "type": "object",
            "properties": {
                "resource_id": {"type": "string"},
                "relative_path": {"type": ["string", "null"]},
                "expected_sha256": {"type": ["string", "null"]}
            },
            "required": ["resource_id"],
            "additionalProperties": false
        }),
        _ => json!({"type": "object", "properties": {}, "additionalProperties": false}),
    })
}

fn capability_output_schema(name: &str) -> JsonObject {
    schema_object(match name {
        "file.list" => json!({
            "type": "object",
            "properties": {
                "resource_id": {"type": "string"},
                "resource_kind": {"type": "string", "enum": ["file", "directory"]},
                "entries": {
                    "type": "array",
                    "maxItems": MAX_DIRECTORY_ENTRIES,
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": {"type": "string"},
                            "display_name": {"type": "string"},
                            "kind": {"type": "string", "enum": ["file", "directory", "symlink", "other"]},
                            "size_bytes": {"type": ["integer", "null"]},
                            "modified_at": {"type": ["string", "null"]}
                        },
                        "required": ["path", "display_name", "kind"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["resource_id", "resource_kind", "entries"],
            "additionalProperties": false
        }),
        "file.read" => json!({
            "type": "object",
            "properties": {
                "resource_id": {"type": "string"},
                "path": {"type": "string"},
                "bytes_read": {"type": "integer"},
                "total_bytes": {"type": "integer"},
                "truncated": {"type": "boolean"},
                "sha256": {"type": "string"},
                "content_base64": {"type": "string"}
            },
            "required": ["resource_id", "path", "bytes_read", "total_bytes", "truncated", "sha256", "content_base64"],
            "additionalProperties": false
        }),
        "file.create-or-replace" => json!({
            "type": "object",
            "properties": {
                "resource_id": {"type": "string"},
                "path": {"type": "string"},
                "bytes_written": {"type": "integer"},
                "replaced": {"type": "boolean"},
                "sha256": {"type": "string"}
            },
            "required": ["resource_id", "path", "bytes_written", "replaced", "sha256"],
            "additionalProperties": false
        }),
        "file.delete" => json!({
            "type": "object",
            "properties": {
                "resource_id": {"type": "string"},
                "path": {"type": "string"},
                "deleted": {"type": "boolean"}
            },
            "required": ["resource_id", "path", "deleted"],
            "additionalProperties": false
        }),
        _ => json!({"type": "object", "properties": {}, "additionalProperties": false}),
    })
}

pub fn file_broker_handlers(
    registry: Arc<Mutex<FileResourceRegistryService>>,
) -> BTreeMap<CapabilityName, CapabilityHandler> {
    let mut handlers = BTreeMap::new();
    handlers.insert(
        CapabilityName::new("file.list"),
        boxed_handler(registry.clone(), FileOperation::List),
    );
    handlers.insert(
        CapabilityName::new("file.read"),
        boxed_handler(registry.clone(), FileOperation::Read),
    );
    handlers.insert(
        CapabilityName::new("file.create-or-replace"),
        boxed_handler(registry.clone(), FileOperation::Write),
    );
    handlers.insert(
        CapabilityName::new("file.delete"),
        boxed_handler(registry, FileOperation::Delete),
    );
    handlers
}

enum FileOperation {
    List,
    Read,
    Write,
    Delete,
}

fn boxed_handler(
    registry: Arc<Mutex<FileResourceRegistryService>>,
    operation: FileOperation,
) -> CapabilityHandler {
    Box::new(move |input, context| match operation {
        FileOperation::List => handle_list(&registry, input, context),
        FileOperation::Read => handle_read(&registry, input, context),
        FileOperation::Write => handle_write(&registry, input, context),
        FileOperation::Delete => handle_delete(&registry, input, context),
    })
}

fn handle_list(
    registry: &Arc<Mutex<FileResourceRegistryService>>,
    input: &JsonObject,
    context: &app_host_kernel::invocation::InvocationContext,
) -> Result<CapabilityOutcome, HandlerFailure> {
    let request = parse_request::<FileResourceOperationRequest>(input)?;
    ensure_authorized_resource(context, &request.resource_id)?;
    let view = registry
        .lock()
        .map_err(|_| HandlerFailure("file resource registry lock poisoned".into()))?
        .list_entries(&request.resource_id, request.relative_path.as_deref())
        .map_err(HandlerFailure)?;
    result(view)
}

fn handle_read(
    registry: &Arc<Mutex<FileResourceRegistryService>>,
    input: &JsonObject,
    context: &app_host_kernel::invocation::InvocationContext,
) -> Result<CapabilityOutcome, HandlerFailure> {
    let request = parse_request::<FileResourceReadRequest>(input)?;
    ensure_authorized_resource(context, &request.resource_id)?;
    let view = registry
        .lock()
        .map_err(|_| HandlerFailure("file resource registry lock poisoned".into()))?
        .read_file(
            &request.resource_id,
            request.relative_path.as_deref(),
            request.limit_bytes,
        )
        .map_err(HandlerFailure)?;
    result(view)
}

fn handle_write(
    registry: &Arc<Mutex<FileResourceRegistryService>>,
    input: &JsonObject,
    context: &app_host_kernel::invocation::InvocationContext,
) -> Result<CapabilityOutcome, HandlerFailure> {
    let request = parse_request::<FileResourceWriteRequest>(input)?;
    ensure_authorized_resource(context, &request.resource_id)?;
    let view = registry
        .lock()
        .map_err(|_| HandlerFailure("file resource registry lock poisoned".into()))?
        .write_file(
            &request.resource_id,
            request.relative_path.as_deref(),
            &request.content_base64,
            request.expected_sha256.as_deref(),
        )
        .map_err(HandlerFailure)?;
    result(view)
}

fn handle_delete(
    registry: &Arc<Mutex<FileResourceRegistryService>>,
    input: &JsonObject,
    context: &app_host_kernel::invocation::InvocationContext,
) -> Result<CapabilityOutcome, HandlerFailure> {
    let request = parse_request::<FileResourceDeleteRequest>(input)?;
    ensure_authorized_resource(context, &request.resource_id)?;
    let view = registry
        .lock()
        .map_err(|_| HandlerFailure("file resource registry lock poisoned".into()))?
        .delete_file(
            &request.resource_id,
            request.relative_path.as_deref(),
            request.expected_sha256.as_deref(),
        )
        .map_err(HandlerFailure)?;
    result(view)
}

fn parse_request<T: for<'de> Deserialize<'de>>(input: &JsonObject) -> Result<T, HandlerFailure> {
    serde_json::from_value(serde_json::Value::Object(input.clone()))
        .map_err(|error| HandlerFailure(error.to_string()))
}

fn result<T: Serialize>(value: T) -> Result<CapabilityOutcome, HandlerFailure> {
    Ok(CapabilityOutcome {
        result: serde_json::to_value(value).map_err(|error| HandlerFailure(error.to_string()))?,
        artifacts: Vec::new(),
    })
}

fn ensure_authorized_resource(
    context: &app_host_kernel::invocation::InvocationContext,
    resource_id: &ResourceId,
) -> Result<(), HandlerFailure> {
    match &context.authorized_data_scope {
        DataScope::Resources { resource_ids } if resource_ids.contains(resource_id) => Ok(()),
        _ => Err(HandlerFailure(format!(
            "resource scope mismatch: authorization did not include '{resource_id}'"
        ))),
    }
}

pub fn file_resource_grant_request(
    holder: AppId,
    resource_id: ResourceId,
    operation: FileResourceGrantOperation,
) -> GrantRequest {
    GrantRequest {
        scope: GrantScope::ExactCapability {
            provider: file_broker_app_id(),
            capability: CapabilityName::new(operation.capability_name()),
        },
        data_scope: DataScope::resources(vec![resource_id]).expect("single resource id is valid"),
        condition: GrantCondition::RequiresApproval,
        reason: format!("allow {holder} to access a registered file resource"),
        duration: GrantDuration::NonExpiring,
    }
}

pub fn file_resource_registry_path(config_dir: &Path) -> PathBuf {
    config_dir.join("file-resources-v1.json")
}

fn schema_object(value: serde_json::Value) -> JsonObject {
    value
        .as_object()
        .cloned()
        .expect("schema helper must build object")
}

fn display_name_for(path: &Path, kind: FileResourceKind) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(|name| name.to_string())
        .unwrap_or_else(|| match kind {
            FileResourceKind::File => "selected file".to_string(),
            FileResourceKind::Directory => "selected folder".to_string(),
        })
}

fn file_kind(path: &Path) -> Result<FileResourceKind, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("inspect selected path '{}' failed: {error}", path.display()))?;
    if metadata.is_file() {
        Ok(FileResourceKind::File)
    } else if metadata.is_dir() {
        Ok(FileResourceKind::Directory)
    } else {
        Err(format!(
            "selected path '{}' must be a regular file or directory",
            path.display()
        ))
    }
}

fn sanitize_relative_path(relative_path: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative_path);
    if path.as_os_str().is_empty() {
        return Ok(PathBuf::new());
    }
    let mut sanitized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            Component::CurDir => continue,
            _ => return Err(format!("invalid relative path: {relative_path}")),
        }
    }
    Ok(sanitized)
}

fn resolve_target(
    root: &Path,
    kind: FileResourceKind,
    relative_path: Option<&str>,
    write: bool,
) -> Result<PathBuf, String> {
    let relative = sanitize_relative_path(relative_path.unwrap_or(""))?;
    if kind == FileResourceKind::File {
        if !relative.as_os_str().is_empty() {
            return Err("file resources do not allow descendant paths".into());
        }
        return Ok(root.to_path_buf());
    }
    if relative.as_os_str().is_empty() {
        if write {
            return Err("directory resources require a descendant file path for writes".into());
        }
        return Ok(root.to_path_buf());
    }
    let target = root.join(relative);
    if write {
        resolve_parent_within_root(root, &target)
    } else {
        resolve_existing_within_root(root, &target)
    }
}

fn resolve_existing_within_root(root: &Path, target: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(target)
        .map_err(|error| format!("canonicalize path '{}' failed: {error}", target.display()))?;
    ensure_within_root(root, &canonical)?;
    Ok(canonical)
}

fn resolve_parent_within_root(root: &Path, target: &Path) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("path '{}' has no parent directory", target.display()))?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| format!("canonicalize path '{}' failed: {error}", parent.display()))?;
    ensure_within_root(root, &canonical_parent)?;
    Ok(canonical_parent.join(
        target
            .file_name()
            .ok_or_else(|| format!("path '{}' has no final component", target.display()))?,
    ))
}

/// Confirm an opened handle still names the file inspected after it was opened.
///
/// `resolve_target` proves a *path* resolves inside the resource root, but a
/// path is only a name: between that check and the open, a component can be
/// replaced with a symlink pointing somewhere else. This identity check catches
/// final-target replacement around the open. Fully preventing concurrent
/// ancestor replacement requires handle-relative traversal on each platform.
///
fn ensure_same_file(file: &File, checked: &fs::Metadata, path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let opened = file
            .metadata()
            .map_err(|error| format!("inspect file '{}' failed: {error}", path.display()))?;
        if opened.dev() != checked.dev() || opened.ino() != checked.ino() {
            return Err(format!(
                "resource target '{}' changed while it was being opened",
                path.display()
            ));
        }
    }
    #[cfg(windows)]
    {
        let _ = checked;
        let checked_file = File::open(path)
            .map_err(|error| format!("reopen file '{}' failed: {error}", path.display()))?;
        if windows_file_identity(file, path)? != windows_file_identity(&checked_file, path)? {
            return Err(format!(
                "resource target '{}' changed while it was being opened",
                path.display()
            ));
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, checked, path);
    }
    Ok(())
}

#[cfg(windows)]
fn windows_file_identity(file: &File, path: &Path) -> Result<(u32, u64), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut information) };
    if succeeded == 0 {
        return Err(format!(
            "inspect file identity '{}' failed: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    let file_index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok((information.dwVolumeSerialNumber, file_index))
}

fn ensure_within_root(root: &Path, target: &Path) -> Result<(), String> {
    if target.starts_with(root) {
        Ok(())
    } else {
        Err(format!(
            "path '{}' escapes resource root '{}'",
            target.display(),
            root.display()
        ))
    }
}

fn sha256_of_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("open file '{}' failed: {error}", path.display()))?;
    let mut buffer = [0u8; 8192];
    let mut hasher = Sha256::new();
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read file '{}' failed: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn entry_view(root: &Path, path: &Path) -> Result<FileEntryView, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect entry '{}' failed: {error}", path.display()))?;
    let kind = if metadata.file_type().is_symlink() {
        FileEntryKind::Symlink
    } else if metadata.is_dir() {
        FileEntryKind::Directory
    } else if metadata.is_file() {
        FileEntryKind::File
    } else {
        FileEntryKind::Other
    };
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    Ok(FileEntryView {
        path: rel,
        display_name: path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        kind,
        size_bytes: metadata.is_file().then_some(metadata.len()),
        modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
    })
}

#[cfg(test)]
mod tests;
