//! Adapter error shape. Every failure a server or transport can produce is
//! mapped into one of these variants; the bridge then contains them as
//! invocation failures — a misbehaving server can never crash the host.

use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// The wire is broken: process died, connection refused, malformed HTTP.
    #[error("MCP transport failure: {0}")]
    Transport(String),

    /// The server answered, but not in the shape the protocol requires.
    #[error("MCP protocol violation: {0}")]
    Protocol(String),

    /// The server did not answer inside the budget.
    #[error("MCP '{method}' timed out after {timeout:?}")]
    Timeout { method: String, timeout: Duration },

    /// The caller cancelled the request (or its deadline passed).
    #[error("MCP request cancelled")]
    Cancelled,

    /// A JSON-RPC error object from the server.
    #[error("MCP server error {code}: {message}")]
    Server { code: i64, message: String },

    /// The tool itself reported failure (`isError: true` result).
    #[error("MCP tool failed: {0}")]
    Tool(String),

    /// A tool advertised a schema that is not valid JSON Schema. Caught
    /// before installation so a bad tool never enters the registry.
    #[error("MCP tool '{tool}' advertises an invalid {which} schema: {reason}")]
    InvalidToolSchema {
        tool: String,
        which: &'static str,
        reason: String,
    },
}
