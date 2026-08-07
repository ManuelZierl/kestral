//! Surface: a visual place where an app renders.
//!
//! Surfaces are sandboxed and intent-only: they receive data and emit
//! `ActionIntent`s to the kernel. They never hold secrets, never call tools,
//! and never talk to other surfaces except through the kernel.
//! MCP Apps / mcp-ui resources are one wire format for surfaces; the
//! primitive is broader than any one protocol.

use serde::{Deserialize, Serialize};

use crate::ids::SurfaceName;
use crate::primitives::capability::CapabilityRef;
use crate::primitives::grant::DataScope;
use crate::JsonObject;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SurfaceKind {
    Panel,
    Card,
    Form,
    Picker,
    Dashboard,
}

/// A surface as declared in an app manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceDeclaration {
    pub name: SurfaceName,
    pub kind: SurfaceKind,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub intents: Vec<CapabilityRef>,
}

/// The only thing a surface can emit: a request, never an execution.
///
/// The kernel turns an accepted intent into a run and drives it through the
/// single action path (grant check, approval, invocation, ledger record).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionIntent {
    pub capability: CapabilityRef,
    pub input: JsonObject,
    pub data_scope: DataScope,
    pub goal: String,
}
