//! Grant: a scoped permission held by an app.
//!
//! Grants say what an app may do, over what data, under what interaction
//! condition, and for how long. They are issued and enforced exclusively by
//! the permission broker; approvals are grant decisions made interactively
//! through trusted chrome.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::errors::{KernelError, KernelResult};
use crate::ids::{AppId, CapabilityName, GrantId, ResourceId};
use crate::primitives::capability::CapabilityRef;

/// How exercising the grant interacts with the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GrantCondition {
    Silent,
    Notify,
    RequiresApproval,
}

/// Why a grant check denies access — closed grant-domain vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DenialReason {
    NoGrant,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GrantStatus {
    Active,
    Revoked,
    Expired,
}

/// Why the broker issued this immutable grant fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GrantOrigin {
    ManifestRequested,
    UserAdded,
    McpExport,
    SystemBundled,
}

/// How long a requested grant lives.
///
/// Required wherever it appears: a manifest must say "non-expiring"
/// explicitly instead of getting the most permissive lifetime by omission.
/// `NonZeroU32` seconds make a dead-on-arrival (zero) or arithmetic-overflow
/// lifetime unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum GrantDuration {
    NonExpiring,
    ExpiresAfter { seconds: std::num::NonZeroU32 },
}

/// What a grant covers — a closed set of scope shapes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum GrantScope {
    /// Permission over one named capability of one provider app.
    ExactCapability {
        provider: AppId,
        capability: CapabilityName,
    },
    /// Permission over every capability one provider app declares.
    AllProviderCapabilities { provider: AppId },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum DataScope {
    None,
    /// Every current and future resource governed by the grant's capability.
    /// Invocation requests must still name their exact resources.
    AllResources,
    Resources {
        resource_ids: Vec<ResourceId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum DataScopeWire {
    None,
    AllResources,
    Resources { resource_ids: Vec<ResourceId> },
}

impl DataScope {
    pub fn none() -> Self {
        Self::None
    }

    pub fn resources(resource_ids: Vec<ResourceId>) -> KernelResult<Self> {
        let data_scope = Self::Resources { resource_ids };
        data_scope.validate()?;
        Ok(data_scope)
    }

    pub fn validate(&self) -> KernelResult<()> {
        match self {
            DataScope::None | DataScope::AllResources => Ok(()),
            DataScope::Resources { resource_ids } => {
                if resource_ids.is_empty() {
                    return Err(KernelError::InvalidGrantDataScope {
                        message: "resource scope must name at least one resource".into(),
                    });
                }
                let unique: BTreeSet<&ResourceId> = resource_ids.iter().collect();
                if unique.len() != resource_ids.len() {
                    return Err(KernelError::InvalidGrantDataScope {
                        message: "resource scope must not repeat resource ids".into(),
                    });
                }
                Ok(())
            }
        }
    }

    pub fn covers(&self, requested: &DataScope) -> bool {
        match (self, requested) {
            (DataScope::None, DataScope::None) => true,
            (DataScope::AllResources, DataScope::AllResources | DataScope::Resources { .. }) => {
                true
            }
            (
                DataScope::Resources {
                    resource_ids: granted,
                },
                DataScope::Resources {
                    resource_ids: requested,
                },
            ) => requested
                .iter()
                .all(|resource_id| granted.contains(resource_id)),
            _ => false,
        }
    }

    pub fn validate_invocation(&self) -> KernelResult<()> {
        self.validate()?;
        if self == &DataScope::AllResources {
            return Err(KernelError::InvalidGrantDataScope {
                message: "invocations must name exact resources; all-resources is grant-only"
                    .into(),
            });
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for DataScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = DataScopeWire::deserialize(deserializer)?;
        let data_scope = match wire {
            DataScopeWire::None => DataScope::None,
            DataScopeWire::AllResources => DataScope::AllResources,
            DataScopeWire::Resources { resource_ids } => DataScope::Resources { resource_ids },
        };
        data_scope.validate().map_err(serde::de::Error::custom)?;
        Ok(data_scope)
    }
}

impl GrantScope {
    /// The provider app this scope points at — every scope shape names
    /// exactly one.
    pub fn provider(&self) -> &AppId {
        match self {
            GrantScope::ExactCapability { provider, .. } => provider,
            GrantScope::AllProviderCapabilities { provider } => provider,
        }
    }

    pub fn covers(&self, capability_ref: &CapabilityRef) -> bool {
        match self {
            GrantScope::ExactCapability {
                provider,
                capability,
            } => capability_ref.provider == *provider && capability_ref.capability == *capability,
            GrantScope::AllProviderCapabilities { provider } => {
                capability_ref.provider == *provider
            }
        }
    }
}

/// An issued permission. A fact of issuance — revocation is broker state.
///
/// `expires_at` of None means the grant does not expire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub grant_id: GrantId,
    pub holder: AppId,
    pub scope: GrantScope,
    pub data_scope: DataScope,
    pub condition: GrantCondition,
    pub origin: GrantOrigin,
    pub issued_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl Grant {
    pub fn validate(&self) -> KernelResult<()> {
        self.data_scope.validate()?;
        if self
            .expires_at
            .is_some_and(|expires_at| expires_at <= self.issued_at)
        {
            return Err(KernelError::InvalidGrantDuration);
        }
        Ok(())
    }

    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|expires_at| now >= expires_at)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantStatusView {
    #[serde(flatten)]
    pub grant: Grant,
    pub status: GrantStatus,
}
