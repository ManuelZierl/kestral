//! One MCP session over any transport: handshake, tool discovery, tool
//! calls, shutdown. This is the only layer that knows MCP method names;
//! callers above it deal in validated tool definitions and plain JSON.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::bridge::McpToolCall;
use crate::errors::McpError;
use crate::protocol::{
    extract_tool_result, parse_tool, validate_tools, McpToolDefinition, LATEST_PROTOCOL_VERSION,
};
use crate::transport::{McpTransport, RequestOptions};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Upper bounds on `tools/list` pagination. An untrusted or defective server
/// must not be able to hold a discovery open indefinitely (a repeated cursor)
/// or force unbounded memory growth (endless distinct pages / tools), even
/// while answering each individual request within its timeout.
const MAX_TOOL_LIST_PAGES: usize = 1_000;
const MAX_TOOLS: usize = 10_000;
const MAX_TOOL_DISCOVERY_BYTES: usize = 16 * 1024 * 1024;

/// A ready (initialized) MCP session. Construction performs the handshake;
/// a value of this type always represents a server that answered it.
pub struct McpClient {
    transport: Box<dyn McpTransport>,
    negotiated_protocol_version: String,
    server_name: String,
}

impl McpClient {
    /// Perform the MCP initialize handshake and protocol-version
    /// negotiation over the given transport.
    pub fn connect(transport: Box<dyn McpTransport>) -> Result<Self, McpError> {
        let result = transport.request(
            "initialize",
            json!({
                "protocolVersion": LATEST_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "ai-app-host", "version": env!("CARGO_PKG_VERSION")},
            }),
            &RequestOptions::with_timeout(HANDSHAKE_TIMEOUT),
        )?;
        let (negotiated, server_name) = match validate_initialize_result(&result) {
            Ok(validated) => validated,
            Err(error) => {
                transport.shutdown();
                return Err(error);
            }
        };
        if let Err(error) = transport.notify("notifications/initialized", Value::Null) {
            transport.shutdown();
            return Err(error);
        }
        Ok(Self {
            transport,
            negotiated_protocol_version: negotiated,
            server_name,
        })
    }

    pub fn protocol_version(&self) -> &str {
        &self.negotiated_protocol_version
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// All advertised tools, following `nextCursor` pagination, with every
    /// schema validated before anything can reach an install path.
    pub fn list_tools(&self) -> Result<Vec<McpToolDefinition>, McpError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = std::collections::HashSet::new();
        let mut discovery_bytes = 0usize;
        for _ in 0..MAX_TOOL_LIST_PAGES {
            let params = match &cursor {
                Some(cursor) => json!({"cursor": cursor}),
                None => json!({}),
            };
            let result = self.transport.request(
                "tools/list",
                params,
                &RequestOptions::with_timeout(HANDSHAKE_TIMEOUT),
            )?;
            let page = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    McpError::Protocol("tools/list result carries no tools array".into())
                })?;
            for tool in page {
                if tools.len() >= MAX_TOOLS {
                    return Err(McpError::Protocol(format!(
                        "server advertised more than {MAX_TOOLS} tools"
                    )));
                }
                discovery_bytes = discovery_bytes
                    .checked_add(
                        serde_json::to_vec(tool)
                            .map_err(|error| {
                                McpError::Protocol(format!(
                                    "tools/list tool is not serializable: {error}"
                                ))
                            })?
                            .len(),
                    )
                    .filter(|size| *size <= MAX_TOOL_DISCOVERY_BYTES)
                    .ok_or_else(|| {
                        McpError::Protocol(format!(
                            "tool discovery exceeded {MAX_TOOL_DISCOVERY_BYTES} bytes"
                        ))
                    })?;
                tools.push(parse_tool(tool)?);
            }
            cursor = match result.get("nextCursor") {
                None => None,
                Some(Value::String(next)) => {
                    discovery_bytes = discovery_bytes
                        .checked_add(next.len())
                        .filter(|size| *size <= MAX_TOOL_DISCOVERY_BYTES)
                        .ok_or_else(|| {
                            McpError::Protocol(format!(
                                "tool discovery exceeded {MAX_TOOL_DISCOVERY_BYTES} bytes"
                            ))
                        })?;
                    Some(next.clone())
                }
                Some(_) => {
                    return Err(McpError::Protocol(
                        "tools/list nextCursor must be a string when present".into(),
                    ))
                }
            };
            let Some(next) = &cursor else {
                validate_tools(&tools)?;
                return Ok(tools);
            };
            if !seen_cursors.insert(next.clone()) {
                return Err(McpError::Protocol(
                    "tools/list returned a repeated pagination cursor".into(),
                ));
            }
        }
        Err(McpError::Protocol(format!(
            "tools/list did not terminate within {MAX_TOOL_LIST_PAGES} pages"
        )))
    }

    /// Invoke one tool. Tool-reported errors, server errors, timeouts, and
    /// transport failures all come back as `McpError` — contained by the
    /// bridge as invocation failures, never a crash.
    pub fn call_tool(
        &self,
        tool: &str,
        arguments: &app_host_kernel::JsonObject,
        options: &RequestOptions,
    ) -> Result<Value, McpError> {
        let result = self.transport.request(
            "tools/call",
            json!({"name": tool, "arguments": arguments}),
            options,
        )?;
        extract_tool_result(tool, &result)
    }

    /// The bridge's `call_tool` seam, backed by this session. The returned
    /// closure keeps the client (and its transport) alive for as long as
    /// the installed handlers exist, and honors the invocation's
    /// cooperative cancellation while waiting on the server.
    pub fn as_tool_call(self: &Arc<Self>) -> McpToolCall {
        let client = self.clone();
        Arc::new(move |tool, arguments, context| {
            let cancellation = context.cancellation.clone();
            let options = RequestOptions {
                timeout: CALL_TIMEOUT,
                cancel: Some(Arc::new(move || cancellation.is_cancelled())),
            };
            client
                .call_tool(tool, arguments, &options)
                .map_err(|error| error.to_string())
        })
    }

    /// End the session (terminate the process / HTTP session). The client
    /// is unusable afterwards; drop also shuts down.
    pub fn shutdown(&self) {
        self.transport.shutdown();
    }
}

fn validate_initialize_result(result: &Value) -> Result<(String, String), McpError> {
    let negotiated = result
        .get("protocolVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::Protocol("initialize result carries no protocolVersion".into()))?
        .to_string();
    if negotiated != LATEST_PROTOCOL_VERSION {
        return Err(McpError::Protocol(format!(
            "server negotiated unsupported MCP protocol version '{negotiated}'; \
             expected '{LATEST_PROTOCOL_VERSION}'"
        )));
    }
    let capabilities = result
        .get("capabilities")
        .and_then(Value::as_object)
        .ok_or_else(|| McpError::Protocol("initialize result carries no capabilities".into()))?;
    if !capabilities.get("tools").is_some_and(Value::is_object) {
        return Err(McpError::Protocol(
            "server does not advertise the MCP tools capability".into(),
        ));
    }
    let server_info = result
        .get("serverInfo")
        .and_then(Value::as_object)
        .ok_or_else(|| McpError::Protocol("initialize result carries no serverInfo".into()))?;
    let server_name = server_info
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| McpError::Protocol("serverInfo carries no name".into()))?
        .to_string();
    if !server_info
        .get("version")
        .and_then(Value::as_str)
        .is_some_and(|version| !version.trim().is_empty())
    {
        return Err(McpError::Protocol("serverInfo carries no version".into()));
    }
    Ok((negotiated, server_name))
}

impl Drop for McpClient {
    fn drop(&mut self) {
        self.transport.shutdown();
    }
}

#[cfg(test)]
mod tests;
