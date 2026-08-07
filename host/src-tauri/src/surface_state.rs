use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use app_host_kernel::ids::{AppId, SurfaceName};
use app_host_kernel::JsonObject;

use crate::atomic_json::{
    load_json_document, persist_json_document, standard_writer, AtomicFileWriter, AtomicJsonError,
};

const STORE_VERSION: u32 = 2;
const STORE_FILE: &str = "surface-state-v2.json";
const MAX_KEY_LENGTH: usize = 200;
const MAX_ENTRIES: usize = 2_000;
const MAX_VALUE_BYTES: usize = 1024 * 1024;
const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SurfaceStateEntry {
    pub revision: u64,
    pub value: Option<JsonObject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SurfaceStateDocument {
    version: u32,
    // Keep this before `surfaces`: native backends use the fixed JSON prefix as
    // a cheap freshness check while holding the same file snapshot open.
    generation: u64,
    surfaces: BTreeMap<String, SurfaceStateCollection>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SurfaceStateCollection {
    entries: BTreeMap<String, SurfaceStateEntry>,
}

pub struct SurfaceStateStore {
    data_root: PathBuf,
    writer: Arc<dyn AtomicFileWriter>,
}

impl SurfaceStateStore {
    pub fn new(data_root: PathBuf) -> Self {
        Self {
            data_root,
            writer: standard_writer(),
        }
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
            if !entry
                .file_type()
                .map_err(|error| format!("inspect app data entry failed: {error}"))?
                .is_dir()
            {
                continue;
            }
            let app_id = AppId::new(entry.file_name().to_string_lossy().into_owned());
            let store = Self::new(data_root.to_path_buf());
            if store.path(&app_id).exists() {
                store.load(&app_id)?;
            }
        }
        Ok(())
    }

    pub fn get(
        &self,
        app_id: &AppId,
        surface: &SurfaceName,
        key: &str,
    ) -> Result<SurfaceStateEntry, String> {
        validate_key(key)?;
        let document = self.load(app_id)?;
        Ok(document
            .surfaces
            .get(surface.as_str())
            .and_then(|collection| collection.entries.get(key))
            .cloned()
            .unwrap_or(SurfaceStateEntry {
                revision: 0,
                value: None,
            }))
    }

    pub fn put(
        &self,
        app_id: &AppId,
        surface: &SurfaceName,
        key: &str,
        expected_revision: u64,
        value: Option<JsonObject>,
    ) -> Result<SurfaceStateEntry, String> {
        validate_key(key)?;
        if let Some(value) = &value {
            let bytes = serde_json::to_vec(value)
                .map_err(|error| format!("serialize surface state value failed: {error}"))?;
            if bytes.len() > MAX_VALUE_BYTES {
                return Err(format!(
                    "surface state value exceeds the {MAX_VALUE_BYTES}-byte limit"
                ));
            }
        }

        let mut document = self.load(app_id)?;
        let entry_count = document
            .surfaces
            .values()
            .map(|collection| collection.entries.len())
            .sum::<usize>();
        let collection = document
            .surfaces
            .entry(surface.as_str().to_string())
            .or_default();
        let current_revision = collection
            .entries
            .get(key)
            .map(|entry| entry.revision)
            .unwrap_or(0);
        if current_revision != expected_revision {
            return Err(format!(
                "surface state changed; expected revision {expected_revision}, found {current_revision}"
            ));
        }
        if current_revision == 0 && entry_count >= MAX_ENTRIES {
            return Err(format!(
                "surface state exceeds its {MAX_ENTRIES}-entry limit"
            ));
        }
        let next = SurfaceStateEntry {
            revision: current_revision
                .checked_add(1)
                .ok_or_else(|| "surface state revision overflow".to_string())?,
            value,
        };
        collection.entries.insert(key.to_string(), next.clone());
        document.generation = document
            .generation
            .checked_add(1)
            .ok_or_else(|| "surface state generation overflow".to_string())?;

        let bytes = serde_json::to_vec(&document)
            .map_err(|error| format!("serialize surface state failed: {error}"))?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(format!(
                "surface state document exceeds the {MAX_DOCUMENT_BYTES}-byte limit"
            ));
        }
        persist_json_document(
            &self.path(app_id),
            &document,
            "surface state",
            self.writer.as_ref(),
        )
        .map_err(AtomicJsonError::into_message)?;
        Ok(next)
    }

    fn load(&self, app_id: &AppId) -> Result<SurfaceStateDocument, String> {
        let Some(document) =
            load_json_document::<SurfaceStateDocument>(&self.path(app_id), "surface state")?
        else {
            return Ok(SurfaceStateDocument {
                version: STORE_VERSION,
                generation: 0,
                surfaces: BTreeMap::new(),
            });
        };
        if document.version != STORE_VERSION {
            return Err(format!(
                "unsupported surface state version {}; expected {STORE_VERSION}",
                document.version
            ));
        }
        if document.generation == 0 {
            return Err("surface state contains a zero generation".into());
        }
        let entry_count = document
            .surfaces
            .values()
            .map(|collection| collection.entries.len())
            .sum::<usize>();
        if entry_count > MAX_ENTRIES {
            return Err(format!(
                "surface state exceeds its {MAX_ENTRIES}-entry limit"
            ));
        }
        let document_bytes = serde_json::to_vec(&document)
            .map_err(|error| format!("serialize surface state failed: {error}"))?;
        if document_bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(format!(
                "surface state document exceeds the {MAX_DOCUMENT_BYTES}-byte limit"
            ));
        }
        for entry in document
            .surfaces
            .values()
            .flat_map(|collection| collection.entries.values())
        {
            if entry.revision == 0 {
                return Err("surface state contains a zero revision".into());
            }
            if let Some(value) = &entry.value {
                let bytes = serde_json::to_vec(value)
                    .map_err(|error| format!("serialize surface state value failed: {error}"))?;
                if bytes.len() > MAX_VALUE_BYTES {
                    return Err(format!(
                        "surface state value exceeds the {MAX_VALUE_BYTES}-byte limit"
                    ));
                }
            }
        }
        Ok(document)
    }

    fn path(&self, app_id: &AppId) -> PathBuf {
        self.data_root.join(app_id.as_str()).join(STORE_FILE)
    }
}

fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty()
        || key.len() > MAX_KEY_LENGTH
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
    {
        return Err(
            "surface state key must be 1-200 ASCII letters, digits, '.', '_', ':', or '-'".into(),
        );
    }
    Ok(())
}

pub fn data_root(app_records_root: &Path) -> PathBuf {
    app_records_root.join(".data")
}

#[cfg(test)]
mod tests;
