use super::*;
use std::collections::HashSet;
use std::time::Duration;

use app_host_kernel::ids::{AppId, CapabilityName};
use app_host_kernel::primitives::grant::{
    DataScope, GrantCondition, GrantDuration, GrantOrigin, GrantScope,
};
use futures_util::StreamExt;
use uuid::Uuid;

/// Command names registered with `tauri::generate_handler!` in `lib.rs`.
fn tauri_command_handlers() -> Vec<String> {
    let source = include_str!("../lib.rs");
    let marker = "generate_handler![";
    let start = source.find(marker).expect("generate_handler! list present");
    let rest = &source[start + marker.len()..];
    let end = rest.find(']').expect("generate_handler! list closes");
    rest[..end]
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

/// Command names handled by `dispatch`: a string literal immediately
/// followed by `=>` or `|` is a match key. Argument-name and message
/// literals are followed by `)`/other punctuation, so they are excluded.
fn dispatched_commands() -> HashSet<String> {
    let file = include_str!("../remote_api.rs");
    let start = file
        .find("match command {")
        .expect("dispatch match present");
    let dispatch = &file[start..];
    let end = dispatch
        .find("struct ServerState")
        .expect("dispatch function ends before server state");
    let source = &dispatch[..end];
    let bytes = source.as_bytes();
    let mut keys = HashSet::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        let mut close = index + 1;
        while close < bytes.len() && bytes[close] != b'"' {
            close += 1;
        }
        if close >= bytes.len() {
            break;
        }
        let mut next = close + 1;
        while next < bytes.len() && bytes[next].is_ascii_whitespace() {
            next += 1;
        }
        let followed_by_arm = matches!(bytes.get(next), Some(b'|'))
            || (bytes.get(next) == Some(&b'=') && bytes.get(next + 1) == Some(&b'>'));
        if followed_by_arm {
            keys.insert(source[index + 1..close].to_string());
        }
        index = close + 1;
    }
    keys
}

#[test]
fn dispatch_covers_every_tauri_command() {
    let registered: HashSet<String> = tauri_command_handlers().into_iter().collect();
    let dispatched = dispatched_commands();
    let missing: Vec<String> = registered.difference(&dispatched).cloned().collect();
    let extra: Vec<String> = dispatched.difference(&registered).cloned().collect();
    assert!(
        missing.is_empty(),
        "remote dispatch is missing commands registered in generate_handler!: {missing:?}"
    );
    assert!(
        extra.is_empty(),
        "remote dispatch exposes commands not registered in generate_handler!: {extra:?}"
    );
}

#[test]
fn remote_create_chat_thread_does_not_nest_async_runtimes() {
    let path = std::env::temp_dir().join(format!("remote-create-chat-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    let events = Arc::new(RemoteEventHub::default());
    let pending = Arc::new(PendingApprovals::default());
    let notices = Arc::new(Mutex::new(
        TrustedNoticeStore::new(path.join("trusted-notices.json")).unwrap(),
    ));
    let chrome = Arc::new(RemoteChrome {
        pending: pending.clone(),
        notices: notices.clone(),
        events: events.clone(),
        next_request_id: AtomicU64::new(0),
    });
    let paths =
        HostPaths::resolve_startup_from(path.clone(), std::iter::empty::<OsString>(), |_| None)
            .unwrap();
    let host = build_host(paths, chrome, pending, notices).unwrap();
    {
        let mut kernel = host.kernel.lock().unwrap();
        let manifest = crate::chat_app::chat_manifest_for_kernel(&kernel);
        let prepared = kernel
            .prepare_install_with_grant_origin(
                manifest,
                crate::chat_app::chat_handlers(host.chat_store.clone()),
                GrantOrigin::SystemBundled,
            )
            .unwrap();
        kernel.commit_install(prepared.await_approval()).unwrap();
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let value = runtime
        .block_on(dispatch(&host, &events, "create_chat_thread", Map::new()))
        .unwrap();

    assert!(value["id"].as_str().is_some());
    drop(host);
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn remote_mcp_server_save_does_not_nest_async_runtimes_or_poison_config() {
    let path = std::env::temp_dir().join(format!("remote-mcp-save-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    let events = Arc::new(RemoteEventHub::default());
    let pending = Arc::new(PendingApprovals::default());
    let notices = Arc::new(Mutex::new(
        TrustedNoticeStore::new(path.join("trusted-notices.json")).unwrap(),
    ));
    let chrome = Arc::new(RemoteChrome {
        pending: pending.clone(),
        notices: notices.clone(),
        events: events.clone(),
        next_request_id: AtomicU64::new(0),
    });
    let paths =
        HostPaths::resolve_startup_from(path.clone(), std::iter::empty::<OsString>(), |_| None)
            .unwrap();
    let host = build_host(paths, chrome, pending, notices).unwrap();
    let server = crate::config::McpServerConfigView {
        id: "remote".into(),
        display_name: "Remote".into(),
        transport: crate::config::McpTransportConfig::StreamableHttp {
            url: "https://mcp.example/mcp".into(),
            authentication: crate::config::McpHttpAuthentication::StaticHeader {
                header_name: "Authorization".into(),
                value_prefix: "Bearer ".into(),
            },
        },
    };
    let mut arguments = Map::new();
    arguments.insert("server".into(), serde_json::to_value(server).unwrap());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let (saved, config) = runtime
        .block_on(async {
            let saved = dispatch(&host, &events, "upsert_mcp_server", arguments).await?;
            let config = dispatch(&host, &events, "get_host_config", Map::new()).await?;
            Ok::<_, String>((saved, config))
        })
        .unwrap();

    assert_eq!(saved["id"], "remote");
    assert_eq!(config["mcp_servers"]["remote"]["display_name"], "Remote");
    assert!(!host.config.is_poisoned());
    drop(host);
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn remote_argument_contract_rejects_missing_and_malformed_values() {
    let mut arguments = Map::new();
    arguments.insert("threadId".into(), json!("thread-1"));
    assert_eq!(
        argument::<String>(&arguments, "threadId").unwrap(),
        "thread-1"
    );
    assert!(argument::<String>(&arguments, "requestId").is_err());
    arguments.insert("threadId".into(), json!(42));
    assert!(argument::<String>(&arguments, "threadId").is_err());
}

#[test]
fn remote_optional_arguments_accept_missing_and_null_values() {
    let mut arguments = Map::new();
    assert_eq!(
        optional_argument::<String>(&arguments, "apiKeySecretName").unwrap(),
        None
    );
    arguments.insert("apiKeySecretName".into(), Value::Null);
    assert_eq!(
        optional_argument::<String>(&arguments, "apiKeySecretName").unwrap(),
        None
    );
    arguments.insert("apiKeySecretName".into(), json!("secret"));
    assert_eq!(
        optional_argument::<String>(&arguments, "apiKeySecretName").unwrap(),
        Some("secret".into())
    );
}

#[test]
fn configured_data_dir_owns_backend_profile_metadata() {
    let current_dir = PathBuf::from("/service/working-directory");
    assert_eq!(
        backend_default_root(Some(OsString::from("/srv/kestral")), current_dir.clone()),
        PathBuf::from("/srv/kestral")
    );
    assert_eq!(
        backend_default_root(None, current_dir),
        PathBuf::from("/service/working-directory/host-data")
    );
}

#[test]
fn event_cursor_is_inclusive_and_ordered() {
    let events = RemoteEventHub::default();
    events.publish("one", &json!({ "n": 1 })).unwrap();
    events.publish("two", &json!({ "n": 2 })).unwrap();
    let batch = events.since(1).unwrap();
    assert_eq!(batch.instance_id, events.instance_id);
    assert_eq!(batch.oldest_sequence, 0);
    assert_eq!(batch.next_sequence, 2);
    assert_eq!(batch.events.len(), 1);
    assert_eq!(batch.events[0].sequence, 1);
    assert_eq!(batch.events[0].event, "two");
}

#[test]
fn event_hubs_identify_distinct_backend_processes() {
    let first = RemoteEventHub::default();
    let second = RemoteEventHub::default();

    assert_ne!(first.instance_id, second.instance_id);
    assert!(Uuid::parse_str(&first.instance_id).is_ok());
    assert!(Uuid::parse_str(&second.instance_id).is_ok());
}

#[test]
fn event_pressure_drops_streaming_before_trusted_chrome() {
    let events = RemoteEventHub::default();
    events
        .publish(CHROME_REQUEST_EVENT, &json!({ "request": 1 }))
        .unwrap();
    for sequence in 0..=MAX_EVENTS {
        events
            .publish("chat-stream:request", &json!({ "sequence": sequence }))
            .unwrap();
    }
    let batch = events.since(0).unwrap();
    assert_eq!(batch.events.len(), MAX_EVENTS);
    assert_eq!(batch.next_sequence, MAX_EVENTS as u64 + 2);
    assert!(batch
        .events
        .iter()
        .any(|event| event.event == CHROME_REQUEST_EVENT));
    assert!(batch
        .events
        .windows(2)
        .any(|pair| pair[1].sequence > pair[0].sequence + 1));
}

#[test]
fn event_stream_sends_initial_replay_then_wakes_for_new_events() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let events = Arc::new(RemoteEventHub::default());
        let stream = event_stream(events.clone(), 0);
        futures_util::pin_mut!(stream);

        assert!(stream.next().await.is_some());
        events.publish("next", &json!({ "n": 1 })).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), stream.next())
                .await
                .expect("stream wakes after publish")
                .is_some()
        );
    });
}

#[test]
fn state_change_notifications_exclude_read_commands() {
    assert_eq!(state_change_scopes("list_apps"), None);
    assert_eq!(state_change_scopes("list_artifacts"), None);
    assert_eq!(
        state_change_scopes("send_chat_message"),
        Some(&["artifacts", "chat", "records"][..])
    );
    assert!(state_change_scopes("update_host_config")
        .unwrap()
        .contains(&"config"));
    for command in [
        "attach_chat_artifact",
        "set_chat_thread_profile",
        "set_chat_agent_engine",
        "remove_chat_contribution",
    ] {
        assert_eq!(
            state_change_scopes(command),
            Some(&["chat"][..]),
            "{command}"
        );
    }
    for command in ["put_mcp_http_auth_secret", "clear_mcp_http_auth_secret"] {
        assert_eq!(
            state_change_scopes(command),
            Some(&["config"][..]),
            "{command}"
        );
    }
    assert_eq!(
        state_change_scopes("revoke_publisher_key"),
        Some(&["publisher-trust"][..])
    );
    assert_eq!(
        state_change_scopes("rotate_mcp_export_token"),
        Some(&["mcp-export"][..])
    );
    assert_eq!(
        state_change_scopes("create_kestral_profile"),
        Some(&["profiles"][..])
    );
}

#[test]
fn remote_chrome_groups_one_apps_permissions_into_one_install_request() {
    let pending = Arc::new(PendingApprovals::default());
    let notice_path = std::env::temp_dir().join(format!("remote-notices-{}.json", Uuid::new_v4()));
    let chrome = Arc::new(RemoteChrome {
        pending: pending.clone(),
        notices: Arc::new(Mutex::new(
            TrustedNoticeStore::new(notice_path.clone()).unwrap(),
        )),
        events: Arc::new(RemoteEventHub::default()),
        next_request_id: AtomicU64::new(0),
    });
    let grant = |capability: &str| GrantIssuancePrompt {
        app_id: AppId::new("notes"),
        app_display_name: "Notes".into(),
        scope: GrantScope::ExactCapability {
            provider: AppId::new("files"),
            capability: CapabilityName::new(capability),
        },
        data_scope: DataScope::None,
        condition: GrantCondition::Silent,
        duration: GrantDuration::NonExpiring,
        reason: format!("Use {capability}"),
    };
    let prompt = InstallApprovalPrompt {
        app_id: AppId::new("notes"),
        app_display_name: "Notes".into(),
        event: None,
        grants: vec![grant("read"), grant("write")],
    };

    let approval = std::thread::spawn(move || chrome.confirm_install(prompt));
    let request_id = (0..100)
        .find_map(|_| {
            let request = pending.pending_requests().into_iter().next();
            if request.is_none() {
                std::thread::sleep(Duration::from_millis(5));
            }
            request
        })
        .map(|request| match request {
            ChromeRequest::InstallApproval { request_id, prompt } => {
                assert_eq!(prompt.app_id, AppId::new("notes"));
                assert_eq!(prompt.grants.len(), 2);
                request_id
            }
            _ => panic!("remote chrome emitted a per-item request"),
        })
        .expect("install approval became pending");
    pending
        .resolve_install(request_id, None, vec![true, false])
        .unwrap();

    let decision = approval.join().unwrap();
    assert_eq!(
        decision.grant_decisions,
        vec![ApprovalDecision::Approved, ApprovalDecision::Denied]
    );
    let _ = std::fs::remove_file(notice_path);
}
