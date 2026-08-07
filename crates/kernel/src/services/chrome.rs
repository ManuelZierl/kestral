//! Trusted chrome.
//!
//! All approval prompts, permission dialogs, identity badges, and warnings
//! are rendered by the kernel through this port — never by apps. The kernel
//! defines the contract; shells (Tauri window, console, test fake)
//! implement it. Only kernel services hold a reference to the chrome, so
//! apps cannot reach it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ids::{AppId, EventTopic, GrantId, RunId};
use crate::primitives::capability::CapabilityRef;
use crate::primitives::grant::{DataScope, GrantCondition, GrantDuration, GrantScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalDecision {
    Approved,
    Denied,
}

/// Shown when an app asks for a permission (typically at install).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantIssuancePrompt {
    pub app_id: AppId,
    pub app_display_name: String,
    pub scope: GrantScope,
    pub data_scope: DataScope,
    pub condition: GrantCondition,
    /// How long the grant would last if approved — so the user can weigh the
    /// time dimension ("until revoked" vs "expires in 24 hours") before
    /// consenting, not just what and how.
    pub duration: GrantDuration,
    pub reason: String,
}

/// Shown when a requires-approval grant is about to be exercised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityApprovalPrompt {
    pub app_id: AppId,
    pub app_display_name: String,
    pub capability: CapabilityRef,
    pub data_scope: DataScope,
    pub grant_id: GrantId,
    pub run_id: RunId,
    pub goal: String,
}

/// Shown at install when an app asks to consume the kernel event feed.
///
/// Event subscriptions use the existing manifest and trusted-chrome seams;
/// they are not a sixth kernel primitive or a cross-app transport grant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventSubscriptionPrompt {
    pub app_id: AppId,
    pub app_display_name: String,
    pub topics: Vec<EventTopic>,
}

/// Everything one app asks for at install time, gathered so the user can weigh
/// and answer the whole request in a single decision instead of clicking
/// through one grant at a time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallApprovalPrompt {
    pub app_id: AppId,
    pub app_display_name: String,
    pub event: Option<EventSubscriptionPrompt>,
    pub grants: Vec<GrantIssuancePrompt>,
}

/// The user's answer to an [`InstallApprovalPrompt`]. `grant_decisions` is
/// positionally aligned with the prompt's `grants`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallApprovalDecision {
    pub event_decision: Option<ApprovalDecision>,
    pub grant_decisions: Vec<ApprovalDecision>,
}

/// Kernel-issued user-facing notices — a closed set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ChromeNotice {
    /// A notify-condition grant was exercised.
    GrantUse {
        app_id: AppId,
        capability: CapabilityRef,
        grant_id: GrantId,
        run_id: RunId,
    },
    /// Two runs contended for the same artifact or workspace path.
    LeaseConflict {
        resource: String,
        holding_run: RunId,
        requesting_run: RunId,
    },
}

/// Notice delivery failures are treated as operational errors, not silent
/// best-effort drops.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChromeNoticeError {
    #[error("trusted notice persistence failed: {message}")]
    Persistence { message: String },

    #[error("trusted notice delivery failed: {message}")]
    Delivery { message: String },
}

/// The shell-implemented rendering port for kernel-owned UI.
pub trait TrustedChrome: Send + Sync {
    fn confirm_grant(&self, prompt: GrantIssuancePrompt) -> ApprovalDecision;

    fn approve_capability(&self, prompt: CapabilityApprovalPrompt) -> ApprovalDecision;

    fn confirm_event_subscriptions(&self, prompt: EventSubscriptionPrompt) -> ApprovalDecision;

    /// Confirm one app's entire install request at once. The default composes
    /// the per-item prompts and preserves the "deny the event feed ⇒ deny
    /// every grant" rule, so non-interactive chromes keep their behavior.
    /// Interactive shells override this to present a single batched modal.
    fn confirm_install(&self, prompt: InstallApprovalPrompt) -> InstallApprovalDecision {
        let event_decision = prompt
            .event
            .map(|event| self.confirm_event_subscriptions(event));
        let grant_decisions = if event_decision == Some(ApprovalDecision::Denied) {
            Vec::new()
        } else {
            prompt
                .grants
                .into_iter()
                .map(|grant| self.confirm_grant(grant))
                .collect()
        };
        InstallApprovalDecision {
            event_decision,
            grant_decisions,
        }
    }

    fn show_notice(&self, notice: ChromeNotice) -> Result<(), ChromeNoticeError>;
}
