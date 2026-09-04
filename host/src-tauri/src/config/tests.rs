use super::*;
use std::sync::Arc;

use serde_json::json;

use crate::atomic_json::{FailingAtomicFileWriter, FailingFileOperation};
use app_host_kernel::kernel::Kernel;
use app_host_kernel::manifest::{AppManifest, ConfigDeclaration};
use app_host_kernel::primitives::surface::{SurfaceDeclaration, SurfaceKind};
use app_host_kernel::services::chrome::{
    ApprovalDecision, CapabilityApprovalPrompt, ChromeNotice, ChromeNoticeError,
    EventSubscriptionPrompt, GrantIssuancePrompt, TrustedChrome,
};

struct SilentChrome;

struct FailingClearSecretStore {
    inner: InMemorySecretStore,
}

impl SecretStorage for FailingClearSecretStore {
    fn read(&self, ref_: &SecretRef) -> Result<Option<String>, String> {
        self.inner.read(ref_)
    }

    fn write(&mut self, ref_: &SecretRef, value: String) -> Result<(), String> {
        self.inner.write(ref_, value)
    }

    fn check(&self, ref_: &SecretRef) -> Result<bool, String> {
        self.inner.check(ref_)
    }

    fn clear(&mut self, _ref_: &SecretRef) -> Result<(), String> {
        Err("injected credential clear failure".into())
    }

    fn all(&self) -> Result<Vec<(SecretRef, String)>, String> {
        self.inner.all()
    }
}

impl TrustedChrome for SilentChrome {
    fn confirm_grant(&self, _prompt: GrantIssuancePrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn approve_capability(&self, _prompt: CapabilityApprovalPrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn confirm_event_subscriptions(&self, _prompt: EventSubscriptionPrompt) -> ApprovalDecision {
        ApprovalDecision::Approved
    }

    fn show_notice(&self, _notice: ChromeNotice) -> Result<(), ChromeNoticeError> {
        Ok(())
    }
}

fn test_manifest() -> AppManifest {
    AppManifest {
        app_id: app_host_kernel::ids::AppId::new("chat"),
        version: "0.1.0".into(),
        display_name: "Chat".into(),
        description: "test".into(),
        capabilities: vec![],
        surfaces: vec![SurfaceDeclaration {
            name: app_host_kernel::ids::SurfaceName::new("conversation"),
            kind: SurfaceKind::Panel,
            title: "Conversation".into(),
            description: "test".into(),
            intents: vec![],
        }],
        agents: vec![],
        skills: vec![],
        assistant_profiles: vec![],
        automations: vec![],
        connectors: vec![],
        config_declarations: vec![ConfigDeclaration {
            name: app_host_kernel::ids::ConfigName::new("chat"),
            title: "Chat".into(),
            description: "test".into(),
            json_schema: serde_json::from_value(json!({
                "type": "object",
                "properties": {
                    "max_iterations": {"type": "integer", "minimum": 1}
                },
                "required": ["max_iterations"],
                "additionalProperties": false
            }))
            .unwrap(),
            default: Some(json!({"max_iterations": 10})),
        }],
        artifact_types: vec![],
        extension_points: vec![],
        extension_contributions: vec![],
        grant_requests: vec![],
        event_subscriptions: vec![],
    }
}

fn llm_provider_id() -> AppId {
    AppId::new("llm-provider")
}

fn owner_ref(local_name: &str) -> SecretRef {
    SecretRef {
        owner: llm_provider_id(),
        name: SecretName::new(local_name),
    }
}

fn write_secret(service: &mut HostConfigService, local_name: &str, value: &str) {
    let ref_ = owner_ref(local_name);
    service.secrets.write(&ref_, value.to_string()).unwrap();
}

fn configure_local_ollama(service: &mut HostConfigService) {
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/local-ollama".into(),
            kind: ConnectorKind::Ollama,
            base_url: ConnectorKind::Ollama.defaults().base_url.into(),
            default_model: "llama3.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::new(),
        })
        .unwrap();
    service
        .update_host_config(
            serde_json::from_value(json!({
                "host": {"default_llm_profile": "local-ollama"}
            }))
            .unwrap(),
        )
        .unwrap();
}

#[test]
fn default_config_is_valid() {
    let config = default_host_config();
    assert!(validate_host_config(&config).is_ok());
    assert_eq!(config.host.default_llm_profile, None);
    assert!(config.connectors.is_empty());
}

#[test]
fn clearing_the_default_keeps_profiles_but_stops_selecting_one() {
    let mut service = HostConfigService::default();
    configure_local_ollama(&mut service);

    let updated = service
        .update_host_config(
            serde_json::from_value(json!({
                "host": {"default_llm_profile": null}
            }))
            .unwrap(),
        )
        .unwrap();

    assert_eq!(updated.host.default_llm_profile, None);
    assert!(service.current_llm_profile().unwrap().is_none());
    assert!(service
        .list_connector_configs()
        .iter()
        .any(|connector| connector.id == "llm-provider/local-ollama"));
    service
        .delete_connector_config("llm-provider/local-ollama")
        .unwrap();
}

#[test]
fn app_data_backup_retention_never_allows_zero() {
    let mut config = default_host_config();
    assert_eq!(config.host.app_data_backup_retention, 1);
    config.host.app_data_backup_retention = 0;
    assert_eq!(
        validate_host_config(&config).unwrap_err(),
        "app_data_backup_retention must be at least 1"
    );
}

#[test]
fn secret_namespace_survives_secret_store_path_moves() {
    let path1 = temp_config_path();
    let path2 = temp_config_path();
    let ref_ = owner_ref("api-key");
    let mut store1 =
        OsProtectedSecretStore::with_namespace(path1.clone(), "stable-profile-id".into()).unwrap();
    store1.write(&ref_, "secret-value".into()).unwrap();

    std::fs::copy(&path1, &path2).unwrap();
    let store2 =
        OsProtectedSecretStore::with_namespace(path2.clone(), "stable-profile-id".into()).unwrap();

    assert_eq!(store2.read(&ref_).unwrap(), Some("secret-value".into()));
}

#[test]
fn connector_kind_defaults_and_auth_requirements_are_exhaustive() {
    let cases = [
        (ConnectorKind::Ollama, "http://localhost:11434", false),
        (
            ConnectorKind::OpenAiCompatible,
            "https://api.openai.com/v1",
            false,
        ),
        (ConnectorKind::Openai, "https://api.openai.com/v1", true),
        (ConnectorKind::Anthropic, "https://api.anthropic.com", true),
        (
            ConnectorKind::Openrouter,
            "https://openrouter.ai/api/v1",
            true,
        ),
        (
            ConnectorKind::Google,
            "https://generativelanguage.googleapis.com",
            true,
        ),
        (ConnectorKind::Mistral, "https://api.mistral.ai/v1", true),
        (
            ConnectorKind::AmazonBedrock,
            "https://bedrock-runtime.us-east-1.amazonaws.com",
            true,
        ),
    ];

    for (kind, base_url, api_key_required) in cases {
        let defaults = kind.defaults();
        assert_eq!(defaults.base_url, base_url);
        assert_eq!(defaults.api_key_required, api_key_required);
        assert!(!defaults.oauth_credential_required);
    }
    for (kind, base_url) in [
        (ConnectorKind::AnthropicOauth, "https://api.anthropic.com"),
        (
            ConnectorKind::OpenaiCodex,
            "https://chatgpt.com/backend-api",
        ),
        (
            ConnectorKind::GithubCopilot,
            "https://api.githubcopilot.com",
        ),
    ] {
        let defaults = kind.defaults();
        assert_eq!(defaults.base_url, base_url);
        assert!(!defaults.api_key_required);
        assert!(defaults.oauth_credential_required);
    }

    assert_eq!(
        runtime_base_url(ConnectorKind::OpenaiCodex, "https://api.openai.com/v1"),
        "https://chatgpt.com/backend-api"
    );
}

#[test]
fn oauth_profiles_require_configured_credentials_before_activation() {
    let mut service = HostConfigService::default();
    let connector_id = "llm-provider/codex";
    let missing_ref = ConnectorConfigView {
        id: connector_id.into(),
        kind: ConnectorKind::OpenaiCodex,
        base_url: ConnectorKind::OpenaiCodex.defaults().base_url.into(),
        default_model: "gpt-5.1-codex".into(),
        default_variant: None,
        default_text_verbosity: None,
        secret_refs: BTreeMap::new(),
    };
    assert!(service
        .upsert_connector_config(missing_ref.clone())
        .unwrap_err()
        .contains("OAuth secret reference is required"));

    service
        .upsert_connector_config(ConnectorConfigView {
            secret_refs: BTreeMap::from([("oauth".into(), "codex-oauth".into())]),
            ..missing_ref
        })
        .unwrap();
    let activation: JsonObject = serde_json::from_value(json!({
        "host": {
            "default_llm_profile": "codex",
            "cloud_llm_egress_accepted_profiles": [connector_id]
        }
    }))
    .unwrap();
    assert!(service
        .update_host_config(activation.clone())
        .unwrap_err()
        .contains("cannot be selected until login completes"));
    write_secret(&mut service, "codex-oauth", "not-json");
    assert!(service
        .update_host_config(activation.clone())
        .unwrap_err()
        .contains("has an invalid credential"));

    let credential = r#"{"type":"oauth","access":"a","refresh":"r","expires":1}"#;
    service
        .write_llm_profile_oauth_credential_persisted(connector_id, credential.into())
        .unwrap();
    service.update_host_config(activation).unwrap();
    let stored = service
        .read_llm_profile_oauth_credential(connector_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&stored).unwrap(),
        serde_json::from_str::<Value>(credential).unwrap()
    );

    let mut kernel = Kernel::new(Arc::new(SilentChrome));
    crate::llm_provider::install_llm_provider(
        &mut kernel,
        Arc::new(std::sync::Mutex::new(HostConfigService::default())),
    )
    .unwrap();
    service.bootstrap_secrets(&mut kernel).unwrap();
    assert_eq!(
        kernel
            .secret_resolver_for(&llm_provider_id())
            .unwrap()
            .resolve(&active_llm_api_key_secret())
            .unwrap(),
        stored
    );
}

#[test]
fn existing_connector_kinds_deserialize_unchanged_and_unknown_kinds_fail() {
    for kind in ["ollama", "open-ai-compatible"] {
        let connector: ConnectorConfig = serde_json::from_value(json!({
            "kind": kind,
            "base_url": "http://localhost:11434",
            "default_model": "model",
            "default_variant": null,
            "default_text_verbosity": null,
            "secret_refs": {}
        }))
        .unwrap();
        assert_eq!(serde_json::to_value(connector.kind).unwrap(), kind);
    }

    let error = serde_json::from_value::<ConnectorConfig>(json!({
        "kind": "not-a-provider",
        "base_url": "https://example.test",
        "default_model": "model",
        "default_variant": null,
        "default_text_verbosity": null,
        "secret_refs": {}
    }))
    .unwrap_err();
    assert!(error.to_string().contains("unknown variant"));
}

#[test]
fn dedicated_provider_profiles_require_auth_and_flow_through_runtime_profiles() {
    let mut service = HostConfigService::default();
    for kind in [
        ConnectorKind::Openai,
        ConnectorKind::Anthropic,
        ConnectorKind::Openrouter,
        ConnectorKind::Google,
        ConnectorKind::Mistral,
        ConnectorKind::AmazonBedrock,
    ] {
        let kind_name = serde_json::to_value(kind)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        let id = format!("llm-provider/{kind_name}");
        let connector = ConnectorConfigView {
            id: id.clone(),
            kind,
            base_url: kind.defaults().base_url.into(),
            default_model: "model".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::new(),
        };
        let error = service
            .upsert_connector_config(connector.clone())
            .unwrap_err();
        assert!(error.contains("API key secret reference is required"));

        service
            .upsert_connector_config(ConnectorConfigView {
                secret_refs: BTreeMap::from([("api_key".into(), format!("{kind_name}-key"))]),
                ..connector
            })
            .unwrap();
        let runtime = service.llm_profile(&id).unwrap();
        let expected_secret_ref = format!("{kind_name}-key");
        assert_eq!(runtime.kind, kind);
        assert_eq!(runtime.base_url, kind.defaults().base_url);
        assert_eq!(
            runtime.api_key_secret_ref.as_deref(),
            Some(expected_secret_ref.as_str())
        );
    }
}

#[test]
fn cloud_policy_covers_all_provider_kinds_and_generic_loopback_is_local() {
    let connector = |kind, base_url: &str| ConnectorConfig {
        kind,
        base_url: base_url.into(),
        default_model: "model".into(),
        default_variant: None,
        default_text_verbosity: None,
        secret_refs: BTreeMap::new(),
    };

    assert!(!connector_is_cloud(&connector(
        ConnectorKind::Ollama,
        "https://remote.example"
    )));
    assert!(!connector_is_cloud(&connector(
        ConnectorKind::OpenAiCompatible,
        "http://127.0.0.1:8080/v1"
    )));
    assert!(connector_is_cloud(&connector(
        ConnectorKind::OpenAiCompatible,
        "https://remote.example/v1"
    )));
    for kind in [
        ConnectorKind::Openai,
        ConnectorKind::Anthropic,
        ConnectorKind::AnthropicOauth,
        ConnectorKind::OpenaiCodex,
        ConnectorKind::GithubCopilot,
        ConnectorKind::Openrouter,
        ConnectorKind::Google,
        ConnectorKind::Mistral,
        ConnectorKind::AmazonBedrock,
    ] {
        assert!(connector_is_cloud(&connector(
            kind,
            "http://localhost:8080"
        )));
    }
}

#[test]
fn update_host_config_switches_active_profile() {
    let mut service = HostConfigService::default();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(String::from("api_key"), String::from("work-key"))]),
        })
        .unwrap();
    write_secret(&mut service, "work-key", "secret-value");

    service
        .update_host_config(
            serde_json::from_value(json!({
                "host": {
                    "default_llm_profile": "work-openai",
                    "cloud_llm_egress_accepted_profiles": ["llm-provider/work-openai"]
                }
            }))
            .unwrap(),
        )
        .unwrap();

    let profile = service.current_llm_profile().unwrap().unwrap();
    assert_eq!(profile.connector_id, "llm-provider/work-openai");
    assert_eq!(profile.api_key_secret_ref.as_deref(), Some("work-key"));
}

#[test]
fn llm_profile_resolves_by_connector_id_and_rejects_unknown() {
    let mut service = HostConfigService::default();
    configure_local_ollama(&mut service);

    let pinned = service.llm_profile("llm-provider/local-ollama").unwrap();
    assert_eq!(pinned.connector_id, "llm-provider/local-ollama");
    assert!(pinned.api_key_secret_ref.is_none());

    let error = service.llm_profile("llm-provider/deleted").unwrap_err();
    assert!(error.contains("unknown LLM profile"));
}

#[test]
fn chat_model_profiles_cannot_select_an_inactive_cloud_credential() {
    let mut service = HostConfigService::default();
    configure_local_ollama(&mut service);
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([("api_key".into(), "work-key".into())]),
        })
        .unwrap();

    assert!(service
        .selectable_chat_llm_profile("llm-provider/local-ollama")
        .is_ok());
    let error = service
        .selectable_chat_llm_profile("llm-provider/work-openai")
        .unwrap_err();
    assert!(error.contains("must be selected as Default for Chat"));
}

#[test]
fn selecting_cloud_default_requires_acknowledgement() {
    let mut service = HostConfigService::default();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(String::from("api_key"), String::from("work-key"))]),
        })
        .unwrap();
    write_secret(&mut service, "work-key", "secret-value");

    let error = service
        .update_host_config(
            serde_json::from_value(json!({
                "host": {"default_llm_profile": "work-openai"}
            }))
            .unwrap(),
        )
        .unwrap_err();

    assert!(error.contains("acknowledge the data-egress policy"));
}

#[test]
fn selecting_cloud_default_succeeds_after_acknowledgement() {
    let mut service = HostConfigService::default();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(String::from("api_key"), String::from("work-key"))]),
        })
        .unwrap();
    write_secret(&mut service, "work-key", "secret-value");

    let updated = service
        .update_host_config(
            serde_json::from_value(json!({
                "host": {
                    "default_llm_profile": "work-openai",
                    "cloud_llm_egress_accepted_profiles": ["llm-provider/work-openai"]
                }
            }))
            .unwrap(),
        )
        .unwrap();

    assert_eq!(
        updated.host.default_llm_profile.as_deref(),
        Some("work-openai")
    );
    assert_eq!(
        updated.host.cloud_llm_egress_accepted_profiles,
        vec![String::from("llm-provider/work-openai")]
    );
}

#[test]
fn changing_active_profile_to_cloud_can_acknowledge_atomically() {
    let mut service = HostConfigService::default();
    configure_local_ollama(&mut service);

    let updated = service
        .upsert_connector_config_with_egress_acknowledgement(ConnectorConfigView {
            id: "llm-provider/local-ollama".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(String::from("api_key"), String::from("remote-key"))]),
        })
        .unwrap();

    assert_eq!(updated.base_url, "https://example.test/v1");
    assert!(service
        .get_host_config()
        .host
        .cloud_llm_egress_accepted_profiles
        .contains(&"llm-provider/local-ollama".to_string()));
}

#[test]
fn sync_active_llm_secret_uses_profile_specific_secret_ref() {
    let mut service = HostConfigService::default();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(String::from("api_key"), String::from("work-key"))]),
        })
        .unwrap();
    write_secret(&mut service, "work-key", "secret-value");
    service
        .update_host_config(
            serde_json::from_value(json!({
                "host": {
                    "default_llm_profile": "work-openai",
                    "cloud_llm_egress_accepted_profiles": ["llm-provider/work-openai"]
                }
            }))
            .unwrap(),
        )
        .unwrap();

    let mut kernel = Kernel::new(Arc::new(SilentChrome));
    let owner = llm_provider_id();
    crate::llm_provider::install_llm_provider(
        &mut kernel,
        Arc::new(std::sync::Mutex::new(HostConfigService::default())),
    )
    .unwrap();
    service
        .put_secret(&mut kernel, &owner, "work-key", "secret-value".into())
        .unwrap();

    let resolver = kernel
        .secret_resolver_for(&crate::llm_provider::llm_provider_app_id())
        .unwrap();
    assert_eq!(
        resolver.resolve(&active_llm_api_key_secret()).unwrap(),
        "secret-value"
    );
}

#[test]
fn sync_active_llm_secret_fails_when_active_secret_missing() {
    let mut service = HostConfigService::default();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(String::from("api_key"), String::from("work-key"))]),
        })
        .unwrap();
    service
        .update_host_config(
            serde_json::from_value(json!({
                "host": {
                    "default_llm_profile": "work-openai",
                    "cloud_llm_egress_accepted_profiles": ["llm-provider/work-openai"]
                }
            }))
            .unwrap(),
        )
        .unwrap();
    let mut kernel = Kernel::new(Arc::new(SilentChrome));
    crate::llm_provider::install_llm_provider(
        &mut kernel,
        Arc::new(std::sync::Mutex::new(HostConfigService::default())),
    )
    .unwrap();
    let error = service.sync_active_llm_secret(&mut kernel).unwrap_err();
    assert!(error.contains("requires stored secret 'work-key'"));
}

#[test]
fn selecting_default_profile_does_not_require_stored_secret() {
    let mut service = HostConfigService::default();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(String::from("api_key"), String::from("work-key"))]),
        })
        .unwrap();

    let updated = service
        .update_host_config(
            serde_json::from_value(json!({
                "host": {
                    "default_llm_profile": "work-openai",
                    "cloud_llm_egress_accepted_profiles": ["llm-provider/work-openai"]
                }
            }))
            .unwrap(),
        )
        .unwrap();

    assert_eq!(
        updated.host.default_llm_profile.as_deref(),
        Some("work-openai")
    );
    assert_eq!(
        service
            .current_llm_profile()
            .unwrap()
            .unwrap()
            .api_key_secret_ref
            .as_deref(),
        Some("work-key")
    );
}

fn temp_config_path() -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("host-config-{nanos}.json"))
}

#[test]
fn fresh_config_seeds_removable_kestral_docs_server_until_user_removes_it() {
    let path = temp_config_path();
    let service = HostConfigService::new(path.clone()).unwrap();

    assert_eq!(
        service.mcp_server(KESTRAL_GITMCP_SERVER_ID),
        Some(McpServerConfig {
            display_name: "Kestral documentation".into(),
            transport: McpTransportConfig::StreamableHttp {
                url: "https://gitmcp.io/ManuelZierl/kestral".into(),
                authentication: McpHttpAuthentication::None,
            },
        })
    );

    let mut reloaded = HostConfigService::new(path.clone()).unwrap();
    reloaded
        .delete_mcp_server(KESTRAL_GITMCP_SERVER_ID)
        .unwrap();
    assert!(reloaded.mcp_server(KESTRAL_GITMCP_SERVER_ID).is_none());

    let _ = std::fs::remove_file(path);
}

#[test]
fn clearing_all_indexed_secrets_removes_the_protected_credentials() {
    let config_path = temp_config_path();
    let secrets_path = config_path.with_extension("secrets.json");
    let namespace = format!("reset-test-{}", uuid::Uuid::new_v4());
    let ref_ = owner_ref("reset-secret");
    let account = {
        let mut store =
            OsProtectedSecretStore::with_namespace(secrets_path.clone(), namespace.clone())
                .unwrap();
        store.write(&ref_, "sensitive".into()).unwrap();
        let account = store.account(&ref_);
        assert_eq!(
            store.backend.read(&account).unwrap().as_deref(),
            Some("sensitive")
        );
        account
    };

    clear_all_indexed_secrets(secrets_path.clone(), namespace).unwrap();

    assert!(OsCredentialBackend.read(&account).unwrap().is_none());
    let raw = std::fs::read_to_string(&secrets_path).unwrap();
    assert!(raw.contains("\"secrets\": []"));
    std::fs::remove_file(secrets_path).unwrap();
}

#[test]
fn mcp_servers_roundtrip_through_persistence() {
    let path = temp_config_path();
    let mut service = HostConfigService::new(path.clone()).unwrap();
    service
        .upsert_mcp_server(McpServerConfigView {
            id: "weather".into(),
            display_name: "Weather".into(),
            transport: McpTransportConfig::Stdio {
                command: "node".into(),
                args: vec!["server.mjs".into()],
            },
        })
        .unwrap();
    service
        .upsert_mcp_server(McpServerConfigView {
            id: "remote".into(),
            display_name: "Remote".into(),
            transport: McpTransportConfig::StreamableHttp {
                url: "https://mcp.example/mcp".into(),
                authentication: McpHttpAuthentication::None,
            },
        })
        .unwrap();

    let reloaded = HostConfigService::new(path).unwrap();
    let servers = reloaded.list_mcp_servers();
    assert_eq!(servers.len(), 3);
    assert_eq!(servers[0].id, KESTRAL_GITMCP_SERVER_ID);
    assert_eq!(servers[1].id, "remote");
    assert_eq!(servers[2].id, "weather");
    assert_eq!(
        reloaded.mcp_server("weather").unwrap().transport,
        McpTransportConfig::Stdio {
            command: "node".into(),
            args: vec!["server.mjs".into()],
        }
    );
}

#[test]
fn mcp_server_delete_removes_the_entry_and_unknown_ids_fail() {
    let mut service = HostConfigService::default();
    service
        .upsert_mcp_server(McpServerConfigView {
            id: "weather".into(),
            display_name: "Weather".into(),
            transport: McpTransportConfig::Stdio {
                command: "node".into(),
                args: vec![],
            },
        })
        .unwrap();
    service.delete_mcp_server("weather").unwrap();
    assert!(service.list_mcp_servers().is_empty());
    assert!(service.delete_mcp_server("weather").is_err());
}

#[test]
fn mcp_http_authentication_stays_in_secret_storage_and_clears_on_endpoint_change() {
    let mut service = HostConfigService::default();
    let authenticated = |url: &str| McpServerConfigView {
        id: "x".into(),
        display_name: "X".into(),
        transport: McpTransportConfig::StreamableHttp {
            url: url.into(),
            authentication: McpHttpAuthentication::StaticHeader {
                header_name: "Authorization".into(),
                value_prefix: "Bearer ".into(),
            },
        },
    };
    service
        .upsert_mcp_server(authenticated("https://api.x.com/mcp"))
        .unwrap();
    assert!(!service.has_mcp_http_auth_secret("x").unwrap());
    service
        .put_mcp_http_auth_secret("x", "test-token".into())
        .unwrap();
    assert_eq!(
        service.mcp_http_auth_header("x").unwrap(),
        Some(("Authorization".into(), "Bearer test-token".into()))
    );
    let serialized = serde_json::to_string(&service.get_host_config()).unwrap();
    assert!(!serialized.contains("test-token"));

    service
        .upsert_mcp_server(authenticated("https://api.x.com/other-mcp"))
        .unwrap();
    assert!(!service.has_mcp_http_auth_secret("x").unwrap());
    assert!(service
        .mcp_http_auth_header("x")
        .unwrap_err()
        .contains("needs an HTTP authentication credential"));
}

#[test]
fn invalid_mcp_server_configs_are_rejected() {
    let mut service = HostConfigService::default();
    let base = |transport: McpTransportConfig| McpServerConfigView {
        id: "server".into(),
        display_name: "Server".into(),
        transport,
    };
    // Empty stdio command.
    assert!(service
        .upsert_mcp_server(base(McpTransportConfig::Stdio {
            command: "  ".into(),
            args: vec![],
        }))
        .is_err());
    // Non-http(s) endpoint.
    assert!(service
        .upsert_mcp_server(base(McpTransportConfig::StreamableHttp {
            url: "ftp://mcp.example".into(),
            authentication: McpHttpAuthentication::None,
        }))
        .is_err());
    // Whitespace in the id.
    assert!(service
        .upsert_mcp_server(McpServerConfigView {
            id: "has space".into(),
            display_name: "X".into(),
            transport: McpTransportConfig::StreamableHttp {
                url: "https://mcp.example".into(),
                authentication: McpHttpAuthentication::None,
            },
        })
        .is_err());
    assert!(service.list_mcp_servers().is_empty());
}

#[test]
fn edits_default_ollama_connector_and_persists() {
    let path = temp_config_path();
    let mut service = HostConfigService::new(path.clone()).unwrap();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/local-ollama".into(),
            kind: ConnectorKind::Ollama,
            base_url: "http://127.0.0.1:11434".into(),
            default_model: "llama3.2".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::new(),
        })
        .unwrap();

    let reloaded = HostConfigService::new(path).unwrap();
    let connector = reloaded
        .list_connector_configs()
        .into_iter()
        .find(|connector| connector.id == "llm-provider/local-ollama")
        .unwrap();
    assert_eq!(connector.base_url, "http://127.0.0.1:11434");
    assert_eq!(connector.default_model, "llama3.2");
}

#[test]
fn host_config_rejects_omitted_canonical_fields() {
    let path = temp_config_path();
    let mut service = HostConfigService::new(path.clone()).unwrap();
    configure_local_ollama(&mut service);
    drop(service);
    let canonical: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();

    let assert_rejected = |label: &str, document: serde_json::Value| {
        fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
        let error = match HostConfigService::new(path.clone()) {
            Ok(_) => panic!("omitting {label} was accepted"),
            Err(error) => error,
        };
        assert!(error.contains("parse host config failed"));
    };

    for field in [
        "apps",
        "connectors",
        "mcp_servers",
        "mcp_exports",
        "mcp_export_transitions",
        "mcp_gateway",
    ] {
        let mut document = canonical.clone();
        let config = document
            .get_mut("config")
            .and_then(serde_json::Value::as_object_mut)
            .unwrap();
        config.remove(field);
        assert_rejected(field, document);
    }

    let mut document = canonical.clone();
    document["config"]["host"]
        .as_object_mut()
        .unwrap()
        .remove("cloud_llm_egress_accepted_profiles");
    assert_rejected("cloud_llm_egress_accepted_profiles", document);

    let mut document = canonical.clone();
    document["config"]["host"]
        .as_object_mut()
        .unwrap()
        .remove("default_llm_profile");
    assert_rejected("default_llm_profile", document);

    let mut document = canonical.clone();
    document["config"]["connectors"]["llm-provider/local-ollama"]
        .as_object_mut()
        .unwrap()
        .remove("default_variant");
    assert_rejected("connector.default_variant", document);

    let mut document = canonical;
    document["config"]["connectors"]["llm-provider/local-ollama"]
        .as_object_mut()
        .unwrap()
        .remove("default_text_verbosity");
    assert_rejected("connector.default_text_verbosity", document);
}

#[test]
fn canonical_config_serializes_optional_values_explicitly() {
    let config = serde_json::to_value(default_host_config()).unwrap();
    assert_eq!(config["host"]["default_llm_profile"], Value::Null);
    assert_eq!(config["connectors"], json!({}));
    assert_eq!(
        config["host"]["cloud_llm_egress_accepted_profiles"],
        json!([])
    );
    let connector = serde_json::to_value(ConnectorConfig {
        kind: ConnectorKind::Ollama,
        base_url: ConnectorKind::Ollama.defaults().base_url.into(),
        default_model: "llama3.1".into(),
        default_variant: None,
        default_text_verbosity: None,
        secret_refs: BTreeMap::new(),
    })
    .unwrap();
    assert_eq!(connector["default_variant"], Value::Null);
    assert_eq!(connector["default_text_verbosity"], Value::Null);
    assert_eq!(config["mcp_gateway"]["allowed_origins"], json!([]));
    let export = serde_json::to_value(McpExportProfile {
        display_name: "Example".into(),
        enabled: false,
        capabilities: vec![],
        interaction: McpExportInteraction::RequiresApproval,
        expires_after_seconds: None,
        rate_limit_per_minute: 60,
        expose_results: false,
        expose_artifacts: false,
    })
    .unwrap();
    assert_eq!(export["expires_after_seconds"], Value::Null);
}

#[test]
fn editing_active_default_into_cloud_requires_acknowledgement() {
    let mut service = HostConfigService::default();
    configure_local_ollama(&mut service);
    let error = service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/local-ollama".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::new(),
        })
        .unwrap_err();

    assert!(error.contains("acknowledge the data-egress policy"));
}

#[test]
fn corrupt_host_config_fails_fast() {
    let path = temp_config_path();
    fs::write(&path, "{not-json").unwrap();
    let error = HostConfigService::new(path).err().unwrap();
    assert!(error.contains("parse host config failed"));
}

#[test]
fn newly_created_connector_can_be_edited_after_save() {
    let path = temp_config_path();
    let connector_id = "llm-provider/work-openai";
    let mut service = HostConfigService::new(path.clone()).unwrap();
    write_secret(&mut service, "work-key", "secret-value");
    service
        .upsert_connector_config(ConnectorConfigView {
            id: connector_id.into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1-mini".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(String::from("api_key"), String::from("work-key"))]),
        })
        .unwrap();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: connector_id.into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(String::from("api_key"), String::from("work-key"))]),
        })
        .unwrap();

    let reloaded = HostConfigService::new(path).unwrap();
    let connector = reloaded
        .list_connector_configs()
        .into_iter()
        .find(|connector| connector.id == connector_id)
        .unwrap();
    assert_eq!(connector.default_model, "gpt-4.1");
}

#[test]
fn put_secret_marks_secret_present_and_persists_across_reload() {
    let path = temp_config_path();
    let mut service = HostConfigService::new(path.clone()).unwrap();
    let mut kernel = Kernel::new(Arc::new(SilentChrome));
    let owner = llm_provider_id();

    service
        .put_secret(&mut kernel, &owner, "work-key", "secret-value".into())
        .unwrap();

    assert!(service.has_secret(&owner, "work-key").unwrap());

    let reloaded = HostConfigService::new(path).unwrap();
    assert!(reloaded.has_secret(&owner, "work-key").unwrap());
}

#[test]
fn oauth_profile_credential_persists_across_reload() {
    let path = temp_config_path();
    let connector_id = "llm-provider/codex";
    let credential = r#"{"type":"oauth","access":"a","refresh":"r","expires":1}"#;
    let mut service = HostConfigService::new(path.clone()).unwrap();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: connector_id.into(),
            kind: ConnectorKind::OpenaiCodex,
            base_url: "https://chatgpt.com/backend-api".into(),
            default_model: "gpt-5.6-sol".into(),
            default_variant: Some(ModelVariant::Xhigh),
            default_text_verbosity: Some(TextVerbosity::High),
            secret_refs: BTreeMap::from([("oauth".into(), "codex-oauth".into())]),
        })
        .unwrap();
    service
        .write_llm_profile_oauth_credential_persisted(connector_id, credential.into())
        .unwrap();
    drop(service);

    let reloaded = HostConfigService::new(path).unwrap();
    assert_eq!(
        reloaded.llm_profile(connector_id).unwrap().default_variant,
        Some(ModelVariant::Xhigh)
    );
    assert_eq!(
        reloaded
            .llm_profile(connector_id)
            .unwrap()
            .default_text_verbosity,
        Some(TextVerbosity::High)
    );
    let stored = reloaded
        .read_llm_profile_oauth_credential(connector_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&stored).unwrap(),
        serde_json::from_str::<Value>(credential).unwrap()
    );
}

#[test]
fn clear_secret_marks_secret_absent_and_persists_across_reload() {
    let path = temp_config_path();
    let mut service = HostConfigService::new(path.clone()).unwrap();
    let mut kernel = Kernel::new(Arc::new(SilentChrome));
    let owner = llm_provider_id();
    service
        .put_secret(&mut kernel, &owner, "work-key", "secret-value".into())
        .unwrap();

    service
        .clear_secret(&mut kernel, &owner, "work-key")
        .unwrap();

    assert!(!service.has_secret(&owner, "work-key").unwrap());

    let reloaded = HostConfigService::new(path).unwrap();
    assert!(!reloaded.has_secret(&owner, "work-key").unwrap());
}

#[test]
fn bootstrap_secrets_rehydrates_persisted_secret_values() {
    let path = temp_config_path();
    let mut initial = HostConfigService::new(path.clone()).unwrap();
    let owner = llm_provider_id();
    initial
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(String::from("api_key"), String::from("work-key"))]),
        })
        .unwrap();
    initial
        .update_host_config(
            serde_json::from_value(json!({
                "host": {
                    "default_llm_profile": "work-openai",
                    "cloud_llm_egress_accepted_profiles": ["llm-provider/work-openai"]
                }
            }))
            .unwrap(),
        )
        .unwrap();
    let mut initial_kernel = Kernel::new(Arc::new(SilentChrome));
    initial
        .put_secret(
            &mut initial_kernel,
            &owner,
            "work-key",
            "secret-value".into(),
        )
        .unwrap();

    let reloaded = HostConfigService::new(path).unwrap();
    let mut kernel = Kernel::new(Arc::new(SilentChrome));
    crate::llm_provider::install_llm_provider(
        &mut kernel,
        Arc::new(std::sync::Mutex::new(HostConfigService::default())),
    )
    .unwrap();

    reloaded.bootstrap_secrets(&mut kernel).unwrap();

    let resolver = kernel
        .secret_resolver_for(&crate::llm_provider::llm_provider_app_id())
        .unwrap();
    assert_eq!(
        resolver.resolve(&active_llm_api_key_secret()).unwrap(),
        "secret-value"
    );
}

#[test]
fn deletes_connector() {
    let path = temp_config_path();
    let mut service = HostConfigService::new(path.clone()).unwrap();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::new(),
        })
        .unwrap();
    service
        .delete_connector_config("llm-provider/work-openai")
        .unwrap();

    let reloaded = HostConfigService::new(path).unwrap();
    assert!(!reloaded
        .list_connector_configs()
        .iter()
        .any(|connector| connector.id == "llm-provider/work-openai"));
}

#[test]
fn cannot_delete_active_default_connector_without_replacement() {
    let mut service = HostConfigService::default();
    configure_local_ollama(&mut service);
    let error = service
        .delete_connector_config("llm-provider/local-ollama")
        .unwrap_err();
    assert!(error.contains("choose a new default first"));
}

#[test]
fn deleting_a_connector_drops_its_egress_acknowledgment() {
    let mut service = HostConfigService::default();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::new(),
        })
        .unwrap();
    service
        .update_host_config(
            serde_json::from_value(json!({
                "host": {
                    "cloud_llm_egress_accepted_profiles": ["llm-provider/work-openai"]
                }
            }))
            .unwrap(),
        )
        .unwrap();

    service
        .delete_connector_config("llm-provider/work-openai")
        .unwrap();

    // A re-created profile under the same id must not inherit the old
    // acknowledgment.
    assert!(service
        .get_host_config()
        .host
        .cloud_llm_egress_accepted_profiles
        .is_empty());
}

#[test]
fn deleting_a_connector_clears_its_stored_credential() {
    let mut service = HostConfigService::default();
    let mut kernel = Kernel::new(Arc::new(SilentChrome));
    let owner = llm_provider_id();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(
                String::from("api_key"),
                String::from("llm-provider/work-openai/api_key"),
            )]),
        })
        .unwrap();
    service
        .put_secret(
            &mut kernel,
            &owner,
            "llm-provider/work-openai/api_key",
            "leaked-key".into(),
        )
        .unwrap();

    service
        .delete_connector_config("llm-provider/work-openai")
        .unwrap();

    // Deleting a profile is how a user revokes a leaked key. Secret names are
    // derived from the connector id, so a leftover key would be silently
    // adopted by the next profile created under that id.
    assert!(!service
        .has_secret(&owner, "llm-provider/work-openai/api_key")
        .unwrap());
}

#[test]
fn credential_clear_failure_keeps_connector_profile() {
    let mut service = HostConfigService::default();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(
                String::from("api_key"),
                String::from("llm-provider/work-openai/api_key"),
            )]),
        })
        .unwrap();
    let secret_ref = owner_ref("llm-provider/work-openai/api_key");
    let mut secrets = FailingClearSecretStore {
        inner: InMemorySecretStore::new(),
    };
    secrets.write(&secret_ref, "secret-value".into()).unwrap();
    service.secrets = Box::new(secrets);

    let error = service
        .delete_connector_config("llm-provider/work-openai")
        .unwrap_err();

    assert!(error.contains("was not removed"));
    assert!(service
        .list_connector_configs()
        .iter()
        .any(|connector| connector.id == "llm-provider/work-openai"));
    assert_eq!(
        service.secrets.read(&secret_ref).unwrap().as_deref(),
        Some("secret-value")
    );
}

#[test]
fn connector_persist_failure_restores_cleared_credential() {
    let mut service = HostConfigService::default();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(
                String::from("api_key"),
                String::from("llm-provider/work-openai/api_key"),
            )]),
        })
        .unwrap();
    let secret_ref = owner_ref("llm-provider/work-openai/api_key");
    service
        .secrets
        .write(&secret_ref, "secret-value".into())
        .unwrap();
    service.path = Some(temp_config_path());
    service.writer = Arc::new(FailingAtomicFileWriter::new(FailingFileOperation::Write));

    let error = service
        .delete_connector_config("llm-provider/work-openai")
        .unwrap_err();

    assert!(error.contains("write host config failed"));
    assert!(service
        .list_connector_configs()
        .iter()
        .any(|connector| connector.id == "llm-provider/work-openai"));
    assert_eq!(
        service.secrets.read(&secret_ref).unwrap().as_deref(),
        Some("secret-value")
    );
}

#[test]
fn deleting_a_connector_keeps_a_credential_another_profile_still_uses() {
    // `upsert_connector_config` rejects two connectors sharing a secret ref,
    // but a config saved before that guard existed is deliberately not
    // re-validated at load (see `upsert_connector_config_inner`). Such a
    // config is therefore the only way this state arises — and deleting one of
    // the two must not pull the key out from under the other.
    let path = temp_config_path();
    let mut seed = HostConfigService::new(path.clone()).unwrap();
    seed.upsert_connector_config(ConnectorConfigView {
        id: "llm-provider/work-openai".into(),
        kind: ConnectorKind::OpenAiCompatible,
        base_url: "https://example.test/v1".into(),
        default_model: "gpt-4.1".into(),
        default_variant: None,
        default_text_verbosity: None,
        secret_refs: BTreeMap::from([(String::from("api_key"), String::from("shared-key"))]),
    })
    .unwrap();
    drop(seed);

    let mut document: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let spare = document["config"]["connectors"]["llm-provider/work-openai"].clone();
    document["config"]["connectors"]["llm-provider/spare-openai"] = spare;
    fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();

    let mut service = HostConfigService::new(path).unwrap();
    let mut kernel = Kernel::new(Arc::new(SilentChrome));
    let owner = llm_provider_id();
    service
        .put_secret(&mut kernel, &owner, "shared-key", "secret-value".into())
        .unwrap();

    service
        .delete_connector_config("llm-provider/work-openai")
        .unwrap();

    assert!(service.has_secret(&owner, "shared-key").unwrap());
}

#[test]
fn frontend_secret_views_do_not_expose_secret_values() {
    let mut service = HostConfigService::default();
    let mut kernel = Kernel::new(Arc::new(SilentChrome));
    let owner = llm_provider_id();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(String::from("api_key"), String::from("work-key"))]),
        })
        .unwrap();
    service
        .put_secret(&mut kernel, &owner, "work-key", "secret-value".into())
        .unwrap();

    let host_config = serde_json::to_string(&service.get_host_config()).unwrap();
    let connectors = serde_json::to_string(&service.list_connector_configs()).unwrap();

    assert!(service.has_secret(&owner, "work-key").unwrap());
    assert!(!service.has_secret(&owner, "missing-key").unwrap());
    assert!(!host_config.contains("secret_values"));
    assert!(!host_config.contains("secret-value"));
    assert!(!connectors.contains("secret-value"));
    assert!(connectors.contains("work-key"));
}

#[test]
fn tauri_command_surface_exposes_secret_presence_not_secret_reads() {
    let host_lib_source = include_str!("../lib.rs");

    assert!(host_lib_source.contains("fn put_secret("));
    assert!(host_lib_source.contains("fn clear_secret("));
    assert!(host_lib_source.contains("fn has_secret("));
    assert!(!host_lib_source.contains("fn get_secret("));
    // Commands must accept owner + secret_name, not bare secret_name.
    assert!(host_lib_source.contains("owner: AppId"));
    assert!(host_lib_source.contains("secret_name: String"));
}

#[test]
fn kernel_has_no_handler_mutation_test_feature() {
    let host_cargo = include_str!("../../Cargo.toml");
    let kernel_cargo = include_str!("../../../../crates/kernel/Cargo.toml");
    let kernel_source = include_str!("../../../../crates/kernel/src/kernel.rs");

    assert!(!kernel_cargo.contains("test-utils"));
    assert!(!kernel_source.contains("set_handler"));
    assert!(host_cargo.contains("app-host-kernel = { path = \"../../crates/kernel\" }"));
    assert!(!host_cargo.contains("features = [\"test-utils\"]"));
}

#[test]
fn direct_probe_endpoints_are_exhaustive_and_unsupported_kinds_fail_before_http() {
    let service = HostConfigService::default();
    for kind in [
        ConnectorKind::OpenAiCompatible,
        ConnectorKind::Openai,
        ConnectorKind::Openrouter,
        ConnectorKind::Mistral,
    ] {
        let probe = service.draft_probe(kind, "https://provider.example/v1/", None);
        assert_eq!(
            probe.url.as_deref(),
            Some("https://provider.example/v1/models")
        );
    }
    assert_eq!(
        service
            .draft_probe(ConnectorKind::Ollama, "http://localhost:11434/", None)
            .url
            .as_deref(),
        Some("http://localhost:11434/api/tags")
    );

    for kind in [
        ConnectorKind::Anthropic,
        ConnectorKind::AnthropicOauth,
        ConnectorKind::OpenaiCodex,
        ConnectorKind::GithubCopilot,
        ConnectorKind::Google,
        ConnectorKind::AmazonBedrock,
    ] {
        let probe = service.draft_probe(kind, kind.defaults().base_url, None);
        let error = run_connector_test(&probe).unwrap_err();
        assert!(error.contains("unsupported"));
        assert!(error.contains("worker model capabilities"));
    }
}

#[test]
fn docs_document_durable_state_and_platform_limits() {
    // The honest-gaps content moved out of README.md into the docs site; the
    // guarantee that these platform limits stay documented did not.
    let honest_gaps = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("docs")
            .join("honest-gaps.md"),
    )
    .unwrap();

    assert!(honest_gaps.contains("Durable kernel state rewrites the complete projection"));
    assert!(honest_gaps.contains("Headless Linux without an unlocked Secret Service"));
}

#[test]
fn owner_scoped_secrets_are_separate_from_host_config() {
    let mut service = HostConfigService::default();
    let mut kernel = Kernel::new(Arc::new(SilentChrome));
    let owner = llm_provider_id();
    service
        .put_secret(&mut kernel, &owner, "api-key", "secret-123".into())
        .unwrap();

    // host_config serialization must not contain secret values
    let serialized = serde_json::to_string(&service.get_host_config()).unwrap();
    assert!(!serialized.contains("secret-123"));
    assert!(!serialized.contains("secret_values"));

    // The secret store owns the value, not the config document.
    let ref_ = owner_ref("api-key");
    assert_eq!(
        service.secrets.read(&ref_).unwrap().as_deref(),
        Some("secret-123")
    );
}

#[test]
fn secrets_do_not_serialize_in_frontend_config_views() {
    // Verify that HostConfig serialization never includes secret values.
    let json = serde_json::to_value(default_host_config()).unwrap();
    let json_str = serde_json::to_string(&json).unwrap();
    assert!(!json_str.contains("secret_values"));
    // The ConnectorConfigView's secret_refs field is safe (it only
    // contains local names, not values).
    let view = ConnectorConfigView {
        id: "llm-provider/test".into(),
        kind: ConnectorKind::OpenAiCompatible,
        base_url: "https://example.test/v1".into(),
        default_model: "gpt-4.1".into(),
        default_variant: None,
        default_text_verbosity: None,
        secret_refs: BTreeMap::from([("api_key".into(), "work-key".into())]),
    };
    let view_str = serde_json::to_string(&view).unwrap();
    assert!(view_str.contains("work-key"));
    assert!(!view_str.contains("secret-value"));
}

#[test]
fn update_app_config_validates_against_manifest_schema() {
    let mut service = HostConfigService::default();
    let manifest = test_manifest();
    let updated = service
        .update_app_config(
            "chat",
            &manifest,
            serde_json::from_value(json!({"max_iterations": 7})).unwrap(),
        )
        .unwrap();
    assert_eq!(
        updated.get("max_iterations").and_then(Value::as_i64),
        Some(7)
    );

    let error = service
        .update_app_config(
            "chat",
            &manifest,
            serde_json::from_value(json!({"max_iterations": 0})).unwrap(),
        )
        .unwrap_err();
    assert!(error.contains("invalid config declaration 'chat' for app 'chat'"));
}

fn assert_update_app_config_failure_keeps_state(operation: FailingFileOperation) {
    let path = temp_config_path();
    HostConfigService::new(path.clone()).unwrap();
    let before = fs::read(&path).unwrap();
    let writer = Arc::new(FailingAtomicFileWriter::new(operation));
    let mut service = HostConfigService::with_writer_and_namespace(
        path.clone(),
        path.display().to_string(),
        writer,
    )
    .unwrap();
    let config_before = service.get_app_config("chat");

    let error = service
        .update_app_config(
            "chat",
            &test_manifest(),
            serde_json::from_value(json!({"max_iterations": 7})).unwrap(),
        )
        .unwrap_err();

    assert!(error.contains("host config failed"));
    assert_eq!(service.get_app_config("chat"), config_before);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn update_app_config_write_failure_keeps_memory_and_disk_unchanged() {
    assert_update_app_config_failure_keeps_state(FailingFileOperation::Write);
}

#[test]
fn update_app_config_rename_failure_keeps_memory_and_disk_unchanged() {
    assert_update_app_config_failure_keeps_state(FailingFileOperation::Rename);
}

#[test]
fn indeterminate_connector_delete_adopts_candidate_without_restoring_secret() {
    let path = temp_config_path();
    let mut service = HostConfigService::default();
    service
        .upsert_connector_config(ConnectorConfigView {
            id: "llm-provider/work-openai".into(),
            kind: ConnectorKind::OpenAiCompatible,
            base_url: "https://example.test/v1".into(),
            default_model: "gpt-4.1".into(),
            default_variant: None,
            default_text_verbosity: None,
            secret_refs: BTreeMap::from([(
                String::from("api_key"),
                String::from("llm-provider/work-openai/api_key"),
            )]),
        })
        .unwrap();
    let secret_ref = owner_ref("llm-provider/work-openai/api_key");
    service
        .secrets
        .write(&secret_ref, "secret-value".into())
        .unwrap();
    service.path = Some(path.clone());
    service.writer = Arc::new(FailingAtomicFileWriter::new(
        FailingFileOperation::SyncParent,
    ));

    let error = service
        .delete_connector_config("llm-provider/work-openai")
        .unwrap_err();

    assert!(error.contains("candidate was committed"), "{error}");
    assert!(!service
        .list_connector_configs()
        .iter()
        .any(|connector| connector.id == "llm-provider/work-openai"));
    assert_eq!(service.secrets.read(&secret_ref).unwrap(), None);
    let raw = std::fs::read_to_string(path).unwrap();
    assert!(!raw.contains("work-openai"));
}
