//! Microkernel of a lean local desktop host for agentic apps.
//!
//! Five services, five primitives, one action path. Everything else — chat included — is
//! userland.
//!
//! The kernel is protocol-agnostic: it consumes generic manifests, JSON
//! schemas, capability handlers, and artifact drafts. Protocol adapters
//! (e.g. the MCP consumer adapter in `crates/mcp-adapter`) live outside
//! this crate and translate foreign protocols into those terms.

pub mod clock;
pub mod durable;
pub mod errors;
pub mod ids;
pub mod invocation;
pub mod kernel;
pub mod manifest;
pub mod primitives;
pub mod schema;
pub mod services;

pub use errors::{KernelError, KernelResult};
pub use invocation::{ProgressReportStatus, ProgressReporter};
pub use kernel::{
    ApprovalResult, AuthorizeInvocation, AuthorizedInvocation, CapabilityAuthorizationView,
    CapabilityUseView, ExecutedInvocation, GrantApproval, InstallApproval, Kernel,
    PrepareInvocation, PreparedGrant, PreparedInstall, PreparedInvocation, SurfaceActionOutcome,
};

/// JSON object as it crosses the kernel boundary (capability inputs and
/// JSON Schemas; artifact content is a full `serde_json::Value` since a
/// declared schema may describe any JSON shape). Inherently open by nature
/// of the wire format; it is validated against a declared schema before
/// entering the trusted core.
pub type JsonObject = serde_json::Map<String, serde_json::Value>;
