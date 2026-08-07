//! MCP consumer adapter.
//!
//! This crate is the boundary between the Model Context Protocol and the
//! host kernel. MCP is an adapter protocol here, not the internal ontology:
//! the kernel receives only generic manifests, JSON schemas, capability
//! handlers, and artifact drafts. Everything MCP-specific — wire types,
//! transports, session handshakes, error shapes — stays on this side.
//!
//! Layers, from the wire up:
//!
//! - [`transport`] — how JSON-RPC messages reach a server: [`stdio`]
//!   (child process, newline-delimited) and [`http`] (MCP Streamable HTTP
//!   with sessions, protocol-version headers, JSON and SSE responses).
//! - [`client`] — one MCP session over any transport: initialize handshake,
//!   protocol-version negotiation, paginated `tools/list` with schema
//!   validation, `tools/call` with contained error mapping, clean shutdown.
//! - [`bridge`] — the degraded-mode bridge: advertised tools
//!   become a sealed kernel manifest (capabilities, form surfaces, result
//!   cards, requires-approval grants) plus bound capability handlers.
//!
//! Nothing here auto-installs or auto-grants: connecting a server yields a
//! manifest and handlers; installation still runs through phased kernel install
//! and its trusted-chrome grant prompts.
//!
//! MCP resources, prompts, and MCP Apps UI are deliberately not modeled yet;
//! when they are, they land in this crate as more bridge surface — not as
//! new kernel primitives.

pub mod bridge;
pub mod client;
pub mod errors;
pub mod http;
pub mod protocol;
pub mod stdio;
pub mod transport;

pub use bridge::{
    handlers_for_mcp_server, manifest_for_mcp_server, McpToolCall, RESULT_CARD_ARTIFACT_TYPE,
};
pub use client::McpClient;
pub use errors::McpError;
pub use http::StreamableHttpTransport;
pub use protocol::McpToolDefinition;
pub use stdio::StdioTransport;
pub use transport::{McpTransport, RequestOptions};
