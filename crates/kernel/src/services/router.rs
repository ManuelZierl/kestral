//! Message Router & Lease Manager.
//!
//! The router currently owns two things: app-facing event subscriptions and
//! advisory leases. It is not a general cross-app RPC bus: work still crosses
//! app boundaries through capability invocation under grants. Subscriptions are
//! a minimized kernel event feed, not a raw ledger export.
//!
//! Leases are advisory, time-bounded locks over artifacts and workspace
//! paths. Conflicts are returned as values (and surfaced to the user through
//! trusted chrome by the kernel), never silently overwritten.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::clock::Clock;
use crate::errors::{KernelError, KernelResult};
use crate::ids::{new_lease_id, AppId, ArtifactId, EventTopic, LeaseId, RunId};
use crate::primitives::capability::CapabilityRef;
use crate::primitives::run::RunTerminalState;
use crate::services::ledger::LedgerEvent;
use crate::services::registry::Registry;

/// The app-facing event envelope exposed to subscribed apps.
///
/// Raw ledger records stay inside the trusted core: userland can observe only a
/// closed tagged envelope with minimized run metadata or data-change metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum AppEventEnvelope {
    RunEvent {
        topic: EventTopic,
        run_id: RunId,
        actor: AppId,
        capability: Option<CapabilityRef>,
        artifact_id: Option<ArtifactId>,
        terminal_state: Option<RunTerminalState>,
    },
    AppDataChanged {
        provider_app_id: AppId,
        resource_ref: String,
        revision: u64,
        change_kind: AppDataChangeKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum AppDataChangeKind {
    Created,
    Updated,
    Deleted,
    Completed,
    AvailabilityChanged,
}

impl AppEventEnvelope {
    pub fn from_event(event: &LedgerEvent, actor: AppId) -> Self {
        let run_id = event.run_id().clone();
        let (capability, artifact_id, terminal_state) = match event {
            LedgerEvent::RunStarted { .. } => (None, None, None),
            LedgerEvent::CapabilityInvoked { capability, .. }
            | LedgerEvent::CapabilityCompleted { capability, .. }
            | LedgerEvent::CapabilityFailed { capability, .. }
            | LedgerEvent::InvocationRefused { capability, .. }
            | LedgerEvent::InvocationCancelled { capability, .. }
            | LedgerEvent::ApprovalRequested { capability, .. }
            | LedgerEvent::ApprovalGranted { capability, .. }
            | LedgerEvent::ApprovalDenied { capability, .. } => {
                (Some(capability.clone()), None, None)
            }
            LedgerEvent::ArtifactProduced { artifact_id, .. } => {
                (None, Some(artifact_id.clone()), None)
            }
            LedgerEvent::RunEnded { terminal_state, .. } => (None, None, Some(*terminal_state)),
        };
        Self::RunEvent {
            topic: event.topic(),
            run_id,
            actor,
            capability,
            artifact_id,
            terminal_state,
        }
    }
}

/// Observable state of one bounded event inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventInboxStatus {
    pub queued_events: usize,
    pub dropped_events: u64,
}

/// What a lease can cover — a closed set of resource shapes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum LeaseTarget {
    Artifact { artifact_id: ArtifactId },
    WorkspacePath { path: String },
}

impl LeaseTarget {
    pub fn resource_name(&self) -> String {
        match self {
            LeaseTarget::Artifact { artifact_id } => format!("artifact:{artifact_id}"),
            LeaseTarget::WorkspacePath { path } => format!("workspace:{path}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lease {
    pub lease_id: LeaseId,
    pub run_id: RunId,
    pub target: LeaseTarget,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl Lease {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LeaseOutcome {
    Acquired(Lease),
    /// Another active run already holds the target.
    Conflict {
        holder: Lease,
    },
}

pub struct LeaseManager {
    clock: Arc<dyn Clock>,
    leases: BTreeMap<LeaseId, Lease>,
}

impl LeaseManager {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            leases: BTreeMap::new(),
        }
    }

    pub fn acquire(
        &mut self,
        run_id: &RunId,
        target: LeaseTarget,
        duration: Duration,
    ) -> KernelResult<LeaseOutcome> {
        if duration <= Duration::zero() {
            return Err(KernelError::InvalidLeaseDuration);
        }
        let now = self.clock.now();
        self.evict_expired(now);
        if let Some(holder) = self.holder(&target).cloned() {
            if holder.run_id != *run_id {
                return Ok(LeaseOutcome::Conflict { holder });
            }
            // Reacquisition by the holding run renews the existing lease —
            // one lease per (run, target), never a stale twin.
            let renewed = Lease {
                lease_id: holder.lease_id.clone(),
                run_id: run_id.clone(),
                target,
                acquired_at: holder.acquired_at,
                expires_at: now + duration,
            };
            self.leases
                .insert(renewed.lease_id.clone(), renewed.clone());
            return Ok(LeaseOutcome::Acquired(renewed));
        }
        let lease = Lease {
            lease_id: new_lease_id(),
            run_id: run_id.clone(),
            target,
            acquired_at: now,
            expires_at: now + duration,
        };
        self.leases.insert(lease.lease_id.clone(), lease.clone());
        Ok(LeaseOutcome::Acquired(lease))
    }

    pub fn release(&mut self, lease_id: &LeaseId) -> KernelResult<()> {
        self.leases
            .remove(lease_id)
            .map(|_| ())
            .ok_or_else(|| KernelError::UnknownLease(lease_id.clone()))
    }

    pub fn release_all_for_run(&mut self, run_id: &RunId) {
        self.leases.retain(|_, lease| lease.run_id != *run_id);
    }

    /// The live lease on `target`, if any. A pure query: expired leases are
    /// skipped here and evicted by the mutating operations.
    pub fn holder(&self, target: &LeaseTarget) -> Option<&Lease> {
        let now = self.clock.now();
        self.leases
            .values()
            .find(|lease| lease.target == *target && !lease.is_expired(now))
    }

    fn evict_expired(&mut self, now: DateTime<Utc>) {
        self.leases.retain(|_, lease| !lease.is_expired(now));
    }
}

pub struct MessageRouter {
    inboxes: BTreeMap<AppId, EventInbox>,
}

struct EventInbox {
    events: VecDeque<AppEventEnvelope>,
    dropped_events: u64,
}

impl MessageRouter {
    /// Per-app limit. Oldest events are discarded first, and the loss remains
    /// observable through [`EventInboxStatus::dropped_events`].
    pub const MAX_INBOX_EVENTS: usize = 256;

    pub fn new() -> Self {
        Self {
            inboxes: BTreeMap::new(),
        }
    }

    /// Deliver a prebuilt event envelope to every app subscribed to its topic.
    ///
    /// Subscription is declared in the manifest (and validated against the
    /// closed topic set at install); an app cannot receive a topic it did
    /// not declare.
    ///
    /// Subscriptions are intentionally metadata-only. Capability input/result
    /// payloads stay in the trusted ledger and are never routed to userland
    /// apps as subscription payloads.
    /// Delivery has no fallible work after the ledger append. That keeps the
    /// audit record authoritative even when a subscriber is slow.
    pub fn publish(&mut self, view: AppEventEnvelope, registry: &Registry) {
        let topic = match &view {
            AppEventEnvelope::RunEvent { topic, .. } => topic.clone(),
            AppEventEnvelope::AppDataChanged { .. } => EventTopic::new("app-data-changed"),
        };
        for installed in registry.installed_apps() {
            let app_id = &installed.manifest.app_id;
            if registry.is_subscribed(app_id, &topic) {
                let inbox = self.inboxes.entry(app_id.clone()).or_insert(EventInbox {
                    events: VecDeque::new(),
                    dropped_events: 0,
                });
                if inbox.events.len() == Self::MAX_INBOX_EVENTS {
                    inbox.events.pop_front();
                    inbox.dropped_events += 1;
                }
                inbox.events.push_back(view.clone());
            }
        }
    }

    pub fn publish_data_change(
        &mut self,
        provider_app_id: &AppId,
        resource_ref: String,
        revision: u64,
        change_kind: AppDataChangeKind,
        registry: &Registry,
    ) {
        let envelope = AppEventEnvelope::AppDataChanged {
            provider_app_id: provider_app_id.clone(),
            resource_ref,
            revision,
            change_kind,
        };
        for installed in registry.installed_apps() {
            let app_id = &installed.manifest.app_id;
            if registry.is_subscribed(app_id, &EventTopic::new("app-data-changed")) {
                let inbox = self.inboxes.entry(app_id.clone()).or_insert(EventInbox {
                    events: VecDeque::new(),
                    dropped_events: 0,
                });
                if inbox.events.len() == Self::MAX_INBOX_EVENTS {
                    inbox.events.pop_front();
                    inbox.dropped_events += 1;
                }
                inbox.events.push_back(envelope.clone());
            }
        }
    }

    pub fn publish_batch(&mut self, views: Vec<AppEventEnvelope>, registry: &Registry) {
        for view in views {
            self.publish(view, registry);
        }
    }

    pub fn drain_inbox(
        &mut self,
        app_id: &AppId,
        registry: &Registry,
    ) -> KernelResult<Vec<AppEventEnvelope>> {
        registry.app(app_id)?;
        Ok(self
            .inboxes
            .get_mut(app_id)
            .map(|inbox| std::mem::take(&mut inbox.events).into())
            .unwrap_or_default())
    }

    pub fn inbox_status(
        &self,
        app_id: &AppId,
        registry: &Registry,
    ) -> KernelResult<EventInboxStatus> {
        registry.app(app_id)?;
        let status = self.inboxes.get(app_id).map_or(
            EventInboxStatus {
                queued_events: 0,
                dropped_events: 0,
            },
            |inbox| EventInboxStatus {
                queued_events: inbox.events.len(),
                dropped_events: inbox.dropped_events,
            },
        );
        Ok(status)
    }

    /// Uninstall-time cleanup: queued events must not survive the app (or
    /// be inherited by a later install under the same AppId).
    pub fn discard_inbox(&mut self, app_id: &AppId) {
        self.inboxes.remove(app_id);
    }
}

impl Default for MessageRouter {
    fn default() -> Self {
        Self::new()
    }
}
