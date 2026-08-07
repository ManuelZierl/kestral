use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::atomic_json::{
    load_json_document, persist_json_document, standard_writer, AtomicFileWriter,
};

const CHAT_STORE_VERSION: u32 = 4;
pub(crate) const MAX_INJECTED_CONTEXTS_PER_SOURCE: usize = 100;
pub(crate) const MAX_INJECTED_CONTEXTS_PER_THREAD: usize = 200;
pub(crate) const MAX_INJECTED_CONTEXT_CHARS: usize = 16 * 1024;
pub(crate) const MAX_INJECTED_CONTEXT_CHARS_PER_SOURCE: usize = 48 * 1024;
pub(crate) const MAX_INJECTED_CONTEXT_CHARS_PER_THREAD: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ChatMessageRole {
    User,
    Assistant,
    System,
    ToolStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum ChatContributionKind {
    TextSnapshot,
    ArtifactRef,
    ResourceRef,
    DraftProposal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum ChatContributionCompleteness {
    Complete,
    Truncated,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum ChatContributionLifecycle {
    Draft,
    Accepted,
    Removed,
    Stale,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatContribution {
    pub source_app_id: String,
    pub source_app_version: String,
    pub source_contract: u32,
    pub item_id: String,
    pub revision: u64,
    pub digest: String,
    pub completeness: ChatContributionCompleteness,
    pub lifecycle: ChatContributionLifecycle,
    pub kind: ChatContributionKind,
    pub title: String,
    pub body: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatInjectedContext {
    pub source_app_id: String,
    pub source_app_version: String,
    pub source_app_content_hash: String,
    pub source_run_id: String,
    pub item_id: String,
    pub revision: u64,
    pub content_digest: String,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatInjectedContextTombstone {
    pub source_app_id: String,
    pub item_id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatInjectedContextEntryReceipt {
    pub source_app_id: String,
    pub source_app_name: String,
    pub source_app_version: String,
    pub item_id: String,
    pub revision: u64,
    pub source_run_id: String,
    pub grant_id: String,
    pub content_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatInjectedContextReceipt {
    pub message_digest: String,
    pub entries: Vec<ChatInjectedContextEntryReceipt>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub exact_message: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct AuthorizedChatInjectedContext {
    pub context: ChatInjectedContext,
    pub source_app_name: String,
    pub grant_id: String,
}

#[derive(Debug, Clone)]
pub(crate) enum ChatInjectedContextUpdate {
    Upsert(ChatInjectedContext),
    Remove {
        source_app_id: String,
        item_id: String,
        revision: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ChatMessageStatus {
    Pending,
    Completed,
    Interrupted,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    #[serde(rename = "id")]
    pub message_id: String,
    pub role: ChatMessageRole,
    pub text: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub reasoning: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub run_id: Option<String>,
    pub artifact_ids: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub status: Option<ChatMessageStatus>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub client_request_id: Option<String>,
    pub created_at: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatThreadRef {
    pub resource_id: String,
    pub thread_id: String,
    pub title: String,
    pub revision: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum PublicChatMessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum ChatMessageViewStatus {
    Pending,
    Completed,
    Interrupted,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatMessageView {
    pub message_id: String,
    pub thread_resource_id: String,
    pub sequence: u64,
    pub role: PublicChatMessageRole,
    pub status: ChatMessageViewStatus,
    pub text: String,
    pub artifact_refs: Vec<String>,
    pub run_ref: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatThreadPage {
    pub thread: ChatThreadRef,
    pub messages: Vec<ChatMessageView>,
    pub next_cursor: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatThreadSummary {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatPromptReceiptLayer {
    pub id: String,
    pub kind: String,
    pub title: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub source: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChatCompositionReceipt {
    pub system_prompt_digest: String,
    pub assistant_profile_ref: String,
    pub assistant_profile_digest: String,
    pub enabled_skill_digests: Vec<String>,
    pub context_block_digests: Vec<String>,
    pub attachment_refs: Vec<String>,
    pub available_capability_refs: Vec<String>,
    pub provider_profile_ref: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub model_profile: Option<crate::chat_model_profiles::ChatModelProfileReceipt>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub agent_engine_ref: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub agent_engine_version: Option<String>,
    pub agent_engine_features: Vec<String>,
    pub assistant_capability_refs: Vec<String>,
    pub created_at: String,
    pub system_prompt: String,
    pub layers: Vec<ChatPromptReceiptLayer>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub injected_context: Option<ChatInjectedContextReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatAgentEngineView {
    pub app_id: String,
    pub display_name: String,
    pub version: String,
    pub contract: String,
    pub features: Vec<String>,
    pub available: bool,
    pub availability_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatProfileReceipt {
    pub app_id: String,
    pub profile_name: String,
    pub version: String,
    pub digest: String,
    pub reviewed_skill_digests: Vec<String>,
    pub capability_refs: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub engine_contract: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatAgentEngineReceipt {
    pub app_id: String,
    pub version: String,
    pub contract: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatAgentEngineState {
    pub status: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatProfileView {
    #[serde(flatten)]
    pub receipt: ChatProfileReceipt,
    pub app_display_name: String,
    pub title: String,
    pub description: String,
    pub suggested_capability_refs: Vec<String>,
    pub suggested_agent_engine_contract: Option<String>,
    pub availability: String,
    pub availability_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatThread {
    pub id: String,
    pub resource_id: String,
    pub revision: u64,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    messages: Vec<ChatMessage>,
    pub prompt_receipts: BTreeMap<String, ChatCompositionReceipt>,
    pub contributions: Vec<ChatContribution>,
    pub injected_contexts: Vec<ChatInjectedContext>,
    pub injected_context_tombstones: Vec<ChatInjectedContextTombstone>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub assistant_profile_ref: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub assistant_profile_receipt: Option<ChatProfileReceipt>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub model_profile_ref: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub model_profile_receipt: Option<crate::chat_model_profiles::ChatModelProfileReceipt>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub chat_agent_engine_ref: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub chat_agent_engine_receipt: Option<ChatAgentEngineReceipt>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub chat_agent_engine_state: Option<ChatAgentEngineState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributionIdentity {
    pub source_app_id: String,
    pub kind: ChatContributionKind,
    pub item_id: String,
}

impl ContributionIdentity {
    fn matches(&self, contribution: &ChatContribution) -> bool {
        self.source_app_id == contribution.source_app_id
            && self.kind == contribution.kind
            && self.item_id == contribution.item_id
    }
}

impl From<&ChatContribution> for ContributionIdentity {
    fn from(contribution: &ChatContribution) -> Self {
        Self {
            source_app_id: contribution.source_app_id.clone(),
            kind: contribution.kind.clone(),
            item_id: contribution.item_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatStoreDocument {
    version: u32,
    threads: Vec<ChatThread>,
}

fn validate_loaded_injected_contexts(thread: &ChatThread) -> Result<(), String> {
    if thread.injected_contexts.len() > MAX_INJECTED_CONTEXTS_PER_THREAD {
        return Err("stored injected Chat context limit exceeded".into());
    }
    if thread.injected_context_tombstones.len() > MAX_INJECTED_CONTEXTS_PER_THREAD {
        return Err("stored injected Chat context tombstone limit exceeded".into());
    }

    let mut identities = BTreeSet::new();
    let mut source_counts = BTreeMap::<&str, usize>::new();
    let mut source_chars = BTreeMap::<&str, usize>::new();
    let mut total_chars = 0usize;
    for context in &thread.injected_contexts {
        if context.source_app_id.is_empty()
            || context.source_app_version.is_empty()
            || context.source_app_content_hash.is_empty()
            || context.source_run_id.is_empty()
            || context.item_id.is_empty()
        {
            return Err("stored injected Chat context identity is incomplete".into());
        }
        if context.item_id.chars().count() > 256 {
            return Err("stored injected Chat context item id is too large".into());
        }
        if !identities.insert((context.source_app_id.clone(), context.item_id.clone())) {
            return Err(format!(
                "duplicate stored injected Chat context: {}/{}",
                context.source_app_id, context.item_id
            ));
        }
        let chars = context.content.chars().count();
        if chars == 0 || chars > MAX_INJECTED_CONTEXT_CHARS {
            return Err("stored injected Chat context content is empty or too large".into());
        }
        let digest = format!("{:x}", Sha256::digest(context.content.as_bytes()));
        if context.content_digest != digest {
            return Err(format!(
                "stored injected Chat context digest mismatch for {}/{}",
                context.source_app_id, context.item_id
            ));
        }
        if chrono::DateTime::parse_from_rfc3339(&context.created_at).is_err()
            || chrono::DateTime::parse_from_rfc3339(&context.updated_at).is_err()
        {
            return Err("stored injected Chat context timestamp is invalid".into());
        }
        let count = source_counts
            .entry(context.source_app_id.as_str())
            .or_default();
        *count += 1;
        if *count > MAX_INJECTED_CONTEXTS_PER_SOURCE {
            return Err("stored injected Chat context source limit exceeded".into());
        }
        let source_total = source_chars
            .entry(context.source_app_id.as_str())
            .or_default();
        *source_total = source_total.saturating_add(chars);
        if *source_total > MAX_INJECTED_CONTEXT_CHARS_PER_SOURCE {
            return Err("stored injected Chat context source size limit exceeded".into());
        }
        total_chars = total_chars.saturating_add(chars);
        if total_chars > MAX_INJECTED_CONTEXT_CHARS_PER_THREAD {
            return Err("stored injected Chat context total size limit exceeded".into());
        }
    }

    let mut tombstone_identities = BTreeSet::new();
    for tombstone in &thread.injected_context_tombstones {
        if tombstone.source_app_id.is_empty() || tombstone.item_id.is_empty() {
            return Err("stored injected Chat context tombstone identity is incomplete".into());
        }
        let identity = (tombstone.source_app_id.clone(), tombstone.item_id.clone());
        if identities.contains(&identity) {
            return Err(format!(
                "stored injected Chat context has a live entry and tombstone: {}/{}",
                tombstone.source_app_id, tombstone.item_id
            ));
        }
        if !tombstone_identities.insert(identity) {
            return Err(format!(
                "duplicate stored injected Chat context tombstone: {}/{}",
                tombstone.source_app_id, tombstone.item_id
            ));
        }
    }
    Ok(())
}

pub struct ChatStore {
    path: PathBuf,
    document: ChatStoreDocument,
    writer: Arc<dyn AtomicFileWriter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatRequestState {
    Pending,
    Completed,
}

impl ChatStore {
    pub(crate) fn validate_persisted(path: &Path) -> Result<(), String> {
        let Some(document) = load_json_document::<ChatStoreDocument>(path, "chat storage")? else {
            return Ok(());
        };
        Self::validate_document(&document)
    }

    fn validate_document(document: &ChatStoreDocument) -> Result<(), String> {
        if document.version != CHAT_STORE_VERSION {
            return Err(format!(
                "unsupported chat storage version: {}",
                document.version
            ));
        }
        let mut thread_ids = std::collections::BTreeSet::new();
        let mut resource_ids = std::collections::BTreeSet::new();
        for thread in &document.threads {
            if thread.id.is_empty() {
                return Err("chat thread id cannot be empty".into());
            }
            if !thread_ids.insert(thread.id.as_str()) {
                return Err(format!("duplicate chat thread id: {}", thread.id));
            }
            if thread.resource_id.is_empty() {
                return Err("chat thread resource id cannot be empty".into());
            }
            if !resource_ids.insert(thread.resource_id.as_str()) {
                return Err(format!(
                    "duplicate chat thread resource id: {}",
                    thread.resource_id
                ));
            }
            validate_loaded_injected_contexts(thread)?;
        }
        Ok(())
    }

    pub fn new(path: PathBuf) -> Result<Self, String> {
        Self::with_writer(path, standard_writer())
    }

    fn with_writer(path: PathBuf, writer: Arc<dyn AtomicFileWriter>) -> Result<Self, String> {
        let mut document =
            load_json_document(&path, "chat storage")?.unwrap_or(ChatStoreDocument {
                version: CHAT_STORE_VERSION,
                threads: vec![],
            });
        Self::validate_document(&document)?;
        let loaded_thread_count = document.threads.len();
        document.threads.retain(ChatThread::should_persist);
        if document.threads.len() != loaded_thread_count {
            persist_json_document(&path, &document, "chat storage", writer.as_ref())
                .map_err(|error| error.into_message())?;
        }
        Ok(Self {
            path,
            document,
            writer,
        })
    }

    pub fn list_thread_refs(&self) -> Vec<ChatThreadRef> {
        let mut threads: Vec<ChatThreadRef> = self
            .document
            .threads
            .iter()
            .map(ChatThreadRef::from)
            .collect();
        threads.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.resource_id.cmp(&right.resource_id))
        });
        threads
    }

    pub fn list_threads(&self) -> Vec<ChatThreadSummary> {
        self.list_thread_refs()
            .into_iter()
            .map(|thread| {
                let message_count = self
                    .find_thread_by_resource_id(&thread.resource_id)
                    .map(|stored| stored.messages.len())
                    .unwrap_or_default();
                ChatThreadSummary {
                    id: thread.thread_id,
                    title: thread.title,
                    created_at: thread.created_at,
                    updated_at: thread.updated_at,
                    message_count,
                }
            })
            .collect()
    }

    pub fn find_thread_by_resource_id(&self, resource_id: &str) -> Option<&ChatThread> {
        self.document
            .threads
            .iter()
            .find(|thread| thread.resource_id == resource_id)
    }

    pub fn get_thread(&self, thread_id: &str) -> Result<ChatThread, String> {
        self.document
            .threads
            .iter()
            .find(|thread| thread.id == thread_id)
            .cloned()
            .ok_or_else(|| format!("unknown chat thread: {thread_id}"))
    }

    pub fn get_thread_page(
        &self,
        resource_id: &str,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<ChatThreadPage, String> {
        if !(1..=100).contains(&limit) {
            return Err("chat page limit must be between 1 and 100".into());
        }
        let thread = self
            .find_thread_by_resource_id(resource_id)
            .ok_or_else(|| format!("unknown chat thread resource: {resource_id}"))?;
        let messages = thread.public_messages(cursor, limit + 1);
        let next_cursor = if messages.len() > limit {
            messages.get(limit - 1).map(|message| message.sequence)
        } else {
            None
        };
        Ok(ChatThreadPage {
            thread: ChatThreadRef::from(thread),
            messages: messages.into_iter().take(limit).collect(),
            next_cursor,
        })
    }

    pub fn create_thread(&mut self) -> Result<ChatThread, String> {
        self.create_thread_with_agent_engine(None, None)
    }

    pub fn create_thread_with_agent_engine(
        &mut self,
        engine_ref: Option<String>,
        engine_receipt: Option<ChatAgentEngineReceipt>,
    ) -> Result<ChatThread, String> {
        let now = now_iso();
        let thread = ChatThread {
            id: new_id("thread"),
            resource_id: new_id("chat-thread"),
            revision: 0,
            title: "New chat".into(),
            created_at: now.clone(),
            updated_at: now,
            messages: vec![],
            prompt_receipts: BTreeMap::new(),
            contributions: vec![],
            injected_contexts: vec![],
            injected_context_tombstones: vec![],
            assistant_profile_ref: Some("chat/standard".into()),
            assistant_profile_receipt: None,
            model_profile_ref: None,
            model_profile_receipt: None,
            chat_agent_engine_ref: engine_ref,
            chat_agent_engine_receipt: engine_receipt,
            chat_agent_engine_state: None,
        };
        self.document.threads.push(thread.clone());
        Ok(thread)
    }

    pub fn rename_thread(&mut self, thread_id: &str, title: &str) -> Result<ChatThread, String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("chat title must not be empty".into());
        }
        let mut candidate = self.document.clone();
        let renamed = {
            let thread = candidate
                .threads
                .iter_mut()
                .find(|thread| thread.id == thread_id)
                .ok_or_else(|| format!("unknown chat thread: {thread_id}"))?;
            thread.title = title.to_string();
            thread.updated_at = now_iso();
            thread.revision += 1;
            thread.clone()
        };
        self.commit(candidate)?;
        Ok(renamed)
    }

    pub fn delete_thread(&mut self, thread_id: &str) -> Result<(), String> {
        let mut candidate = self.document.clone();
        let previous_len = candidate.threads.len();
        candidate.threads.retain(|thread| thread.id != thread_id);
        if candidate.threads.len() == previous_len {
            return Err(format!("unknown chat thread: {thread_id}"));
        }
        self.commit(candidate)
    }

    pub fn set_assistant_profile(
        &mut self,
        thread_id: &str,
        profile_ref: Option<String>,
        receipt: Option<ChatProfileReceipt>,
    ) -> Result<ChatThread, String> {
        let mut candidate = self.document.clone();
        let snapshot = {
            let thread = candidate
                .threads
                .iter_mut()
                .find(|thread| thread.id == thread_id)
                .ok_or_else(|| format!("unknown chat thread: {thread_id}"))?;
            thread.assistant_profile_ref = profile_ref;
            thread.assistant_profile_receipt = receipt;
            thread.updated_at = now_iso();
            thread.revision += 1;
            thread.clone()
        };
        self.commit(candidate)?;
        Ok(snapshot)
    }

    pub fn set_chat_agent_engine(
        &mut self,
        thread_id: &str,
        engine_ref: Option<String>,
        receipt: Option<ChatAgentEngineReceipt>,
        state: Option<ChatAgentEngineState>,
    ) -> Result<ChatThread, String> {
        let mut candidate = self.document.clone();
        let snapshot = {
            let thread = candidate
                .threads
                .iter_mut()
                .find(|thread| thread.id == thread_id)
                .ok_or_else(|| format!("unknown chat thread: {thread_id}"))?;
            thread.chat_agent_engine_ref = engine_ref;
            thread.chat_agent_engine_receipt = receipt;
            thread.chat_agent_engine_state = state;
            thread.updated_at = now_iso();
            thread.revision += 1;
            thread.clone()
        };
        self.commit(candidate)?;
        Ok(snapshot)
    }

    pub fn set_model_profile(
        &mut self,
        thread_id: &str,
        profile_ref: Option<String>,
        receipt: Option<crate::chat_model_profiles::ChatModelProfileReceipt>,
    ) -> Result<ChatThread, String> {
        let mut candidate = self.document.clone();
        let snapshot = {
            let thread = candidate
                .threads
                .iter_mut()
                .find(|thread| thread.id == thread_id)
                .ok_or_else(|| format!("unknown chat thread: {thread_id}"))?;
            thread.model_profile_ref = profile_ref;
            thread.model_profile_receipt = receipt;
            thread.updated_at = now_iso();
            thread.revision += 1;
            thread.clone()
        };
        self.commit(candidate)?;
        Ok(snapshot)
    }

    pub fn set_chat_agent_engine_state(
        &mut self,
        thread_id: &str,
        state: Option<ChatAgentEngineState>,
    ) -> Result<ChatThread, String> {
        let current = self
            .document
            .threads
            .iter()
            .find(|thread| thread.id == thread_id)
            .ok_or_else(|| format!("unknown chat thread: {thread_id}"))?;
        if current.chat_agent_engine_state == state {
            return Ok(current.clone());
        }

        let mut candidate = self.document.clone();
        let snapshot = {
            let thread = candidate
                .threads
                .iter_mut()
                .find(|thread| thread.id == thread_id)
                .expect("thread existence checked above");
            thread.chat_agent_engine_state = state;
            thread.updated_at = now_iso();
            thread.revision += 1;
            thread.clone()
        };
        self.commit(candidate)?;
        Ok(snapshot)
    }

    pub fn upsert_contribution(
        &mut self,
        thread_id: &str,
        contribution: ChatContribution,
    ) -> Result<ChatThread, String> {
        self.upsert_contributions(thread_id, vec![contribution], 128, 128)
    }

    pub fn upsert_contributions(
        &mut self,
        thread_id: &str,
        contributions: Vec<ChatContribution>,
        max_per_source: usize,
        max_total: usize,
    ) -> Result<ChatThread, String> {
        let mut candidate = self.document.clone();
        let snapshot = {
            let thread = candidate
                .threads
                .iter_mut()
                .find(|thread| thread.id == thread_id)
                .ok_or_else(|| format!("unknown chat thread: {thread_id}"))?;
            for contribution in contributions {
                if let Some(existing) = thread.contributions.iter_mut().find(|item| {
                    item.item_id == contribution.item_id
                        && item.source_app_id == contribution.source_app_id
                        && item.kind == contribution.kind
                }) {
                    *existing = contribution;
                } else {
                    thread.contributions.push(contribution);
                }
            }
            if thread.contributions.len() > max_total {
                return Err("chat contributions limit reached".into());
            }
            let mut source_counts = BTreeMap::<&str, usize>::new();
            for contribution in &thread.contributions {
                let count = source_counts
                    .entry(contribution.source_app_id.as_str())
                    .or_default();
                *count += 1;
                if *count > max_per_source {
                    return Err("chat contributor limit reached".into());
                }
            }
            thread.contributions.sort_by(|left, right| {
                left.source_app_id
                    .cmp(&right.source_app_id)
                    .then(left.kind.cmp(&right.kind))
                    .then(left.item_id.cmp(&right.item_id))
            });
            thread.updated_at = now_iso();
            thread.revision += 1;
            thread.clone()
        };
        self.commit(candidate)?;
        Ok(snapshot)
    }

    pub fn remove_contribution(
        &mut self,
        thread_id: &str,
        identity: &ContributionIdentity,
    ) -> Result<ChatThread, String> {
        let mut candidate = self.document.clone();
        let snapshot = {
            let thread = candidate
                .threads
                .iter_mut()
                .find(|thread| thread.id == thread_id)
                .ok_or_else(|| format!("unknown chat thread: {thread_id}"))?;
            let before = thread.contributions.len();
            thread.contributions.retain(|item| !identity.matches(item));
            if thread.contributions.len() == before {
                return Err(format!(
                    "unknown chat contribution: {}/{:?}/{}",
                    identity.source_app_id, identity.kind, identity.item_id
                ));
            }
            thread.updated_at = now_iso();
            thread.revision += 1;
            thread.clone()
        };
        self.commit(candidate)?;
        Ok(snapshot)
    }

    pub fn remove_contributions(
        &mut self,
        thread_id: &str,
        identities: &[ContributionIdentity],
    ) -> Result<ChatThread, String> {
        let mut candidate = self.document.clone();
        let snapshot = {
            let thread = candidate
                .threads
                .iter_mut()
                .find(|thread| thread.id == thread_id)
                .ok_or_else(|| format!("unknown chat thread: {thread_id}"))?;
            thread
                .contributions
                .retain(|item| !identities.iter().any(|identity| identity.matches(item)));
            thread.updated_at = now_iso();
            thread.revision += 1;
            thread.clone()
        };
        self.commit(candidate)?;
        Ok(snapshot)
    }

    pub(crate) fn apply_injected_context_updates(
        &mut self,
        thread_id: &str,
        updates: Vec<ChatInjectedContextUpdate>,
        max_per_source: usize,
        max_total: usize,
        max_chars_per_source: usize,
        max_total_chars: usize,
    ) -> Result<ChatThread, String> {
        let mut candidate = self.document.clone();
        let snapshot = {
            let thread = candidate
                .threads
                .iter_mut()
                .find(|thread| thread.id == thread_id)
                .ok_or_else(|| format!("unknown chat thread: {thread_id}"))?;
            for update in updates {
                match update {
                    ChatInjectedContextUpdate::Upsert(context) => {
                        let context_source = context.source_app_id.clone();
                        let context_item = context.item_id.clone();
                        if let Some(tombstone) =
                            thread.injected_context_tombstones.iter().find(|item| {
                                item.source_app_id == context.source_app_id
                                    && item.item_id == context.item_id
                            })
                        {
                            if context.revision <= tombstone.revision {
                                return Err(format!(
                                    "stale injected context revision for {}/{}",
                                    context.source_app_id, context.item_id
                                ));
                            }
                        }
                        if let Some(existing) = thread.injected_contexts.iter_mut().find(|item| {
                            item.source_app_id == context.source_app_id
                                && item.item_id == context.item_id
                        }) {
                            if context.revision < existing.revision {
                                return Err(format!(
                                    "stale injected context revision for {}/{}",
                                    context.source_app_id, context.item_id
                                ));
                            }
                            if context.revision == existing.revision
                                && context.content_digest != existing.content_digest
                            {
                                return Err(format!(
                                    "injected context revision changed content for {}/{}",
                                    context.source_app_id, context.item_id
                                ));
                            }
                            *existing = context;
                        } else {
                            thread.injected_contexts.push(context);
                        }
                        thread.injected_context_tombstones.retain(|item| {
                            item.source_app_id != context_source || item.item_id != context_item
                        });
                    }
                    ChatInjectedContextUpdate::Remove {
                        source_app_id,
                        item_id,
                        revision,
                    } => {
                        let existing_revision = thread
                            .injected_contexts
                            .iter()
                            .find(|item| {
                                item.source_app_id == source_app_id && item.item_id == item_id
                            })
                            .map(|existing| existing.revision);
                        if let Some(existing_revision) = existing_revision {
                            if revision < existing_revision {
                                return Err(format!(
                                    "stale injected context revision for {source_app_id}/{item_id}"
                                ));
                            }
                        }
                        if let Some(tombstone) =
                            thread.injected_context_tombstones.iter_mut().find(|item| {
                                item.source_app_id == source_app_id && item.item_id == item_id
                            })
                        {
                            tombstone.revision = tombstone.revision.max(revision);
                        } else {
                            thread
                                .injected_context_tombstones
                                .push(ChatInjectedContextTombstone {
                                    source_app_id: source_app_id.clone(),
                                    item_id: item_id.clone(),
                                    revision,
                                });
                        }
                        thread.injected_contexts.retain(|item| {
                            item.source_app_id != source_app_id || item.item_id != item_id
                        });
                    }
                }
            }
            if thread.injected_contexts.len() > max_total {
                return Err("injected Chat context limit reached".into());
            }
            if thread.injected_context_tombstones.len() > max_total {
                return Err("injected Chat context tombstone limit reached".into());
            }
            let mut source_counts = BTreeMap::<&str, usize>::new();
            let mut source_chars = BTreeMap::<&str, usize>::new();
            let mut total_chars = 0usize;
            for context in &thread.injected_contexts {
                let count = source_counts
                    .entry(context.source_app_id.as_str())
                    .or_default();
                *count += 1;
                if *count > max_per_source {
                    return Err("injected Chat context source limit reached".into());
                }
                let chars = context.content.chars().count();
                total_chars = total_chars.saturating_add(chars);
                let source_total = source_chars
                    .entry(context.source_app_id.as_str())
                    .or_default();
                *source_total = source_total.saturating_add(chars);
                if *source_total > max_chars_per_source {
                    return Err("injected Chat context source size limit reached".into());
                }
                if total_chars > max_total_chars {
                    return Err("injected Chat context total size limit reached".into());
                }
            }
            thread.injected_contexts.sort_by(|left, right| {
                left.source_app_id
                    .cmp(&right.source_app_id)
                    .then(left.item_id.cmp(&right.item_id))
            });
            thread.updated_at = now_iso();
            thread.revision += 1;
            thread.clone()
        };
        self.commit(candidate)?;
        Ok(snapshot)
    }

    pub fn append_message(
        &mut self,
        thread_id: &str,
        role: ChatMessageRole,
        text: String,
        run_id: Option<String>,
        artifact_ids: Vec<String>,
        status: Option<ChatMessageStatus>,
    ) -> Result<ChatThread, String> {
        self.append_message_view(
            thread_id,
            ChatMessage {
                message_id: new_id("message"),
                role,
                text: text.trim().to_string(),
                reasoning: None,
                run_id,
                artifact_ids,
                status,
                client_request_id: None,
                created_at: now_iso(),
                completed_at: None,
            },
        )
    }

    /// Append the user's turn, recording the client-supplied idempotency key
    /// so a retried send can be recognized instead of re-executed.
    pub fn append_user_message(
        &mut self,
        thread_id: &str,
        text: String,
        client_request_id: String,
    ) -> Result<ChatThread, String> {
        self.append_message_view(
            thread_id,
            ChatMessage {
                message_id: new_id("message"),
                role: ChatMessageRole::User,
                text: text.trim().to_string(),
                reasoning: None,
                run_id: None,
                artifact_ids: vec![],
                status: Some(ChatMessageStatus::Pending),
                client_request_id: Some(client_request_id),
                created_at: now_iso(),
                completed_at: None,
            },
        )
    }

    pub fn request_state(
        &self,
        thread_id: &str,
        client_request_id: &str,
        message: &str,
    ) -> Result<Option<ChatRequestState>, String> {
        let thread = self
            .document
            .threads
            .iter()
            .find(|thread| thread.id == thread_id)
            .ok_or_else(|| format!("unknown chat thread: {thread_id}"))?;
        let Some(request) = thread
            .messages
            .iter()
            .find(|candidate| candidate.client_request_id.as_deref() == Some(client_request_id))
        else {
            return Ok(None);
        };
        if request.text != message.trim() {
            return Err("chat request id was already used with different content".into());
        }
        Ok(Some(
            if request.status == Some(ChatMessageStatus::Pending) {
                ChatRequestState::Pending
            } else {
                ChatRequestState::Completed
            },
        ))
    }

    /// Commit the terminal request marker and all response records together.
    /// A crash before this commit leaves a visible pending request whose
    /// external effects are ambiguous and must never be replayed automatically.
    pub fn complete_request(
        &mut self,
        thread_id: &str,
        client_request_id: &str,
        responses: Vec<ChatMessage>,
    ) -> Result<ChatThread, String> {
        self.complete_request_with_prompt_receipt(thread_id, client_request_id, responses, None)
    }

    pub fn complete_request_with_prompt_receipt(
        &mut self,
        thread_id: &str,
        client_request_id: &str,
        responses: Vec<ChatMessage>,
        prompt_receipt: Option<ChatCompositionReceipt>,
    ) -> Result<ChatThread, String> {
        self.complete_request_with_prompt_receipt_and_consumed_contributions(
            thread_id,
            client_request_id,
            responses,
            prompt_receipt,
            &[],
        )
    }

    pub fn complete_request_with_prompt_receipt_and_consumed_contributions(
        &mut self,
        thread_id: &str,
        client_request_id: &str,
        mut responses: Vec<ChatMessage>,
        prompt_receipt: Option<ChatCompositionReceipt>,
        consumed_contributions: &[ContributionIdentity],
    ) -> Result<ChatThread, String> {
        let mut candidate = self.document.clone();
        let snapshot = {
            let thread = candidate
                .threads
                .iter_mut()
                .find(|thread| thread.id == thread_id)
                .ok_or_else(|| format!("unknown chat thread: {thread_id}"))?;
            let request = thread
                .messages
                .iter_mut()
                .find(|message| message.client_request_id.as_deref() == Some(client_request_id))
                .ok_or_else(|| format!("unknown chat request: {client_request_id}"))?;
            if request.status != Some(ChatMessageStatus::Pending) {
                return Err(format!(
                    "chat request already completed: {client_request_id}"
                ));
            }
            request.status = Some(ChatMessageStatus::Completed);
            request.completed_at = Some(now_iso());
            for response in &mut responses {
                if response.message_id.is_empty() {
                    response.message_id = new_id("message");
                }
                if response.created_at.is_empty() {
                    response.created_at = now_iso();
                }
                // A response is appended only once its text is complete, so
                // its creation time *is* the earliest moment the full text
                // was available. Stamping it keeps `completed_at` meaningful
                // for readers that bound how long a response could have been
                // read, rather than leaving the field permanently null.
                if response.completed_at.is_none() {
                    response.completed_at = Some(response.created_at.clone());
                }
            }
            thread.messages.extend(responses);
            if let Some(mut receipt) = prompt_receipt {
                receipt.created_at = now_iso();
                thread
                    .prompt_receipts
                    .insert(client_request_id.to_string(), receipt);
                thread.contributions.retain(|item| {
                    !consumed_contributions
                        .iter()
                        .any(|identity| identity.matches(item))
                });
            }
            thread.updated_at = now_iso();
            thread.revision += 1;
            thread.clone()
        };
        self.commit(candidate)?;
        Ok(snapshot)
    }

    fn append_message_view(
        &mut self,
        thread_id: &str,
        message: ChatMessage,
    ) -> Result<ChatThread, String> {
        let mut candidate = self.document.clone();
        let snapshot = {
            let thread = candidate
                .threads
                .iter_mut()
                .find(|thread| thread.id == thread_id)
                .ok_or_else(|| format!("unknown chat thread: {thread_id}"))?;
            if thread.messages.is_empty()
                && thread.title == "New chat"
                && matches!(message.role, ChatMessageRole::User)
            {
                thread.title = title_from_first_message(&message.text);
            }
            thread.messages.push(message);
            thread.updated_at = now_iso();
            thread.revision += 1;
            thread.clone()
        };
        self.commit(candidate)?;
        Ok(snapshot)
    }

    fn commit(&mut self, candidate: ChatStoreDocument) -> Result<(), String> {
        let mut persisted = candidate.clone();
        persisted.threads.retain(ChatThread::should_persist);
        match persist_json_document(&self.path, &persisted, "chat storage", self.writer.as_ref()) {
            Ok(()) => {
                self.document = candidate;
                Ok(())
            }
            Err(error) if error.is_indeterminate() => {
                self.document = candidate;
                Err(error.into_message())
            }
            Err(error) => Err(error.into_message()),
        }
    }
}

impl ChatThread {
    fn should_persist(&self) -> bool {
        self.revision > 0
            || !self.messages.is_empty()
            || !self.contributions.is_empty()
            || !self.injected_contexts.is_empty()
            || !self.injected_context_tombstones.is_empty()
    }

    pub fn messages(&self) -> Vec<ChatMessage> {
        self.messages.clone()
    }

    fn public_messages(&self, cursor: Option<u64>, limit: usize) -> Vec<ChatMessageView> {
        let start = cursor.map(|value| value.saturating_add(1)).unwrap_or(0);
        self.messages
            .iter()
            .enumerate()
            .filter_map(|(sequence, message)| {
                let sequence = sequence as u64;
                if sequence < start {
                    return None;
                }
                matches!(
                    message.role,
                    ChatMessageRole::User | ChatMessageRole::Assistant
                )
                .then(|| ChatMessageView::from((self, sequence, message)))
            })
            .take(limit)
            .collect()
    }
}

impl From<(&ChatThread, u64, &ChatMessage)> for ChatMessageView {
    fn from(value: (&ChatThread, u64, &ChatMessage)) -> Self {
        let (thread, sequence, message) = value;
        let status = match message
            .status
            .clone()
            .unwrap_or(ChatMessageStatus::Completed)
        {
            ChatMessageStatus::Pending => ChatMessageViewStatus::Pending,
            ChatMessageStatus::Completed => ChatMessageViewStatus::Completed,
            ChatMessageStatus::Failed => ChatMessageViewStatus::Failed,
            ChatMessageStatus::Cancelled => ChatMessageViewStatus::Cancelled,
            ChatMessageStatus::Interrupted => ChatMessageViewStatus::Interrupted,
        };
        Self {
            message_id: message.message_id.clone(),
            thread_resource_id: thread.resource_id.clone(),
            sequence,
            role: match message.role {
                ChatMessageRole::User => PublicChatMessageRole::User,
                ChatMessageRole::Assistant => PublicChatMessageRole::Assistant,
                ChatMessageRole::System | ChatMessageRole::ToolStatus => {
                    unreachable!("non-public messages are filtered before projection")
                }
            },
            status,
            text: message.text.clone(),
            artifact_refs: message.artifact_ids.clone(),
            run_ref: message.run_id.clone(),
            created_at: message.created_at.clone(),
            completed_at: message.completed_at.clone(),
        }
    }
}

impl From<&ChatThread> for ChatThreadRef {
    fn from(thread: &ChatThread) -> Self {
        Self {
            resource_id: thread.resource_id.clone(),
            thread_id: thread.id.clone(),
            title: thread.title.clone(),
            revision: thread.revision,
            created_at: thread.created_at.clone(),
            updated_at: thread.updated_at.clone(),
        }
    }
}

pub fn title_from_first_message(message: &str) -> String {
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return "New chat".into();
    }
    let truncated = normalized.chars().take(48).collect::<String>();
    if normalized.chars().count() > 48 {
        format!("{}...", truncated.trim_end())
    } else {
        truncated
    }
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4())
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

pub(crate) fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

#[cfg(test)]
mod tests;
