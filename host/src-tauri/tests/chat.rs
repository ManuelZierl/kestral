//! Chat end to end: fallback assistant replies through the public kernel API.
//!
//! Uses a fake LLM handler so tests are deterministic (no live backend needed).

use std::sync::{Arc, Mutex};

use serde_json::json;

use app_host_kernel::ids::CapabilityName;
use app_host_kernel::invocation::{CapabilityHandler, CapabilityOutcome, InvocationContext};
use app_host_kernel::kernel::Kernel;
use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, TrustedChrome,
};
use app_host_kernel::JsonObject;

/// Approves everything and counts the capability approvals it was asked for.
struct CountingChrome {
    capability_prompts: Mutex<u32>,
}

impl TrustedChrome for CountingChrome {
    fn confirm_grant(&self, _prompt: GrantIssuancePrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn approve_capability(&self, _prompt: CapabilityApprovalPrompt) -> ApprovalDecision {
        *self.capability_prompts.lock().unwrap() += 1;
        ApprovalDecision::Approved
    }

    fn show_notice(&self, _notice: ChromeNotice) -> Result<(), ChromeNoticeError> {
        Ok(())
    }

    fn confirm_event_subscriptions(&self, _prompt: EventSubscriptionPrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }
}

fn llm_reply(content: &str) -> serde_json::Value {
    json!({
        "message": {"role": "assistant", "content": content},
        "finish_reason": "stop"
    })
}

fn make_fake_handler(
    f: impl Fn() -> serde_json::Value + Send + Sync + 'static,
) -> CapabilityHandler {
    Box::new(move |_input: &JsonObject, _context: &InvocationContext| {
        Ok(CapabilityOutcome {
            result: f(),
            artifacts: vec![],
        })
    })
}

#[test]
fn chat_drives_real_work_through_the_public_api_with_fake_llm() {
    let chrome = Arc::new(CountingChrome {
        capability_prompts: Mutex::new(0),
    });
    let mut kernel = Kernel::new(chrome.clone());
    install_parts(
        &mut kernel,
        app_host_kernel::manifest::seal(host_lib::llm_provider::llm_provider_manifest()),
        std::collections::BTreeMap::from([
            (
                CapabilityName::new("llm.generate"),
                make_fake_handler(|| llm_reply("Hello from Chat!")),
            ),
            (
                CapabilityName::new("llm.models.list"),
                make_fake_handler(|| json!({"models": [], "refreshed": false})),
            ),
            (
                CapabilityName::new("llm.models.refresh"),
                make_fake_handler(|| json!({"models": [], "refreshed": true})),
            ),
        ]),
        app_host_kernel::primitives::grant::GrantOrigin::SystemBundled,
    )
    .unwrap();
    let chat_manifest = host_lib::chat_app::chat_manifest_for_kernel(&kernel);
    let chat_store = Arc::new(Mutex::new(
        host_lib::chat_store::ChatStore::new(
            std::env::temp_dir().join(format!("chat-e2e-{}.json", uuid::Uuid::new_v4())),
        )
        .unwrap(),
    ));
    install_parts(
        &mut kernel,
        chat_manifest,
        host_lib::chat_app::chat_handlers(chat_store),
        app_host_kernel::primitives::grant::GrantOrigin::SystemBundled,
    )
    .unwrap();

    let reply = drive_chat_message(&mut kernel, "say hi").unwrap();
    assert!(reply.run_id.is_some());
    assert!(reply.text.contains("Hello from Chat!"));
    assert!(reply.artifacts.is_empty());
    assert_eq!(*chrome.capability_prompts.lock().unwrap(), 0);

    let reply = drive_chat_message(&mut kernel, "help").unwrap();
    assert!(reply.run_id.is_none());
    assert!(reply.artifacts.is_empty());
}
mod support;
use support::*;
