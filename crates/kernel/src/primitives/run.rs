//! Run: a concrete execution attempt — the unifying primitive.
//!
//! Everything that does work is a run, regardless of initiator. The run
//! ledger is the authoritative record of what happened; `RunView` is a read
//! model aggregated from ledger events, not a second source of truth.
//!
//! Derived, non-kernel concepts (automation, task, agent, skill) do not
//! appear here: the kernel only ever sees runs being started.

use serde::{Deserialize, Serialize};

use crate::ids::{AppId, ArtifactId, GrantId, RunId, SurfaceName};
use crate::primitives::capability::CapabilityRef;
use crate::primitives::grant::{DataScope, DenialReason};

/// Who caused a run — a closed set of initiator shapes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Initiator {
    /// Run started by a user acting in an app's surface.
    SurfaceAction { app_id: AppId, surface: SurfaceName },
    /// Run started programmatically by an installed app. Chat messages
    /// arriving and automations firing both look like this to the kernel;
    /// `reason` carries the app's own wording.
    App { app_id: AppId, reason: String },
    /// Run started by another run (on behalf of the same app).
    Run { app_id: AppId, parent_run_id: RunId },
}

impl Initiator {
    pub fn app_id(&self) -> &AppId {
        match self {
            Initiator::SurfaceAction { app_id, .. } => app_id,
            Initiator::App { app_id, .. } => app_id,
            Initiator::Run { app_id, .. } => app_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunTerminalState {
    Completed,
    Failed,
    Cancelled,
    /// The host recovered a run that had no durable terminal event.
    Interrupted,
}

/// One interactive grant decision made during a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRecord {
    pub capability: CapabilityRef,
    pub grant_id: GrantId,
    pub approved: bool,
    pub data_scope: DataScope,
}

/// One capability invocation as it appears in a run view — a closed set of
/// outcomes. A refused invocation exercised no grant, so unlike the other
/// cases it carries the denial, not a grant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum InvocationRecord {
    /// The handler returned a result.
    Completed {
        capability: CapabilityRef,
        grant_id: GrantId,
        data_scope: DataScope,
    },
    /// The handler failed or returned invalid artifacts.
    Failed {
        capability: CapabilityRef,
        grant_id: GrantId,
        data_scope: DataScope,
    },
    /// The broker refused before any code ran.
    Refused {
        capability: CapabilityRef,
        reason: DenialReason,
        data_scope: DataScope,
    },
    /// The invocation was authorized but cancelled (explicit cancel or
    /// deadline) before its result was committed. Distinct from `Refused`:
    /// the grant was valid, so no `DenialReason` applies.
    Cancelled {
        capability: CapabilityRef,
        data_scope: DataScope,
    },
}

/// Read model of a run, aggregated from ledger events.
///
/// `terminal_state` of None means the run is still active.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunView {
    pub run_id: RunId,
    pub initiator: Initiator,
    pub goal: String,
    pub grants_exercised: Vec<GrantId>,
    pub invocations: Vec<InvocationRecord>,
    pub approvals: Vec<ApprovalRecord>,
    pub artifacts_produced: Vec<ArtifactId>,
    pub terminal_state: Option<RunTerminalState>,
}

impl RunView {
    pub fn initiating_app(&self) -> &AppId {
        self.initiator.app_id()
    }

    pub fn is_active(&self) -> bool {
        self.terminal_state.is_none()
    }
}
