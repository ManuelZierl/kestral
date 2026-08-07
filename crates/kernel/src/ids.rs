//! Typed identifiers for kernel concepts.
//!
//! Distinct newtypes keep an `AppId` from being passed where a `RunId` is
//! expected. Generated IDs are created at construction time so objects never
//! exist with placeholder identities.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(AppId);
id_type!(RunId);
id_type!(ArtifactId);
id_type!(GrantId);
id_type!(LeaseId);
id_type!(CapabilityName);
id_type!(SurfaceName);
id_type!(ExtensionPointName);
id_type!(ArtifactTypeName);
id_type!(ConfigName);
id_type!(SecretName);
id_type!(EventTopic);
id_type!(SurfaceInstanceId);
id_type!(ResourceId);

/// An owner-scoped secret reference: only the owning app's connector code
/// may resolve this secret. Two apps may use the same `SecretName` without
/// collision because the `owner` partition is enforced by the broker.
///
/// No shared or delegated secrets yet: an app cannot grant another app access
/// to one of its own secrets. Explicit delegation is future work.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretRef {
    pub owner: AppId,
    pub name: SecretName,
}

pub fn new_run_id() -> RunId {
    RunId::new(format!("run-{}", Uuid::new_v4()))
}

pub fn new_artifact_id() -> ArtifactId {
    ArtifactId::new(format!("artifact-{}", Uuid::new_v4()))
}

pub fn new_grant_id() -> GrantId {
    GrantId::new(format!("grant-{}", Uuid::new_v4()))
}

pub fn new_lease_id() -> LeaseId {
    LeaseId::new(format!("lease-{}", Uuid::new_v4()))
}

pub fn new_surface_instance_id() -> SurfaceInstanceId {
    SurfaceInstanceId::new(format!("si-{}", Uuid::new_v4()))
}
