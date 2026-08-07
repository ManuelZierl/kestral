use super::*;

#[test]
fn provider_timeout_is_not_reported_as_user_cancellation() {
    assert_eq!(
        user_facing_chat_error("LLM call failed: pi-ai worker response timed out"),
        "The model provider did not respond within 120 seconds. Completed tool calls remain in the run history."
    );
}

#[test]
fn unconfigured_provider_failure_has_actionable_guidance() {
    assert_eq!(
        user_facing_chat_error(&format!(
            "LLM call failed: {}",
            crate::llm_provider::NO_PROVIDER_CONFIGURED_ERROR
        )),
        "I can't generate a model response because no model provider is configured. Use Configure model provider below or open Settings -> Model providers, then add a profile and select it as the default for Chat."
    );
}

#[test]
fn unavailable_execution_path_does_not_point_to_a_nonexistent_run() {
    assert_eq!(
        user_facing_chat_error("chat has no available execution path"),
        "Chat has no available model execution path. Configure a model provider or restore Chat's model or agent permission in Settings -> Permissions."
    );
}

#[test]
fn provider_failure_points_to_provider_settings() {
    assert_eq!(
        user_facing_chat_error("LLM call failed: provider authentication failed"),
        "The selected model provider could not complete the request. Check its connection and credentials in Settings -> Model providers. Completed tool calls remain in the run history."
    );
}

#[test]
fn agent_denial_points_to_permissions() {
    assert_eq!(
        user_facing_chat_error("Agent Engine was denied. Technical detail: GrantRevoked"),
        "Chat's Agent Engine permission is no longer available. Open Settings -> Permissions to review it, then try again."
    );
}

#[test]
fn agent_failure_does_not_prescribe_reinstalling_the_engine() {
    assert_eq!(
        user_facing_chat_error(
            "Agent Engine could not complete this message. Technical detail: invalid tool input"
        ),
        "Agent Engine could not complete this request. Retry once. If it keeps failing, open System for the run details."
    );
}
