use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;

use super::*;
use crate::llm_client::LlmResponse;

#[test]
fn bundled_manifest_identity_is_pinned_for_durable_installs() {
    assert_eq!(
        seal(llm_provider_manifest()).content_hash,
        "760ee04c7f37196354d1db459a09bd4c4c3e027750b7bd46b5971522b5dea321"
    );
}

#[test]
fn generate_schema_exactly_describes_host_messages_and_tools() {
    let schema = generate_input_schema();
    let properties = schema["properties"].as_object().unwrap();

    let mut property_names = properties.keys().map(String::as_str).collect::<Vec<_>>();
    property_names.sort_unstable();
    assert_eq!(
        property_names,
        vec![
            "max_output_tokens",
            "messages",
            "model",
            "profile",
            "reasoning",
            "response_format",
            "temperature",
            "tools",
        ]
    );
    assert_eq!(properties["messages"]["items"], message_schema());
    assert_eq!(properties["tools"]["items"], tool_definition_schema());
    assert_eq!(properties["temperature"]["minimum"], 0);
    assert_eq!(properties["temperature"]["maximum"], 2);
    assert_eq!(properties["max_output_tokens"]["minimum"], 1);
    assert_eq!(schema["required"], json!(["messages"]));
    assert_eq!(schema["additionalProperties"], false);

    let variants = message_schema()["oneOf"].as_array().unwrap().clone();
    assert_eq!(variants.len(), 4);
    assert_eq!(
        variants[0]["properties"]["role"],
        json!({"const": "system"})
    );
    assert_eq!(variants[0]["required"], json!(["role", "content"]));
    assert_eq!(variants[1]["properties"]["role"], json!({"const": "user"}));
    assert_eq!(
        variants[2]["properties"]["tool_calls"],
        json!({"type": "array", "items": tool_call_schema()})
    );
    assert_eq!(
        variants[3]["required"],
        json!(["role", "content", "tool_call_id"])
    );
    assert!(variants
        .iter()
        .all(|variant| variant["additionalProperties"] == false));

    assert_eq!(
        tool_definition_schema(),
        json!({
            "type": "object",
            "properties": {
                "type": {"const": "function"},
                "function": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "minLength": 1},
                        "description": {"type": "string"},
                        "parameters": {"type": "object"}
                    },
                    "required": ["name", "description", "parameters"],
                    "additionalProperties": false
                }
            },
            "required": ["type", "function"],
            "additionalProperties": false
        })
    );
}

#[test]
fn output_schemas_are_strict_and_exact() {
    assert_eq!(
        llm_response_schema(),
        object(json!({
            "type": "object",
            "properties": {
                "message": assistant_message_schema(),
                "reasoning": {"type": "string"},
                "finish_reason": {"type": "string"},
                "usage": {
                    "type": "object",
                    "properties": {
                        "prompt_tokens": {"type": "integer", "minimum": 0},
                        "completion_tokens": {"type": "integer", "minimum": 0},
                        "total_tokens": {"type": "integer", "minimum": 0},
                        "cache_read_tokens": {"type": "integer", "minimum": 0},
                        "cache_write_tokens": {"type": "integer", "minimum": 0},
                        "cost": {"type": "number", "minimum": 0}
                    },
                    "additionalProperties": false
                },
                "provider_metrics": {
                    "type": "object",
                    "properties": {
                        "time_to_first_token_ms": {"type": "integer", "minimum": 0},
                        "total_latency_ms": {"type": "integer", "minimum": 0}
                    },
                    "required": ["total_latency_ms"],
                    "additionalProperties": false
                }
            },
            "required": ["message", "finish_reason"],
            "additionalProperties": false
        }))
    );
    assert_eq!(
        models_input_schema(),
        object(json!({
            "type": "object",
            "properties": {"profile": {"type": "string", "minLength": 1}},
            "additionalProperties": false
        }))
    );
    assert_eq!(
        models_output_schema(),
        object(json!({
            "type": "object",
            "properties": {
                "models": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string"},
                            "display_name": {"type": "string"},
                            "reasoning": {"type": "boolean"},
                            "context_window": {"type": "integer", "minimum": 0},
                            "max_output_tokens": {"type": "integer", "minimum": 0}
                        },
                        "required": ["id", "display_name", "reasoning", "context_window", "max_output_tokens"],
                        "additionalProperties": false
                    }
                },
                "refreshed": {"type": "boolean"}
            },
            "required": ["models", "refreshed"],
            "additionalProperties": false
        }))
    );
}

#[test]
fn models_capability_keeps_its_sealed_output_shape() {
    let result = models_capability_result(
        vec![NormalizedModel {
            id: "gpt-5.6-sol".into(),
            display_name: "GPT-5.6 Sol".into(),
            reasoning: true,
            variants: vec![crate::config::ModelVariant::High],
            text_verbosity: vec![crate::config::TextVerbosity::High],
            context_window: 372_000,
            max_output_tokens: 128_000,
        }],
        false,
    );

    assert_eq!(
        result,
        json!({
            "models": [{
                "id": "gpt-5.6-sol",
                "display_name": "GPT-5.6 Sol",
                "reasoning": true,
                "context_window": 372_000,
                "max_output_tokens": 128_000,
            }],
            "refreshed": false,
        })
    );
}

#[test]
fn llm_capability_contracts_declare_effects_and_output_schemas() {
    let manifest = llm_provider_manifest();
    assert_eq!(manifest.version, "0.4.0");
    assert_eq!(manifest.capabilities.len(), 3);

    let contracts = manifest
        .capabilities
        .iter()
        .map(|capability| {
            (
                capability.name.as_str(),
                capability.effect,
                capability.output_schema.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        contracts,
        vec![
            (
                "llm.generate",
                CapabilityEffect::ExternalWrite,
                Some(llm_response_schema())
            ),
            (
                "llm.models.list",
                CapabilityEffect::ReadOnly,
                Some(models_output_schema())
            ),
            (
                "llm.models.refresh",
                CapabilityEffect::ExternalWrite,
                Some(models_output_schema())
            ),
        ]
    );
}

#[test]
fn completion_forwards_the_cancellation_callback() {
    struct CancellationBackend<'a>(&'a AtomicBool);

    impl LlmBackend for CancellationBackend<'_> {
        fn complete(&self, _: &CompletionRequest) -> Result<LlmResponse, LlmProviderError> {
            unreachable!("interruptible completion must be used")
        }

        fn complete_stream_interruptible(
            &self,
            _: &CompletionRequest,
            _: &dyn Fn(&str, &str),
            is_cancelled: &dyn Fn() -> bool,
        ) -> Result<LlmResponse, LlmProviderError> {
            self.0.store(is_cancelled(), Ordering::Release);
            Err(LlmProviderError("cancelled".into()))
        }
    }

    let observed = AtomicBool::new(false);
    let request = CompletionRequest {
        model: "model".into(),
        messages: vec![],
        tools: None,
        response_format: None,
        reasoning: None,
        text_verbosity: None,
        temperature: None,
        max_tokens: None,
    };

    let error = complete_provider_request(
        &CancellationBackend(&observed),
        &request,
        &|_, _| {},
        &|| true,
    )
    .unwrap_err();

    assert_eq!(error.to_string(), "cancelled");
    assert!(observed.load(Ordering::Acquire));
}

#[test]
fn profile_variant_defaults_reasoning_without_overriding_a_request() {
    let input = JsonObject::new();
    assert_eq!(
        reasoning_for_request(&input, Some(crate::config::ModelVariant::Xhigh)).as_deref(),
        Some("xhigh")
    );

    let input = JsonObject::from_iter([("reasoning".into(), json!("low"))]);
    assert_eq!(
        reasoning_for_request(&input, Some(crate::config::ModelVariant::Xhigh)).as_deref(),
        Some("low")
    );
}

#[test]
fn profile_resolution_waits_for_a_brief_config_transition() {
    let config = Arc::new(Mutex::new(HostConfigService::default()));
    let guard = config.lock().unwrap();
    let worker_config = config.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        result_tx
            .send(resolve_profile(&JsonObject::new(), &worker_config))
            .unwrap();
    });

    started_rx.recv().unwrap();
    assert!(result_rx
        .recv_timeout(std::time::Duration::from_millis(50))
        .is_err());
    drop(guard);

    assert!(result_rx.recv().unwrap().unwrap().is_none());
    worker.join().unwrap();
}

#[test]
fn generation_persists_oauth_rotation_on_success_and_failure() {
    struct RotatingBackend {
        result: Result<LlmResponse, LlmProviderError>,
        update: StdMutex<Option<OAuthCredential>>,
    }

    impl LlmBackend for RotatingBackend {
        fn complete(&self, _: &CompletionRequest) -> Result<LlmResponse, LlmProviderError> {
            self.result.clone()
        }

        fn take_credential_update(&self) -> Result<Option<OAuthCredential>, LlmProviderError> {
            Ok(self.update.lock().unwrap().take())
        }
    }

    let config = Arc::new(Mutex::new(HostConfigService::default()));
    let connector_id = "llm-provider/codex";
    config
        .lock()
        .unwrap()
        .upsert_connector_config(crate::config::ConnectorConfigView {
            id: connector_id.into(),
            kind: crate::config::ConnectorKind::OpenaiCodex,
            base_url: "https://api.openai.com/v1".into(),
            default_model: "model".into(),
            default_variant: None,
            default_text_verbosity: Some(crate::config::TextVerbosity::High),
            secret_refs: BTreeMap::from([("oauth".into(), "codex-oauth".into())]),
        })
        .unwrap();
    let profile = config.lock().unwrap().llm_profile(connector_id).unwrap();
    let request = CompletionRequest {
        model: "model".into(),
        messages: vec![],
        tools: None,
        response_format: None,
        reasoning: None,
        text_verbosity: Some(crate::config::TextVerbosity::High),
        temperature: None,
        max_tokens: None,
    };
    let response = LlmResponse {
        message: ChatMessage {
            role: "assistant".into(),
            content: "ok".into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        },
        reasoning: None,
        usage: None,
        provider_metrics: None,
        finish_reason: "stop".into(),
    };

    for (result, access) in [
        (Ok(response.clone()), "rotated-success"),
        (
            Err(LlmProviderError("provider failed".into())),
            "rotated-failure",
        ),
    ] {
        let credential = OAuthCredential::parse_serialized(&format!(
            r#"{{"type":"oauth","access":"{access}","refresh":"refresh","expires":1}}"#
        ))
        .unwrap();
        let backend = RotatingBackend {
            result,
            update: StdMutex::new(Some(credential)),
        };
        let _ = complete_provider_request_and_persist(
            &backend,
            &request,
            &|_, _| {},
            &|| false,
            &config,
            &profile,
        );
        let stored = config
            .lock()
            .unwrap()
            .read_llm_profile_oauth_credential(connector_id)
            .unwrap()
            .unwrap();
        assert!(stored.contains(access));
    }
}
