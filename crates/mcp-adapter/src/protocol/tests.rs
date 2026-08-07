use super::*;
use serde_json::json;

fn object(value: Value) -> JsonObject {
    match value {
        Value::Object(object) => object,
        _ => unreachable!("test literals are objects"),
    }
}

#[test]
fn parses_tool_with_output_schema() {
    let tool = parse_tool(&json!({
        "name": "get_forecast",
        "description": "Weather",
        "inputSchema": {"type": "object"},
        "outputSchema": {"type": "object", "properties": {"city": {"type": "string"}}},
    }))
    .unwrap();
    assert_eq!(tool.name, "get_forecast");
    assert!(tool.output_schema.is_some());
}

#[test]
fn nameless_or_malformed_tools_are_protocol_errors() {
    assert!(matches!(
        parse_tool(&json!({"description": "no name"})),
        Err(McpError::Protocol(_))
    ));
    assert!(matches!(
        parse_tool(&json!({"name": "x", "inputSchema": "not-an-object"})),
        Err(McpError::Protocol(_))
    ));
}

#[test]
fn missing_input_schema_is_rejected() {
    assert!(matches!(
        parse_tool(&json!({"name": "x"})),
        Err(McpError::Protocol(message)) if message == "tool 'x' has no inputSchema"
    ));
}

#[test]
fn invalid_advertised_schema_is_rejected_before_install() {
    let tools = vec![McpToolDefinition {
        name: "broken".into(),
        description: String::new(),
        input_schema: object(json!({"type": "no-such-type"})),
        output_schema: None,
    }];
    assert!(matches!(
        validate_tools(&tools),
        Err(McpError::InvalidToolSchema { which: "input", .. })
    ));
}

#[test]
fn non_object_tool_schema_roots_are_rejected() {
    let tools = vec![McpToolDefinition {
        name: "broken".into(),
        description: String::new(),
        input_schema: object(json!({"type": "string"})),
        output_schema: Some(object(json!({"type": "array"}))),
    }];
    assert!(matches!(
        validate_tools(&tools),
        Err(McpError::InvalidToolSchema { which: "input", .. })
    ));
}

#[test]
fn duplicate_tool_names_are_rejected() {
    let tool = McpToolDefinition {
        name: "twice".into(),
        description: String::new(),
        input_schema: object(json!({"type": "object"})),
        output_schema: None,
    };
    assert!(matches!(
        validate_tools(&[tool.clone(), tool]),
        Err(McpError::Protocol(_))
    ));
}

#[test]
fn tool_error_results_are_contained_failures() {
    let error = extract_tool_result(
        "get_forecast",
        &json!({"isError": true, "content": [{"type": "text", "text": "city unknown"}]}),
    )
    .unwrap_err();
    assert!(matches!(error, McpError::Tool(message) if message == "city unknown"));
}

#[test]
fn structured_content_wins_over_text() {
    let result = extract_tool_result(
        "t",
        &json!({
            "content": [{"type": "text", "text": "{\"a\": 1}"}],
            "structuredContent": {"a": 2},
        }),
    )
    .unwrap();
    assert_eq!(result, json!({"a": 2}));
}

#[test]
fn text_only_results_parse_as_json_when_possible() {
    let parsed = extract_tool_result(
        "t",
        &json!({"content": [{"type": "text", "text": "{\"a\": 1}"}]}),
    )
    .unwrap();
    assert_eq!(parsed, json!({"a": 1}));

    let plain = extract_tool_result(
        "t",
        &json!({"content": [{"type": "text", "text": "just words"}]}),
    )
    .unwrap();
    assert_eq!(plain, json!("just words"));
}

#[test]
fn non_text_content_is_preserved_without_fake_empty_text() {
    let content = json!([{
        "type": "image",
        "data": "aGVsbG8=",
        "mimeType": "image/png"
    }]);
    let result = extract_tool_result("t", &json!({"content": content})).unwrap();
    assert_eq!(result, content);
}

#[test]
fn malformed_tool_results_are_protocol_errors() {
    for result in [
        json!({}),
        json!({"content": "not-an-array"}),
        json!({"content": [{"type": "text"}]}),
        json!({"content": [], "structuredContent": []}),
        json!({"content": [], "isError": "yes"}),
    ] {
        assert!(matches!(
            extract_tool_result("t", &result),
            Err(McpError::Protocol(_))
        ));
    }
}

#[test]
fn malformed_json_rpc_responses_never_become_success() {
    for response in [
        json!({"id": 1, "result": {}}),
        json!({"jsonrpc": "1.0", "id": 1, "result": {}}),
        json!({"jsonrpc": "2.0", "id": 1}),
        json!({"jsonrpc": "2.0", "id": 1, "result": {}, "error": {"code": -1, "message": "x"}}),
        json!({"jsonrpc": "2.0", "id": 1, "error": {"message": "missing code"}}),
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "result": {}}),
    ] {
        assert!(matches!(
            extract_result_or_server_error(&response),
            Err(McpError::Protocol(_))
        ));
    }
}
