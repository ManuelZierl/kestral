use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use app_host_kernel::ids::{AppId, ConfigName, ExtensionPointName};
use app_host_kernel::kernel::Kernel;
use app_host_kernel::services::registry::InstalledApp;
use app_host_kernel::JsonObject;

use crate::chat_store::deserialize_required_option;

pub const MODEL_PROFILE_EXTENSION_POINT: &str = "model-profile-editor";
pub const MODEL_PROFILE_CONTRACT_VERSION: u32 = 1;
pub const MODEL_PROFILE_CONFIG: &str = "model-profiles";
const MAX_PROFILES: usize = 64;
const MAX_TOOLS: usize = 64;
const MAX_PROMPT_LAYER_IDS: usize = 64;
const MAX_PROMPT_CUSTOM_TEXTS: usize = 8;
const MAX_PROMPT_CUSTOM_TEXT_CHARS: usize = 16 * 1024;
const MAX_PROMPT_TOTAL_CUSTOM_TEXT_CHARS: usize = 32 * 1024;
const MAX_OUTPUT_TOKENS: u64 = 1_000_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChatModelProfilePrompt {
    pub layer_ids: Vec<String>,
    pub custom_texts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChatModelProfile {
    pub id: String,
    pub title: String,
    pub description: String,
    pub connector_id: String,
    pub model: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub reasoning: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub temperature: Option<f64>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub max_output_tokens: Option<u64>,
    pub tools: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub prompt: Option<ChatModelProfilePrompt>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelProfilesConfig {
    // An app with no saved config has no HostConfig entry yet, so the generic
    // config service returns an empty object until the app performs its first
    // schema-validated write.
    #[serde(default)]
    profiles: Vec<ChatModelProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChatModelProfileReceipt {
    pub source_app_id: String,
    pub source_app_version: String,
    pub profile_id: String,
    pub profile_digest: String,
    pub title: String,
    pub connector_id: String,
    pub model: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub reasoning: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub temperature: Option<f64>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub max_output_tokens: Option<u64>,
    pub tool_refs: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub prompt: Option<ChatModelProfilePrompt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChatModelProfileView {
    #[serde(flatten)]
    pub receipt: ChatModelProfileReceipt,
    pub source_app_name: String,
    pub description: String,
    pub effective_tool_refs: Vec<String>,
    pub unavailable_tool_refs: Vec<String>,
    pub available: bool,
    pub availability_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelProfileSource {
    pub app_id: String,
    pub display_name: String,
    pub version: String,
}

pub fn parse_profiles(config: &JsonObject) -> Result<Vec<ChatModelProfile>, String> {
    let parsed: ModelProfilesConfig =
        serde_json::from_value(serde_json::Value::Object(config.clone()))
            .map_err(|error| format!("invalid model profiles config: {error}"))?;
    if parsed.profiles.len() > MAX_PROFILES {
        return Err(format!(
            "model profiles config exceeds {MAX_PROFILES} profiles"
        ));
    }

    let mut ids = BTreeSet::new();
    for profile in &parsed.profiles {
        validate_profile(profile)?;
        if !ids.insert(profile.id.as_str()) {
            return Err(format!("duplicate model profile id '{}'", profile.id));
        }
    }
    Ok(parsed.profiles)
}

pub fn resolve_profile(
    config: &JsonObject,
    profile_id: &str,
    source_app_id: &str,
    source_app_version: &str,
) -> Result<ChatModelProfileReceipt, String> {
    let profile = parse_profiles(config)?
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("unknown model profile: {profile_id}"))?;
    receipt_for(&profile, source_app_id, source_app_version)
}

pub fn profile_is_current(
    config: &JsonObject,
    source_app_id: &str,
    source_app_version: &str,
    receipt: &ChatModelProfileReceipt,
) -> Result<bool, String> {
    if receipt.source_app_id != source_app_id || receipt.source_app_version != source_app_version {
        return Ok(false);
    }
    let Some(profile) = parse_profiles(config)?
        .into_iter()
        .find(|profile| profile.id == receipt.profile_id)
    else {
        return Ok(false);
    };
    let current = receipt_for(&profile, source_app_id, source_app_version)?;
    Ok(current.profile_digest == receipt.profile_digest)
}

pub fn profile_views(
    config: &JsonObject,
    source: &ModelProfileSource,
    granted_tools: &BTreeSet<String>,
    configured_connectors: &BTreeSet<String>,
    selectable_connectors: &BTreeSet<String>,
    current_prompt_layers: &BTreeSet<String>,
) -> Result<Vec<ChatModelProfileView>, String> {
    parse_profiles(config)?
        .into_iter()
        .map(|profile| {
            let receipt = receipt_for(&profile, &source.app_id, &source.version)?;
            let effective_tool_refs = profile
                .tools
                .iter()
                .filter(|tool| granted_tools.contains(*tool))
                .cloned()
                .collect();
            let unavailable_tool_refs = profile
                .tools
                .iter()
                .filter(|tool| !granted_tools.contains(*tool))
                .cloned()
                .collect();
            let configured = configured_connectors.contains(&profile.connector_id);
            let available = selectable_connectors.contains(&profile.connector_id);
            let prompt_available = profile
                .prompt
                .as_ref()
                .is_none_or(|prompt| prompt.layer_ids.iter().all(|layer_id| current_prompt_layers.contains(layer_id)));
            let available = available && prompt_available;
            Ok(ChatModelProfileView {
                receipt,
                source_app_name: source.display_name.clone(),
                description: profile.description,
                effective_tool_refs,
                unavailable_tool_refs,
                available,
                availability_reason: (!available).then(|| {
                    if !prompt_available {
                        "Model profile prompt references chat layers that are currently unavailable".into()
                    } else if configured {
                        format!(
                            "Model provider profile '{}' uses a credential; choose it as Default for Chat first",
                            profile.connector_id
                        )
                    } else {
                        format!(
                            "Model provider profile '{}' is not configured",
                            profile.connector_id
                        )
                    }
                }),
            })
        })
        .collect()
}

fn receipt_for(
    profile: &ChatModelProfile,
    source_app_id: &str,
    source_app_version: &str,
) -> Result<ChatModelProfileReceipt, String> {
    let encoded = serde_json::to_vec(profile)
        .map_err(|error| format!("serialize model profile failed: {error}"))?;
    Ok(ChatModelProfileReceipt {
        source_app_id: source_app_id.into(),
        source_app_version: source_app_version.into(),
        profile_id: profile.id.clone(),
        profile_digest: format!("{:x}", Sha256::digest(encoded)),
        title: profile.title.clone(),
        connector_id: profile.connector_id.clone(),
        model: profile.model.clone(),
        reasoning: profile.reasoning.clone(),
        temperature: profile.temperature,
        max_output_tokens: profile.max_output_tokens,
        tool_refs: profile.tools.clone(),
        prompt: profile.prompt.clone(),
    })
}

pub fn is_model_profile_source(app: &InstalledApp) -> bool {
    let contributes = app
        .manifest
        .extension_contributions
        .iter()
        .any(|contribution| {
            contribution.target_app == AppId::new("chat")
                && contribution.extension_point
                    == ExtensionPointName::new(MODEL_PROFILE_EXTENSION_POINT)
                && contribution.contract_version == MODEL_PROFILE_CONTRACT_VERSION
                && app
                    .manifest
                    .surfaces
                    .iter()
                    .any(|surface| surface.name == contribution.surface)
        });
    contributes
        && app
            .manifest
            .config_declarations
            .iter()
            .any(|declaration| declaration.name == ConfigName::new(MODEL_PROFILE_CONFIG))
}

pub fn model_profile_sources(kernel: &Kernel) -> Vec<&InstalledApp> {
    let mut sources = kernel
        .installed_apps()
        .filter(|app| is_model_profile_source(app))
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| left.manifest.app_id.cmp(&right.manifest.app_id));
    sources
}

pub fn model_profile_source<'a>(
    kernel: &'a Kernel,
    app_id: &str,
) -> Result<&'a InstalledApp, String> {
    let app = kernel
        .installed_app(&AppId::new(app_id))
        .map_err(|_| format!("model profile source is not installed or enabled: {app_id}"))?;
    is_model_profile_source(app)
        .then_some(app)
        .ok_or_else(|| format!("app does not contribute the model profile contract: {app_id}"))
}

fn validate_profile(profile: &ChatModelProfile) -> Result<(), String> {
    if !valid_profile_id(&profile.id) {
        return Err(format!("invalid model profile id '{}'", profile.id));
    }
    validate_text(&profile.title, "title", 120, false)?;
    validate_text(&profile.description, "description", 1000, true)?;
    validate_text(&profile.connector_id, "connector_id", 256, false)?;
    validate_text(&profile.model, "model", 256, false)?;
    if let Some(reasoning) = &profile.reasoning {
        if !matches!(
            reasoning.as_str(),
            "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        ) {
            return Err(format!("invalid reasoning value '{reasoning}'"));
        }
    }
    if let Some(temperature) = profile.temperature {
        if !temperature.is_finite() || !(0.0..=2.0).contains(&temperature) {
            return Err("temperature must be between 0 and 2".into());
        }
    }
    if profile
        .max_output_tokens
        .is_some_and(|value| value == 0 || value > MAX_OUTPUT_TOKENS)
    {
        return Err(format!(
            "max_output_tokens must be between 1 and {MAX_OUTPUT_TOKENS}"
        ));
    }
    if profile.tools.len() > MAX_TOOLS {
        return Err(format!("model profile exceeds {MAX_TOOLS} tools"));
    }
    let mut tools = BTreeSet::new();
    for tool in &profile.tools {
        validate_tool_ref(tool)?;
        if !tools.insert(tool.as_str()) {
            return Err(format!("duplicate model profile tool '{tool}'"));
        }
    }
    if let Some(prompt) = &profile.prompt {
        validate_prompt(prompt)?;
    }
    Ok(())
}

fn validate_prompt(prompt: &ChatModelProfilePrompt) -> Result<(), String> {
    if prompt.layer_ids.len() > MAX_PROMPT_LAYER_IDS {
        return Err(format!(
            "model profile prompt exceeds {MAX_PROMPT_LAYER_IDS} layer ids"
        ));
    }
    let mut layer_ids = BTreeSet::new();
    for layer_id in &prompt.layer_ids {
        validate_text(layer_id, "prompt layer id", 256, false)?;
        if layer_id == "protocol" {
            return Err("model profile prompt cannot override protocol layer".into());
        }
        if !layer_ids.insert(layer_id.as_str()) {
            return Err(format!("duplicate model profile prompt layer '{layer_id}'"));
        }
    }
    if prompt.custom_texts.len() > MAX_PROMPT_CUSTOM_TEXTS {
        return Err(format!(
            "model profile prompt exceeds {MAX_PROMPT_CUSTOM_TEXTS} custom texts"
        ));
    }
    let mut total_chars = 0usize;
    for custom_text in &prompt.custom_texts {
        validate_text(
            custom_text,
            "prompt custom text",
            MAX_PROMPT_CUSTOM_TEXT_CHARS,
            false,
        )?;
        total_chars += custom_text.chars().count();
    }
    if total_chars > MAX_PROMPT_TOTAL_CUSTOM_TEXT_CHARS {
        return Err(format!(
            "model profile prompt custom texts exceed {MAX_PROMPT_TOTAL_CUSTOM_TEXT_CHARS} characters"
        ));
    }
    Ok(())
}

fn valid_profile_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn validate_text(value: &str, field: &str, max: usize, allow_empty: bool) -> Result<(), String> {
    if (!allow_empty && value.trim().is_empty()) || value != value.trim() {
        return Err(format!(
            "model profile {field} must be trimmed and non-empty"
        ));
    }
    if value.chars().count() > max {
        return Err(format!("model profile {field} exceeds {max} characters"));
    }
    Ok(())
}

fn validate_tool_ref(value: &str) -> Result<(), String> {
    if value.chars().count() > 257 || value != value.trim() {
        return Err(format!("invalid model profile tool '{value}'"));
    }
    let Some((provider, capability)) = value.split_once('/') else {
        return Err(format!("invalid model profile tool '{value}'"));
    };
    if provider.is_empty()
        || capability.is_empty()
        || capability.contains('/')
        || value.chars().any(char::is_whitespace)
    {
        return Err(format!("invalid model profile tool '{value}'"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
