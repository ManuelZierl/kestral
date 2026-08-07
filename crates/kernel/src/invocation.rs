//! The executable capability contract between apps and the kernel.
//!
//! Apps bind a handler to every capability they declare. Handlers receive
//! validated input plus an [`InvocationContext`] and return a
//! [`CapabilityOutcome`]: a result value and artifact drafts. The kernel —
//! not the handler — validates drafts, stamps provenance, and writes the
//! ledger.
//!
//! The outcome of an invocation is a closed set of variants. Refusals (no
//! grant, user said no) and app failures are expected outcomes the caller
//! must handle, not errors.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{AppId, RunId};
use crate::primitives::artifact::{Artifact, ArtifactDraft};
use crate::primitives::grant::DataScope;
use crate::services::artifacts::ArtifactSnapshotResolver;
use crate::services::broker::SecretResolver;
use crate::JsonObject;

/// What a handler may know: the run it serves, who asked, and the provider's
/// own broker-mediated secret access. No kernel handle — a handler cannot
/// start runs or invoke capabilities directly.
#[derive(Clone)]
pub struct InvocationContext {
    pub run_id: RunId,
    pub invoked_by: AppId,
    pub invoked_by_version: String,
    pub invoked_by_content_hash: String,
    pub authorized_data_scope: DataScope,
    pub secrets: SecretResolver,
    pub artifacts: ArtifactSnapshotResolver,
    pub cancellation: CancellationHandle,
    /// Transient provider progress. It is never committed as a result or
    /// ledger record and carries no authority back into the kernel.
    pub progress: ProgressReporter,
}

#[derive(Clone)]
pub struct ProgressReporter {
    report: Arc<dyn Fn(Value) -> Result<(), ()> + Send + Sync>,
    last_emitted: Arc<Mutex<Option<Instant>>>,
    cancellation: Option<CancellationHandle>,
}

const MAX_PROGRESS_EVENT_BYTES: usize = 64 * 1024;
const MAX_PROGRESS_KIND_BYTES: usize = 64;
const MIN_PROGRESS_INTERVAL: Duration = Duration::from_micros(8_333);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressReportStatus {
    Emitted,
    Invalid,
    Oversized,
    ConsumerGone,
}

impl ProgressReporter {
    pub fn new(report: impl Fn(Value) + Send + Sync + 'static) -> Self {
        Self::new_checked(move |value| {
            report(value);
            Ok(())
        })
    }

    pub fn new_checked(report: impl Fn(Value) -> Result<(), ()> + Send + Sync + 'static) -> Self {
        Self {
            report: Arc::new(report),
            last_emitted: Arc::new(Mutex::new(None)),
            cancellation: None,
        }
    }

    pub(crate) fn with_cancellation(mut self, cancellation: CancellationHandle) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    pub fn report(&self, value: Value) -> ProgressReportStatus {
        let valid_kind = value
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| !kind.is_empty() && kind.len() <= MAX_PROGRESS_KIND_BYTES);
        if !value.is_object() || !valid_kind {
            return ProgressReportStatus::Invalid;
        }
        if match serde_json::to_vec(&value) {
            Ok(bytes) => bytes.len() > MAX_PROGRESS_EVENT_BYTES,
            Err(_) => true,
        } {
            return ProgressReportStatus::Oversized;
        }
        let mut last_emitted = self
            .last_emitted
            .lock()
            .expect("progress reporter mutex poisoned");
        if let Some(remaining) = (*last_emitted)
            .and_then(|last| MIN_PROGRESS_INTERVAL.checked_sub(Instant::now().duration_since(last)))
        {
            std::thread::sleep(remaining);
        }
        *last_emitted = Some(Instant::now());
        drop(last_emitted);
        if (self.report)(value).is_err() {
            if let Some(cancellation) = &self.cancellation {
                cancellation.cancel();
            }
            return ProgressReportStatus::ConsumerGone;
        }
        ProgressReportStatus::Emitted
    }
}

impl Default for ProgressReporter {
    fn default() -> Self {
        Self::new(|_| {})
    }
}

/// Cooperative cancellation supplied to provider code. It is intentionally
/// independent of the kernel so work running outside the host mutex cannot
/// re-enter trusted state.
#[derive(Clone)]
pub struct CancellationHandle {
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}

impl CancellationHandle {
    pub(crate) fn new(cancelled: Arc<AtomicBool>, deadline: Instant) -> Self {
        Self {
            cancelled,
            deadline,
        }
    }

    /// Returns whether cancellation was requested. A deadline crossing also
    /// requests cancellation so every clone observes the same terminal state.
    /// This is cooperative only: it cannot roll back side effects that a
    /// handler or a child task started before observing the signal.
    pub fn is_cancelled(&self) -> bool {
        if Instant::now() >= self.deadline {
            self.cancel();
        }
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// What a handler returns: a result plus proposed artifacts.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityOutcome {
    pub result: Value,
    pub artifacts: Vec<ArtifactDraft>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationRequest {
    pub input: JsonObject,
    pub data_scope: DataScope,
}

/// An app-level failure inside a handler. Contained by the kernel: it fails
/// the invocation, never the kernel.
#[derive(Debug, Clone, PartialEq)]
pub struct HandlerFailure(pub String);

pub type CapabilityHandler = Box<
    dyn Fn(&JsonObject, &InvocationContext) -> Result<CapabilityOutcome, HandlerFailure>
        + Send
        + Sync,
>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefusalReason {
    NoGrant,
    GrantExpired,
    GrantRevoked,
    ApprovalDenied,
    /// The invocation was cancelled — by an explicit cancel signal or by
    /// crossing its deadline — after grant authorization but before its
    /// result could be committed. This is distinct from a grant denial: the
    /// grant remained valid; the work was stopped. The two mechanisms
    /// (explicit cancel, timeout) are not distinguished here because the
    /// cooperative cancellation flag does not currently record which fired.
    Cancelled,
}

/// The closed set of invocation outcomes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum InvocationResult {
    Completed {
        result: Value,
        artifacts: Vec<Artifact>,
    },
    /// The broker or the user refused before any app code ran.
    Refused { reason: RefusalReason },
    /// The provider's handler failed or returned invalid artifacts.
    Failed { error: String },
}

#[cfg(test)]
mod tests;
