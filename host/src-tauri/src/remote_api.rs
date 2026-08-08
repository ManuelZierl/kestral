//! Single-owner remote transport for the host command surface.
//!
//! Tauri remains the default transport. This module calls the same command
//! functions with the same `Host` instance; it does not put localhost HTTP in
//! the desktop path or duplicate application logic.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, InstallApprovalDecision, InstallApprovalPrompt,
    TrustedChrome,
};
use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, CONTENT_TYPE, COOKIE, ORIGIN, SET_COOKIE, VARY,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Router;
use chrono::Utc;
use futures_util::stream::{self, Stream};
use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};
use uuid::Uuid;

#[cfg(test)]
use crate::build_host;
use crate::chrome::{
    ChromeRequest, PendingApprovals, TrustedNoticeStore, CHROME_NOTICE_EVENT, CHROME_OAUTH_EVENT,
    CHROME_REQUEST_EVENT, CHROME_REQUEST_EXPIRED_EVENT,
};
use crate::remote_auth::{AuthFailure, AuthenticationFinish, RegistrationFinish, RemoteOwnerAuth};
use crate::{host_paths::HostPaths, ChatStreamEvent, Host, HostState};

const DEFAULT_BIND: &str = "127.0.0.1:4310";
const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
const MAX_EVENTS: usize = 512;
const MAX_CONCURRENT_COMMANDS: usize = 6;
const HOST_STATE_CHANGED_EVENT: &str = "host-state:changed";

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct RemoteEvent {
    sequence: u64,
    event: String,
    payload: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct RemoteEventBatch {
    instance_id: String,
    oldest_sequence: u64,
    next_sequence: u64,
    events: Vec<RemoteEvent>,
}

#[derive(Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct RemoteApprovalBatch {
    instance_id: String,
    requests: Vec<ChromeRequest>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct HostStateChanged {
    scopes: &'static [&'static str],
}

struct RemoteEventHub {
    instance_id: String,
    next_sequence: AtomicU64,
    events: Mutex<VecDeque<RemoteEvent>>,
    revision: tokio::sync::watch::Sender<u64>,
}

impl Default for RemoteEventHub {
    fn default() -> Self {
        Self {
            instance_id: Uuid::new_v4().to_string(),
            next_sequence: AtomicU64::new(0),
            events: Mutex::new(VecDeque::new()),
            revision: tokio::sync::watch::channel(0).0,
        }
    }
}

impl RemoteEventHub {
    fn publish<T: Serialize>(&self, event: &str, payload: &T) -> Result<(), String> {
        let payload = serde_json::to_value(payload)
            .map_err(|error| format!("serialize remote event failed: {error}"))?;
        let mut events = self
            .events
            .lock()
            .map_err(|_| "remote event queue lock poisoned".to_string())?;
        let record = RemoteEvent {
            sequence: self.next_sequence.fetch_add(1, Ordering::Relaxed),
            event: event.to_string(),
            payload,
        };
        events.push_back(record);
        while events.len() > MAX_EVENTS {
            let expendable = events
                .iter()
                .position(|event| event.event.starts_with("chat-stream:"));
            if let Some(index) = expendable {
                events.remove(index);
            } else {
                events.pop_front();
            }
        }
        drop(events);
        self.revision
            .send_replace(self.next_sequence.load(Ordering::Relaxed));
        Ok(())
    }

    fn since(&self, sequence: u64) -> Result<RemoteEventBatch, String> {
        self.events
            .lock()
            .map_err(|_| "remote event queue lock poisoned".to_string())
            .map(|events| {
                let next_sequence = self.next_sequence.load(Ordering::Relaxed);
                let oldest_sequence = events
                    .front()
                    .map(|event| event.sequence)
                    .unwrap_or(next_sequence);
                let events = events
                    .iter()
                    .filter(|event| event.sequence >= sequence)
                    .cloned()
                    .collect();
                RemoteEventBatch {
                    instance_id: self.instance_id.clone(),
                    oldest_sequence,
                    next_sequence,
                    events,
                }
            })
    }
}

fn event_stream(
    hub: Arc<RemoteEventHub>,
    after: u64,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let revision = hub.revision.subscribe();
    stream::unfold(
        (hub, revision, after, true, false),
        |(hub, mut revision, cursor, initial, terminal)| async move {
            if terminal {
                return None;
            }
            let batch = loop {
                match hub.since(cursor) {
                    Ok(batch)
                        if initial
                            || batch.next_sequence != cursor
                            || batch.oldest_sequence > cursor =>
                    {
                        break batch;
                    }
                    Ok(_) => {
                        if revision.changed().await.is_err() {
                            return None;
                        }
                    }
                    Err(error) => {
                        let event = Event::default().event("transport-error").data(error);
                        return Some((Ok(event), (hub, revision, cursor, false, true)));
                    }
                }
            };
            let next_cursor = batch.next_sequence;
            let event = Event::default()
                .id(next_cursor.to_string())
                .event("remote-events")
                .json_data(batch)
                .expect("remote event batch serializes");
            Some((Ok(event), (hub, revision, next_cursor, false, false)))
        },
    )
}

struct RemoteChrome {
    pending: Arc<PendingApprovals>,
    notices: Arc<Mutex<TrustedNoticeStore>>,
    events: Arc<RemoteEventHub>,
    next_request_id: AtomicU64,
}

impl RemoteChrome {
    fn ask(&self, request_id: u64, request: ChromeRequest) -> ApprovalDecision {
        let events = self.events.clone();
        self.pending.wait_for_decision(
            request_id,
            request.clone(),
            || self.events.publish(CHROME_REQUEST_EVENT, &request).is_ok(),
            move || {
                let _ = events.publish(CHROME_REQUEST_EXPIRED_EVENT, &request_id);
            },
        )
    }
}

impl TrustedChrome for RemoteChrome {
    fn confirm_grant(&self, prompt: GrantIssuancePrompt) -> ApprovalDecision {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        self.ask(
            request_id,
            ChromeRequest::GrantIssuance { request_id, prompt },
        )
    }

    fn approve_capability(&self, prompt: CapabilityApprovalPrompt) -> ApprovalDecision {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        self.ask(
            request_id,
            ChromeRequest::CapabilityApproval { request_id, prompt },
        )
    }

    fn show_notice(&self, notice: ChromeNotice) -> Result<(), ChromeNoticeError> {
        let record = self
            .notices
            .lock()
            .expect("trusted notice store lock poisoned")
            .record(notice)
            .map_err(|error| ChromeNoticeError::Persistence {
                message: error.to_string(),
            })?;
        self.events
            .publish(CHROME_NOTICE_EVENT, &record)
            .map_err(|error| ChromeNoticeError::Delivery {
                message: error.to_string(),
            })
    }

    fn confirm_event_subscriptions(&self, prompt: EventSubscriptionPrompt) -> ApprovalDecision {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        self.ask(
            request_id,
            ChromeRequest::EventSubscription { request_id, prompt },
        )
    }

    fn confirm_install(&self, prompt: InstallApprovalPrompt) -> InstallApprovalDecision {
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
        let events = self.events.clone();
        self.pending.wait_for_install_decision(
            request_id,
            request.clone(),
            denied,
            || self.events.publish(CHROME_REQUEST_EVENT, &request).is_ok(),
            move || {
                let _ = events.publish(CHROME_REQUEST_EXPIRED_EVENT, &request_id);
            },
        )
    }
}

fn argument<T: DeserializeOwned>(arguments: &Map<String, Value>, name: &str) -> Result<T, String> {
    let value = arguments
        .get(name)
        .cloned()
        .ok_or_else(|| format!("missing command argument '{name}'"))?;
    serde_json::from_value(value)
        .map_err(|error| format!("invalid command argument '{name}': {error}"))
}

fn optional_argument<T: DeserializeOwned>(
    arguments: &Map<String, Value>,
    name: &str,
) -> Result<Option<T>, String> {
    let Some(value) = arguments.get(name) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|error| format!("invalid command argument '{name}': {error}"))
}

fn output<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| format!("serialize command result failed: {error}"))
}

/// Route a remote command to the same command function the Tauri IPC layer
/// calls. Every arm here must correspond to an entry in the
/// `tauri::generate_handler!` list in `lib.rs::run()`; a command registered
/// there but missing here works in the desktop build and returns
/// `"unknown host command"` only in backend-only mode. The
/// `dispatch_covers_every_tauri_command` test enforces that parity so the
/// drift is caught at `cargo test`, not at runtime.
async fn dispatch(
    host: &Arc<Host>,
    events: &Arc<RemoteEventHub>,
    command: &str,
    arguments: Map<String, Value>,
) -> Result<Value, String> {
    macro_rules! state {
        () => {
            HostState::direct(host)
        };
    }
    macro_rules! done {
        ($result:expr) => {
            output($result?)
        };
    }

    match command {
        "bootstrap_startup_apps" => done!(crate::bootstrap_startup_apps(state!()).await),
        "attach_chat_artifact" => {
            done!(
                crate::attach_chat_artifact(
                    state!(),
                    argument(&arguments, "threadId")?,
                    argument(&arguments, "artifactId")?,
                    argument(&arguments, "title")?
                )
                .await
            )
        }
        "list_chat_threads" => done!(crate::list_chat_threads(state!())),
        "get_chat_thread" => done!(crate::get_chat_thread(
            state!(),
            argument(&arguments, "threadId")?
        )),
        "get_chat_prompt_preview" => done!(
            crate::get_chat_prompt_preview(
                state!(),
                optional_argument(&arguments, "candidateConfig")?,
                optional_argument(&arguments, "threadId")?
            )
            .await
        ),
        "list_chat_profiles" => done!(crate::list_chat_profiles(state!()).await),
        "list_chat_model_profiles" => done!(crate::list_chat_model_profiles(state!()).await),
        "list_chat_agent_engines" => done!(crate::list_chat_agent_engines(state!()).await),
        "set_chat_model_profile" => {
            done!(
                crate::set_chat_model_profile(
                    state!(),
                    argument(&arguments, "threadId")?,
                    optional_argument(&arguments, "profileRef")?
                )
                .await
            )
        }
        "set_chat_agent_engine" => {
            done!(
                crate::set_chat_agent_engine(
                    state!(),
                    argument(&arguments, "threadId")?,
                    optional_argument(&arguments, "appId")?
                )
                .await
            )
        }
        "set_chat_thread_profile" => {
            done!(
                crate::set_chat_thread_profile(
                    state!(),
                    argument(&arguments, "threadId")?,
                    argument(&arguments, "appId")?,
                    argument(&arguments, "profileName")?
                )
                .await
            )
        }
        "remove_chat_contribution" => {
            done!(
                crate::remove_chat_contribution(
                    state!(),
                    argument(&arguments, "threadId")?,
                    argument(&arguments, "sourceAppId")?,
                    argument(&arguments, "kind")?,
                    argument(&arguments, "itemId")?
                )
                .await
            )
        }
        "create_chat_thread" => done!(crate::create_chat_thread(state!()).await),
        "rename_chat_thread" => done!(crate::rename_chat_thread(
            state!(),
            argument(&arguments, "threadId")?,
            argument(&arguments, "title")?
        )),
        "delete_chat_thread" => {
            done!(crate::delete_chat_thread(state!(), argument(&arguments, "threadId")?).await)
        }
        "send_chat_message" => {
            let request_id: String = argument(&arguments, "requestId")?;
            let event_name = format!("chat-stream:{request_id}");
            let event_hub = events.clone();
            let progress = Arc::new(move |event: ChatStreamEvent| {
                let _ = event_hub.publish(&event_name, &event);
            });
            done!(
                crate::send_chat_message_with_progress(
                    state!(),
                    argument(&arguments, "threadId")?,
                    argument(&arguments, "message")?,
                    request_id,
                    progress
                )
                .await
            )
        }
        "cancel_chat_message" => {
            done!(crate::cancel_chat_message(state!(), argument(&arguments, "threadId")?).await)
        }
        "start_llm_oauth" => {
            done!(crate::start_llm_oauth(state!(), argument(&arguments, "connectorId")?).await)
        }
        "resolve_llm_oauth_prompt" => done!(crate::resolve_llm_oauth_prompt(
            state!(),
            argument(&arguments, "sessionId")?,
            argument(&arguments, "promptId")?,
            optional_argument(&arguments, "value")?,
            argument(&arguments, "cancelled")?
        )),
        "cancel_llm_oauth" => done!(crate::cancel_llm_oauth(
            state!(),
            argument(&arguments, "sessionId")?
        )),
        "list_apps" => done!(crate::list_apps(state!())),
        "list_installed_apps" => done!(crate::list_installed_apps(state!())),
        "list_publisher_trust" => done!(crate::list_publisher_trust(state!())),
        "list_managed_app_revisions" => done!(crate::list_managed_app_revisions(
            state!(),
            argument(&arguments, "appId")?
        )),
        "inspect_package" => done!(crate::inspect_package(
            state!(),
            argument(&arguments, "packageDir")?
        )),
        "inspect_git_package" => {
            done!(crate::inspect_git_package(state!(), argument(&arguments, "gitUrl")?).await)
        }
        "plan_managed_app_transition" => done!(crate::plan_managed_app_transition(
            state!(),
            argument(&arguments, "request")?
        )),
        "apply_managed_app_transition" => done!(
            crate::apply_managed_app_transition(state!(), argument(&arguments, "transitionId")?)
                .await
        ),
        "install_app" => done!(
            crate::install_app(
                state!(),
                argument(&arguments, "stagedId")?,
                argument(&arguments, "packageDigest")?
            )
            .await
        ),
        "set_app_enabled" => done!(
            crate::set_app_enabled(
                state!(),
                argument(&arguments, "appId")?,
                argument(&arguments, "enabled")?
            )
            .await
        ),
        "trust_publisher_key" => done!(crate::trust_publisher_key(
            state!(),
            argument(&arguments, "request")?
        )),
        "revoke_publisher_key" => done!(crate::revoke_publisher_key(
            state!(),
            argument(&arguments, "request")?
        )),
        "uninstall_app" => done!(
            crate::uninstall_app(
                state!(),
                argument(&arguments, "appId")?,
                argument(&arguments, "purgeSecrets")?,
                argument(&arguments, "purgeData")?
            )
            .await
        ),
        "get_host_config" => done!(crate::get_host_config(state!())),
        "get_config_storage_info" => done!(crate::get_config_storage_info(state!())),
        "get_active_kestral_profile" => done!(crate::get_active_kestral_profile(state!())),
        "request_system_reset" => {
            let confirmation: String = argument(&arguments, "confirmation")?;
            crate::system_reset::stage(&host.paths, &confirmation)?;
            done!(Ok::<_, String>(crate::SystemResetRequestResult {
                restart_required: true,
            }))
        }
        "update_host_config" => {
            done!(crate::update_host_config(state!(), argument(&arguments, "patch")?).await)
        }
        "get_app_config" => done!(crate::get_app_config(
            state!(),
            argument(&arguments, "appId")?
        )),
        "update_app_config" => done!(
            crate::update_app_config(
                state!(),
                argument(&arguments, "appId")?,
                argument(&arguments, "config")?
            )
            .await
        ),
        "list_connector_configs" => done!(crate::list_connector_configs(state!())),
        "upsert_connector_config" => done!(
            crate::upsert_connector_config(
                state!(),
                argument(&arguments, "connector")?,
                argument(&arguments, "acknowledgeDataEgress")?
            )
            .await
        ),
        "delete_connector_config" => done!(crate::delete_connector_config(
            state!(),
            argument(&arguments, "connectorId")?
        )),
        "put_secret" => done!(
            crate::put_secret(
                state!(),
                argument(&arguments, "owner")?,
                argument(&arguments, "secretName")?,
                argument(&arguments, "value")?
            )
            .await
        ),
        "clear_secret" => done!(
            crate::clear_secret(
                state!(),
                argument(&arguments, "owner")?,
                argument(&arguments, "secretName")?
            )
            .await
        ),
        "has_secret" => done!(crate::has_secret(
            state!(),
            argument(&arguments, "owner")?,
            argument(&arguments, "secretName")?
        )),
        "list_file_resources" => done!(crate::list_file_resources(state!())),
        "list_kestral_profiles" => done!(crate::list_kestral_profiles(state!())),
        "list_trusted_file_resources" => done!(crate::list_trusted_file_resources(state!())),
        "register_file_resource" => {
            done!(crate::register_file_resource(state!(), argument(&arguments, "path")?).await)
        }
        "remove_file_resource" => {
            done!(crate::remove_file_resource(state!(), argument(&arguments, "resourceId")?).await)
        }
        "grant_file_resource_access" => done!(
            crate::grant_file_resource_access(
                state!(),
                argument(&arguments, "holder")?,
                argument(&arguments, "resourceId")?,
                argument(&arguments, "operations")?
            )
            .await
        ),
        "grant_artifact_access" => done!(
            crate::grant_artifact_access(
                state!(),
                argument(&arguments, "holder")?,
                argument(&arguments, "target")?
            )
            .await
        ),
        "test_connector_config" => done!(
            crate::test_connector_config(state!(), argument(&arguments, "connectorId")?).await
        ),
        "discover_connector_models_draft" => done!(
            crate::discover_connector_models_draft(
                state!(),
                argument(&arguments, "kind")?,
                argument(&arguments, "baseUrl")?,
                optional_argument(&arguments, "defaultModel")?,
                optional_argument(&arguments, "apiKeySecretName")?
            )
            .await
        ),
        "list_mcp_servers" => done!(crate::list_mcp_servers(state!())),
        "upsert_mcp_server" => done!(crate::upsert_mcp_server(
            state!(),
            argument(&arguments, "server")?
        )),
        "delete_mcp_server" => done!(crate::delete_mcp_server(
            state!(),
            argument(&arguments, "serverId")?
        )),
        "put_mcp_http_auth_secret" => done!(crate::put_mcp_http_auth_secret(
            state!(),
            argument(&arguments, "serverId")?,
            argument(&arguments, "value")?
        )),
        "clear_mcp_http_auth_secret" => done!(crate::clear_mcp_http_auth_secret(
            state!(),
            argument(&arguments, "serverId")?
        )),
        "has_mcp_http_auth_secret" => done!(crate::has_mcp_http_auth_secret(
            state!(),
            argument(&arguments, "serverId")?
        )),
        "connect_mcp_server" => {
            done!(crate::connect_mcp_server(state!(), argument(&arguments, "serverId")?).await)
        }
        "disconnect_mcp_server" => {
            done!(crate::disconnect_mcp_server(state!(), argument(&arguments, "serverId")?).await)
        }
        "list_mcp_export_profiles" => done!(crate::list_mcp_export_profiles(state!())),
        "upsert_mcp_export_profile" => done!(
            crate::upsert_mcp_export_profile(state!(), argument(&arguments, "profile")?).await
        ),
        "set_mcp_export_enabled" => done!(
            crate::set_mcp_export_enabled(
                state!(),
                argument(&arguments, "profileId")?,
                argument(&arguments, "enabled")?
            )
            .await
        ),
        "delete_mcp_export_profile" => done!(
            crate::delete_mcp_export_profile(state!(), argument(&arguments, "profileId")?).await
        ),
        "rotate_mcp_export_token" => done!(crate::rotate_mcp_export_token(
            state!(),
            argument(&arguments, "profileId")?
        )),
        "revoke_mcp_export_token" => done!(crate::revoke_mcp_export_token(
            state!(),
            argument(&arguments, "profileId")?
        )),
        "has_mcp_export_token" => done!(crate::has_mcp_export_token(
            state!(),
            argument(&arguments, "profileId")?
        )),
        "start_mcp_gateway" => done!(crate::start_mcp_gateway(state!())),
        "stop_mcp_gateway" => done!(crate::stop_mcp_gateway(state!())),
        "mcp_gateway_status" => done!(crate::mcp_gateway_status(state!())),
        "mcp_export_recent_activity" => output(crate::mcp_export_recent_activity(state!())),
        "available_capabilities_for" => {
            done!(crate::available_capabilities_for(state!(), argument(&arguments, "appId")?).await)
        }
        "validate_extension_context" => done!(
            crate::validate_extension_context(
                state!(),
                argument(&arguments, "targetApp")?,
                argument(&arguments, "extensionPoint")?,
                argument(&arguments, "context")?
            )
            .await
        ),
        "list_grants" => done!(crate::list_grants(state!())),
        "ledger_records" => done!(crate::ledger_records(state!())),
        "list_artifacts" => done!(crate::list_artifacts(state!())),
        "list_app_artifacts" => done!(crate::list_app_artifacts(
            state!(),
            argument(&arguments, "appId")?
        )),
        "app_surface_events" => done!(crate::app_surface_events(
            state!(),
            argument(&arguments, "appId")?
        )),
        "get_surface_ui" => done!(crate::get_surface_ui(
            state!(),
            argument(&arguments, "appId")?,
            argument(&arguments, "surface")?,
            true
        )),
        "get_surface_state" => done!(
            crate::get_surface_state(
                state!(),
                argument(&arguments, "binding")?,
                argument(&arguments, "key")?
            )
            .await
        ),
        "put_surface_state" => done!(
            crate::put_surface_state(
                state!(),
                argument(&arguments, "binding")?,
                argument(&arguments, "key")?,
                argument(&arguments, "expectedRevision")?,
                optional_argument(&arguments, "value")?
            )
            .await
        ),
        "managed_data_request" => done!(
            crate::managed_data_request(
                state!(),
                argument(&arguments, "binding")?,
                argument(&arguments, "request")?
            )
            .await
        ),
        "open_surface" => done!(
            crate::open_surface(
                state!(),
                argument(&arguments, "appId")?,
                argument(&arguments, "surface")?
            )
            .await
        ),
        "close_surface" => {
            done!(crate::close_surface(state!(), argument(&arguments, "binding")?).await)
        }
        "submit_action" => done!(
            crate::submit_action(
                state!(),
                argument(&arguments, "binding")?,
                argument(&arguments, "intent")?
            )
            .await
        ),
        "submit_action_with_progress" => {
            let request_id: String = argument(&arguments, "requestId")?;
            let event_name = format!("host-progress:submit_action_with_progress:{request_id}");
            let event_hub = events.clone();
            let progress = app_host_kernel::ProgressReporter::new_checked(move |value| {
                event_hub.publish(&event_name, &value).map_err(|_| ())
            });
            done!(
                crate::submit_action_inner(
                    host.clone(),
                    argument(&arguments, "binding")?,
                    argument(&arguments, "intent")?,
                    progress
                )
                .await
            )
        }
        "cancel_surface_action" => done!(
            crate::cancel_surface_action(
                state!(),
                argument(&arguments, "binding")?,
                argument(&arguments, "runId")?
            )
            .await
        ),
        "revoke_grant" => {
            done!(crate::revoke_grant(state!(), argument(&arguments, "grantId")?).await)
        }
        "request_app_grants" => {
            done!(crate::request_app_grants(state!(), argument(&arguments, "appId")?).await)
        }
        "request_manifest_grant" => done!(
            crate::request_manifest_grant(
                state!(),
                argument(&arguments, "appId")?,
                argument(&arguments, "request")?
            )
            .await
        ),
        "submit_permission_proposal" => done!(
            crate::submit_permission_proposal(state!(), argument(&arguments, "artifactId")?).await
        ),
        "create_kestral_profile" => done!(crate::create_kestral_profile(
            state!(),
            argument(&arguments, "request")?
        )),
        "issue_editor_grant" => {
            done!(crate::issue_editor_grant(state!(), argument(&arguments, "request")?).await)
        }
        "replace_grant" => done!(
            crate::replace_grant(
                state!(),
                argument(&arguments, "grantId")?,
                argument(&arguments, "request")?
            )
            .await
        ),
        "list_trusted_notices" => done!(crate::list_trusted_notices(state!())),
        "delete_kestral_profile" => done!(crate::delete_kestral_profile(
            state!(),
            argument(&arguments, "profileId")?
        )),
        "resolve_approval" => done!(crate::resolve_approval(
            state!(),
            argument(&arguments, "requestId")?,
            argument(&arguments, "approved")?
        )),
        "resolve_install_approval" => done!(crate::resolve_install_approval(
            state!(),
            argument(&arguments, "requestId")?,
            optional_argument(&arguments, "eventApproved")?,
            argument(&arguments, "grantApprovals")?
        )),
        _ => Err(format!("unknown host command '{command}'")),
    }
}

struct ServerState {
    host: Arc<Host>,
    events: Arc<RemoteEventHub>,
    auth: Mutex<RemoteOwnerAuth>,
    auth_revision: tokio::sync::watch::Sender<u64>,
    allowed_origin: String,
    active_commands: AtomicUsize,
}

struct ActiveCommandGuard<'a>(&'a AtomicUsize);

impl Drop for ActiveCommandGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

fn state_change_scopes(command: &str) -> Option<&'static [&'static str]> {
    match command {
        "bootstrap_startup_apps"
        | "apply_managed_app_transition"
        | "install_app"
        | "set_app_enabled"
        | "uninstall_app"
        | "connect_mcp_server"
        | "disconnect_mcp_server" => {
            Some(&["apps", "artifacts", "chat", "config", "grants", "records"])
        }
        "create_chat_thread"
        | "rename_chat_thread"
        | "delete_chat_thread"
        | "attach_chat_artifact"
        | "set_chat_model_profile"
        | "set_chat_thread_profile"
        | "set_chat_agent_engine"
        | "remove_chat_contribution" => Some(&["chat"]),
        "send_chat_message" | "cancel_chat_message" => Some(&["artifacts", "chat", "records"]),
        "update_host_config"
        | "update_app_config"
        | "upsert_connector_config"
        | "delete_connector_config"
        | "put_secret"
        | "clear_secret"
        | "upsert_mcp_server"
        | "delete_mcp_server"
        | "put_mcp_http_auth_secret"
        | "clear_mcp_http_auth_secret" => Some(&["config"]),
        "register_file_resource" | "remove_file_resource" | "grant_file_resource_access" => {
            Some(&["config", "grants"])
        }
        "grant_artifact_access" => Some(&["grants", "records"]),
        "submit_action" | "submit_action_with_progress" | "cancel_surface_action" => {
            Some(&["artifacts", "records"])
        }
        "revoke_grant"
        | "request_app_grants"
        | "request_manifest_grant"
        | "submit_permission_proposal"
        | "issue_editor_grant"
        | "replace_grant" => Some(&["grants", "records"]),
        "trust_publisher_key" | "revoke_publisher_key" => Some(&["publisher-trust"]),
        "start_mcp_gateway"
        | "stop_mcp_gateway"
        | "upsert_mcp_export_profile"
        | "set_mcp_export_enabled"
        | "delete_mcp_export_profile"
        | "rotate_mcp_export_token"
        | "revoke_mcp_export_token" => Some(&["mcp-export"]),
        "create_kestral_profile" | "delete_kestral_profile" => Some(&["profiles"]),
        _ => None,
    }
}

fn backend_default_root(data_dir: Option<OsString>, current_dir: PathBuf) -> PathBuf {
    data_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| current_dir.join("host-data"))
}

fn json_response(status: u16, value: Value, allowed_origin: Option<&str>) -> Response<Body> {
    json_response_with_headers(status, value, allowed_origin, HeaderMap::new())
}

fn json_response_with_headers(
    status: u16,
    value: Value,
    allowed_origin: Option<&str>,
    mut headers: HeaderMap,
) -> Response<Body> {
    let body = serde_json::to_vec(&value).expect("JSON value serializes");
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(origin) = allowed_origin {
        headers.insert(
            ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_str(origin).expect("validated remote origin is a header value"),
        );
        headers.insert(
            ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
        headers.insert(VARY, HeaderValue::from_static("Origin"));
    }
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = StatusCode::from_u16(status).expect("valid response status");
    *response.headers_mut() = headers;
    response
}

fn request_origin(state: &ServerState, headers: &HeaderMap) -> Result<Option<String>, ()> {
    let origin = match headers.get(ORIGIN) {
        None => None,
        Some(value) => Some(value.to_str().map_err(|_| ())?.to_string()),
    };
    match &origin {
        None => Ok(None),
        Some(origin) if origin == &state.allowed_origin => Ok(Some(origin.clone())),
        _ => Err(()),
    }
}

async fn read_json_body<T: DeserializeOwned>(body: Body) -> Result<T, AuthFailure> {
    let body = to_bytes(body, MAX_REQUEST_BYTES as usize)
        .await
        .map_err(|_| AuthFailure {
            status: 413,
            message: "request body is too large".into(),
        })?;
    serde_json::from_slice(&body)
        .map_err(|error| AuthFailure::bad_request(format!("invalid request body: {error}")))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistrationStartRequest {
    pairing_code: String,
}

fn auth_failure_response(failure: AuthFailure, origin: Option<&str>) -> Response<Body> {
    json_response(failure.status, json!({ "error": failure.message }), origin)
}

fn cookie_header(headers: &HeaderMap) -> Option<&str> {
    headers.get(COOKIE).and_then(|value| value.to_str().ok())
}

fn preflight_response(origin: Option<&str>) -> Response<Body> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    let headers = response.headers_mut();
    headers.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type"),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    if let Some(origin) = origin {
        headers.insert(
            ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_str(origin).expect("validated remote origin is a header value"),
        );
    }
    response
}

fn sse_response(
    state: &Arc<ServerState>,
    headers: &HeaderMap,
    query: Option<&str>,
    origin: Option<&str>,
) -> Response<Body> {
    let query_cursor = query
        .and_then(|query| {
            query
                .split('&')
                .find_map(|part| part.strip_prefix("after="))
        })
        .and_then(|value| value.parse::<u64>().ok());
    let last_event_cursor = headers
        .get(HeaderName::from_static("last-event-id"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let after = last_event_cursor.or(query_cursor).unwrap_or(0);
    let session_cookie = cookie_header(headers).unwrap_or_default().to_string();
    let mut absolute_deadline = match state.auth.lock() {
        Ok(mut auth) => match auth.authenticate_cookie_until(Some(&session_cookie)) {
            Some(deadline) => deadline,
            None => {
                return json_response(
                    401,
                    json!({ "error": "owner session is missing or expired" }),
                    origin,
                );
            }
        },
        Err(_) => {
            return json_response(
                500,
                json!({ "error": "remote owner authentication lock poisoned" }),
                origin,
            );
        }
    };
    let session_state = state.clone();
    let mut auth_revision = state.auth_revision.subscribe();
    let session_ended = async move {
        loop {
            let until_absolute = (absolute_deadline - Utc::now())
                .to_std()
                .unwrap_or(Duration::ZERO);
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(15)) => {}
                changed = auth_revision.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
                _ = tokio::time::sleep(until_absolute) => break,
            }
            let next_deadline = match session_state.auth.lock() {
                Ok(mut auth) => auth.authenticate_cookie_until(Some(&session_cookie)),
                Err(_) => None,
            };
            let Some(next_deadline) = next_deadline else {
                break;
            };
            absolute_deadline = next_deadline;
        }
    };
    let stream = event_stream(state.events.clone(), after).take_until(session_ended);
    let mut response = Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    if let Some(origin) = origin {
        response.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_str(origin).expect("validated remote origin is a header value"),
        );
        response.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
        response
            .headers_mut()
            .insert(VARY, HeaderValue::from_static("Origin"));
    }
    response
}

async fn handle_request(
    State(state): State<Arc<ServerState>>,
    request: Request<Body>,
) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let origin = match request_origin(&state, &parts.headers) {
        Ok(origin) => origin,
        Err(()) => {
            return json_response(403, json!({ "error": "origin not allowed" }), None);
        }
    };
    if parts.method == Method::OPTIONS {
        return preflight_response(origin.as_deref());
    }

    let path = parts.uri.path();
    let cookie = cookie_header(&parts.headers);
    if parts.method == Method::GET && path == "/api/auth/status" {
        let result = state
            .auth
            .lock()
            .map(|mut auth| {
                let authenticated = auth.authenticate_cookie(cookie);
                json!({ "paired": auth.is_paired(), "authenticated": authenticated })
            })
            .map_err(|_| AuthFailure {
                status: 500,
                message: "remote owner authentication lock poisoned".into(),
            });
        return match result {
            Ok(value) => json_response(200, value, origin.as_deref()),
            Err(failure) => auth_failure_response(failure, origin.as_deref()),
        };
    }
    if path.starts_with("/api/auth/") && parts.method != Method::POST {
        return json_response(
            405,
            json!({ "error": "method not allowed" }),
            origin.as_deref(),
        );
    }
    if parts.method == Method::POST && path.starts_with("/api/auth/") {
        if origin.is_none() {
            return json_response(403, json!({ "error": "browser origin required" }), None);
        }
        return match path {
            "/api/auth/register/start" => {
                let input = match read_json_body::<RegistrationStartRequest>(body).await {
                    Ok(input) => input,
                    Err(failure) => return auth_failure_response(failure, origin.as_deref()),
                };
                let result = state
                    .auth
                    .lock()
                    .map_err(|_| AuthFailure {
                        status: 500,
                        message: "remote owner authentication lock poisoned".into(),
                    })
                    .and_then(|mut auth| auth.start_registration(&input.pairing_code));
                match result {
                    Ok(start) => json_response(200, json!(start), origin.as_deref()),
                    Err(failure) => auth_failure_response(failure, origin.as_deref()),
                }
            }
            "/api/auth/register/finish" => {
                let input = match read_json_body::<RegistrationFinish>(body).await {
                    Ok(input) => input,
                    Err(failure) => return auth_failure_response(failure, origin.as_deref()),
                };
                let result = state
                    .auth
                    .lock()
                    .map_err(|_| AuthFailure {
                        status: 500,
                        message: "remote owner authentication lock poisoned".into(),
                    })
                    .and_then(|mut auth| {
                        let token = auth.finish_registration(input)?;
                        Ok(auth.session_cookie(&token))
                    });
                match result {
                    Ok(cookie) => {
                        state.auth_revision.send_modify(|revision| *revision += 1);
                        let mut headers = HeaderMap::new();
                        headers.insert(
                            SET_COOKIE,
                            HeaderValue::from_str(&cookie)
                                .expect("session cookie is a header value"),
                        );
                        json_response_with_headers(
                            200,
                            json!({ "authenticated": true }),
                            origin.as_deref(),
                            headers,
                        )
                    }
                    Err(failure) => auth_failure_response(failure, origin.as_deref()),
                }
            }
            "/api/auth/login/start" => {
                let result = state
                    .auth
                    .lock()
                    .map_err(|_| AuthFailure {
                        status: 500,
                        message: "remote owner authentication lock poisoned".into(),
                    })
                    .and_then(|mut auth| auth.start_authentication());
                match result {
                    Ok(start) => json_response(200, json!(start), origin.as_deref()),
                    Err(failure) => auth_failure_response(failure, origin.as_deref()),
                }
            }
            "/api/auth/login/finish" => {
                let input = match read_json_body::<AuthenticationFinish>(body).await {
                    Ok(input) => input,
                    Err(failure) => return auth_failure_response(failure, origin.as_deref()),
                };
                let result = state
                    .auth
                    .lock()
                    .map_err(|_| AuthFailure {
                        status: 500,
                        message: "remote owner authentication lock poisoned".into(),
                    })
                    .and_then(|mut auth| {
                        let token = auth.finish_authentication(input)?;
                        Ok(auth.session_cookie(&token))
                    });
                match result {
                    Ok(cookie) => {
                        state.auth_revision.send_modify(|revision| *revision += 1);
                        let mut headers = HeaderMap::new();
                        headers.insert(
                            SET_COOKIE,
                            HeaderValue::from_str(&cookie)
                                .expect("session cookie is a header value"),
                        );
                        json_response_with_headers(
                            200,
                            json!({ "authenticated": true }),
                            origin.as_deref(),
                            headers,
                        )
                    }
                    Err(failure) => auth_failure_response(failure, origin.as_deref()),
                }
            }
            "/api/auth/logout" => {
                let result = state.auth.lock().map(|mut auth| {
                    auth.logout_cookie(cookie);
                    auth.clear_session_cookie()
                });
                match result {
                    Ok(cookie) => {
                        state.auth_revision.send_modify(|revision| *revision += 1);
                        let mut headers = HeaderMap::new();
                        headers.insert(
                            SET_COOKIE,
                            HeaderValue::from_str(&cookie)
                                .expect("session cookie is a header value"),
                        );
                        json_response_with_headers(
                            200,
                            json!({ "authenticated": false }),
                            origin.as_deref(),
                            headers,
                        )
                    }
                    Err(_) => json_response(
                        500,
                        json!({ "error": "remote owner authentication lock poisoned" }),
                        origin.as_deref(),
                    ),
                }
            }
            _ => json_response(404, json!({ "error": "not found" }), origin.as_deref()),
        };
    }

    let authenticated = match state.auth.lock() {
        Ok(mut auth) => Ok(auth.authenticate_cookie(cookie)),
        Err(_) => Err(()),
    };
    match authenticated {
        Ok(true) => {}
        Ok(false) => {
            return json_response(
                401,
                json!({ "error": "owner session is missing or expired" }),
                origin.as_deref(),
            );
        }
        Err(_) => {
            return json_response(
                500,
                json!({ "error": "remote owner authentication lock poisoned" }),
                origin.as_deref(),
            );
        }
    }

    if parts.method == Method::GET && path == "/api/health" {
        return json_response(200, json!({ "ok": true }), origin.as_deref());
    }
    if parts.method == Method::GET {
        if let Some(route_token) = path.strip_prefix("/api/surfaces/") {
            let document = match state.host.surface_ui.lock() {
                Ok(registry) => registry.document(route_token),
                Err(_) => {
                    return Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
                        .header(CACHE_CONTROL, "no-store")
                        .body(Body::from("surface UI registry lock poisoned"))
                        .expect("static surface error response is valid");
                }
            };
            let Some(document) = document else {
                return Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header(CONTENT_TYPE, "text/plain; charset=utf-8")
                    .header(CACHE_CONTROL, "no-store")
                    .body(Body::from("surface document not found"))
                    .expect("static surface error response is valid");
            };
            return Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "text/html; charset=utf-8")
                .header(CACHE_CONTROL, "no-store")
                .header(
                    "Content-Security-Policy",
                    format!("{}; frame-ancestors {}", document.csp, state.allowed_origin),
                )
                .header("Referrer-Policy", "no-referrer")
                .header("X-Content-Type-Options", "nosniff")
                .body(Body::from(document.html))
                .expect("host-authored surface response headers are valid");
        }
    }
    if parts.method == Method::GET && path == "/api/approvals" {
        return json_response(
            200,
            json!(RemoteApprovalBatch {
                instance_id: state.events.instance_id.clone(),
                requests: state.host.pending.pending_requests(),
            }),
            origin.as_deref(),
        );
    }
    if parts.method == Method::GET && path == "/api/events" {
        let after = parts
            .uri
            .query()
            .and_then(|query| {
                query
                    .split('&')
                    .find_map(|part| part.strip_prefix("after="))
            })
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        return match state.events.since(after) {
            Ok(events) => json_response(200, json!(events), origin.as_deref()),
            Err(error) => json_response(500, json!({ "error": error }), origin.as_deref()),
        };
    }
    if parts.method == Method::GET && path == "/api/events/stream" {
        return sse_response(&state, &parts.headers, parts.uri.query(), origin.as_deref());
    }
    let Some(command) = path.strip_prefix("/api/invoke/") else {
        return json_response(404, json!({ "error": "not found" }), origin.as_deref());
    };
    if parts.method != Method::POST {
        return json_response(
            405,
            json!({ "error": "method not allowed" }),
            origin.as_deref(),
        );
    }
    let _active_command = if matches!(
        command,
        "resolve_approval"
            | "resolve_install_approval"
            | "cancel_chat_message"
            | "resolve_llm_oauth_prompt"
            | "cancel_llm_oauth"
    ) {
        None
    } else {
        if state.active_commands.fetch_add(1, Ordering::Acquire) >= MAX_CONCURRENT_COMMANDS {
            state.active_commands.fetch_sub(1, Ordering::Release);
            return json_response(
                429,
                json!({ "error": "too many host commands are already running" }),
                origin.as_deref(),
            );
        }
        Some(ActiveCommandGuard(&state.active_commands))
    };
    let arguments = match read_json_body::<Map<String, Value>>(body).await {
        Ok(arguments) => arguments,
        Err(failure) => return auth_failure_response(failure, origin.as_deref()),
    };
    let result = dispatch(&state.host, &state.events, command, arguments).await;
    match result {
        Ok(value) => {
            if let Some(scopes) = state_change_scopes(command) {
                let _ = state
                    .events
                    .publish(HOST_STATE_CHANGED_EVENT, &HostStateChanged { scopes });
            }
            json_response(200, value, origin.as_deref())
        }
        Err(error) => json_response(400, json!({ "error": error }), origin.as_deref()),
    }
}

pub fn run_from_env() -> Result<(), String> {
    let bind = std::env::var("HOST_REMOTE_BIND").unwrap_or_else(|_| DEFAULT_BIND.into());
    let requested: SocketAddr = bind
        .parse()
        .map_err(|_| format!("invalid HOST_REMOTE_BIND address: {bind}"))?;
    if !requested.ip().is_loopback()
        && !std::env::var("HOST_REMOTE_ALLOW_INSECURE_HTTP")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    {
        return Err(
            "non-loopback HTTP requires TLS termination; set HOST_REMOTE_ALLOW_INSECURE_HTTP=true only on a trusted private network"
                .into(),
        );
    }
    let current_dir = std::env::current_dir().map_err(|error| error.to_string())?;
    let default_root = backend_default_root(std::env::var_os("KESTRAL_DATA_DIR"), current_dir);
    let registry_lock = crate::kernel_state::ProfileRegistryLock::acquire(&default_root)?;
    let host_paths = HostPaths::resolve_startup(default_root)?;
    let profile_lock = crate::kernel_state::ProfileLock::acquire_for_startup(
        host_paths.kernel_state_path(),
        registry_lock,
    )?;
    crate::profile_migration::run(&host_paths)?;
    crate::system_reset::apply_pending(&host_paths)?;
    if std::env::var_os("KESTRAL_WORKER_RESOURCE_DIR").is_none() {
        if let Some(resource_dir) = std::env::var_os("HOST_RESOURCE_DIR") {
            std::env::set_var("KESTRAL_WORKER_RESOURCE_DIR", resource_dir);
        }
    }
    let allowed_origin = std::env::var("HOST_REMOTE_ORIGIN")
        .map_err(|_| "HOST_REMOTE_ORIGIN is required for backend-only mode".to_string())?;
    let auth = RemoteOwnerAuth::open(
        host_paths.remote_owner_auth_path(),
        host_paths.remote_owner_pairing_path(),
        &allowed_origin,
        std::env::var("HOST_REMOTE_RP_ID").ok().as_deref(),
    )?;
    let allowed_origin = auth.origin().to_string();
    let pending = Arc::new(PendingApprovals::default());
    let notices = Arc::new(Mutex::new(
        TrustedNoticeStore::new(host_paths.notices_path().to_path_buf())
            .map_err(|error| error.to_string())?,
    ));
    let events = Arc::new(RemoteEventHub::default());
    let chrome = Arc::new(RemoteChrome {
        pending: pending.clone(),
        notices: notices.clone(),
        events: events.clone(),
        next_request_id: AtomicU64::new(0),
    });
    let host = crate::build_host_with_lock(host_paths, profile_lock, chrome, pending, notices)?;
    let oauth_events = events.clone();
    host.oauth.set_publisher(Arc::new(move |event| {
        oauth_events.publish(CHROME_OAUTH_EVENT, event)
    }))?;
    let state = Arc::new(ServerState {
        host,
        events,
        auth: Mutex::new(auth),
        auth_revision: tokio::sync::watch::channel(0).0,
        allowed_origin,
        active_commands: AtomicUsize::new(0),
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("build remote API runtime failed: {error}"))?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(requested)
            .await
            .map_err(|error| format!("remote API failed to bind {bind}: {error}"))?;
        let local_addr = listener
            .local_addr()
            .map_err(|error| format!("read remote API listen address failed: {error}"))?;
        eprintln!("Kestral backend listening on http://{local_addr}");
        let app = Router::new().fallback(handle_request).with_state(state);
        axum::serve(listener, app)
            .await
            .map_err(|error| format!("remote API server failed: {error}"))
    })
}

pub fn create_owner_pairing_code_from_env() -> Result<(), String> {
    let current_dir = std::env::current_dir().map_err(|error| error.to_string())?;
    let default_root = backend_default_root(std::env::var_os("KESTRAL_DATA_DIR"), current_dir);
    let host_paths = HostPaths::resolve_startup(default_root)?;
    let code = crate::remote_auth::create_pairing_code(&host_paths.remote_owner_pairing_path())?;
    println!("Kestral owner pairing code (valid for 10 minutes): {code}");
    Ok(())
}

pub fn reset_owner_authentication_from_env() -> Result<(), String> {
    let current_dir = std::env::current_dir().map_err(|error| error.to_string())?;
    let default_root = backend_default_root(std::env::var_os("KESTRAL_DATA_DIR"), current_dir);
    let registry_lock =
        crate::kernel_state::ProfileRegistryLock::acquire(&default_root).map_err(|error| {
            format!("stop the backend before resetting owner authentication: {error}")
        })?;
    let host_paths = HostPaths::resolve_startup(default_root)?;
    let _profile_lock = crate::kernel_state::ProfileLock::acquire_for_startup(
        host_paths.kernel_state_path(),
        registry_lock,
    )
    .map_err(|error| format!("stop the backend before resetting owner authentication: {error}"))?;
    crate::profile_migration::run(&host_paths)?;
    let auth_path = host_paths.remote_owner_auth_path();
    if !auth_path.exists() {
        return Err("no remote owner passkeys are registered for this profile".into());
    }
    let pairing_path = host_paths.remote_owner_pairing_path();
    match std::fs::remove_file(pairing_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("remove remote owner pairing code failed: {error}")),
    }
    std::fs::remove_file(&auth_path)
        .map_err(|error| format!("remove remote owner passkeys failed: {error}"))?;
    println!("Removed every remote owner passkey. Start the backend and pair a new browser.");
    Ok(())
}

#[cfg(test)]
mod tests;
