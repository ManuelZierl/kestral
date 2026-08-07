//! Artifact: a durable object produced or consumed by work.
//!
//! Artifacts are the medium of composition between apps. Provenance is
//! written by the kernel, never self-reported: apps hand the kernel an
//! `ArtifactDraft` and the kernel stamps run, capability, grant, and
//! producer onto the stored `Artifact`.
//!
//! Memory is not a separate primitive — it is a query over the artifact
//! store and the run ledger.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{AppId, ArtifactId, ArtifactTypeName, GrantId, RunId};
use crate::primitives::capability::CapabilityRef;

/// Kernel-written origin of an artifact: run, capability, grant, producer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub run_id: RunId,
    pub capability: CapabilityRef,
    pub grant_id: GrantId,
    pub produced_by: AppId,
    pub recorded_at: DateTime<Utc>,
}

/// What an app may propose: content only, never provenance.
///
/// `artifact_type` must name an artifact type the producing app declared;
/// content is validated against that type's schema before storage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactDraft {
    pub artifact_type: ArtifactTypeName,
    pub title: String,
    pub content: Value,
}

/// A stored, provenance-carrying durable object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub artifact_id: ArtifactId,
    pub artifact_type: ArtifactTypeName,
    pub title: String,
    pub content: Value,
    pub provenance: Provenance,
}
