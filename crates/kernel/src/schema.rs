//! JSON Schema validation at the kernel boundary.
//!
//! Capability inputs and artifact contents are external data; they are
//! checked against their declared schemas once, on the way in, and rejected
//! with located errors (undeclared behavior is impossible by
//! construction).

use serde_json::Value;

use crate::errors::{KernelError, KernelResult};
use crate::JsonObject;

// TODO: schemas are re-compiled on every validation even though their
// validity is established once at install; the compiled validator should be
// stored with the installed declaration (a registry-side runtime object).

/// Reject malformed JSON Schemas at declaration time, not first use.
pub fn require_valid_schema(schema: &JsonObject, described_as: &str) -> KernelResult<()> {
    jsonschema::validator_for(&Value::Object(schema.clone())).map_err(|error| {
        KernelError::InvalidSchema {
            described_as: described_as.to_string(),
            message: error.to_string(),
        }
    })?;
    Ok(())
}

/// The two boundary shapes a schema can reject.
pub enum SchemaViolation {
    CapabilityInput,
    CapabilityOutput,
    ArtifactContent,
}

pub fn validate_against_schema(
    instance: &Value,
    schema: &JsonObject,
    violation: SchemaViolation,
    described_as: &str,
) -> KernelResult<()> {
    let validator = jsonschema::validator_for(&Value::Object(schema.clone())).map_err(|error| {
        KernelError::InvalidSchema {
            described_as: described_as.to_string(),
            message: error.to_string(),
        }
    })?;
    let mut messages: Vec<String> = validator
        .iter_errors(instance)
        .map(|error| format!("{}: {}", error.instance_path, error))
        .collect();
    if messages.is_empty() {
        return Ok(());
    }
    messages.sort();
    let message = messages.join("; ");
    let described_as = described_as.to_string();
    Err(match violation {
        SchemaViolation::CapabilityInput => KernelError::InvalidCapabilityInput {
            described_as,
            message,
        },
        SchemaViolation::CapabilityOutput => KernelError::InvalidCapabilityOutput {
            described_as,
            message,
        },
        SchemaViolation::ArtifactContent => KernelError::InvalidArtifactContent {
            described_as,
            message,
        },
    })
}
