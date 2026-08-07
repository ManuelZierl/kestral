//! The shell's trusted chrome.
//!
//! The kernel calls this port whenever the user must decide or be told
//! something. The shell renders those prompts in kernel-owned UI (a modal
//! the frontend reserves for chrome, visually distinct from app content) and
//! blocks the requesting kernel operation until the user answers.
//!
//! Approval is never assumed: if the frontend is gone or the user does not
//! answer within the timeout, the decision is Denied.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use tauri::{AppHandle, Emitter};

use app_host_kernel::ids::AppId;
use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, InstallApprovalDecision, InstallApprovalPrompt,
    TrustedChrome,
};

mod notices;

pub(crate) use notices::TrustedNoticeStore;

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

pub const CHROME_REQUEST_EVENT: &str = "trusted-chrome:request";
pub const CHROME_REQUEST_EXPIRED_EVENT: &str = "trusted-chrome:request-expired";
pub const CHROME_NOTICE_EVENT: &str = "trusted-chrome:notice";
pub const CHROME_OAUTH_EVENT: &str = "trusted-chrome:oauth";
const MAX_TRUSTED_NOTICE_HISTORY: usize = 1000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum OAuthPublicEvent {
    AuthUrl {
        session_id: String,
        url: String,
        instructions: Option<String>,
    },
    DeviceCode {
        session_id: String,
        user_code: String,
        verification_uri: String,
        interval_seconds: Option<u64>,
        expires_in_seconds: Option<u64>,
    },
    Progress {
        session_id: String,
        message: String,
    },
    Prompt {
        session_id: String,
        prompt_id: String,
        prompt: crate::llm_client::OAuthPrompt,
    },
    Completed {
        session_id: String,
    },
    Failed {
        session_id: String,
        message: String,
    },
}

#[derive(Debug)]
pub enum OAuthControl {
    PromptResponse {
        prompt_id: String,
        value: Option<String>,
        cancelled: bool,
    },
    Cancel,
}

struct OAuthSession {
    sender: mpsc::Sender<OAuthControl>,
    prompt_id: Option<String>,
}

type OAuthPublisher = dyn Fn(&OAuthPublicEvent) -> Result<(), String> + Send + Sync;

#[derive(Default)]
pub struct PendingOAuthSessions {
    entries: Mutex<HashMap<String, OAuthSession>>,
    publisher: Mutex<Option<Arc<OAuthPublisher>>>,
}

impl PendingOAuthSessions {
    pub fn set_publisher(&self, publisher: Arc<OAuthPublisher>) -> Result<(), String> {
        let mut slot = self
            .publisher
            .lock()
            .map_err(|_| "OAuth publisher lock poisoned".to_string())?;
        if slot.is_some() {
            return Err("OAuth event publisher is already configured".into());
        }
        *slot = Some(publisher);
        Ok(())
    }

    pub fn register(
        &self,
        session_id: String,
        _connector_id: String,
        sender: mpsc::Sender<OAuthControl>,
    ) -> Result<(), String> {
        if self
            .publisher
            .lock()
            .map_err(|_| "OAuth publisher lock poisoned".to_string())?
            .is_none()
        {
            return Err("OAuth events are unavailable for this host transport".into());
        }
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "OAuth sessions lock poisoned".to_string())?;
        if entries.contains_key(&session_id) {
            return Err("OAuth session id collision".into());
        }
        entries.insert(
            session_id,
            OAuthSession {
                sender,
                prompt_id: None,
            },
        );
        Ok(())
    }

    pub fn publish(&self, event: OAuthPublicEvent) -> Result<(), String> {
        let publisher = self
            .publisher
            .lock()
            .map_err(|_| "OAuth publisher lock poisoned".to_string())?
            .clone()
            .ok_or_else(|| "OAuth events are unavailable for this host transport".to_string())?;
        publisher(&event)
    }

    pub fn set_prompt(&self, session_id: &str, prompt_id: String) -> Result<(), String> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "OAuth sessions lock poisoned".to_string())?;
        let session = entries
            .get_mut(session_id)
            .ok_or_else(|| format!("no active OAuth session '{session_id}'"))?;
        session.prompt_id = Some(prompt_id);
        Ok(())
    }

    pub fn resolve_prompt(
        &self,
        session_id: &str,
        prompt_id: String,
        value: Option<String>,
        cancelled: bool,
    ) -> Result<(), String> {
        if cancelled == value.is_some() {
            return Err("provide either a prompt value or cancelled=true".into());
        }
        if value.as_ref().is_some_and(|value| value.len() > 16_384) {
            return Err("OAuth prompt response is too long".into());
        }
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "OAuth sessions lock poisoned".to_string())?;
        let session = entries
            .get_mut(session_id)
            .ok_or_else(|| format!("no active OAuth session '{session_id}'"))?;
        if session.prompt_id.as_deref() != Some(&prompt_id) {
            return Err(format!(
                "OAuth prompt '{prompt_id}' does not match session '{session_id}'"
            ));
        }
        session.prompt_id = None;
        session
            .sender
            .send(OAuthControl::PromptResponse {
                prompt_id,
                value,
                cancelled,
            })
            .map_err(|_| "OAuth session has already ended".to_string())
    }

    pub fn cancel(&self, session_id: &str) -> Result<(), String> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| "OAuth sessions lock poisoned".to_string())?;
        let session = entries
            .get(session_id)
            .ok_or_else(|| format!("no active OAuth session '{session_id}'"))?;
        session
            .sender
            .send(OAuthControl::Cancel)
            .map_err(|_| "OAuth session has already ended".to_string())
    }

    pub fn finish(&self, session_id: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(session_id);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedNoticeRecord {
    pub sequence: u64,
    pub recorded_at: DateTime<Utc>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub notice: ChromeNotice,
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

/// Decisions the frontend still owes us, keyed by request id.
#[derive(Default)]
pub struct PendingApprovals {
    entries: Mutex<HashMap<u64, PendingApproval>>,
}

struct PendingApproval {
    reply: PendingReply,
    request: ChromeRequest,
    publish_removed: Option<Box<dyn FnOnce() + Send>>,
}

/// A pending chrome request answers on exactly one of these channels: a single
/// yes/no prompt, or a whole-app install checklist.
enum PendingReply {
    Single(mpsc::Sender<bool>),
    Install(mpsc::Sender<InstallApprovalDecision>),
}

fn decision(approved: bool) -> ApprovalDecision {
    if approved {
        ApprovalDecision::Approved
    } else {
        ApprovalDecision::Denied
    }
}

impl PendingApprovals {
    pub fn resolve(&self, request_id: u64, approved: bool) -> Result<(), String> {
        let mut entries = self
            .entries
            .lock()
            .expect("pending approvals lock poisoned");
        // Verify the request is a single yes/no prompt *before* removing it: a
        // mismatched call (e.g. an install checklist answered on this path) must
        // be a harmless error, never a destructive drop of the pending sender.
        match entries.get(&request_id) {
            None => return Err(format!("no pending chrome request '{request_id}'")),
            Some(entry) if matches!(entry.reply, PendingReply::Install(_)) => {
                return Err(format!(
                    "chrome request '{request_id}' is an install checklist"
                ))
            }
            Some(_) => {}
        }
        let PendingReply::Single(sender) = entries
            .remove(&request_id)
            .expect("entry present under the same lock")
            .reply
        else {
            unreachable!("variant confirmed Single above under the same lock");
        };
        // A dropped receiver means the kernel side already timed out (and
        // denied); the late answer is irrelevant.
        let _ = sender.send(approved);
        Ok(())
    }

    /// Answer a batched install request. `grant_approvals` is positionally
    /// aligned with the prompt's grants; `event_approved` mirrors its optional
    /// event subscription.
    pub fn resolve_install(
        &self,
        request_id: u64,
        event_approved: Option<bool>,
        grant_approvals: Vec<bool>,
    ) -> Result<(), String> {
        let mut entries = self
            .entries
            .lock()
            .expect("pending approvals lock poisoned");
        // Same discipline as `resolve`: confirm the variant before removing so a
        // wrong-channel answer cannot silently discard the pending request.
        match entries.get(&request_id) {
            None => return Err(format!("no pending chrome request '{request_id}'")),
            Some(entry) if matches!(entry.reply, PendingReply::Single(_)) => {
                return Err(format!(
                    "chrome request '{request_id}' is not an install checklist"
                ))
            }
            Some(_) => {}
        }
        let PendingReply::Install(sender) = entries
            .remove(&request_id)
            .expect("entry present under the same lock")
            .reply
        else {
            unreachable!("variant confirmed Install above under the same lock");
        };
        let decision = InstallApprovalDecision {
            event_decision: event_approved.map(decision),
            grant_decisions: grant_approvals.into_iter().map(decision).collect(),
        };
        let _ = sender.send(decision);
        Ok(())
    }

    pub(crate) fn wait_for_decision(
        &self,
        request_id: u64,
        request: ChromeRequest,
        publish: impl FnOnce() -> bool,
        publish_expired: impl FnOnce() + Send + 'static,
    ) -> ApprovalDecision {
        let (sender, receiver) = mpsc::channel();
        self.entries
            .lock()
            .expect("pending approvals lock poisoned")
            .insert(
                request_id,
                PendingApproval {
                    reply: PendingReply::Single(sender),
                    request,
                    publish_removed: Some(Box::new(publish_expired)),
                },
            );
        if !publish() {
            self.remove(request_id);
            return ApprovalDecision::Denied;
        }
        match receiver.recv_timeout(APPROVAL_TIMEOUT) {
            Ok(approved) => decision(approved),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.expire(request_id);
                ApprovalDecision::Denied
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.remove(request_id);
                ApprovalDecision::Denied
            }
        }
    }

    pub(crate) fn wait_for_install_decision(
        &self,
        request_id: u64,
        request: ChromeRequest,
        denied: InstallApprovalDecision,
        publish: impl FnOnce() -> bool,
        publish_expired: impl FnOnce() + Send + 'static,
    ) -> InstallApprovalDecision {
        let (sender, receiver) = mpsc::channel();
        self.entries
            .lock()
            .expect("pending approvals lock poisoned")
            .insert(
                request_id,
                PendingApproval {
                    reply: PendingReply::Install(sender),
                    request,
                    publish_removed: Some(Box::new(publish_expired)),
                },
            );
        if !publish() {
            self.remove(request_id);
            return denied;
        }
        match receiver.recv_timeout(APPROVAL_TIMEOUT) {
            Ok(decision) => decision,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.expire(request_id);
                denied
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.remove(request_id);
                denied
            }
        }
    }

    fn remove(&self, request_id: u64) {
        self.entries
            .lock()
            .expect("pending approvals lock poisoned")
            .remove(&request_id);
    }

    fn expire(&self, request_id: u64) {
        let pending = self
            .entries
            .lock()
            .expect("pending approvals lock poisoned")
            .remove(&request_id);
        if let Some(mut pending) = pending {
            if let Some(publish_removed) = pending.publish_removed.take() {
                publish_removed();
            }
        }
    }

    pub(crate) fn deny_app_id_prefix(&self, prefix: &str) {
        let pending = {
            let mut entries = self
                .entries
                .lock()
                .expect("pending approvals lock poisoned");
            let request_ids = entries
                .iter()
                .filter(|(_, pending)| pending.request.app_id().as_str().starts_with(prefix))
                .map(|(request_id, _)| *request_id)
                .collect::<Vec<_>>();
            request_ids
                .into_iter()
                .filter_map(|request_id| entries.remove(&request_id))
                .collect::<Vec<_>>()
        };
        for mut pending in pending {
            match (pending.reply, &pending.request) {
                (PendingReply::Single(sender), _) => {
                    let _ = sender.send(false);
                }
                (PendingReply::Install(sender), ChromeRequest::InstallApproval { prompt, .. }) => {
                    let _ = sender.send(InstallApprovalDecision {
                        event_decision: prompt.event.as_ref().map(|_| ApprovalDecision::Denied),
                        grant_decisions: vec![ApprovalDecision::Denied; prompt.grants.len()],
                    });
                }
                (PendingReply::Install(_), _) => {
                    unreachable!("install reply is paired with an install request")
                }
            }
            if let Some(publish_removed) = pending.publish_removed.take() {
                publish_removed();
            }
        }
    }

    pub(crate) fn pending_requests(&self) -> Vec<ChromeRequest> {
        self.entries
            .lock()
            .expect("pending approvals lock poisoned")
            .values()
            .map(|entry| entry.request.clone())
            .collect()
    }
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum ChromeRequest {
    GrantIssuance {
        request_id: u64,
        prompt: GrantIssuancePrompt,
    },
    CapabilityApproval {
        request_id: u64,
        prompt: CapabilityApprovalPrompt,
    },
    EventSubscription {
        request_id: u64,
        prompt: EventSubscriptionPrompt,
    },
    /// One app's full install request, answered as a single checklist.
    InstallApproval {
        request_id: u64,
        prompt: InstallApprovalPrompt,
    },
}

impl ChromeRequest {
    fn app_id(&self) -> &AppId {
        match self {
            Self::GrantIssuance { prompt, .. } => &prompt.app_id,
            Self::CapabilityApproval { prompt, .. } => &prompt.app_id,
            Self::EventSubscription { prompt, .. } => &prompt.app_id,
            Self::InstallApproval { prompt, .. } => &prompt.app_id,
        }
    }
}

/// How many capability approvals one app may have waiting at once.
///
/// Each `requires-approval` call raises its own modal, so an app calling in a
/// loop can bury the user in identical prompts. That is not just noise: a user
/// facing an unbroken stream of dialogs starts clicking the affirmative button
/// to clear them, which is precisely how deny-by-default stops meaning
/// anything. Past this many outstanding prompts the app's further calls are
/// refused outright, and the user is told once.
const MAX_PENDING_APPROVALS_PER_APP: usize = 3;

/// Per-app counter of capability approvals currently on screen or queued.
#[derive(Default)]
pub(crate) struct ApprovalSlots {
    outstanding: Mutex<HashMap<AppId, usize>>,
}

impl ApprovalSlots {
    /// Claim one of an app's slots, or refuse if it has none left.
    pub(crate) fn claim(&self, app_id: &AppId) -> bool {
        let mut outstanding = self
            .outstanding
            .lock()
            .expect("outstanding approvals lock poisoned");
        let count = outstanding.entry(app_id.clone()).or_insert(0);
        if *count >= MAX_PENDING_APPROVALS_PER_APP {
            return false;
        }
        *count += 1;
        true
    }

    pub(crate) fn release(&self, app_id: &AppId) {
        let mut outstanding = self
            .outstanding
            .lock()
            .expect("outstanding approvals lock poisoned");
        if let Some(count) = outstanding.get_mut(app_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                outstanding.remove(app_id);
            }
        }
    }
}

pub struct ShellChrome {
    app: AppHandle,
    pending: Arc<PendingApprovals>,
    notices: Arc<Mutex<TrustedNoticeStore>>,
    next_request_id: AtomicU64,
    approval_slots: ApprovalSlots,
}

impl ShellChrome {
    pub fn new(
        app: AppHandle,
        pending: Arc<PendingApprovals>,
        notices: Arc<Mutex<TrustedNoticeStore>>,
    ) -> Self {
        Self {
            app,
            pending,
            notices,
            next_request_id: AtomicU64::new(0),
            approval_slots: ApprovalSlots::default(),
        }
    }

    fn ask(&self, request: ChromeRequest, request_id: u64) -> ApprovalDecision {
        let app = self.app.clone();
        self.pending.wait_for_decision(
            request_id,
            request.clone(),
            || self.app.emit(CHROME_REQUEST_EVENT, &request).is_ok(),
            move || {
                let _ = app.emit(CHROME_REQUEST_EXPIRED_EVENT, request_id);
            },
        )
    }
}

impl TrustedChrome for ShellChrome {
    fn confirm_grant(&self, prompt: GrantIssuancePrompt) -> ApprovalDecision {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        self.ask(
            ChromeRequest::GrantIssuance { request_id, prompt },
            request_id,
        )
    }

    fn approve_capability(&self, prompt: CapabilityApprovalPrompt) -> ApprovalDecision {
        let app_id = prompt.app_id.clone();
        if !self.approval_slots.claim(&app_id) {
            // Denying is the safe direction and matches the port's contract
            // (an unanswered prompt is already a denial). The refusal is not
            // silent to the user: the kernel records `InvocationRefused`
            // against the run, which the ledger and the caller both surface.
            return ApprovalDecision::Denied;
        }
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let decision = self.ask(
            ChromeRequest::CapabilityApproval { request_id, prompt },
            request_id,
        );
        self.approval_slots.release(&app_id);
        decision
    }

    fn show_notice(&self, notice: ChromeNotice) -> Result<(), ChromeNoticeError> {
        // Persist first, then publish the live toast. The kernel must see any
        // notice-store failure instead of continuing as though the notice were
        // durably recorded.
        let record = self
            .notices
            .lock()
            .expect("trusted notice store lock poisoned")
            .record(notice)
            .map_err(|error| ChromeNoticeError::Persistence {
                message: error.to_string(),
            })?;
        self.app
            .emit(CHROME_NOTICE_EVENT, &record)
            .map_err(|error| ChromeNoticeError::Delivery {
                message: error.to_string(),
            })
    }

    fn confirm_event_subscriptions(&self, prompt: EventSubscriptionPrompt) -> ApprovalDecision {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        self.ask(
            ChromeRequest::EventSubscription { request_id, prompt },
            request_id,
        )
    }

    fn confirm_install(&self, prompt: InstallApprovalPrompt) -> InstallApprovalDecision {
        // Nothing to decide (no grants, no event feed) — approve without a modal.
        if prompt.event.is_none() && prompt.grants.is_empty() {
            return InstallApprovalDecision {
                event_decision: None,
                grant_decisions: Vec::new(),
            };
        }
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let denied = InstallApprovalDecision {
            event_decision: prompt.event.as_ref().map(|_| ApprovalDecision::Denied),
            grant_decisions: vec![ApprovalDecision::Denied; prompt.grants.len()],
        };
        let request = ChromeRequest::InstallApproval { request_id, prompt };
        let app = self.app.clone();
        self.pending.wait_for_install_decision(
            request_id,
            request.clone(),
            denied,
            || self.app.emit(CHROME_REQUEST_EVENT, &request).is_ok(),
            move || {
                let _ = app.emit(CHROME_REQUEST_EXPIRED_EVENT, request_id);
            },
        )
    }
}

#[cfg(test)]
mod tests;
