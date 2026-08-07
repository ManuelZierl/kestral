//! Pure persistence port and the complete durable kernel projection.
//!
//! The host owns the storage implementation. The kernel only knows that one
//! candidate state is durably committed before the corresponding in-memory
//! service views become visible.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::ids::GrantId;
use crate::primitives::artifact::Artifact;
use crate::primitives::grant::Grant;
use crate::services::ledger::LedgerRecord;
use crate::services::registry::InstalledApp;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableKernelState {
    pub installed_apps: Vec<InstalledApp>,
    pub grants: Vec<Grant>,
    pub revoked_grant_ids: Vec<GrantId>,
    pub ledger_records: Vec<LedgerRecord>,
    pub artifacts: Vec<Artifact>,
}

impl DurableKernelState {
    pub fn empty() -> Self {
        Self {
            installed_apps: Vec::new(),
            grants: Vec::new(),
            revoked_grant_ids: Vec::new(),
            ledger_records: Vec::new(),
            artifacts: Vec::new(),
        }
    }
}

impl Default for DurableKernelState {
    fn default() -> Self {
        Self::empty()
    }
}

pub trait KernelStateStore: Send + Sync {
    fn load(&self) -> Result<Option<DurableKernelState>, String>;
    fn commit(&self, state: &DurableKernelState) -> CommitOutcome;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOutcome {
    Committed,
    NotCommitted(String),
    /// The durable write may have taken effect, but the adapter cannot prove
    /// it. The live kernel must not perform another transition from stale
    /// memory; recovery from the durable store is required.
    Indeterminate(String),
}

/// Deterministic adapter for tests and non-persistent embedders.
#[derive(Default)]
pub struct MemoryKernelStateStore {
    state: Mutex<Option<DurableKernelState>>,
}

impl MemoryKernelStateStore {
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl KernelStateStore for MemoryKernelStateStore {
    fn load(&self) -> Result<Option<DurableKernelState>, String> {
        Ok(self
            .state
            .lock()
            .expect("state store mutex poisoned")
            .clone())
    }

    fn commit(&self, state: &DurableKernelState) -> CommitOutcome {
        *self.state.lock().expect("state store mutex poisoned") = Some(state.clone());
        CommitOutcome::Committed
    }
}
