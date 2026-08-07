use super::*;
use crate::atomic_json::{FailingAtomicFileWriter, FailingFileOperation};

fn contribution(kind: ChatContributionKind, item_id: &str) -> ChatContribution {
    ChatContribution {
        source_app_id: "com.example.context".into(),
        source_app_version: "1.0.0".into(),
        source_contract: 1,
        item_id: item_id.into(),
        revision: 1,
        digest: format!("digest-{kind:?}"),
        completeness: ChatContributionCompleteness::Complete,
        lifecycle: ChatContributionLifecycle::Accepted,
        kind,
        title: "Context".into(),
        body: serde_json::json!({"text": "selected"}),
        created_at: now_iso(),
        updated_at: now_iso(),
    }
}

fn injected_context(item_id: &str, revision: u64, content: &str) -> ChatInjectedContext {
    ChatInjectedContext {
        source_app_id: "org.example.context".into(),
        source_app_version: "1.0.0".into(),
        source_app_content_hash: "a".repeat(64),
        source_run_id: format!("run-{revision}"),
        item_id: item_id.into(),
        revision,
        content_digest: format!("{:x}", Sha256::digest(content.as_bytes())),
        content: content.into(),
        created_at: "2026-08-02T10:00:00Z".into(),
        updated_at: "2026-08-02T10:00:00Z".into(),
    }
}

#[test]
fn empty_thread_remains_in_memory_without_being_persisted() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path.clone()).unwrap();
    let thread = store.create_thread().unwrap();
    assert_eq!(thread.title, "New chat");
    assert_eq!(store.list_threads().len(), 1);
    assert!(!path.exists());
    assert!(ChatStore::new(path).unwrap().list_threads().is_empty());
}

#[test]
fn previously_persisted_untouched_empty_threads_are_removed_on_load() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path.clone()).unwrap();
    store.create_thread().unwrap();
    std::fs::write(&path, serde_json::to_vec_pretty(&store.document).unwrap()).unwrap();

    let reloaded = ChatStore::new(path.clone()).unwrap();

    assert!(reloaded.list_threads().is_empty());
    let persisted: ChatStoreDocument =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(persisted.threads.is_empty());
}

#[test]
fn rename_returns_the_authoritative_revision() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path).unwrap();
    let original = store.create_thread().unwrap();

    let renamed = store.rename_thread(&original.id, "Renamed").unwrap();

    assert_eq!(renamed.title, "Renamed");
    assert_eq!(renamed.revision, original.revision + 1);
    let persisted = store.get_thread(&original.id).unwrap();
    assert_eq!(persisted.title, renamed.title);
    assert_eq!(persisted.revision, renamed.revision);
    assert_eq!(persisted.updated_at, renamed.updated_at);
}

#[test]
fn creates_thread_with_initial_agent_engine_in_one_snapshot() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path.clone()).unwrap();
    let receipt = ChatAgentEngineReceipt {
        app_id: "com.example.agent".into(),
        version: "1.0.0".into(),
        contract: "agent-worker/v1".into(),
    };

    let thread = store
        .create_thread_with_agent_engine(Some(receipt.app_id.clone()), Some(receipt.clone()))
        .unwrap();

    assert_eq!(
        thread.chat_agent_engine_ref.as_deref(),
        Some("com.example.agent")
    );
    assert_eq!(thread.chat_agent_engine_receipt, Some(receipt));
    assert_eq!(thread.revision, 0);
    store
        .append_user_message(&thread.id, "hello".into(), "request-1".into())
        .unwrap();
    let reloaded = ChatStore::new(path)
        .unwrap()
        .get_thread(&thread.id)
        .unwrap();
    assert_eq!(reloaded.chat_agent_engine_ref, thread.chat_agent_engine_ref);
    assert_eq!(
        reloaded.chat_agent_engine_receipt,
        thread.chat_agent_engine_receipt
    );
}

#[test]
fn first_user_message_generates_title() {
    assert_eq!(
        title_from_first_message("note that milk is out"),
        "note that milk is out"
    );
    assert_eq!(title_from_first_message("   "), "New chat");
    assert!(title_from_first_message(&"a".repeat(80)).ends_with("..."));
}

#[test]
fn appends_message_with_status_and_artifacts() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path).unwrap();
    let thread = store.create_thread().unwrap();
    let updated = store
        .append_message(
            &thread.id,
            ChatMessageRole::Assistant,
            "refused".into(),
            Some("run-1".into()),
            vec!["artifact-1".into()],
            Some(ChatMessageStatus::Failed),
        )
        .unwrap();
    assert_eq!(updated.messages.len(), 1);
    assert_eq!(
        updated.messages[0].artifact_ids,
        vec![String::from("artifact-1")]
    );
    assert_eq!(updated.messages[0].status, Some(ChatMessageStatus::Failed));
}

#[test]
fn user_message_idempotency_key_survives_reload() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path.clone()).unwrap();
    let thread = store.create_thread().unwrap();
    let updated = store
        .append_user_message(&thread.id, "note the milk".into(), "request-abc".into())
        .unwrap();
    assert_eq!(
        updated.messages[0].client_request_id.as_deref(),
        Some("request-abc")
    );
    assert_eq!(updated.messages[0].status, Some(ChatMessageStatus::Pending));

    let reloaded = ChatStore::new(path).unwrap();
    let persisted = reloaded.get_thread(&thread.id).unwrap();
    assert_eq!(
        persisted.messages[0].client_request_id.as_deref(),
        Some("request-abc")
    );
}

#[test]
fn model_profile_ref_survives_reload_with_its_source_receipt() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path.clone()).unwrap();
    let thread = store.create_thread().unwrap();
    let receipt = crate::chat_model_profiles::ChatModelProfileReceipt {
        source_app_id: "com.example.model-setup".into(),
        source_app_version: "0.1.0".into(),
        profile_id: "focused-work".into(),
        profile_digest: "digest".into(),
        title: "Focused work".into(),
        connector_id: "llm-provider/local".into(),
        model: "model-a".into(),
        reasoning: None,
        temperature: None,
        max_output_tokens: None,
        tool_refs: vec![],
        prompt: None,
    };
    store
        .set_model_profile(
            &thread.id,
            Some("focused-work".into()),
            Some(receipt.clone()),
        )
        .unwrap();

    let reloaded = ChatStore::new(path)
        .unwrap()
        .get_thread(&thread.id)
        .unwrap();

    assert_eq!(reloaded.model_profile_ref.as_deref(), Some("focused-work"));
    assert_eq!(reloaded.model_profile_receipt, Some(receipt));
}

#[test]
fn prompt_receipt_is_recorded_for_the_exact_request_and_survives_reload() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path.clone()).unwrap();
    let thread = store.create_thread().unwrap();
    store
        .append_user_message(&thread.id, "hello".into(), "request-1".into())
        .unwrap();
    store
        .complete_request_with_prompt_receipt(
            &thread.id,
            "request-1",
            vec![ChatMessage {
                message_id: String::new(),
                role: ChatMessageRole::Assistant,
                text: "done".into(),
                reasoning: None,
                run_id: None,
                artifact_ids: vec![],
                status: Some(ChatMessageStatus::Completed),
                client_request_id: None,
                created_at: String::new(),
                completed_at: None,
            }],
            Some(ChatCompositionReceipt {
                system_prompt_digest: "abc".into(),
                assistant_profile_ref: "assistant-profile".into(),
                assistant_profile_digest: "assistant-digest".into(),
                enabled_skill_digests: vec![],
                context_block_digests: vec![],
                attachment_refs: vec![],
                available_capability_refs: vec![],
                provider_profile_ref: "provider-profile".into(),
                model_profile: None,
                agent_engine_ref: None,
                agent_engine_version: None,
                agent_engine_features: vec![],
                assistant_capability_refs: vec![],
                created_at: String::new(),
                system_prompt: "protocol\n\ninstructions".into(),
                layers: vec![ChatPromptReceiptLayer {
                    id: "protocol".into(),
                    kind: "protocol".into(),
                    title: "Kestral protocol".into(),
                    source: Some("Kestral host".into()),
                    content: "protocol".into(),
                }],
                injected_context: None,
            }),
        )
        .unwrap();

    let reloaded = ChatStore::new(path).unwrap();
    let receipt = &reloaded.get_thread(&thread.id).unwrap().prompt_receipts["request-1"];
    assert_eq!(receipt.system_prompt_digest, "abc");
    assert_eq!(receipt.layers[0].kind, "protocol");
    assert!(!receipt.created_at.is_empty());
}

#[test]
fn prompt_receipt_and_response_fail_atomically_for_an_unknown_request() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path.clone()).unwrap();
    let thread = store.create_thread().unwrap();
    store
        .append_user_message(&thread.id, "existing".into(), "request-1".into())
        .unwrap();
    let before = std::fs::read_to_string(&path).unwrap();

    let error = store
        .complete_request_with_prompt_receipt(
            &thread.id,
            "missing",
            vec![],
            Some(ChatCompositionReceipt {
                system_prompt_digest: "abc".into(),
                assistant_profile_ref: "assistant-profile".into(),
                assistant_profile_digest: "assistant-digest".into(),
                enabled_skill_digests: vec![],
                context_block_digests: vec![],
                attachment_refs: vec![],
                available_capability_refs: vec![],
                provider_profile_ref: "provider-profile".into(),
                model_profile: None,
                agent_engine_ref: None,
                agent_engine_version: None,
                agent_engine_features: vec![],
                assistant_capability_refs: vec![],
                created_at: String::new(),
                system_prompt: "prompt".into(),
                layers: vec![],
                injected_context: None,
            }),
        )
        .unwrap_err();

    assert!(error.contains("unknown chat request"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), before);
}

#[test]
fn request_lifecycle_rejects_content_mismatch_and_completes_atomically() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path.clone()).unwrap();
    let thread = store.create_thread().unwrap();
    store
        .append_user_message(&thread.id, "original".into(), "request-1".into())
        .unwrap();

    assert_eq!(
        store
            .request_state(&thread.id, "request-1", "original")
            .unwrap(),
        Some(ChatRequestState::Pending)
    );
    assert!(store
        .request_state(&thread.id, "request-1", "different")
        .unwrap_err()
        .contains("different content"));

    let completed = store
        .complete_request(
            &thread.id,
            "request-1",
            vec![ChatMessage {
                message_id: String::new(),
                role: ChatMessageRole::Assistant,
                text: "done".into(),
                reasoning: None,
                run_id: None,
                artifact_ids: vec![],
                status: Some(ChatMessageStatus::Completed),
                client_request_id: None,
                created_at: String::new(),
                completed_at: None,
            }],
        )
        .unwrap();
    assert_eq!(
        completed.messages[0].status,
        Some(ChatMessageStatus::Completed)
    );
    assert_eq!(completed.messages[1].text, "done");
    assert!(!completed.messages[1].message_id.is_empty());
    assert_eq!(
        ChatStore::new(path)
            .unwrap()
            .request_state(&thread.id, "request-1", "original")
            .unwrap(),
        Some(ChatRequestState::Completed)
    );
}

#[test]
fn contribution_identity_includes_kind_for_update_and_removal() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path).unwrap();
    let thread = store.create_thread().unwrap();
    let text = contribution(ChatContributionKind::TextSnapshot, "shared-id");
    let artifact = contribution(ChatContributionKind::ArtifactRef, "shared-id");

    store.upsert_contribution(&thread.id, text.clone()).unwrap();
    let with_both = store
        .upsert_contribution(&thread.id, artifact.clone())
        .unwrap();
    assert_eq!(with_both.contributions.len(), 2);

    let remaining = store
        .remove_contribution(&thread.id, &ContributionIdentity::from(&text))
        .unwrap();
    assert_eq!(remaining.contributions.len(), 1);
    assert_eq!(
        remaining.contributions[0].kind,
        ChatContributionKind::ArtifactRef
    );
}

#[test]
fn injected_context_updates_are_revisioned_bounded_and_persistent() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path.clone()).unwrap();
    let thread = store.create_thread().unwrap();

    store
        .apply_injected_context_updates(
            &thread.id,
            vec![ChatInjectedContextUpdate::Upsert(injected_context(
                "item-1", 2, "current",
            ))],
            2,
            2,
            32,
            32,
        )
        .unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    let error = store
        .apply_injected_context_updates(
            &thread.id,
            vec![ChatInjectedContextUpdate::Upsert(injected_context(
                "item-1", 1, "stale",
            ))],
            2,
            2,
            32,
            32,
        )
        .unwrap_err();
    assert!(error.contains("stale injected context revision"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

    let mut reloaded = ChatStore::new(path).unwrap();
    let stored = reloaded.get_thread(&thread.id).unwrap().injected_contexts;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].revision, 2);
    assert_eq!(stored[0].content, "current");

    reloaded
        .apply_injected_context_updates(
            &thread.id,
            vec![ChatInjectedContextUpdate::Remove {
                source_app_id: "org.example.context".into(),
                item_id: "item-1".into(),
                revision: 3,
            }],
            2,
            2,
            32,
            32,
        )
        .unwrap();
    let error = reloaded
        .apply_injected_context_updates(
            &thread.id,
            vec![ChatInjectedContextUpdate::Upsert(injected_context(
                "item-1", 2, "delayed",
            ))],
            2,
            2,
            32,
            32,
        )
        .unwrap_err();
    assert!(error.contains("stale injected context revision"));
}

#[test]
fn removal_of_an_absent_context_blocks_delayed_upserts() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path).unwrap();
    let thread = store.create_thread().unwrap();

    let removed = store
        .apply_injected_context_updates(
            &thread.id,
            vec![ChatInjectedContextUpdate::Remove {
                source_app_id: "org.example.context".into(),
                item_id: "item-1".into(),
                revision: 3,
            }],
            2,
            2,
            32,
            32,
        )
        .unwrap();
    assert_eq!(removed.injected_context_tombstones.len(), 1);

    let error = store
        .apply_injected_context_updates(
            &thread.id,
            vec![ChatInjectedContextUpdate::Upsert(injected_context(
                "item-1", 3, "delayed",
            ))],
            2,
            2,
            32,
            32,
        )
        .unwrap_err();
    assert!(error.contains("stale injected context revision"));

    let republished = store
        .apply_injected_context_updates(
            &thread.id,
            vec![ChatInjectedContextUpdate::Upsert(injected_context(
                "item-1",
                4,
                "republished",
            ))],
            2,
            2,
            32,
            32,
        )
        .unwrap();
    assert_eq!(republished.injected_contexts[0].content, "republished");
    assert!(republished.injected_context_tombstones.is_empty());
}

#[test]
fn unchanged_agent_engine_state_does_not_create_a_new_revision() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path).unwrap();
    let thread = store.create_thread().unwrap();
    let state = ChatAgentEngineState {
        status: "fallback".into(),
        fallback_reason: Some("Unavailable".into()),
    };

    let changed = store
        .set_chat_agent_engine_state(&thread.id, Some(state.clone()))
        .unwrap();
    let unchanged = store
        .set_chat_agent_engine_state(&thread.id, Some(state))
        .unwrap();

    assert_eq!(unchanged.revision, changed.revision);
}

#[test]
fn deletes_thread() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path).unwrap();
    let first = store.create_thread().unwrap();
    let second = store.create_thread().unwrap();
    store.delete_thread(&first.id).unwrap();
    let remaining = store.list_threads();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, second.id);
}

#[test]
fn reloads_persisted_threads() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let thread_id = {
        let mut store = ChatStore::new(path.clone()).unwrap();
        let thread = store.create_thread().unwrap();
        let updated = store
            .append_message(
                &thread.id,
                ChatMessageRole::ToolStatus,
                "Used notes / create and produced 1 artifact.".into(),
                Some("run-1".into()),
                vec![],
                Some(ChatMessageStatus::Completed),
            )
            .unwrap();
        assert_eq!(updated.messages.len(), 1);
        thread.id
    };

    let store = ChatStore::new(path).unwrap();
    let reloaded = store.get_thread(&thread_id).unwrap();
    assert_eq!(reloaded.messages.len(), 1);
    assert_eq!(reloaded.messages[0].role, ChatMessageRole::ToolStatus);
    assert_eq!(
        reloaded.messages[0].status,
        Some(ChatMessageStatus::Completed)
    );
    assert_eq!(reloaded.messages[0].run_id.as_deref(), Some("run-1"));
}

#[test]
fn missing_current_v4_thread_field_is_rejected() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path.clone()).unwrap();
    let thread = store.create_thread().unwrap();
    store
        .append_user_message(&thread.id, "hello".into(), "request-1".into())
        .unwrap();
    let mut document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    document["threads"][0]
        .as_object_mut()
        .unwrap()
        .remove("prompt_receipts");
    std::fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();

    let error = ChatStore::new(path).err().unwrap();

    assert!(error.contains("missing field `prompt_receipts`"), "{error}");
}

#[test]
fn public_page_preserves_sequence_gaps_and_paginates_by_cursor() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path).unwrap();
    let thread = store.create_thread().unwrap();
    store
        .append_message(
            &thread.id,
            ChatMessageRole::User,
            "first".into(),
            None,
            vec![],
            Some(ChatMessageStatus::Completed),
        )
        .unwrap();
    store
        .append_message(
            &thread.id,
            ChatMessageRole::System,
            "internal".into(),
            None,
            vec![],
            Some(ChatMessageStatus::Completed),
        )
        .unwrap();
    store
        .append_message(
            &thread.id,
            ChatMessageRole::Assistant,
            "second".into(),
            None,
            vec!["artifact-1".into()],
            Some(ChatMessageStatus::Completed),
        )
        .unwrap();
    store
        .append_message(
            &thread.id,
            ChatMessageRole::ToolStatus,
            "tool status".into(),
            None,
            vec![],
            Some(ChatMessageStatus::Completed),
        )
        .unwrap();
    store
        .append_message(
            &thread.id,
            ChatMessageRole::User,
            "third".into(),
            Some("run-1".into()),
            vec![],
            Some(ChatMessageStatus::Pending),
        )
        .unwrap();

    let first = store.get_thread_page(&thread.resource_id, None, 1).unwrap();
    assert_eq!(first.thread.resource_id, thread.resource_id);
    assert_eq!(first.messages.len(), 1);
    assert_eq!(first.messages[0].sequence, 0);
    assert_eq!(first.messages[0].text, "first");
    assert_eq!(
        first.messages[0].created_at,
        store.get_thread(&thread.id).unwrap().messages()[0].created_at
    );
    assert_eq!(first.next_cursor, Some(0));

    let second = store
        .get_thread_page(&thread.resource_id, first.next_cursor, 2)
        .unwrap();
    assert_eq!(second.messages.len(), 2);
    assert_eq!(second.messages[0].sequence, 2);
    assert_eq!(second.messages[0].text, "second");
    assert_eq!(second.messages[0].artifact_refs, vec!["artifact-1"]);
    assert_eq!(second.messages[1].sequence, 4);
    assert_eq!(second.messages[1].run_ref.as_deref(), Some("run-1"));
    assert_eq!(second.messages[1].status, ChatMessageViewStatus::Pending);
    assert_eq!(second.next_cursor, None);
}

#[test]
fn corrupt_chat_store_fails_fast_and_preserves_the_file() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let corrupt = "{not-json";
    std::fs::write(&path, corrupt).unwrap();

    let error = ChatStore::new(path.clone()).err().unwrap();

    assert!(error.contains("parse chat storage failed"));
    assert!(error.contains("preserved"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), corrupt);
}

#[test]
fn duplicate_thread_identities_fail_fast_on_load() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path.clone()).unwrap();
    let thread = store.create_thread().unwrap();
    store
        .append_user_message(&thread.id, "hello".into(), "request-1".into())
        .unwrap();
    let mut document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let duplicate = document["threads"][0].clone();
    document["threads"].as_array_mut().unwrap().push(duplicate);
    std::fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();

    let error = ChatStore::new(path).err().unwrap();

    assert!(error.contains("duplicate chat thread id"));
}

#[test]
fn duplicate_thread_resource_identities_fail_fast_on_load() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path.clone()).unwrap();
    let thread = store.create_thread().unwrap();
    store
        .append_user_message(&thread.id, "hello".into(), "request-1".into())
        .unwrap();
    let mut document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut duplicate = document["threads"][0].clone();
    duplicate["id"] = serde_json::Value::String(new_id("thread"));
    document["threads"].as_array_mut().unwrap().push(duplicate);
    std::fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();

    let error = ChatStore::new(path).err().unwrap();

    assert!(error.contains("duplicate chat thread resource id"));
}

#[test]
fn injected_context_integrity_and_bounds_are_revalidated_on_load() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let mut store = ChatStore::new(path.clone()).unwrap();
    let thread = store.create_thread().unwrap();
    store
        .apply_injected_context_updates(
            &thread.id,
            vec![ChatInjectedContextUpdate::Upsert(injected_context(
                "item-1", 1, "current",
            ))],
            MAX_INJECTED_CONTEXTS_PER_SOURCE,
            MAX_INJECTED_CONTEXTS_PER_THREAD,
            MAX_INJECTED_CONTEXT_CHARS_PER_SOURCE,
            MAX_INJECTED_CONTEXT_CHARS_PER_THREAD,
        )
        .unwrap();

    let mut document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    document["threads"][0]["injected_contexts"][0]["content"] =
        serde_json::Value::String("changed on disk".into());
    std::fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    let error = ChatStore::new(path.clone()).err().unwrap();
    assert!(error.contains("digest mismatch"));

    let oversized = "x".repeat(MAX_INJECTED_CONTEXT_CHARS + 1);
    document["threads"][0]["injected_contexts"][0]["content"] =
        serde_json::Value::String(oversized.clone());
    document["threads"][0]["injected_contexts"][0]["content_digest"] =
        serde_json::Value::String(format!("{:x}", Sha256::digest(oversized.as_bytes())));
    std::fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    let error = ChatStore::new(path).err().unwrap();
    assert!(error.contains("content is empty or too large"));
}

#[test]
fn unsupported_chat_store_version_fails_fast() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    std::fs::write(&path, r#"{"version":1,"threads":[]}"#).unwrap();

    let error = ChatStore::new(path).err().unwrap();

    assert!(error.contains("unsupported chat storage version: 1"));
}

#[test]
fn stale_transaction_temp_is_discarded_when_primary_exists() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    std::fs::write(&path, r#"{"version":4,"threads":[]}"#).unwrap();
    let file_name = path.file_name().unwrap().to_string_lossy();
    let stale_path = path.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    std::fs::write(&stale_path, "uncommitted candidate").unwrap();

    let store = ChatStore::new(path).unwrap();

    assert!(store.list_threads().is_empty());
    assert!(!stale_path.exists());
}

#[test]
fn failed_chat_write_keeps_memory_and_disk_unchanged() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let writer = Arc::new(FailingAtomicFileWriter::new(FailingFileOperation::Write));
    let mut store = ChatStore::with_writer(path.clone(), writer).unwrap();

    let thread = store.create_thread().unwrap();
    let error = store
        .append_user_message(&thread.id, "hello".into(), "request-1".into())
        .unwrap_err();

    assert!(error.contains("injected write failure"));
    assert_eq!(store.list_threads().len(), 1);
    assert!(store.get_thread(&thread.id).unwrap().messages().is_empty());
    assert!(!path.exists());
}

#[test]
fn failed_chat_rename_rolls_back_memory_and_disk() {
    let path = std::env::temp_dir().join(format!("chat-store-{}.json", new_id("test")));
    let first = {
        let mut store = ChatStore::new(path.clone()).unwrap();
        let thread = store.create_thread().unwrap();
        store
            .append_user_message(&thread.id, "hello".into(), "request-1".into())
            .unwrap()
    };
    let before = std::fs::read_to_string(&path).unwrap();
    let writer = Arc::new(FailingAtomicFileWriter::new(FailingFileOperation::Rename));
    let mut store = ChatStore::with_writer(path.clone(), writer).unwrap();

    let error = store.rename_thread(&first.id, "Renamed").unwrap_err();

    assert!(error.contains("injected rename failure"));
    assert_eq!(store.get_thread(&first.id).unwrap().title, "hello");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    assert_eq!(
        ChatStore::new(path)
            .unwrap()
            .get_thread(&first.id)
            .unwrap()
            .title,
        "hello"
    );
}

#[test]
fn generated_chat_ids_are_uuid_based_and_distinct() {
    let first = new_id("thread");
    let second = new_id("thread");

    assert_ne!(first, second);
    assert!(Uuid::parse_str(first.strip_prefix("thread-").unwrap()).is_ok());
}
