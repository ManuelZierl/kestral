//! Artifact store — the durable-object substrate of the ledger.
//!
//! Not a sixth kernel service: the Run Ledger records production and the
//! store holds the objects. Artifacts enter only through the kernel's action
//! path, already stamped with kernel-written provenance; apps have no write
//! access.
//!
//! Memory-as-retrieval is a userland query over this store plus
//! the ledger.

use std::collections::BTreeMap;

use crate::errors::{KernelError, KernelResult};
use crate::ids::ArtifactId;
use crate::primitives::artifact::Artifact;
use crate::primitives::grant::DataScope;
use serde::{Deserialize, Serialize};

pub const MAX_ARTIFACT_SNAPSHOT_ITEMS: usize = 50;
pub const MAX_ARTIFACT_CONTENT_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub struct ArtifactSnapshotResolver {
    artifacts: BTreeMap<ArtifactId, Artifact>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSnapshotSummary {
    pub artifact_id: ArtifactId,
    pub artifact_type: crate::ids::ArtifactTypeName,
    pub title: String,
    pub provenance: crate::primitives::artifact::Provenance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSnapshotPage {
    pub items: Vec<ArtifactSnapshotSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSnapshot {
    pub artifact_id: ArtifactId,
    pub artifact_type: crate::ids::ArtifactTypeName,
    pub title: String,
    pub content: serde_json::Value,
    pub provenance: crate::primitives::artifact::Provenance,
}

#[derive(Clone)]
pub struct ArtifactStore {
    artifacts: BTreeMap<ArtifactId, Artifact>,
}

impl ArtifactStore {
    pub fn new() -> Self {
        Self {
            artifacts: BTreeMap::new(),
        }
    }

    /// Commit a prevalidated artifact group together. This in-memory store is
    /// infallible; a future persistent adapter must preserve this batch seam.
    pub fn put_all(&mut self, artifacts: impl IntoIterator<Item = Artifact>) {
        for artifact in artifacts {
            self.artifacts
                .insert(artifact.artifact_id.clone(), artifact);
        }
    }

    pub fn get(&self, artifact_id: &ArtifactId) -> KernelResult<&Artifact> {
        self.artifacts
            .get(artifact_id)
            .ok_or_else(|| KernelError::UnknownArtifact(artifact_id.clone()))
    }

    pub fn all(&self) -> impl Iterator<Item = &Artifact> {
        self.artifacts.values()
    }

    pub fn snapshot_resolver_for(&self, data_scope: &DataScope) -> ArtifactSnapshotResolver {
        let artifacts = match data_scope {
            DataScope::None | DataScope::AllResources => BTreeMap::new(),
            DataScope::Resources { resource_ids } => resource_ids
                .iter()
                .filter_map(|id| {
                    let artifact_id = ArtifactId::new(id.as_str().to_string());
                    self.artifacts
                        .get(&artifact_id)
                        .map(|artifact| (artifact_id, artifact.clone()))
                })
                .collect(),
        };
        ArtifactSnapshotResolver { artifacts }
    }

    pub fn restore(artifacts: Vec<Artifact>) -> KernelResult<Self> {
        let mut store = Self::new();
        for artifact in artifacts {
            if store.artifacts.contains_key(&artifact.artifact_id) {
                return Err(KernelError::Durability(format!(
                    "duplicate artifact id '{}'",
                    artifact.artifact_id
                )));
            }
            store
                .artifacts
                .insert(artifact.artifact_id.clone(), artifact);
        }
        Ok(store)
    }
}

impl Default for ArtifactStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtifactSnapshotResolver {
    pub fn query(&self, cursor: Option<&str>, limit: usize) -> KernelResult<ArtifactSnapshotPage> {
        if limit == 0 || limit > MAX_ARTIFACT_SNAPSHOT_ITEMS {
            return Err(KernelError::Durability("invalid artifact page size".into()));
        }
        let mut items: Vec<_> = self
            .artifacts
            .values()
            .map(|artifact| ArtifactSnapshotSummary {
                artifact_id: artifact.artifact_id.clone(),
                artifact_type: artifact.artifact_type.clone(),
                title: artifact.title.clone(),
                provenance: artifact.provenance.clone(),
            })
            .collect();
        items.sort_by(|a, b| a.artifact_id.cmp(&b.artifact_id));
        let start = match cursor {
            None => 0,
            Some(value) => {
                let parsed = value
                    .parse::<usize>()
                    .map_err(|_| KernelError::Durability("invalid artifact page cursor".into()))?;
                if parsed > items.len() {
                    return Err(KernelError::Durability(
                        "invalid artifact page cursor".into(),
                    ));
                }
                parsed
            }
        };
        let end = (start + limit).min(items.len());
        Ok(ArtifactSnapshotPage {
            items: items[start..end].to_vec(),
            next_cursor: (end < items.len()).then(|| end.to_string()),
        })
    }

    pub fn read(&self, artifact_id: &ArtifactId) -> KernelResult<ArtifactSnapshot> {
        let artifact = self
            .artifacts
            .get(artifact_id)
            .ok_or_else(|| KernelError::UnknownArtifact(artifact_id.clone()))?;
        let content_bytes = serde_json::to_vec(&artifact.content)
            .map_err(|error| KernelError::Durability(error.to_string()))?;
        if content_bytes.len() > MAX_ARTIFACT_CONTENT_BYTES {
            return Err(KernelError::Durability(
                "artifact content exceeds snapshot bound".into(),
            ));
        }
        Ok(ArtifactSnapshot {
            artifact_id: artifact.artifact_id.clone(),
            artifact_type: artifact.artifact_type.clone(),
            title: artifact.title.clone(),
            content: artifact.content.clone(),
            provenance: artifact.provenance.clone(),
        })
    }
}
