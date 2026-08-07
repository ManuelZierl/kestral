use super::*;
use app_host_kernel::ids::{AppId, CapabilityName, GrantId, RunId};
use app_host_kernel::primitives::capability::CapabilityRef;
use uuid::Uuid;

#[test]
fn notice_inbox_returns_recent_notices_newest_first() {
    let path = std::env::temp_dir().join(format!("trusted-notices-{}.json", Uuid::new_v4()));
    let mut inbox = TrustedNoticeStore::new(path.clone()).unwrap();
    let capability = CapabilityRef {
        provider: AppId::new("notes"),
        capability: CapabilityName::new("create_note"),
    };
    let first = inbox
        .record(ChromeNotice::GrantUse {
            app_id: AppId::new("chat"),
            capability: capability.clone(),
            grant_id: GrantId::new("grant-1"),
            run_id: RunId::new("run-1"),
        })
        .unwrap();
    let second = inbox
        .record(ChromeNotice::LeaseConflict {
            resource: "workspace:file.txt".into(),
            holding_run: RunId::new("run-2"),
            requesting_run: RunId::new("run-3"),
        })
        .unwrap();

    assert_eq!(first.sequence, 0);
    assert_eq!(second.sequence, 1);
    assert_eq!(inbox.recent(), vec![second, first]);

    let _ = std::fs::remove_file(path);
}

#[test]
fn oauth_sessions_correlate_prompts_and_cancel_independently() {
    let sessions = PendingOAuthSessions::default();
    sessions.set_publisher(Arc::new(|_| Ok(()))).unwrap();
    let (first_sender, first_receiver) = mpsc::channel();
    let (second_sender, second_receiver) = mpsc::channel();
    sessions
        .register("session-1".into(), "connector-1".into(), first_sender)
        .unwrap();
    sessions
        .register("session-2".into(), "connector-2".into(), second_sender)
        .unwrap();
    sessions.set_prompt("session-1", "prompt-1".into()).unwrap();

    assert!(sessions
        .resolve_prompt("session-1", "wrong".into(), Some("value".into()), false)
        .is_err());
    sessions
        .resolve_prompt("session-1", "prompt-1".into(), Some("value".into()), false)
        .unwrap();
    assert!(matches!(
        first_receiver.recv().unwrap(),
        OAuthControl::PromptResponse { prompt_id, value, cancelled: false }
            if prompt_id == "prompt-1" && value.as_deref() == Some("value")
    ));

    sessions.cancel("session-2").unwrap();
    assert!(matches!(
        second_receiver.recv().unwrap(),
        OAuthControl::Cancel
    ));
    sessions.finish("session-2");
    assert!(sessions.cancel("session-2").is_err());
}

#[test]
fn public_oauth_events_never_serialize_credentials() {
    let event = OAuthPublicEvent::Completed {
        session_id: "session-1".into(),
    };
    let serialized = serde_json::to_string(&event).unwrap();
    assert_eq!(
        serialized,
        r#"{"kind":"completed","session_id":"session-1"}"#
    );
    assert!(!serialized.contains("credential"));
    assert!(!serialized.contains("access"));
    assert!(!serialized.contains("refresh"));
}

#[test]
fn approval_slots_bound_one_apps_concurrent_prompts() {
    let slots = ApprovalSlots::default();
    let noisy = AppId::new("com.example.noisy");

    // An app gets its allowance, and no more.
    for _ in 0..MAX_PENDING_APPROVALS_PER_APP {
        assert!(slots.claim(&noisy));
    }
    assert!(
        !slots.claim(&noisy),
        "an app calling in a loop must stop producing new modals"
    );

    // Answering one prompt frees exactly one slot.
    slots.release(&noisy);
    assert!(slots.claim(&noisy));
    assert!(!slots.claim(&noisy));
}

#[test]
fn approval_slots_are_scoped_to_one_app() {
    let slots = ApprovalSlots::default();
    let noisy = AppId::new("com.example.noisy");
    let quiet = AppId::new("com.example.quiet");

    for _ in 0..MAX_PENDING_APPROVALS_PER_APP {
        assert!(slots.claim(&noisy));
    }

    // One misbehaving app must not spend another app's allowance.
    assert!(slots.claim(&quiet));
}

#[test]
fn releasing_more_than_claimed_does_not_underflow() {
    let slots = ApprovalSlots::default();
    let app = AppId::new("com.example.app");
    slots.release(&app);
    slots.release(&app);
    for _ in 0..MAX_PENDING_APPROVALS_PER_APP {
        assert!(slots.claim(&app));
    }
    assert!(!slots.claim(&app));
}

#[test]
fn cancelling_one_app_prefix_denies_and_removes_its_pending_approval() {
    let pending = Arc::new(PendingApprovals::default());
    let removed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let request = ChromeRequest::CapabilityApproval {
        request_id: 7,
        prompt: CapabilityApprovalPrompt {
            app_id: AppId::new("mcp-export/profile-1"),
            app_display_name: "Remote profile".into(),
            capability: CapabilityRef {
                provider: AppId::new("notes"),
                capability: CapabilityName::new("create"),
            },
            data_scope: app_host_kernel::primitives::grant::DataScope::None,
            grant_id: GrantId::new("grant-1"),
            run_id: RunId::new("run-1"),
            goal: "Create a note".into(),
        },
    };
    let waiter = {
        let pending = pending.clone();
        let removed = removed.clone();
        std::thread::spawn(move || {
            pending.wait_for_decision(
                7,
                request,
                || true,
                move || {
                    removed.store(true, Ordering::Relaxed);
                },
            )
        })
    };
    for _ in 0..100 {
        if !pending.pending_requests().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    pending.deny_app_id_prefix("mcp-export/");

    assert_eq!(waiter.join().unwrap(), ApprovalDecision::Denied);
    assert!(removed.load(Ordering::Relaxed));
    assert!(pending.pending_requests().is_empty());
}
