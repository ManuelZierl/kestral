//! Run Ledger.
//!
//! Append-only event log recording every run: initiator, goal, grants
//! exercised, capability invocations, artifacts produced, approvals given or
//! denied, and terminal state. The ledger is the system's memory of what
//! happened and the substrate for audit, replay, debugging, and provenance.
//!
//! Events are only ever written by kernel code on the action path — apps
//! cannot append, edit, or omit records.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::clock::Clock;
use crate::errors::{KernelError, KernelResult};
use crate::ids::{AppId, ArtifactId, ArtifactTypeName, EventTopic, GrantId, RunId};
use crate::primitives::capability::CapabilityRef;
use crate::primitives::grant::{DataScope, DenialReason};
use crate::primitives::run::{
    ApprovalRecord, Initiator, InvocationRecord, RunTerminalState, RunView,
};

/// Everything the ledger can record — a closed set of event shapes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum LedgerEvent {
    RunStarted {
        run_id: RunId,
        initiator: Initiator,
        goal: String,
    },
    CapabilityInvoked {
        run_id: RunId,
        capability: CapabilityRef,
        grant_id: GrantId,
        input_sha256: String,
        data_scope: DataScope,
    },
    CapabilityCompleted {
        run_id: RunId,
        capability: CapabilityRef,
        grant_id: GrantId,
        result_sha256: String,
        data_scope: DataScope,
    },
    CapabilityFailed {
        run_id: RunId,
        capability: CapabilityRef,
        grant_id: GrantId,
        error: String,
        data_scope: DataScope,
    },
    /// The broker refused the invocation before any code ran.
    InvocationRefused {
        run_id: RunId,
        capability: CapabilityRef,
        reason: DenialReason,
        data_scope: DataScope,
    },
    /// An authorized invocation was cancelled (explicit cancel or deadline)
    /// before its result could be committed. Recorded honestly as a
    /// cancellation, never as a grant denial — the grant remained valid.
    InvocationCancelled {
        run_id: RunId,
        capability: CapabilityRef,
        data_scope: DataScope,
    },
    ApprovalRequested {
        run_id: RunId,
        capability: CapabilityRef,
        grant_id: GrantId,
        data_scope: DataScope,
    },
    ApprovalGranted {
        run_id: RunId,
        capability: CapabilityRef,
        grant_id: GrantId,
        data_scope: DataScope,
    },
    ApprovalDenied {
        run_id: RunId,
        capability: CapabilityRef,
        grant_id: GrantId,
        data_scope: DataScope,
    },
    ArtifactProduced {
        run_id: RunId,
        artifact_id: ArtifactId,
        artifact_type: ArtifactTypeName,
    },
    RunEnded {
        run_id: RunId,
        terminal_state: RunTerminalState,
    },
}

/// Named kind constants — the single source of truth for the serialized
/// `kind` tag of each [`LedgerEvent`] variant. Both [`LedgerEvent::kind`]
/// and [`LedgerEvent::ALL_KINDS`] reference these so a new variant's kind
/// string can never drift between the two.
pub mod kinds {
    pub const RUN_STARTED: &str = "run-started";
    pub const CAPABILITY_INVOKED: &str = "capability-invoked";
    pub const CAPABILITY_COMPLETED: &str = "capability-completed";
    pub const CAPABILITY_FAILED: &str = "capability-failed";
    pub const INVOCATION_REFUSED: &str = "invocation-refused";
    pub const INVOCATION_CANCELLED: &str = "invocation-cancelled";
    pub const APPROVAL_REQUESTED: &str = "approval-requested";
    pub const APPROVAL_GRANTED: &str = "approval-granted";
    pub const APPROVAL_DENIED: &str = "approval-denied";
    pub const ARTIFACT_PRODUCED: &str = "artifact-produced";
    pub const RUN_ENDED: &str = "run-ended";
}

impl LedgerEvent {
    pub fn run_id(&self) -> &RunId {
        match self {
            LedgerEvent::RunStarted { run_id, .. }
            | LedgerEvent::CapabilityInvoked { run_id, .. }
            | LedgerEvent::CapabilityCompleted { run_id, .. }
            | LedgerEvent::CapabilityFailed { run_id, .. }
            | LedgerEvent::InvocationRefused { run_id, .. }
            | LedgerEvent::InvocationCancelled { run_id, .. }
            | LedgerEvent::ApprovalRequested { run_id, .. }
            | LedgerEvent::ApprovalGranted { run_id, .. }
            | LedgerEvent::ApprovalDenied { run_id, .. }
            | LedgerEvent::ArtifactProduced { run_id, .. }
            | LedgerEvent::RunEnded { run_id, .. } => run_id,
        }
    }

    /// Routing topic — identical to the serialized `kind` tag.
    pub fn kind(&self) -> &'static str {
        match self {
            LedgerEvent::RunStarted { .. } => kinds::RUN_STARTED,
            LedgerEvent::CapabilityInvoked { .. } => kinds::CAPABILITY_INVOKED,
            LedgerEvent::CapabilityCompleted { .. } => kinds::CAPABILITY_COMPLETED,
            LedgerEvent::CapabilityFailed { .. } => kinds::CAPABILITY_FAILED,
            LedgerEvent::InvocationRefused { .. } => kinds::INVOCATION_REFUSED,
            LedgerEvent::InvocationCancelled { .. } => kinds::INVOCATION_CANCELLED,
            LedgerEvent::ApprovalRequested { .. } => kinds::APPROVAL_REQUESTED,
            LedgerEvent::ApprovalGranted { .. } => kinds::APPROVAL_GRANTED,
            LedgerEvent::ApprovalDenied { .. } => kinds::APPROVAL_DENIED,
            LedgerEvent::ArtifactProduced { .. } => kinds::ARTIFACT_PRODUCED,
            LedgerEvent::RunEnded { .. } => kinds::RUN_ENDED,
        }
    }

    /// The closed set of routing topics — one per variant, in declaration
    /// order, matching [`LedgerEvent::kind`]. A manifest cannot subscribe to
    /// a topic outside this set.
    pub const ALL_KINDS: [&'static str; 11] = [
        kinds::RUN_STARTED,
        kinds::CAPABILITY_INVOKED,
        kinds::CAPABILITY_COMPLETED,
        kinds::CAPABILITY_FAILED,
        kinds::INVOCATION_REFUSED,
        kinds::INVOCATION_CANCELLED,
        kinds::APPROVAL_REQUESTED,
        kinds::APPROVAL_GRANTED,
        kinds::APPROVAL_DENIED,
        kinds::ARTIFACT_PRODUCED,
        kinds::RUN_ENDED,
    ];

    pub fn topic(&self) -> EventTopic {
        EventTopic::new(self.kind())
    }
}

/// An event as recorded: sequence and timestamp are ledger-assigned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerRecord {
    pub sequence: u64,
    pub recorded_at: DateTime<Utc>,
    pub event: LedgerEvent,
}

#[derive(Clone)]
pub struct RunLedger {
    clock: Arc<dyn Clock>,
    records: Vec<LedgerRecord>,
    runs: BTreeMap<RunId, RunIndexEntry>,
}

#[derive(Clone)]
struct RunIndexEntry {
    initiating_app: AppId,
    active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ApprovalKey {
    capability: CapabilityRef,
    grant_id: GrantId,
    data_scope: DataScope,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct InvocationKey {
    capability: CapabilityRef,
    grant_id: GrantId,
    data_scope: DataScope,
}

#[derive(Default)]
struct OutstandingActions {
    requested_approvals: Vec<ApprovalKey>,
    granted_approvals: Vec<ApprovalKey>,
    invocations: Vec<InvocationKey>,
}

impl RunLedger {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            records: Vec::new(),
            runs: BTreeMap::new(),
        }
    }

    pub fn append(&mut self, event: LedgerEvent) -> KernelResult<LedgerRecord> {
        Self::validate_append(&self.runs, &event)?;
        let record = LedgerRecord {
            sequence: self.records.len() as u64,
            recorded_at: self.clock.now(),
            event,
        };
        Self::index_event(&mut self.runs, &record.event);
        self.records.push(record.clone());
        Ok(record)
    }

    /// Append a related group only after every record has passed the ledger
    /// state guards. Completion uses this for all artifact and terminal
    /// records, so an invalid later record cannot leave earlier ones behind.
    pub fn append_batch(&mut self, events: Vec<LedgerEvent>) -> KernelResult<Vec<LedgerRecord>> {
        let mut staged_runs = BTreeMap::new();
        let mut records = Vec::with_capacity(events.len());
        for event in events {
            Self::validate_batch_append(&self.runs, &staged_runs, &event)?;
            if !staged_runs.contains_key(event.run_id()) {
                if let Some(state) = self.runs.get(event.run_id()) {
                    staged_runs.insert(event.run_id().clone(), state.clone());
                }
            }
            let record = LedgerRecord {
                sequence: (self.records.len() + records.len()) as u64,
                recorded_at: self.clock.now(),
                event,
            };
            Self::index_event(&mut staged_runs, &record.event);
            records.push(record);
        }
        for (run_id, state) in staged_runs {
            self.runs.insert(run_id, state);
        }
        self.records.extend(records.iter().cloned());
        Ok(records)
    }

    pub fn records(&self) -> &[LedgerRecord] {
        &self.records
    }

    pub fn restore(clock: Arc<dyn Clock>, records: Vec<LedgerRecord>) -> KernelResult<Self> {
        let mut validator = Self::new(clock.clone());
        let mut compacted = Vec::with_capacity(records.len());
        for (expected, record) in records.into_iter().enumerate() {
            if record.sequence != expected as u64 {
                return Err(KernelError::Durability(format!(
                    "ledger sequence {} is not contiguous at {}",
                    record.sequence, expected
                )));
            }
            if let LedgerEvent::RunStarted {
                initiator:
                    Initiator::Run {
                        app_id,
                        parent_run_id,
                    },
                ..
            } = &record.event
            {
                let parent = validator.runs.get(parent_run_id).ok_or_else(|| {
                    KernelError::Durability(format!(
                        "child run references unknown parent '{parent_run_id}'"
                    ))
                })?;
                if !parent.active {
                    return Err(KernelError::Durability(format!(
                        "child run references ended parent '{parent_run_id}'"
                    )));
                }
                if &parent.initiating_app != app_id {
                    return Err(KernelError::Durability(format!(
                        "child run attribution '{app_id}' disagrees with parent '{parent_run_id}'"
                    )));
                }
            }
            validator.append(record.event.clone())?;
            compacted.push(record);
        }
        Self::replay_action_state(&compacted)?;
        validator.records = compacted;
        Ok(validator)
    }

    /// Pending work is session state, but its durable start/approval records
    /// survive a crash. Recovery closes each one before interrupting its run so
    /// every recorded attempt still has an honest terminal event.
    pub(crate) fn incomplete_invocation_cancellations(&self) -> KernelResult<Vec<LedgerEvent>> {
        let outstanding = Self::replay_action_state(&self.records)?;
        let mut events = Vec::new();
        for (run_id, actions) in outstanding {
            events.extend(
                actions
                    .requested_approvals
                    .into_iter()
                    .chain(actions.granted_approvals)
                    .map(|approval| LedgerEvent::InvocationCancelled {
                        run_id: run_id.clone(),
                        capability: approval.capability,
                        data_scope: approval.data_scope,
                    }),
            );
            events.extend(actions.invocations.into_iter().map(|invocation| {
                LedgerEvent::InvocationCancelled {
                    run_id: run_id.clone(),
                    capability: invocation.capability,
                    data_scope: invocation.data_scope,
                }
            }));
        }
        Ok(events)
    }

    pub fn records_for_run<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> impl Iterator<Item = &'a LedgerRecord> {
        self.records
            .iter()
            .filter(move |record| record.event.run_id() == run_id)
    }

    /// All active (non-ended) runs initiated by `app_id`, including child
    /// runs. Used by `Kernel::uninstall` to cancel orphaned runs and release
    /// their leases.
    pub fn active_runs_for_app(&self, app_id: &AppId) -> Vec<RunId> {
        self.runs
            .iter()
            .filter(|(_, state)| state.active && &state.initiating_app == app_id)
            .map(|(run_id, _)| run_id.clone())
            .collect()
    }

    pub fn active_run_ids(&self) -> Vec<RunId> {
        self.runs
            .iter()
            .filter(|(_, state)| state.active)
            .map(|(run_id, _)| run_id.clone())
            .collect()
    }

    /// Aggregate one run's events into its read model.
    pub fn run_view(&self, run_id: &RunId) -> KernelResult<RunView> {
        let (initiator, goal) = self
            .find_start(run_id)
            .ok_or_else(|| KernelError::UnknownRun(run_id.clone()))?;

        let mut view = RunView {
            run_id: run_id.clone(),
            initiator,
            goal,
            grants_exercised: Vec::new(),
            invocations: Vec::new(),
            approvals: Vec::new(),
            artifacts_produced: Vec::new(),
            terminal_state: None,
        };

        for record in self.records_for_run(run_id) {
            match &record.event {
                LedgerEvent::CapabilityInvoked { grant_id, .. } => {
                    view.grants_exercised.push(grant_id.clone());
                }
                LedgerEvent::CapabilityCompleted {
                    capability,
                    grant_id,
                    data_scope,
                    ..
                } => view.invocations.push(InvocationRecord::Completed {
                    capability: capability.clone(),
                    grant_id: grant_id.clone(),
                    data_scope: data_scope.clone(),
                }),
                LedgerEvent::CapabilityFailed {
                    capability,
                    grant_id,
                    data_scope,
                    ..
                } => view.invocations.push(InvocationRecord::Failed {
                    capability: capability.clone(),
                    grant_id: grant_id.clone(),
                    data_scope: data_scope.clone(),
                }),
                LedgerEvent::InvocationRefused {
                    capability,
                    reason,
                    data_scope,
                    ..
                } => view.invocations.push(InvocationRecord::Refused {
                    capability: capability.clone(),
                    reason: *reason,
                    data_scope: data_scope.clone(),
                }),
                LedgerEvent::InvocationCancelled {
                    capability,
                    data_scope,
                    ..
                } => view.invocations.push(InvocationRecord::Cancelled {
                    capability: capability.clone(),
                    data_scope: data_scope.clone(),
                }),
                LedgerEvent::ApprovalGranted {
                    capability,
                    grant_id,
                    data_scope,
                    ..
                } => view.approvals.push(ApprovalRecord {
                    capability: capability.clone(),
                    grant_id: grant_id.clone(),
                    approved: true,
                    data_scope: data_scope.clone(),
                }),
                LedgerEvent::ApprovalDenied {
                    capability,
                    grant_id,
                    data_scope,
                    ..
                } => view.approvals.push(ApprovalRecord {
                    capability: capability.clone(),
                    grant_id: grant_id.clone(),
                    approved: false,
                    data_scope: data_scope.clone(),
                }),
                LedgerEvent::ArtifactProduced { artifact_id, .. } => {
                    view.artifacts_produced.push(artifact_id.clone());
                }
                LedgerEvent::RunEnded { terminal_state, .. } => {
                    view.terminal_state = Some(*terminal_state);
                }
                LedgerEvent::RunStarted { .. } | LedgerEvent::ApprovalRequested { .. } => {}
            }
        }
        Ok(view)
    }

    fn find_start(&self, run_id: &RunId) -> Option<(Initiator, String)> {
        self.records.iter().find_map(|record| match &record.event {
            LedgerEvent::RunStarted {
                run_id: started,
                initiator,
                goal,
            } if started == run_id => Some((initiator.clone(), goal.clone())),
            _ => None,
        })
    }

    fn replay_action_state(
        records: &[LedgerRecord],
    ) -> KernelResult<BTreeMap<RunId, OutstandingActions>> {
        let mut by_run: BTreeMap<RunId, OutstandingActions> = BTreeMap::new();
        for record in records {
            let run_id = record.event.run_id().clone();
            let actions = by_run.entry(run_id.clone()).or_default();
            match &record.event {
                LedgerEvent::RunStarted { .. } | LedgerEvent::ArtifactProduced { .. } => {}
                LedgerEvent::ApprovalRequested {
                    capability,
                    grant_id,
                    data_scope,
                    ..
                } => actions.requested_approvals.push(ApprovalKey {
                    capability: capability.clone(),
                    grant_id: grant_id.clone(),
                    data_scope: data_scope.clone(),
                }),
                LedgerEvent::ApprovalGranted {
                    capability,
                    grant_id,
                    data_scope,
                    ..
                } => {
                    let key = ApprovalKey {
                        capability: capability.clone(),
                        grant_id: grant_id.clone(),
                        data_scope: data_scope.clone(),
                    };
                    Self::remove_exact(
                        &mut actions.requested_approvals,
                        &key,
                        "approval grant has no matching request",
                    )?;
                    actions.granted_approvals.push(key);
                }
                LedgerEvent::ApprovalDenied {
                    capability,
                    grant_id,
                    data_scope,
                    ..
                } => Self::remove_exact(
                    &mut actions.requested_approvals,
                    &ApprovalKey {
                        capability: capability.clone(),
                        grant_id: grant_id.clone(),
                        data_scope: data_scope.clone(),
                    },
                    "approval denial has no matching request",
                )?,
                LedgerEvent::CapabilityInvoked {
                    capability,
                    grant_id,
                    data_scope,
                    ..
                } => {
                    let approval = ApprovalKey {
                        capability: capability.clone(),
                        grant_id: grant_id.clone(),
                        data_scope: data_scope.clone(),
                    };
                    if let Some(index) = actions
                        .granted_approvals
                        .iter()
                        .position(|candidate| candidate == &approval)
                    {
                        actions.granted_approvals.remove(index);
                    }
                    actions.invocations.push(InvocationKey {
                        capability: capability.clone(),
                        grant_id: grant_id.clone(),
                        data_scope: data_scope.clone(),
                    });
                }
                LedgerEvent::CapabilityCompleted {
                    capability,
                    grant_id,
                    data_scope,
                    ..
                }
                | LedgerEvent::CapabilityFailed {
                    capability,
                    grant_id,
                    data_scope,
                    ..
                } => Self::remove_exact(
                    &mut actions.invocations,
                    &InvocationKey {
                        capability: capability.clone(),
                        grant_id: grant_id.clone(),
                        data_scope: data_scope.clone(),
                    },
                    "invocation outcome has no matching invocation",
                )?,
                LedgerEvent::InvocationCancelled {
                    capability,
                    data_scope,
                    ..
                } => {
                    if !Self::remove_by_capability_scope(
                        &mut actions.invocations,
                        capability,
                        data_scope,
                    ) && !Self::remove_approval_by_capability_scope(
                        &mut actions.granted_approvals,
                        capability,
                        data_scope,
                    ) && !Self::remove_approval_by_capability_scope(
                        &mut actions.requested_approvals,
                        capability,
                        data_scope,
                    ) {
                        return Err(KernelError::Durability(format!(
                            "invocation cancellation in run '{run_id}' has no pending action"
                        )));
                    }
                }
                LedgerEvent::InvocationRefused {
                    capability,
                    data_scope,
                    ..
                } => {
                    // A refusal can be the complete no-grant path, or it can
                    // close work whose grant disappeared after preparation.
                    if !Self::remove_by_capability_scope(
                        &mut actions.invocations,
                        capability,
                        data_scope,
                    ) && !Self::remove_approval_by_capability_scope(
                        &mut actions.granted_approvals,
                        capability,
                        data_scope,
                    ) {
                        Self::remove_approval_by_capability_scope(
                            &mut actions.requested_approvals,
                            capability,
                            data_scope,
                        );
                    }
                }
                LedgerEvent::RunEnded { .. } => {
                    if !actions.requested_approvals.is_empty()
                        || !actions.granted_approvals.is_empty()
                        || !actions.invocations.is_empty()
                    {
                        return Err(KernelError::Durability(format!(
                            "run '{run_id}' ended with pending capability work"
                        )));
                    }
                }
            }
        }
        Ok(by_run)
    }

    fn remove_exact<T: PartialEq>(
        values: &mut Vec<T>,
        expected: &T,
        message: &str,
    ) -> KernelResult<()> {
        let index = values.iter().position(|candidate| candidate == expected);
        match index {
            Some(index) => {
                values.remove(index);
                Ok(())
            }
            None => Err(KernelError::Durability(message.into())),
        }
    }

    fn remove_by_capability_scope(
        values: &mut Vec<InvocationKey>,
        capability: &CapabilityRef,
        data_scope: &DataScope,
    ) -> bool {
        let index = values.iter().position(|candidate| {
            &candidate.capability == capability && &candidate.data_scope == data_scope
        });
        index.is_some_and(|index| {
            values.remove(index);
            true
        })
    }

    fn remove_approval_by_capability_scope(
        values: &mut Vec<ApprovalKey>,
        capability: &CapabilityRef,
        data_scope: &DataScope,
    ) -> bool {
        let index = values.iter().position(|candidate| {
            &candidate.capability == capability && &candidate.data_scope == data_scope
        });
        index.is_some_and(|index| {
            values.remove(index);
            true
        })
    }

    fn validate_append(
        runs: &BTreeMap<RunId, RunIndexEntry>,
        event: &LedgerEvent,
    ) -> KernelResult<()> {
        match event {
            LedgerEvent::RunStarted { run_id, .. } if runs.contains_key(run_id) => {
                Err(KernelError::RunAlreadyStarted(run_id.clone()))
            }
            LedgerEvent::RunStarted { .. } => Ok(()),
            _ => match runs.get(event.run_id()) {
                None => Err(KernelError::UnknownRun(event.run_id().clone())),
                Some(state) if !state.active => {
                    Err(KernelError::RunAlreadyEnded(event.run_id().clone()))
                }
                Some(_) => Ok(()),
            },
        }
    }

    fn validate_batch_append(
        runs: &BTreeMap<RunId, RunIndexEntry>,
        staged_runs: &BTreeMap<RunId, RunIndexEntry>,
        event: &LedgerEvent,
    ) -> KernelResult<()> {
        if staged_runs.contains_key(event.run_id()) {
            Self::validate_append(staged_runs, event)
        } else {
            Self::validate_append(runs, event)
        }
    }

    fn index_event(runs: &mut BTreeMap<RunId, RunIndexEntry>, event: &LedgerEvent) {
        match event {
            LedgerEvent::RunStarted {
                run_id, initiator, ..
            } => {
                runs.insert(
                    run_id.clone(),
                    RunIndexEntry {
                        initiating_app: initiator.app_id().clone(),
                        active: true,
                    },
                );
            }
            LedgerEvent::RunEnded { run_id, .. } => {
                runs.get_mut(run_id)
                    .expect("validated run must be indexed")
                    .active = false;
            }
            _ => {}
        }
    }
}

pub(crate) fn payload_sha256(value: &Value) -> KernelResult<String> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        KernelError::Durability(format!("serialize ledger payload failed: {error}"))
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests;
