//! Kernel error type.
//!
//! Errors here mean the caller violated a kernel invariant (undeclared
//! behavior, malformed data at a boundary, unknown identity). They fail fast
//! and carry the offending identifiers. Expected outcomes that are not
//! programming errors — a user denying approval, a missing grant, a lease
//! conflict — are modeled as result variants on the relevant operations,
//! not errors.

use thiserror::Error;

use crate::ids::{
    AppId, ArtifactId, ArtifactTypeName, CapabilityName, EventTopic, GrantId, LeaseId, RunId,
    SecretName, SurfaceName,
};
use crate::primitives::capability::CapabilityRef;
use crate::primitives::grant::DenialReason;

pub type KernelResult<T> = Result<T, KernelError>;

#[derive(Debug, Error)]
pub enum KernelError {
    #[error("app '{0}' is not installed")]
    UnknownApp(AppId),

    #[error("app '{0}' is already installed")]
    AppAlreadyInstalled(AppId),

    #[error("content hash does not match manifest content of '{0}'")]
    ManifestContentHashMismatch(AppId),

    #[error("manifest identity field '{field}' must be non-empty")]
    ManifestIdentityInvalid { field: &'static str },

    #[error("manifest of '{app}' declares duplicate {contribution} names: {names:?}")]
    ManifestContributionInvalid {
        app: AppId,
        contribution: &'static str,
        names: Vec<String>,
    },

    #[error("surface '{surface}' of app '{app}' declares invalid intents: {message}")]
    ManifestSurfaceIntentInvalid {
        app: AppId,
        surface: SurfaceName,
        message: String,
    },

    #[error("extension contribution surface '{surface}' of app '{app}' is invalid: {message}")]
    ManifestExtensionContributionInvalid {
        app: AppId,
        surface: SurfaceName,
        message: String,
    },

    // There is deliberately no "extension target/point unavailable" error. An
    // extension contribution whose target app, extension point, or contract
    // version is absent is dormant, not invalid: the kernel stores it and the
    // host simply mounts nothing for it. Making that an error would stop the
    // kernel restoring after a user uninstalls a target app.
    #[error(
        "grouped grant approval contains grants from different holders or trusted-chrome contexts"
    )]
    PreparedGrantGroupMismatch,

    #[error("app '{app}' subscribes to unknown event topic '{topic}'")]
    UnknownEventTopic { app: AppId, topic: EventTopic },

    #[error("event-feed subscription was denied for app '{0}'")]
    EventSubscriptionDenied(AppId),

    #[error(
        "app '{app}': declared capabilities {declared:?} but handlers were \
         offered for {offered:?}"
    )]
    HandlerBindingMismatch {
        app: AppId,
        declared: Vec<String>,
        offered: Vec<String>,
    },

    #[error("app '{app}' does not declare capability '{capability}'")]
    UndeclaredCapability {
        app: AppId,
        capability: CapabilityName,
    },

    #[error("app '{app}' has no bound handler for capability '{capability}'")]
    HandlerNotBound {
        app: AppId,
        capability: CapabilityName,
    },

    #[error("app '{app}' does not declare surface '{surface}'")]
    UndeclaredSurface { app: AppId, surface: SurfaceName },

    #[error("surface '{surface}' of app '{app}' is not open")]
    SurfaceNotOpen { app: AppId, surface: SurfaceName },

    #[error(
        "surface '{surface}' of app '{app}' does not declare intent for capability \
         '{capability:?}'"
    )]
    UndeclaredSurfaceIntent {
        app: AppId,
        surface: SurfaceName,
        capability: CapabilityRef,
    },

    #[error("app '{app}' does not declare artifact type '{artifact_type}'")]
    UndeclaredArtifactType {
        app: AppId,
        artifact_type: ArtifactTypeName,
    },

    #[error("secret '{0}' is not declared by this app")]
    UndeclaredSecret(SecretName),

    #[error("secret '{0}' has not been stored")]
    UnknownSecret(SecretName),

    #[error("{described_as} is not a valid JSON Schema: {message}")]
    InvalidSchema {
        described_as: String,
        message: String,
    },

    #[error("input to '{described_as}' rejected by declared schema: {message}")]
    InvalidCapabilityInput {
        described_as: String,
        message: String,
    },

    #[error("output of '{described_as}' rejected by declared schema: {message}")]
    InvalidCapabilityOutput {
        described_as: String,
        message: String,
    },

    #[error("artifact '{described_as}' rejected by declared schema: {message}")]
    InvalidArtifactContent {
        described_as: String,
        message: String,
    },

    #[error("no run '{0}' in the ledger")]
    UnknownRun(RunId),

    #[error("run '{0}' was already started")]
    RunAlreadyStarted(RunId),

    #[error("run '{0}' has already ended")]
    RunAlreadyEnded(RunId),

    #[error(
        "child run must run on behalf of '{expected}', the app of its parent run — got '{got}'"
    )]
    ChildRunAttributionMismatch { expected: AppId, got: AppId },

    #[error("no artifact '{0}'")]
    UnknownArtifact(ArtifactId),

    #[error("no active lease '{0}'")]
    UnknownLease(LeaseId),

    #[error("no grant '{0}' was ever issued")]
    UnknownGrant(GrantId),

    #[error("grant reason must not be empty")]
    GrantReasonRequired,

    #[error("grant data scope is invalid: {message}")]
    InvalidGrantDataScope { message: String },

    #[error("grant expiry must be later than issuance")]
    InvalidGrantDuration,

    #[error("lease duration must be positive")]
    InvalidLeaseDuration,

    #[error("invocation timeout is too large for the system clock")]
    InvalidInvocationTimeout,

    #[error("prepared invocation became stale before finalization")]
    PreparedInvocationStale,

    #[error("prepared invocation was cancelled or timed out")]
    PreparedInvocationCancelled,

    #[error("prepared invocation was already consumed")]
    PreparedInvocationConsumed,

    #[error("prepared invocation was denied: {0:?}")]
    PreparedInvocationDenied(DenialReason),

    #[error("prepared install became stale before commit")]
    PreparedInstallStale,

    #[error("upgrade of app '{app}' changes its authority or behavioral contract")]
    AppUpgradeContractChanged { app: AppId },

    #[error("durable kernel state failed: {0}")]
    Durability(String),
}
