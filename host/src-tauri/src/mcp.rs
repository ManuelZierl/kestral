//! Host-side MCP consumption: configured servers become installed apps only
//! on an explicit user action, never automatically.
//!
//! Split of responsibilities:
//! - `config.rs` persists which servers exist (host-owned config)
//! - `mcp-adapter` speaks the protocol (transports, session, bridge)
//! - this module owns live connections and the connect/disconnect flow
//!
//! Connect = dial + handshake + tool discovery **off** the kernel lock,
//! then phased kernel installation under it (trusted chrome confirms the
//! requires-approval grants). Disconnect = `Kernel::uninstall` + transport
//! shutdown.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use app_host_kernel::ids::AppId;
use app_host_kernel::kernel::Kernel;
use mcp_adapter::{
    handlers_for_mcp_server, manifest_for_mcp_server, McpClient, McpToolDefinition, McpTransport,
    StdioTransport, StreamableHttpTransport,
};

use crate::config::{McpServerConfig, McpTransportConfig};

/// Live MCP sessions, keyed by configured server id. Holding the client
/// keeps its transport (child process / HTTP session) alive alongside the
/// installed handlers; disconnect shuts it down deliberately.
///
/// A connect or disconnect in flight owns its slot from the start, so no
/// other transition for the same server can begin concurrently.
enum Slot {
    Connecting,
    Ready(Arc<McpClient>),
    Disconnecting {
        generation: u64,
        client: Arc<McpClient>,
    },
}

#[derive(Default)]
struct ConnectionState {
    clients: BTreeMap<String, Slot>,
    next_generation: u64,
}

#[derive(Default)]
pub struct McpConnections {
    state: Mutex<ConnectionState>,
}

pub struct DisconnectTransition {
    server_id: String,
    generation: u64,
    client: Arc<McpClient>,
}

impl McpConnections {
    /// Connected or currently transitioning.
    pub fn is_active(&self, server_id: &str) -> Result<bool, String> {
        self.state
            .lock()
            .map(|state| state.clients.contains_key(server_id))
            .map_err(|_| "MCP state poisoned".to_string())
    }

    /// Reserve the slot for a connect in flight.
    pub fn begin(&self, server_id: &str) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "MCP state poisoned".to_string())?;
        if state.clients.contains_key(server_id) {
            return Err(format!("MCP server '{server_id}' is already connected"));
        }
        state
            .clients
            .insert(server_id.to_string(), Slot::Connecting);
        Ok(())
    }

    /// The connect succeeded: keep the session alive only if its reservation
    /// is still current. The caller must surface a state error rather than
    /// report a session that was never recorded.
    pub fn complete(&self, server_id: &str, client: Arc<McpClient>) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "MCP state poisoned".to_string())?;
        match state.clients.get(server_id) {
            Some(Slot::Connecting) => {
                state
                    .clients
                    .insert(server_id.to_string(), Slot::Ready(client));
                Ok(())
            }
            Some(_) => Err(format!(
                "MCP server '{server_id}' connection transition is no longer current"
            )),
            None => Err(format!(
                "MCP server '{server_id}' connection transition is missing"
            )),
        }
    }

    /// The connect failed: free its reservation again.
    pub fn abort(&self, server_id: &str) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "MCP state poisoned".to_string())?;
        if matches!(state.clients.get(server_id), Some(Slot::Connecting)) {
            state.clients.remove(server_id);
        }
        Ok(())
    }

    /// Reserve a ready session for disconnect without removing it. Keeping the
    /// slot blocks reconnect until uninstall either commits or rolls back.
    pub fn begin_disconnect(&self, server_id: &str) -> Result<DisconnectTransition, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "MCP state poisoned".to_string())?;
        match state.clients.get(server_id) {
            Some(Slot::Ready(client)) => {
                let client = client.clone();
                let generation = state.next_generation;
                state.next_generation = state
                    .next_generation
                    .checked_add(1)
                    .ok_or_else(|| "MCP disconnect generation exhausted".to_string())?;
                state.clients.insert(
                    server_id.to_string(),
                    Slot::Disconnecting {
                        generation,
                        client: client.clone(),
                    },
                );
                Ok(DisconnectTransition {
                    server_id: server_id.to_string(),
                    generation,
                    client,
                })
            }
            Some(Slot::Connecting) => Err(format!(
                "MCP server '{server_id}' is still connecting; wait for it to finish"
            )),
            Some(Slot::Disconnecting { .. }) => {
                Err(format!("MCP server '{server_id}' is already disconnecting"))
            }
            None => Err(format!("MCP server '{server_id}' is not connected")),
        }
    }

    /// Commit the exact disconnect transition and return its client for
    /// shutdown by the caller outside the kernel lock.
    pub fn complete_disconnect(
        &self,
        transition: &DisconnectTransition,
    ) -> Result<Arc<McpClient>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "MCP state poisoned".to_string())?;
        self.validate_disconnect(&state, transition)?;
        let Some(Slot::Disconnecting { client, .. }) = state.clients.remove(&transition.server_id)
        else {
            unreachable!("validated above");
        };
        Ok(client)
    }

    /// Roll back the exact disconnect transition after uninstall fails.
    pub fn rollback_disconnect(&self, transition: &DisconnectTransition) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "MCP state poisoned".to_string())?;
        self.validate_disconnect(&state, transition)?;
        state.clients.insert(
            transition.server_id.clone(),
            Slot::Ready(transition.client.clone()),
        );
        Ok(())
    }

    fn validate_disconnect(
        &self,
        state: &ConnectionState,
        transition: &DisconnectTransition,
    ) -> Result<(), String> {
        match state.clients.get(&transition.server_id) {
            Some(Slot::Disconnecting { generation, client })
                if *generation == transition.generation
                    && Arc::ptr_eq(client, &transition.client) =>
            {
                Ok(())
            }
            _ => Err(format!(
                "MCP server '{}' disconnect transition is no longer current",
                transition.server_id
            )),
        }
    }
}

/// One row of the servers list: persisted config plus live status.
#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerStatusView {
    pub id: String,
    pub display_name: String,
    pub transport: McpTransportConfig,
    pub connected: bool,
}

/// Installed app id for a configured server. Prefixed so a server id can
/// never collide with (or impersonate) a bundled app.
pub fn app_id_for_server(server_id: &str) -> AppId {
    AppId::new(format!("mcp-{server_id}"))
}

/// Dial the configured transport, handshake, and discover tools. Runs
/// without any host lock — a slow or dead server must not stall the shell.
/// Tool schemas are validated by `list_tools` before anything can install.
pub fn dial_server(
    config: &McpServerConfig,
    http_auth_header: Option<(String, String)>,
) -> Result<(Arc<McpClient>, Vec<McpToolDefinition>), String> {
    let transport: Box<dyn McpTransport> = match &config.transport {
        McpTransportConfig::Stdio { command, args } => {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            Box::new(StdioTransport::spawn(command, &args).map_err(|error| error.to_string())?)
        }
        McpTransportConfig::StreamableHttp { url, .. } => {
            let headers = http_auth_header.into_iter().collect();
            Box::new(
                StreamableHttpTransport::with_headers(url, headers)
                    .map_err(|error| error.to_string())?,
            )
        }
    };
    let client = Arc::new(McpClient::connect(transport).map_err(|error| error.to_string())?);
    let tools = client.list_tools().map_err(|error| error.to_string())?;
    Ok((client, tools))
}

pub fn dialed_server_install_parts(
    server_id: &str,
    config: &McpServerConfig,
    client: &Arc<McpClient>,
    tools: &[McpToolDefinition],
) -> (
    app_host_kernel::manifest::SealedManifest,
    std::collections::BTreeMap<
        app_host_kernel::ids::CapabilityName,
        app_host_kernel::invocation::CapabilityHandler,
    >,
) {
    let manifest = manifest_for_mcp_server(
        &app_id_for_server(server_id),
        &config.display_name,
        "0.1.0",
        &format!(
            "MCP server '{}' bridged in degraded mode: auto-generated forms, result cards, approval-gated tools",
            config.display_name
        ),
        tools,
    );
    let handlers = handlers_for_mcp_server(tools, client.as_tool_call());
    (manifest, handlers)
}

/// Reserve a connected server while its kernel app is being uninstalled.
pub fn begin_server_disconnect(
    connections: &McpConnections,
    server_id: &str,
) -> Result<DisconnectTransition, String> {
    connections.begin_disconnect(server_id)
}

/// Uninstall the bridged app. External transport shutdown is deliberately not
/// part of this function because it may block on a child process or network.
pub fn uninstall_server(kernel: &mut Kernel, server_id: &str) -> Result<(), String> {
    kernel
        .uninstall(&app_id_for_server(server_id))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
