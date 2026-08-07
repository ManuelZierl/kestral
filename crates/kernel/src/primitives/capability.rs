//! Capability: something the system can do.
//!
//! Capabilities are declared by apps in their manifest, invoked only through
//! the kernel, and gated by grants. An MCP tool is one kind of capability.

use serde::{Deserialize, Serialize};

use crate::ids::{AppId, CapabilityName};
use crate::JsonObject;

/// Fully qualified reference to one app's capability.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRef {
    pub provider: AppId,
    pub capability: CapabilityName,
}

impl CapabilityRef {
    pub fn qualified_name(&self) -> String {
        format!("{}/{}", self.provider, self.capability)
    }
}

/// Advisory effect metadata for UI and export defaults. Effects never grant
/// authority; grants remain the only permission mechanism.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityEffect {
    #[default]
    Unspecified,
    ReadOnly,
    LocalWrite,
    ExternalWrite,
    Destructive,
}

/// A capability as declared in an app manifest.
///
/// `input_schema` is a JSON Schema; the kernel validates every invocation
/// input against it before the provider's handler runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDeclaration {
    pub name: CapabilityName,
    pub description: String,
    pub input_schema: JsonObject,
    #[serde(default)]
    pub effect: CapabilityEffect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<JsonObject>,
}
