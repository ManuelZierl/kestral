//! MCP wire vocabulary: protocol versions, tool declarations, and result
//! extraction. Pure parsing — no I/O.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use app_host_kernel::JsonObject;

use crate::errors::McpError;

/// The protocol revision this adapter speaks natively.
pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";

/// A tool as an MCP server advertises it, reduced to what the bridge needs.
/// Input schemas are mandatory; output schemas are imported when the server
/// provides one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: JsonObject,
    #[serde(default)]
    pub output_schema: Option<JsonObject>,
}

/// Parse one entry of a `tools/list` result.
pub fn parse_tool(tool: &Value) -> Result<McpToolDefinition, McpError> {
    let name = tool
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| McpError::Protocol("advertised tool has no name".into()))?;
    let input_schema = match tool.get("inputSchema") {
        Some(Value::Object(schema)) => schema.clone(),
        None => {
            return Err(McpError::Protocol(format!(
                "tool '{name}' has no inputSchema"
            )))
        }
        Some(other) => {
            return Err(McpError::Protocol(format!(
                "tool '{name}' has a non-object inputSchema: {other}"
            )))
        }
    };
    let output_schema = match tool.get("outputSchema") {
        None => None,
        Some(Value::Object(schema)) => Some(schema.clone()),
        Some(other) => {
            return Err(McpError::Protocol(format!(
                "tool '{name}' has a non-object outputSchema: {other}"
            )))
        }
    };
    Ok(McpToolDefinition {
        name: name.to_string(),
        description: tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        input_schema,
        output_schema,
    })
}

/// Reject tools whose advertised schemas do not compile as JSON Schema —
/// before anything reaches the kernel registry. Also rejects duplicate
/// tool names, which would collide as capability names.
pub fn validate_tools(tools: &[McpToolDefinition]) -> Result<(), McpError> {
    let mut seen = std::collections::BTreeSet::new();
    for tool in tools {
        if !seen.insert(tool.name.as_str()) {
            return Err(McpError::Protocol(format!(
                "server advertises duplicate tool name '{}'",
                tool.name
            )));
        }
        validate_schema(&tool.name, "input", &tool.input_schema)?;
        if let Some(output_schema) = &tool.output_schema {
            validate_schema(&tool.name, "output", output_schema)?;
        }
    }
    Ok(())
}

fn validate_schema(tool: &str, which: &'static str, schema: &JsonObject) -> Result<(), McpError> {
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err(McpError::InvalidToolSchema {
            tool: tool.to_string(),
            which,
            reason: "MCP tool schemas must declare an object root".into(),
        });
    }
    jsonschema::validator_for(&Value::Object(schema.clone())).map_err(|error| {
        McpError::InvalidToolSchema {
            tool: tool.to_string(),
            which,
            reason: error.to_string(),
        }
    })?;
    Ok(())
}

/// Reduce a `tools/call` result to one JSON value, mapping tool-reported
/// errors (`isError: true`) to a contained failure.
pub fn extract_tool_result(tool: &str, result: &Value) -> Result<Value, McpError> {
    let object = result
        .as_object()
        .ok_or_else(|| McpError::Protocol(format!("tool '{tool}' returned a non-object result")))?;
    let content = object
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            McpError::Protocol(format!("tool '{tool}' result carries no content array"))
        })?;
    for item in content {
        let item_type = item.get("type").and_then(Value::as_str).ok_or_else(|| {
            McpError::Protocol(format!("tool '{tool}' returned an untyped content item"))
        })?;
        if item_type == "text" && item.get("text").and_then(Value::as_str).is_none() {
            return Err(McpError::Protocol(format!(
                "tool '{tool}' returned text content without text"
            )));
        }
    }
    let is_error = match object.get("isError") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Err(McpError::Protocol(format!(
                "tool '{tool}' result has a non-boolean isError"
            )))
        }
    };
    let text = joined_text_content(content);
    if is_error {
        return Err(McpError::Tool(if text.is_empty() {
            format!("tool '{tool}' reported an error")
        } else {
            text
        }));
    }
    if let Some(structured) = object.get("structuredContent") {
        return structured
            .as_object()
            .cloned()
            .map(Value::Object)
            .ok_or_else(|| {
                McpError::Protocol(format!(
                    "tool '{tool}' returned non-object structuredContent"
                ))
            });
    }
    if content
        .iter()
        .any(|item| item.get("type").and_then(Value::as_str) != Some("text"))
    {
        return Ok(Value::Array(content.clone()));
    }
    // Text-only servers: surface parsed JSON when the text is JSON,
    // otherwise the text itself.
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

/// Extract the `result` member of an already-id-matched JSON-RPC response,
/// mapping an `error` member to `McpError::Server`. Shared by every
/// transport's response handling (stdio, HTTP JSON, HTTP SSE) so the
/// error-code/message defaulting lives in exactly one place.
pub(crate) fn extract_result_or_server_error(message: &Value) -> Result<Value, McpError> {
    let object = message
        .as_object()
        .ok_or_else(|| McpError::Protocol("JSON-RPC response is not an object".into()))?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(McpError::Protocol(
            "JSON-RPC response has an invalid or missing version".into(),
        ));
    }
    if object.contains_key("method") {
        return Err(McpError::Protocol(
            "JSON-RPC response must not carry a method".into(),
        ));
    }
    let result = object.get("result");
    let error = object.get("error");
    if result.is_some() == error.is_some() {
        return Err(McpError::Protocol(
            "JSON-RPC response must carry exactly one of result or error".into(),
        ));
    }
    if let Some(error) = error {
        let error = error
            .as_object()
            .ok_or_else(|| McpError::Protocol("JSON-RPC error is not an object".into()))?;
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .ok_or_else(|| McpError::Protocol("JSON-RPC error carries no integer code".into()))?;
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::Protocol("JSON-RPC error carries no string message".into()))?;
        return Err(McpError::Server {
            code,
            message: message.to_string(),
        });
    }
    Ok(result.cloned().expect("validated response carries result"))
}

fn joined_text_content(items: &[Value]) -> String {
    items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests;
