use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_host_kernel::ids::{AppId, CapabilityName, ResourceId, RunId};
use app_host_kernel::invocation::RefusalReason;
use app_host_kernel::primitives::capability::CapabilityRef;
use app_host_kernel::primitives::grant::{DataScope, DenialReason, GrantStatus};
use app_host_kernel::primitives::run::{InvocationRecord, RunTerminalState};
use app_host_kernel::services::ledger::{LedgerEvent, LedgerRecord};
use sha2::Digest;

use crate::chat_app;
use crate::chat_store::{
    AuthorizedChatInjectedContext, ChatAgentEngineState, ChatCompositionReceipt, ChatContribution,
    ChatInjectedContext, ChatMessage, ChatMessageRole, ChatMessageStatus, ChatPromptReceiptLayer,
    ChatRequestState, ContributionIdentity,
};
use crate::{
    with_kernel_blocking, ActiveChatSend, ActiveChatSendGuard, ChatStreamEvent, Host,
    SendChatMessageResult,
};

pub(crate) async fn send_chat_message_with_progress(
    host: Arc<Host>,
    thread_id: String,
    message: String,
    request_id: String,
    on_event: Arc<dyn Fn(ChatStreamEvent) + Send + Sync>,
) -> Result<SendChatMessageResult, String> {
    let trimmed = message.trim().to_string();
    if trimmed.is_empty() {
        return Err("chat message must not be empty".into());
    }
    if request_id.trim().is_empty() {
        return Err("chat request id must not be empty".into());
    }
    if request_id.len() > 128 {
        return Err("chat request id must be at most 128 characters".into());
    }

    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut sends = host
            .active_chat_sends
            .lock()
            .map_err(|_| "chat execution lock poisoned".to_string())?;
        let store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        match store.request_state(&thread_id, &request_id, &trimmed)? {
            Some(ChatRequestState::Completed) => {
                return Ok(SendChatMessageResult {
                    thread: store.get_thread(&thread_id)?,
                });
            }
            Some(ChatRequestState::Pending) => {
                if let Some(active) = sends.get(&thread_id) {
                    if active.request_id == request_id && active.message == trimmed {
                        return Err("this chat request is still running".into());
                    }
                }
                return Err(
                    "this chat request was interrupted; it was not replayed because external effects may already have occurred"
                        .into(),
                );
            }
            None => {
                let thread = store.get_thread(&thread_id)?;
                if thread.messages().iter().any(|message| {
                    message.client_request_id.is_some()
                        && message.status == Some(ChatMessageStatus::Pending)
                }) {
                    return Err(if sends.contains_key(&thread_id) {
                        "a message is already running for this chat".into()
                    } else {
                        "this chat has an interrupted request; start a new chat to avoid repeating possible external effects"
                            .into()
                    });
                }
            }
        }
        if sends.contains_key(&thread_id) {
            return Err("a message is already running for this chat".into());
        }
        sends.insert(
            thread_id.clone(),
            ActiveChatSend {
                cancelled: cancelled.clone(),
                run_ids: vec![],
                request_id: request_id.clone(),
                message: trimmed.clone(),
            },
        );
    }
    let _send_guard = ActiveChatSendGuard {
        sends: host.active_chat_sends.clone(),
        thread_id: thread_id.clone(),
    };

    let (pinned_model_profile, selected_agent_engine_receipt) = {
        let store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        let thread = store.get_thread(&thread_id)?;
        (
            thread.model_profile_receipt,
            thread.chat_agent_engine_receipt,
        )
    };
    let model_profile_source_version = if let Some(receipt) = &pinned_model_profile {
        let source_app_id = receipt.source_app_id.clone();
        with_kernel_blocking(host.clone(), move |kernel| {
            Ok(
                crate::chat_model_profiles::model_profile_source(kernel, &source_app_id)
                    .ok()
                    .map(|app| app.manifest.version.clone()),
            )
        })
        .await?
    } else {
        None
    };
    let (
        max_iterations,
        agent_timeout_secs,
        model_settings,
        prompt_config,
        runtime_input,
        selected_prompt,
        model_profile_stale,
    ) = {
        let config = host
            .config
            .lock()
            .map_err(|_| "config lock poisoned".to_string())?;
        let max_iterations = config
            .get_app_config("chat")
            .get("max_iterations")
            .and_then(|value| value.as_u64())
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(chat_app::DEFAULT_MAX_LLM_ITERATIONS);
        let configured_agent_timeout_secs = selected_agent_engine_receipt
            .as_ref()
            .and_then(|receipt| {
                config
                    .get_app_config(&receipt.app_id)
                    .get("max_duration_secs")
                    .and_then(|value| value.as_u64())
            })
            .unwrap_or(crate::agent_worker::DEFAULT_MAX_DURATION_SECS);
        if !(crate::agent_worker::MIN_AGENT_DURATION_SECS
            ..=crate::agent_worker::MAX_AGENT_DURATION_SECS)
            .contains(&configured_agent_timeout_secs)
        {
            return Err(format!(
                "agent max_duration_secs must be between {} and {}",
                crate::agent_worker::MIN_AGENT_DURATION_SECS,
                crate::agent_worker::MAX_AGENT_DURATION_SECS
            ));
        }
        let agent_timeout_secs = configured_agent_timeout_secs;
        let model_profile_source = pinned_model_profile
            .as_ref()
            .map(|receipt| receipt.source_app_id.as_str());
        let app_config = model_profile_source
            .map(|app_id| config.get_app_config(app_id))
            .unwrap_or_default();
        let selected_is_current = match (&pinned_model_profile, &model_profile_source_version) {
            (Some(receipt), Some(version)) => crate::chat_model_profiles::profile_is_current(
                &app_config,
                &receipt.source_app_id,
                version,
                receipt,
            )?,
            (None, _) => false,
            (Some(_), None) => false,
        };
        let selected = pinned_model_profile
            .clone()
            .filter(|_| selected_is_current)
            .filter(|receipt| {
                config
                    .selectable_chat_llm_profile(&receipt.connector_id)
                    .is_ok()
            });
        let selected_is_usable = selected.is_some();
        let (llm_profile, model_settings) = if let Some(receipt) = selected {
            let llm_profile = config.selectable_chat_llm_profile(&receipt.connector_id)?;
            let allowed_tool_refs = receipt.tool_refs.iter().cloned().collect();
            (
                Some(llm_profile),
                chat_app::ChatModelSettings {
                    provider_profile_ref: Some(receipt.connector_id.clone()),
                    model: Some(receipt.model.clone()),
                    reasoning: receipt.reasoning.clone(),
                    temperature: receipt.temperature,
                    max_output_tokens: receipt.max_output_tokens,
                    allowed_tool_refs: Some(allowed_tool_refs),
                    receipt: Some(receipt),
                },
            )
        } else {
            let llm_profile = config.current_llm_profile()?;
            (
                llm_profile.clone(),
                llm_profile
                    .map(|profile| chat_app::ChatModelSettings {
                        provider_profile_ref: Some(profile.connector_id),
                        ..chat_app::ChatModelSettings::default()
                    })
                    .unwrap_or_default(),
            )
        };
        let prompt_config = chat_app::ChatPromptConfig::parse(&config.get_app_config("chat"))?;
        let selected_prompt = model_settings
            .receipt
            .as_ref()
            .and_then(|receipt| receipt.prompt.as_ref())
            .cloned();
        let runtime_input = chat_app::ChatPromptRuntimeInput {
            host_version: crate::package::HOST_VERSION.into(),
            mode: String::new(),
            model_id: model_settings
                .model
                .clone()
                .or_else(|| {
                    llm_profile
                        .as_ref()
                        .map(|profile| profile.default_model.clone())
                })
                .unwrap_or_default(),
            connector_kind: llm_profile
                .as_ref()
                .map(|profile| profile.kind.as_str().to_string())
                .unwrap_or_default(),
            connector_id: llm_profile
                .as_ref()
                .map(|profile| profile.connector_id.clone())
                .unwrap_or_default(),
            profile_id: llm_profile
                .as_ref()
                .map(|profile| {
                    profile
                        .connector_id
                        .split_once('/')
                        .map(|(_, profile)| profile)
                        .unwrap_or(&profile.connector_id)
                        .to_string()
                })
                .unwrap_or_default(),
        };
        (
            max_iterations,
            agent_timeout_secs,
            model_settings,
            prompt_config,
            runtime_input,
            selected_prompt,
            pinned_model_profile.is_some() && !selected_is_usable,
        )
    };

    if model_profile_stale {
        host.chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?
            .set_model_profile(&thread_id, None, None)?;
    }

    let (transcript, contributions, injected_contexts, thread_resource_id) = {
        let store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        let thread = store.get_thread(&thread_id)?;
        (
            thread.messages(),
            thread.contributions.clone(),
            thread.injected_contexts.clone(),
            thread.resource_id.clone(),
        )
    };
    let (profile_ref, pinned_profile_receipt) = {
        let store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        let thread = store.get_thread(&thread_id)?;
        (
            thread
                .assistant_profile_ref
                .clone()
                .unwrap_or_else(|| "chat/standard".into()),
            thread.assistant_profile_receipt.clone(),
        )
    };
    let profile_ref_for_resolution = profile_ref.clone();
    let resolved_profile = with_kernel_blocking(host.clone(), move |kernel| {
        chat_app::resolve_profile_selection(kernel, &profile_ref_for_resolution)
    })
    .await;
    let selected_is_current = matches!(
        (&resolved_profile, &pinned_profile_receipt),
        (Ok((live, _)), Some(pinned)) if live.digest == pinned.digest
    ) || (profile_ref == "chat/standard" && resolved_profile.is_ok());
    let (assistant_profile_ref, profile_receipt, assistant_profile_skills) = if selected_is_current
    {
        let (receipt, skills) = resolved_profile?;
        (profile_ref, receipt, skills)
    } else {
        let (receipt, skills) = with_kernel_blocking(host.clone(), move |kernel| {
            chat_app::resolve_profile_selection(kernel, "chat/standard")
        })
        .await?;
        let mut store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        store.set_assistant_profile(
            &thread_id,
            Some("chat/standard".into()),
            Some(receipt.clone()),
        )?;
        ("chat/standard".into(), receipt, skills)
    };
    let assistant_profile_digest = profile_receipt.digest.clone();
    let assistant_capability_refs = profile_receipt.capability_refs.clone();
    let (execution_engine, engine_state) = if let Some(receipt) = selected_agent_engine_receipt {
        let app_id = receipt.app_id;
        let resolution = with_kernel_blocking(host.clone(), move |kernel| {
            chat_app::resolve_chat_agent_engine_selection(kernel, &app_id).map(|_| {
                app_host_kernel::primitives::capability::CapabilityRef {
                    provider: app_host_kernel::ids::AppId::new(app_id),
                    capability: app_host_kernel::ids::CapabilityName::new(
                        crate::chat_app::CHAT_AGENT_ENGINE_CONTRACT,
                    ),
                }
            })
        })
        .await;
        match resolution {
            Ok(capability) => (chat_app::ChatExecutionEngine::Selected(capability), None),
            Err(_) => (
                chat_app::ChatExecutionEngine::PlainLlm,
                Some(ChatAgentEngineState {
                    status: "fallback".into(),
                    fallback_reason: Some(
                        "The selected engine is unavailable, incompatible, or no longer granted."
                            .into(),
                    ),
                }),
            ),
        }
    } else {
        (chat_app::ChatExecutionEngine::PlainLlm, None)
    };
    {
        let mut store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        store.set_chat_agent_engine_state(&thread_id, engine_state)?;
    }
    let contribution_receipt = receipt_for_contributions(&contributions)?;
    let authorized_injected_contexts = with_kernel_blocking(host.clone(), move |kernel| {
        authorize_injected_contexts(kernel, &thread_resource_id, injected_contexts)
    })
    .await?;
    let injected_context = chat_app::prepare_injected_context(
        &authorized_injected_contexts,
        prompt_config.records_injected_context(),
    )?;
    let history = {
        let mut store = host
            .chat_store
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        let history = chat_app::conversation_history(&transcript);
        store.append_user_message(&thread_id, trimmed.clone(), request_id.clone())?;
        history
    };

    let prepared_chat = with_kernel_blocking(host.clone(), {
        let history = history.clone();
        let trimmed = trimmed.clone();
        let current_thread_id = thread_id.clone();
        let model_contributions = contribution_receipt.clone();
        move |kernel| {
            chat_app::prepare_chat_message_with_prompt(
                kernel,
                &history,
                &trimmed,
                &current_thread_id,
                assistant_profile_ref.clone(),
                assistant_profile_digest.clone(),
                assistant_capability_refs.clone(),
                assistant_profile_skills.clone(),
                model_contributions,
                injected_context,
                &prompt_config,
                &runtime_input,
                selected_prompt.as_ref(),
                max_iterations,
                Duration::from_secs(agent_timeout_secs),
                model_settings,
                execution_engine.clone(),
            )
        }
    })
    .await;

    let run_outcome = match prepared_chat {
        Err(error) => Err(error),
        Ok(chat_app::ChatStart::Immediate(reply)) => {
            with_kernel_blocking(host.clone(), move |kernel| {
                Ok((
                    build_assistant_messages(kernel.records(), reply, None),
                    None,
                ))
            })
            .await
        }
        Ok(chat_app::ChatStart::Active(session)) => {
            execute_chat_session(
                host.clone(),
                on_event,
                cancelled,
                thread_id.clone(),
                *session,
            )
            .await
        }
    };

    match run_outcome {
        Ok((messages, prompt_receipt)) => {
            let identities: Vec<ContributionIdentity> = contributions
                .iter()
                .map(ContributionIdentity::from)
                .collect();
            let thread = {
                let mut store = host
                    .chat_store
                    .lock()
                    .map_err(|_| "chat store lock poisoned".to_string())?;
                store.complete_request_with_prompt_receipt_and_consumed_contributions(
                    &thread_id,
                    &request_id,
                    messages,
                    prompt_receipt.map(|mut receipt| {
                        receipt.context_block_digests = contribution_receipt
                            .iter()
                            .map(|item| item.digest.clone())
                            .collect();
                        receipt.attachment_refs = contribution_receipt
                            .iter()
                            .map(|item| format!("{}/{}", item.source_app_id, item.item_id))
                            .collect();
                        receipt
                    }),
                    &identities,
                )?
            };
            crate::publish_chat_thread_change(
                &host,
                thread.resource_id.clone(),
                thread.revision,
                crate::AppDataChangeKind::Completed,
            )
            .await;
            Ok(SendChatMessageResult { thread })
        }
        Err(error) => {
            let status = if error.contains("cancelled") {
                ChatMessageStatus::Cancelled
            } else {
                ChatMessageStatus::Failed
            };
            let thread = {
                let mut store = host
                    .chat_store
                    .lock()
                    .map_err(|_| "chat store lock poisoned".to_string())?;
                store.complete_request(
                    &thread_id,
                    &request_id,
                    vec![ChatMessage {
                        message_id: String::new(),
                        role: ChatMessageRole::Assistant,
                        status: Some(status),
                        text: user_facing_chat_error(&error),
                        reasoning: None,
                        run_id: None,
                        artifact_ids: vec![],
                        client_request_id: None,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        completed_at: None,
                    }],
                )?
            };
            crate::publish_chat_thread_change(
                &host,
                thread.resource_id.clone(),
                thread.revision,
                crate::AppDataChangeKind::Completed,
            )
            .await;
            Ok(SendChatMessageResult { thread })
        }
    }
}

pub(crate) fn authorize_injected_contexts(
    kernel: &app_host_kernel::kernel::Kernel,
    thread_resource_id: &str,
    contexts: Vec<ChatInjectedContext>,
) -> Result<Vec<AuthorizedChatInjectedContext>, String> {
    let capability = CapabilityRef {
        provider: chat_app::chat_app_id(),
        capability: CapabilityName::new(chat_app::CHAT_INJECT_USER_CONTEXT),
    };
    let requested_scope = DataScope::Resources {
        resource_ids: vec![ResourceId::new(thread_resource_id)],
    };
    let mut authorized = Vec::new();
    for context in contexts {
        let digest = format!("{:x}", sha2::Sha256::digest(context.content.as_bytes()));
        if digest != context.content_digest {
            return Err(format!(
                "stored injected context digest mismatch for {}/{}",
                context.source_app_id, context.item_id
            ));
        }
        let source = AppId::new(context.source_app_id.clone());
        let Ok(installed) = kernel.installed_app(&source) else {
            continue;
        };
        if installed.content_hash != context.source_app_content_hash {
            continue;
        }
        let source_run_id = RunId::new(context.source_run_id.clone());
        let Ok(run) = kernel.run_view(&source_run_id) else {
            continue;
        };
        if run.initiating_app() != &source
            || run.terminal_state != Some(RunTerminalState::Completed)
        {
            continue;
        }
        let Some((grant_id, invocation_scope)) = run.invocations.iter().find_map(|invocation| {
            if let InvocationRecord::Completed {
                capability: invoked,
                grant_id,
                data_scope,
            } = invocation
            {
                (invoked == &capability).then_some((grant_id, data_scope))
            } else {
                None
            }
        }) else {
            continue;
        };
        if !invocation_scope.covers(&requested_scope) {
            continue;
        }
        let grant_is_active = kernel
            .grant_statuses_for(&source)
            .into_iter()
            .any(|status| {
                status.grant.grant_id == *grant_id
                    && status.status == GrantStatus::Active
                    && status.grant.scope.covers(&capability)
                    && status.grant.data_scope.covers(&requested_scope)
            });
        if !grant_is_active {
            continue;
        }
        authorized.push(AuthorizedChatInjectedContext {
            context,
            source_app_name: installed.manifest.display_name.clone(),
            grant_id: grant_id.to_string(),
        });
    }
    Ok(authorized)
}

async fn execute_chat_session(
    host: Arc<Host>,
    on_event: Arc<dyn Fn(ChatStreamEvent) + Send + Sync>,
    cancelled: Arc<AtomicBool>,
    thread_id: String,
    session: chat_app::ChatSession,
) -> Result<(Vec<ChatMessage>, Option<ChatCompositionReceipt>), String> {
    let prompt_receipt = prompt_receipt(&session);
    let mut prompt_was_sent = false;
    let session = Arc::new(Mutex::new(session));
    let parent_run_id = session
        .lock()
        .map_err(|_| "chat session lock poisoned".to_string())?
        .parent_run_id()
        .clone();
    let status_message = session
        .lock()
        .map_err(|_| "chat session lock poisoned".to_string())?
        .status_message();

    if let Ok(mut sends) = host.active_chat_sends.lock() {
        if let Some(send) = sends.get_mut(&thread_id) {
            send.run_ids.push(parent_run_id.clone());
        }
    }

    let mut reply = None;
    let mut failure = None;
    loop {
        if cancelled.load(Ordering::Acquire) {
            failure = Some("chat request cancelled".into());
            break;
        }
        let step = with_kernel_blocking(host.clone(), {
            let session = session.clone();
            move |kernel| {
                session
                    .lock()
                    .map_err(|_| "chat session lock poisoned".to_string())?
                    .prepare_next(kernel)
            }
        })
        .await;

        let step = match step {
            Err(error) => {
                failure = Some(error);
                break;
            }
            Ok(step) => step,
        };

        match step {
            chat_app::ChatStep::Complete(completed) => {
                reply = Some(completed);
                break;
            }
            chat_app::ChatStep::Continue => continue,
            chat_app::ChatStep::Execute(mut invocation) => {
                let uses_system_prompt = invocation.uses_system_prompt();
                let child_run_id = invocation.child_run_id.clone();
                if let Ok(mut sends) = host.active_chat_sends.lock() {
                    if let Some(send) = sends.get_mut(&thread_id) {
                        send.run_ids.push(child_run_id.clone());
                    }
                }

                if cancelled.load(Ordering::Acquire) {
                    let cleanup = with_kernel_blocking(host.clone(), move |kernel| {
                        if let Some(token) = invocation.prepared.take() {
                            kernel
                                .abort_prepared_invocation(token)
                                .map_err(|error| error.to_string())?;
                        }
                        kernel
                            .end_run(
                                &invocation.child_run_id,
                                app_host_kernel::primitives::run::RunTerminalState::Cancelled,
                            )
                            .map_err(|error| error.to_string())
                    })
                    .await;
                    failure = Some(match cleanup {
                        Ok(()) => "chat request cancelled".into(),
                        Err(error) => format!("chat request cancelled; cleanup failed: {error}"),
                    });
                    break;
                }

                let _chat_tool_context = if invocation.is_agent_run() {
                    match host
                        .kernel_invoker
                        .bind_chat_thread(&child_run_id, &thread_id)
                    {
                        Ok(binding) => Some(binding),
                        Err(error) => {
                            // The invocation is already prepared. Dropping it
                            // here would leak the kernel-side reservation for
                            // a run we are about to fail, so release it the
                            // same way the cancellation branch above does.
                            let abort = with_kernel_blocking(host.clone(), move |kernel| {
                                let mut invocation = invocation;
                                if let Some(token) = invocation.prepared.take() {
                                    kernel
                                        .abort_prepared_invocation(token)
                                        .map_err(|error| error.to_string())?;
                                }
                                Ok::<(), String>(())
                            })
                            .await;
                            let mut message =
                                fail_child_run(host.clone(), child_run_id.clone(), error).await;
                            if let Err(abort_error) = abort {
                                message = format!(
                                    "{message}; releasing the prepared invocation failed: \
                                     {abort_error}"
                                );
                            }
                            failure = Some(message);
                            break;
                        }
                    }
                } else {
                    None
                };

                let approval = tauri::async_runtime::spawn_blocking(move || {
                    let mut invocation = invocation;
                    let approval = invocation
                        .prepared
                        .take()
                        .expect("chat invocation token is present")
                        .await_approval();
                    (invocation, approval)
                })
                .await
                .map_err(|error| format!("chat approval task failed: {error}"));

                let (invocation, approval) = match approval {
                    Ok(value) => value,
                    Err(error) => {
                        failure =
                            Some(fail_child_run(host.clone(), child_run_id.clone(), error).await);
                        break;
                    }
                };

                let authorized = with_kernel_blocking(host.clone(), move |kernel| {
                    kernel
                        .authorize_invocation(approval)
                        .map_err(|error| error.to_string())
                })
                .await;

                match authorized {
                    Ok(app_host_kernel::AuthorizeInvocation::Authorized(authorized)) => {
                        let on_event = Arc::clone(&on_event);
                        let executed = tauri::async_runtime::spawn_blocking(move || {
                            authorized.execute_with_progress(
                                app_host_kernel::ProgressReporter::new(move |value| {
                                    let kind = value
                                        .get("kind")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or("");
                                    if !kind.starts_with("llm-stream-") {
                                        return;
                                    }
                                    on_event(ChatStreamEvent {
                                        kind: kind.to_string(),
                                        content: value
                                            .get("content")
                                            .and_then(serde_json::Value::as_str)
                                            .unwrap_or("")
                                            .to_string(),
                                        reasoning: value
                                            .get("reasoning")
                                            .and_then(serde_json::Value::as_str)
                                            .unwrap_or("")
                                            .to_string(),
                                    });
                                }),
                            )
                        })
                        .await
                        .map_err(|error| format!("chat invocation task failed: {error}"));

                        let executed = match executed {
                            Ok(executed) => executed,
                            Err(error) => {
                                failure = Some(
                                    fail_child_run(host.clone(), child_run_id.clone(), error).await,
                                );
                                break;
                            }
                        };

                        let finalized = with_kernel_blocking(host.clone(), {
                            let session = session.clone();
                            move |kernel| {
                                let result = kernel
                                    .finalize_invocation(executed)
                                    .map_err(|error| error.to_string())?;
                                session
                                    .lock()
                                    .map_err(|_| "chat session lock poisoned".to_string())?
                                    .finalize_next(kernel, *invocation, result)
                                    .map_err(|error| error.to_string())
                            }
                        })
                        .await;

                        if finalized.is_ok() && uses_system_prompt {
                            prompt_was_sent = true;
                        }

                        match finalized {
                            Ok(Some(completed)) => {
                                reply = Some(completed);
                                break;
                            }
                            Ok(None) => continue,
                            Err(error) => {
                                failure = Some(
                                    fail_child_run(host.clone(), child_run_id.clone(), error).await,
                                );
                                break;
                            }
                        }
                    }
                    Ok(app_host_kernel::AuthorizeInvocation::Refused(result)) => {
                        let finalized = with_kernel_blocking(host.clone(), {
                            let session = session.clone();
                            move |kernel| {
                                session
                                    .lock()
                                    .map_err(|_| "chat session lock poisoned".to_string())?
                                    .finalize_next(kernel, *invocation, result)
                                    .map_err(|error| error.to_string())
                            }
                        })
                        .await;

                        match finalized {
                            Ok(Some(completed)) => {
                                reply = Some(completed);
                                break;
                            }
                            Ok(None) => continue,
                            Err(error) => {
                                failure = Some(error);
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        failure =
                            Some(fail_child_run(host.clone(), child_run_id.clone(), error).await);
                        break;
                    }
                }
            }
        }
    }

    // The session can be marked failed without a `failure` string (e.g. a
    // finalize that produced an apology reply), so fold that into the run's
    // terminal state; the rest of the teardown is shared with the early-return
    // paths above.
    let session_failed = session
        .lock()
        .map(|session| session.failed())
        .unwrap_or(true);
    let messages = finish_chat_session(
        host,
        parent_run_id,
        cancelled,
        failure.is_some() || session_failed,
        failure,
        reply,
        status_message,
    )
    .await?;
    Ok((messages, prompt_was_sent.then_some(prompt_receipt)))
}

fn prompt_receipt(session: &chat_app::ChatSession) -> ChatCompositionReceipt {
    let preview = session.prompt_preview();
    ChatCompositionReceipt {
        system_prompt_digest: preview.digest.clone(),
        assistant_profile_ref: session.assistant_profile_ref(),
        assistant_profile_digest: session.assistant_profile_digest(),
        enabled_skill_digests: session.enabled_skill_digests(),
        context_block_digests: vec![],
        attachment_refs: vec![],
        available_capability_refs: session.available_capability_refs(),
        provider_profile_ref: session.provider_profile_ref(),
        model_profile: session.model_profile_receipt(),
        agent_engine_ref: session.agent_engine_ref(),
        agent_engine_version: session.agent_engine_version(),
        agent_engine_features: session.agent_engine_features(),
        assistant_capability_refs: session.assistant_capability_refs(),
        created_at: String::new(),
        system_prompt: preview.system_prompt.clone(),
        layers: preview
            .layers
            .iter()
            .filter(|layer| layer.included)
            .map(|layer| ChatPromptReceiptLayer {
                id: layer.id.clone(),
                kind: serde_json::to_value(&layer.kind)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_else(|| "unknown".into()),
                title: layer.title.clone(),
                source: layer.source.clone(),
                content: layer.content.clone(),
            })
            .collect(),
        injected_context: session.injected_context_receipt(),
    }
}

fn receipt_for_contributions(
    contributions: &[ChatContribution],
) -> Result<Vec<ChatContribution>, String> {
    for contribution in contributions {
        if contribution.completeness == crate::chat_store::ChatContributionCompleteness::Unavailable
        {
            return Err(format!(
                "{} / {} is unavailable",
                contribution.source_app_id, contribution.item_id
            ));
        }
        if contribution.kind == crate::chat_store::ChatContributionKind::ResourceRef {
            let body = contribution.body.as_object().ok_or_else(|| {
                format!(
                    "{} / {} has invalid resource contribution body",
                    contribution.source_app_id, contribution.item_id
                )
            })?;
            if body
                .get("resource_id")
                .and_then(serde_json::Value::as_str)
                .is_none()
            {
                return Err(format!(
                    "{} / {} needs a resolvable resource reference",
                    contribution.source_app_id, contribution.item_id
                ));
            }
        }
    }
    Ok(contributions.to_vec())
}

// Close out a chat parent run and build its assistant messages. `failed` sets
// the run's terminal state and may be true even when `failure` is None — a run
// can end Failed while still returning a reply (e.g. an internal build error
// that produced an apology). `failure: Some(_)` means no reply is returned.
async fn finish_chat_session(
    host: Arc<Host>,
    parent_run_id: app_host_kernel::ids::RunId,
    cancelled: Arc<AtomicBool>,
    failed: bool,
    failure: Option<String>,
    reply: Option<chat_app::ChatReply>,
    status_message: Option<ChatMessage>,
) -> Result<Vec<ChatMessage>, String> {
    let end_result = with_kernel_blocking(host.clone(), {
        let parent_run_id = parent_run_id.clone();
        move |kernel| {
            let terminal_state = if cancelled.load(Ordering::Acquire) {
                app_host_kernel::primitives::run::RunTerminalState::Cancelled
            } else if failed {
                app_host_kernel::primitives::run::RunTerminalState::Failed
            } else {
                app_host_kernel::primitives::run::RunTerminalState::Completed
            };
            kernel
                .end_run(&parent_run_id, terminal_state)
                .map_err(|error| error.to_string())?;
            Ok(kernel.records().to_vec())
        }
    })
    .await;
    match (reply, failure, end_result) {
        (Some(reply), None, Ok(records)) => {
            Ok(build_assistant_messages(&records, reply, status_message))
        }
        (_, Some(error), _) => Err(error),
        (_, _, Err(error)) => Err(error),
        _ => Err("chat session ended without a reply".into()),
    }
}

async fn fail_child_run(
    host: Arc<Host>,
    run_id: app_host_kernel::ids::RunId,
    error: String,
) -> String {
    match with_kernel_blocking(host, move |kernel| {
        kernel
            .end_run(
                &run_id,
                app_host_kernel::primitives::run::RunTerminalState::Failed,
            )
            .map_err(|value| value.to_string())
    })
    .await
    {
        Ok(()) => error,
        Err(end_error) => format!("{error}; failed to close child run: {end_error}"),
    }
}

pub(crate) fn build_assistant_messages(
    records: &[LedgerRecord],
    reply: chat_app::ChatReply,
    status_message: Option<ChatMessage>,
) -> Vec<ChatMessage> {
    let run_id = reply
        .run_id
        .as_ref()
        .map(|run_id| run_id.as_str().to_string());
    let status = derive_run_status(records, run_id.as_deref());
    let assistant = ChatMessage {
        message_id: String::new(),
        role: ChatMessageRole::Assistant,
        status: Some(status),
        text: reply.text,
        reasoning: reply.reasoning,
        run_id: run_id.clone(),
        artifact_ids: reply
            .artifacts
            .iter()
            .map(|artifact| artifact.artifact_id.as_str().to_string())
            .collect(),
        client_request_id: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
    };
    // Chronological order: an optional permission/status note, then the tool
    // activity that ran during the turn, then the assistant's final reply.
    // (Previously the tool feedback was appended after the reply, inverting the
    // narrative it exists to convey.)
    let mut messages = Vec::new();
    if let Some(status_message) = status_message {
        messages.push(status_message);
    }
    if let Some(run_id) = run_id.as_deref() {
        messages.extend(tool_feedback(records, run_id));
    }
    messages.push(assistant);
    messages
}

pub(crate) fn capability_label(
    capability: &app_host_kernel::primitives::capability::CapabilityRef,
) -> String {
    format!(
        "{} / {}",
        capability.provider.as_str(),
        capability.capability.as_str()
    )
}

fn tool_completed_message(
    capability: &app_host_kernel::primitives::capability::CapabilityRef,
    artifact_count: usize,
) -> String {
    let label = capability_label(capability);
    match artifact_count {
        0 => format!("Used {label}."),
        1 => format!("Used {label} and produced 1 artifact."),
        count => format!("Used {label} and produced {count} artifacts."),
    }
}

fn derive_run_status(records: &[LedgerRecord], run_id: Option<&str>) -> ChatMessageStatus {
    let Some(run_id) = run_id else {
        return ChatMessageStatus::Completed;
    };
    let mut status = ChatMessageStatus::Completed;
    for record in records {
        if record.event.run_id().as_str() != run_id {
            continue;
        }
        match &record.event {
            LedgerEvent::InvocationRefused { .. } | LedgerEvent::ApprovalDenied { .. } => {
                return ChatMessageStatus::Failed;
            }
            LedgerEvent::CapabilityFailed { .. }
            | LedgerEvent::RunEnded {
                terminal_state: app_host_kernel::primitives::run::RunTerminalState::Failed,
                ..
            } => status = ChatMessageStatus::Failed,
            LedgerEvent::RunEnded {
                terminal_state: app_host_kernel::primitives::run::RunTerminalState::Cancelled,
                ..
            } => status = ChatMessageStatus::Cancelled,
            _ => {}
        }
    }
    status
}

pub(crate) fn tool_feedback(records: &[LedgerRecord], root_run_id: &str) -> Vec<ChatMessage> {
    let run_ids = descendant_run_ids(records, root_run_id);
    let mut artifact_counts = std::collections::BTreeMap::<String, usize>::new();
    for record in records {
        if !run_ids.contains(record.event.run_id().as_str()) {
            continue;
        }
        if let LedgerEvent::ArtifactProduced { .. } = &record.event {
            *artifact_counts
                .entry(record.event.run_id().as_str().to_string())
                .or_default() += 1;
        }
    }
    let mut messages = vec![];
    for record in records {
        if !run_ids.contains(record.event.run_id().as_str()) {
            continue;
        }
        match &record.event {
            LedgerEvent::CapabilityCompleted { capability, .. }
                if is_model_visible_tool(capability) =>
            {
                messages.push(ChatMessage {
                    message_id: String::new(),
                    role: ChatMessageRole::ToolStatus,
                    reasoning: None,
                    status: Some(ChatMessageStatus::Completed),
                    text: tool_completed_message(
                        capability,
                        artifact_counts
                            .get(record.event.run_id().as_str())
                            .copied()
                            .unwrap_or_default(),
                    ),
                    run_id: Some(record.event.run_id().as_str().to_string()),
                    artifact_ids: vec![],
                    client_request_id: None,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    completed_at: None,
                })
            }
            LedgerEvent::InvocationRefused {
                capability,
                reason,
                data_scope,
                ..
            } if is_model_visible_tool(capability) => messages.push(ChatMessage {
                message_id: String::new(),
                role: ChatMessageRole::ToolStatus,
                reasoning: None,
                status: Some(ChatMessageStatus::Failed),
                text: tool_refusal_message(
                    capability,
                    match reason {
                        DenialReason::NoGrant => RefusalReason::NoGrant,
                        DenialReason::Expired => RefusalReason::GrantExpired,
                        DenialReason::Revoked => RefusalReason::GrantRevoked,
                    },
                    data_scope,
                ),
                run_id: Some(record.event.run_id().as_str().to_string()),
                artifact_ids: vec![],
                client_request_id: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                completed_at: None,
            }),
            LedgerEvent::ApprovalDenied {
                capability,
                data_scope,
                ..
            } if is_model_visible_tool(capability) => messages.push(ChatMessage {
                message_id: String::new(),
                role: ChatMessageRole::ToolStatus,
                reasoning: None,
                status: Some(ChatMessageStatus::Failed),
                text: tool_refusal_message(capability, RefusalReason::ApprovalDenied, data_scope),
                run_id: Some(record.event.run_id().as_str().to_string()),
                artifact_ids: vec![],
                client_request_id: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                completed_at: None,
            }),
            LedgerEvent::CapabilityFailed {
                capability, error, ..
            } if is_model_visible_tool(capability) => messages.push(ChatMessage {
                message_id: String::new(),
                role: ChatMessageRole::ToolStatus,
                reasoning: None,
                status: Some(ChatMessageStatus::Failed),
                text: format!("{} failed: {error}", capability_label(capability)),
                run_id: Some(record.event.run_id().as_str().to_string()),
                artifact_ids: vec![],
                client_request_id: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                completed_at: None,
            }),
            _ => {}
        }
    }
    messages
}

fn is_model_visible_tool(
    capability: &app_host_kernel::primitives::capability::CapabilityRef,
) -> bool {
    capability.provider != AppId::new("llm-provider")
        && capability.capability != app_host_kernel::ids::CapabilityName::new("agent.run")
}

pub(crate) fn tool_refusal_message(
    capability: &app_host_kernel::primitives::capability::CapabilityRef,
    reason: RefusalReason,
    data_scope: &DataScope,
) -> String {
    let capability_name = format!(
        "{}/{}",
        capability.provider.as_str(),
        capability.capability.as_str()
    );
    match (reason, data_scope) {
        (
            RefusalReason::NoGrant | RefusalReason::GrantRevoked,
            DataScope::Resources { .. },
        ) if capability.provider == crate::file_resources::file_broker_app_id() => format!(
            "Chat cannot access the requested file resource with {capability_name}. Add or review the resource in Settings -> File resources, then grant Chat this operation."
        ),
        (
            RefusalReason::NoGrant | RefusalReason::GrantRevoked,
            DataScope::Resources { .. },
        ) => format!(
            "No active Chat permission covers {capability_name} for the requested resource. Review Chat's resource access in Settings -> Permissions."
        ),
        (RefusalReason::NoGrant, _) => format!(
            "Chat has no active permission for {capability_name}. Open Settings -> Permissions to review grants."
        ),
        (RefusalReason::GrantRevoked, _) => format!(
            "Chat's permission for {capability_name} was revoked. Open Settings -> Permissions to grant it again."
        ),
        (RefusalReason::GrantExpired, _) => format!(
            "{capability_name} is no longer permitted because its grant expired. Open Settings -> Permissions to review grants."
        ),
        (RefusalReason::ApprovalDenied, _) => format!(
            "You declined approval for {capability_name}. No action was performed."
        ),
        (RefusalReason::Cancelled, _) => "Request cancelled.".into(),
    }
}

fn descendant_run_ids(records: &[LedgerRecord], root_run_id: &str) -> BTreeSet<String> {
    let mut run_ids = BTreeSet::from([root_run_id.to_string()]);
    let mut changed = true;
    while changed {
        changed = false;
        for record in records {
            let LedgerEvent::RunStarted {
                run_id,
                initiator: app_host_kernel::primitives::run::Initiator::Run { parent_run_id, .. },
                ..
            } = &record.event
            else {
                continue;
            };
            if run_ids.contains(parent_run_id.as_str())
                && run_ids.insert(run_id.as_str().to_string())
            {
                changed = true;
            }
        }
    }
    run_ids
}

fn user_facing_chat_error(error: &str) -> String {
    if error.contains("cancelled") {
        return "You cancelled this request. Work that completed before cancellation remains in the run history.".into();
    }
    if error.contains(crate::llm_provider::NO_PROVIDER_CONFIGURED_ERROR) {
        return "I can't generate a model response because no model provider is configured. Use Configure model provider below or open Settings -> Model providers, then add a profile and select it as the default for Chat.".into();
    }
    if error.contains("chat has no available execution path") {
        return "Chat has no available model execution path. Configure a model provider or restore Chat's model or agent permission in Settings -> Permissions.".into();
    }
    if error.contains("pi-ai worker response timed out") {
        return format!(
            "The model provider did not respond within {} seconds. Completed tool calls remain in the run history.",
            crate::llm_client::RESPONSE_TIMEOUT.as_secs()
        );
    }
    if error.contains("deadline") || error.contains("timed out") {
        return "The request timed out before Chat could finish. Completed tool calls remain in the run history.".into();
    }
    if error.contains("kernel busy") {
        return "The host is waiting on another trusted decision. Finish that prompt and try again.".into();
    }
    if error.contains("LLM call refused:") {
        return "Chat's model permission is no longer available. Open Settings -> Permissions to review it, then try again.".into();
    }
    if error.contains("LLM call failed:") {
        return "The selected model provider could not complete the request. Check its connection and credentials in Settings -> Model providers. Completed tool calls remain in the run history.".into();
    }
    if error.contains("Agent Engine was denied.") {
        return "Chat's Agent Engine permission is no longer available. Open Settings -> Permissions to review it, then try again.".into();
    }
    if error.contains("Agent Engine could not complete this message.") {
        return "Agent Engine could not complete this request. Retry once. If it keeps failing, open System for the run details.".into();
    }
    "The request failed before Chat could finish. Open System for run and grant details.".into()
}

#[cfg(test)]
mod tests;
