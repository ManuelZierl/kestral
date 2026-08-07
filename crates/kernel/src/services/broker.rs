//! Permission Broker.
//!
//! The single authority for issuing, checking, and revoking grants, and the
//! sole holder of secrets. Every capability invocation passes through
//! [`PermissionBroker::check`]; there is no second path.
//!
//! A missing grant or a user's "no" is an expected outcome, not a
//! programming error, so check and issuance return variants instead of
//! failing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::Duration;

use crate::clock::Clock;
use crate::errors::{KernelError, KernelResult};
use crate::ids::{new_grant_id, AppId, GrantId, ResourceId, SecretName, SecretRef};
use crate::manifest::GrantRequest;
use crate::primitives::capability::CapabilityRef;
use crate::primitives::grant::{
    DataScope, DenialReason, Grant, GrantCondition, GrantDuration, GrantOrigin, GrantStatus,
    GrantStatusView,
};
use crate::services::chrome::{ApprovalDecision, GrantIssuancePrompt, TrustedChrome};

#[derive(Debug, Clone, PartialEq)]
pub enum IssueResult {
    Issued(Grant),
    /// The user declined the permission through trusted chrome.
    Refused,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GrantCheck {
    Allowed(Grant),
    /// A covering grant exists but each use needs an interactive approval.
    ApprovalRequired(Grant),
    Denied(DenialReason),
}

/// Broker-scoped secret access for one app's connector handlers.
///
/// Resolves only secret names the app's manifest declares, and only for
/// that app's own owner scope. No shared or delegated secrets yet: an app
/// cannot grant another app access to one of its own secrets. Explicit
/// delegation is future work.
///
/// Raw values stay out of app-facing data: they are handed to the provider's
/// handler code at execution time and never appear in manifests, surfaces,
/// or the ledger.
/// TODO: in-process code cannot be stopped from copying a resolved secret
/// into its result; real containment needs the process sandbox the shell
/// provides. The kernel-side contract is still the right shape.
#[derive(Clone)]
pub struct SecretResolver {
    declared: BTreeSet<SecretName>,
    secrets: BTreeMap<SecretName, String>,
}

impl SecretResolver {
    pub fn resolve(&self, name: &SecretName) -> KernelResult<&str> {
        if !self.declared.contains(name) {
            return Err(KernelError::UndeclaredSecret(name.clone()));
        }
        self.secrets
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| KernelError::UnknownSecret(name.clone()))
    }
}

#[derive(Clone)]
pub struct PermissionBroker {
    clock: Arc<dyn Clock>,
    chrome: Arc<dyn TrustedChrome>,
    grants: BTreeMap<GrantId, Grant>,
    revoked: BTreeSet<GrantId>,
    secrets: BTreeMap<SecretRef, String>,
}

impl PermissionBroker {
    pub fn new(clock: Arc<dyn Clock>, chrome: Arc<dyn TrustedChrome>) -> Self {
        Self {
            clock,
            chrome,
            grants: BTreeMap::new(),
            revoked: BTreeSet::new(),
            secrets: BTreeMap::new(),
        }
    }

    /// Issue a grant to an installed app, confirmed through trusted chrome.
    /// The caller (the kernel) supplies the holder's verified display name.
    pub fn issue(
        &mut self,
        holder: &AppId,
        holder_display_name: &str,
        request: &GrantRequest,
        origin: GrantOrigin,
    ) -> IssueResult {
        let decision = self.chrome.confirm_grant(GrantIssuancePrompt {
            app_id: holder.clone(),
            app_display_name: holder_display_name.to_string(),
            scope: request.scope.clone(),
            data_scope: request.data_scope.clone(),
            condition: request.condition,
            duration: request.duration,
            reason: request.reason.clone(),
        });
        self.issue_with_decision(holder, request, origin, decision)
    }

    /// Commit a grant decision that was collected outside the kernel mutex.
    pub fn issue_with_decision(
        &mut self,
        holder: &AppId,
        request: &GrantRequest,
        origin: GrantOrigin,
        decision: ApprovalDecision,
    ) -> IssueResult {
        if decision == ApprovalDecision::Denied {
            return IssueResult::Refused;
        }
        let issued_at = self.clock.now();
        let expires_at = match request.duration {
            GrantDuration::NonExpiring => None,
            GrantDuration::ExpiresAfter { seconds } => {
                Some(issued_at + Duration::seconds(i64::from(seconds.get())))
            }
        };
        let grant = Grant {
            grant_id: new_grant_id(),
            holder: holder.clone(),
            scope: request.scope.clone(),
            data_scope: request.data_scope.clone(),
            condition: request.condition,
            origin,
            issued_at,
            expires_at,
        };
        grant
            .validate()
            .expect("validated grant request produced invalid grant");
        self.grants.insert(grant.grant_id.clone(), grant.clone());
        IssueResult::Issued(grant)
    }

    /// Decide whether `holder` may invoke `capability` right now.
    pub fn check(
        &self,
        holder: &AppId,
        capability: &CapabilityRef,
        requested_data_scope: &DataScope,
    ) -> GrantCheck {
        let covering: Vec<&Grant> = self
            .grants
            .values()
            .filter(|grant| {
                grant.holder == *holder
                    && grant.scope.covers(capability)
                    && grant.data_scope.covers(requested_data_scope)
            })
            .collect();
        if covering.is_empty() {
            return GrantCheck::Denied(DenialReason::NoGrant);
        }

        let live: Vec<&Grant> = covering
            .into_iter()
            .filter(|grant| !self.revoked.contains(&grant.grant_id))
            .collect();
        if live.is_empty() {
            return GrantCheck::Denied(DenialReason::Revoked);
        }

        let now = self.clock.now();
        // Ordered by issuance (grant_id as tiebreak) so which covering grant
        // wins is deterministic, not an artifact of container iteration order.
        let mut current: Vec<&Grant> = live.into_iter().filter(|g| !g.is_expired(now)).collect();
        current.sort_by(|a, b| (a.issued_at, &a.grant_id).cmp(&(b.issued_at, &b.grant_id)));
        let Some(first) = current.first() else {
            return GrantCheck::Denied(DenialReason::Expired);
        };

        // Prefer the least interactive covering grant; every use still lands
        // in the ledger regardless of condition.
        for condition in [GrantCondition::Silent, GrantCondition::Notify] {
            if let Some(grant) = current.iter().find(|g| g.condition == condition) {
                return GrantCheck::Allowed((*grant).clone());
            }
        }
        GrantCheck::ApprovalRequired((*first).clone())
    }

    pub fn revoke(&mut self, grant_id: &GrantId) -> KernelResult<()> {
        if !self.grants.contains_key(grant_id) {
            return Err(KernelError::UnknownGrant(grant_id.clone()));
        }
        self.revoked.insert(grant_id.clone());
        Ok(())
    }

    /// Uninstall-time cleanup: authority must not outlive the app, or a
    /// reinstall under the same AppId would silently inherit it.
    pub fn revoke_all_for(&mut self, holder: &AppId) {
        for (grant_id, grant) in &self.grants {
            if grant.holder == *holder {
                self.revoked.insert(grant_id.clone());
            }
        }
    }

    /// Uninstall-time cleanup, provider side: consumers' grants *over* the
    /// uninstalled provider die with it. The user consented to specific
    /// provider code; different code installed later under the same AppId
    /// must not be reachable through the old grants.
    pub fn revoke_all_over(&mut self, provider: &AppId) {
        for (grant_id, grant) in &self.grants {
            if grant.scope.provider() == provider {
                self.revoked.insert(grant_id.clone());
            }
        }
    }

    pub fn grant_ids_over_resource(&self, resource_id: &ResourceId) -> Vec<GrantId> {
        self.grants
            .values()
            .filter(|grant| match &grant.data_scope {
                DataScope::Resources { resource_ids } => resource_ids.contains(resource_id),
                DataScope::None | DataScope::AllResources => false,
            })
            .map(|grant| grant.grant_id.clone())
            .collect()
    }

    /// The holder's live grants: issued, not revoked, not expired.
    pub fn grants_for(&self, holder: &AppId) -> Vec<&Grant> {
        let now = self.clock.now();
        self.grants
            .values()
            .filter(|grant| {
                grant.holder == *holder
                    && !self.revoked.contains(&grant.grant_id)
                    && !grant.is_expired(now)
            })
            .collect()
    }

    pub fn grant_statuses_for(&self, holder: &AppId) -> Vec<GrantStatusView> {
        let now = self.clock.now();
        self.grants
            .values()
            .filter(|grant| grant.holder == *holder)
            .map(|grant| GrantStatusView {
                grant: grant.clone(),
                status: if self.revoked.contains(&grant.grant_id) {
                    GrantStatus::Revoked
                } else if grant.is_expired(now) {
                    GrantStatus::Expired
                } else {
                    GrantStatus::Active
                },
            })
            .collect()
    }

    pub fn durable_grants(&self) -> Vec<Grant> {
        self.grants.values().cloned().collect()
    }

    pub fn durable_revocations(&self) -> Vec<GrantId> {
        self.revoked.iter().cloned().collect()
    }

    pub fn restore(
        clock: Arc<dyn Clock>,
        chrome: Arc<dyn TrustedChrome>,
        grants: Vec<Grant>,
        revoked: Vec<GrantId>,
    ) -> KernelResult<Self> {
        let mut by_id = BTreeMap::new();
        for grant in grants {
            grant.validate()?;
            if by_id.insert(grant.grant_id.clone(), grant).is_some() {
                return Err(KernelError::Durability("duplicate grant id".into()));
            }
        }
        let revoked: BTreeSet<_> = revoked.into_iter().collect();
        if revoked.iter().any(|id| !by_id.contains_key(id)) {
            return Err(KernelError::Durability(
                "revocation references an unknown grant".into(),
            ));
        }
        Ok(Self {
            clock,
            chrome,
            grants: by_id,
            revoked,
            secrets: BTreeMap::new(),
        })
    }

    /// Store a secret with the broker, owned by a specific app.
    /// In a full host this is entered through trusted chrome; apps never
    /// call this. Two apps may use the same `SecretName` without collision.
    pub fn put_secret(&mut self, secret_ref: SecretRef, value: String) {
        self.secrets.insert(secret_ref, value);
    }

    pub fn clear_secret(&mut self, secret_ref: &SecretRef) {
        self.secrets.remove(secret_ref);
    }

    /// Remove every secret owned by `owner`. Called during uninstall so a
    /// later reinstall under the same AppId starts with no inherited secrets.
    pub fn clear_all_for(&mut self, owner: &AppId) {
        self.secrets.retain(|ref_, _| &ref_.owner != owner);
    }

    /// Secret access for the given `owner` app, scoped to its declared
    /// secret names. A snapshot: the resolver handed into handler code must
    /// neither observe later broker state nor carry other apps' secrets.
    ///
    /// Only secrets where both the owner matches and the name is declared
    /// are included, providing per-app namespacing without a global
    /// namespace.
    pub fn secret_resolver_for(&self, owner: &AppId, declared: Vec<SecretName>) -> SecretResolver {
        let declared: BTreeSet<SecretName> = declared.into_iter().collect();
        let secrets = self
            .secrets
            .iter()
            .filter(|(ref_, _)| &ref_.owner == owner && declared.contains(&ref_.name))
            .map(|(ref_, value)| (ref_.name.clone(), value.clone()))
            .collect();
        SecretResolver { declared, secrets }
    }
}
