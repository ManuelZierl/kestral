use std::collections::BTreeSet;

use serde_json::json;

use super::*;

const SOURCE_ID: &str = "com.example.model-profiles";
const SOURCE_NAME: &str = "Model Setup";

fn source() -> ModelProfileSource {
    ModelProfileSource {
        app_id: SOURCE_ID.into(),
        display_name: SOURCE_NAME.into(),
        version: "0.1.0".into(),
    }
}

fn config() -> JsonObject {
    json!({
        "profiles": [{
            "id": "focused-work",
            "title": "Focused work",
            "description": "Uses only note reads.",
            "connector_id": "llm-provider/local",
            "model": "model-a",
            "reasoning": "high",
            "temperature": 0.2,
            "max_output_tokens": 4096,
            "tools": ["notes/read", "notes/write"],
            "prompt": null
        }]
    })
    .as_object()
    .cloned()
    .unwrap()
}

#[test]
fn unsaved_first_run_config_has_no_profiles() {
    assert!(parse_profiles(&JsonObject::new()).unwrap().is_empty());

    let null_profiles = json!({"profiles": null}).as_object().cloned().unwrap();
    assert!(parse_profiles(&null_profiles).is_err());
}

fn prompt_config(layer_ids: Vec<&str>, custom_texts: Vec<&str>) -> serde_json::Value {
    json!({
        "layer_ids": layer_ids,
        "custom_texts": custom_texts,
    })
}

#[test]
fn profile_tools_are_intersected_with_chat_grants() {
    let views = profile_views(
        &config(),
        &source(),
        &BTreeSet::from(["notes/read".into(), "other/tool".into()]),
        &BTreeSet::from(["llm-provider/local".into()]),
        &BTreeSet::from(["llm-provider/local".into()]),
        &BTreeSet::from([
            "protocol".into(),
            "assistant-instructions".into(),
            "runtime-context".into(),
        ]),
    )
    .unwrap();
    assert_eq!(views[0].effective_tool_refs, vec!["notes/read"]);
    assert_eq!(views[0].unavailable_tool_refs, vec!["notes/write"]);
    assert_eq!(views[0].receipt.source_app_id, SOURCE_ID);
    assert_eq!(views[0].source_app_name, SOURCE_NAME);
    assert!(views[0].available);
}

#[test]
fn receipts_detect_changed_profile_content() {
    let receipt = resolve_profile(&config(), "focused-work", SOURCE_ID, "0.1.0").unwrap();
    assert!(profile_is_current(&config(), SOURCE_ID, "0.1.0", &receipt).unwrap());
    let mut changed = config();
    changed["profiles"][0]["model"] = json!("model-b");
    assert!(!profile_is_current(&changed, SOURCE_ID, "0.1.0", &receipt).unwrap());
    assert!(!profile_is_current(&config(), SOURCE_ID, "0.2.0", &receipt).unwrap());
    assert!(!profile_is_current(&config(), "com.example.other", "0.1.0", &receipt).unwrap());
}

#[test]
fn invalid_and_duplicate_authority_entries_fail_closed() {
    let mut invalid = config();
    invalid["profiles"][0]["tools"] = json!(["notes/read", "notes/read"]);
    assert!(parse_profiles(&invalid).unwrap_err().contains("duplicate"));

    invalid["profiles"][0]["tools"] = json!(["missing-slash"]);
    assert!(parse_profiles(&invalid)
        .unwrap_err()
        .contains("invalid model profile tool"));

    invalid["profiles"][0]["temperature"] = json!(2.1);
    assert!(parse_profiles(&invalid)
        .unwrap_err()
        .contains("temperature"));
}

#[test]
fn tool_length_uses_json_schema_character_count() {
    let mut unicode = config();
    unicode["profiles"][0]["tools"] = json!([format!("{}/a", "😀".repeat(64))]);
    assert!(parse_profiles(&unicode).is_ok());

    unicode["profiles"][0]["tools"] = json!([format!("{}/a", "a".repeat(256))]);
    assert!(parse_profiles(&unicode)
        .unwrap_err()
        .contains("invalid model profile tool"));
}

#[test]
fn missing_connector_marks_profile_unavailable_without_expanding_tools() {
    let views = profile_views(
        &config(),
        &source(),
        &BTreeSet::new(),
        &BTreeSet::new(),
        &BTreeSet::new(),
        &BTreeSet::from(["protocol".into()]),
    )
    .unwrap();
    assert!(!views[0].available);
    assert_eq!(views[0].effective_tool_refs, Vec::<String>::new());
    assert_eq!(views[0].unavailable_tool_refs.len(), 2);
}

#[test]
fn prompt_validation_rejects_protocol_duplicates_and_excess_text() {
    let mut invalid = config();
    invalid["profiles"][0]["prompt"] = prompt_config(vec!["protocol"], vec![]);
    assert!(parse_profiles(&invalid)
        .unwrap_err()
        .contains("protocol layer"));

    invalid["profiles"][0]["prompt"] = prompt_config(
        vec!["assistant-instructions", "assistant-instructions"],
        vec![],
    );
    assert!(parse_profiles(&invalid).unwrap_err().contains("duplicate"));

    invalid["profiles"][0]["prompt"] = prompt_config(vec!["assistant-instructions"], vec![" "; 1]);
    assert!(parse_profiles(&invalid)
        .unwrap_err()
        .contains("trimmed and non-empty"));
}

#[test]
fn profile_prompt_availability_fails_closed_for_unavailable_layer() {
    let mut cfg = config();
    cfg["profiles"][0]["prompt"] = prompt_config(vec!["skill:notes/read"], vec![]);
    let views = profile_views(
        &cfg,
        &source(),
        &BTreeSet::from(["notes/read".into(), "notes/write".into()]),
        &BTreeSet::from(["llm-provider/local".into()]),
        &BTreeSet::from(["llm-provider/local".into()]),
        &BTreeSet::from(["protocol".into(), "assistant-instructions".into()]),
    )
    .unwrap();
    assert!(!views[0].available);
    assert!(views[0]
        .availability_reason
        .as_ref()
        .unwrap()
        .contains("unavailable"));
}
