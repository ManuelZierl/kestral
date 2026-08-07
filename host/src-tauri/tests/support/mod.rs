#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use app_host_kernel::ids::{CapabilityName, RunId};
use app_host_kernel::invocation::{CapabilityHandler, InvocationResult};
use app_host_kernel::kernel::{
    AuthorizeInvocation, Kernel, PrepareInvocation, SurfaceActionOutcome,
};
use app_host_kernel::manifest::SealedManifest;
use app_host_kernel::primitives::capability::CapabilityRef;
use app_host_kernel::primitives::run::RunTerminalState;
use app_host_kernel::primitives::surface::ActionIntent;
use app_host_kernel::services::broker::IssueResult;
use app_host_kernel::services::surfaces::SurfaceBinding;
use app_host_kernel::{JsonObject, KernelResult};
use host_lib::app_manager::{AppManager, InstallRecord, PreparedActivation};
use host_lib::config::HostConfigService;
use host_lib::surface_ui::SurfaceUiRegistry;
use serde_json::json;
use sha2::{Digest, Sha256};

pub const LIFECYCLE_FIXTURE_ID: &str = "com.example.lifecycle-fixture";

const LIFECYCLE_BACKEND: &str = r#"import { createInterface } from "node:readline";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const dataDir = process.env.APP_HOST_DATA_DIR;
if (!dataDir) throw new Error("APP_HOST_DATA_DIR is required");
const dataFile = join(dataDir, "items.json");

function loadItems() {
  if (!existsSync(dataFile)) return [];
  return JSON.parse(readFileSync(dataFile, "utf8")).items;
}

function send(message) {
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
}

const tools = [
  {
    name: "add_item",
    description: "Add an item",
    inputSchema: {
      type: "object",
      properties: { title: { type: "string", minLength: 1 } },
      required: ["title"],
      additionalProperties: false,
    },
  },
  {
    name: "list_items",
    description: "List items",
    inputSchema: { type: "object", properties: {}, additionalProperties: false },
  },
];

const lines = createInterface({ input: process.stdin });
lines.on("line", (line) => {
  if (!line.trim()) return;
  const request = JSON.parse(line);
  if (request.id === undefined) return;
  if (request.method === "initialize") {
    send({ id: request.id, result: { protocolVersion: request.params.protocolVersion, capabilities: { tools: {} }, serverInfo: { name: "lifecycle-fixture", version: "1.0.0" } } });
  } else if (request.method === "ping") {
    send({ id: request.id, result: {} });
  } else if (request.method === "tools/list") {
    send({ id: request.id, result: { tools } });
  } else if (request.method === "tools/call") {
    const items = loadItems();
    if (request.params.name === "add_item") {
      items.push({ title: request.params.arguments.title });
      writeFileSync(dataFile, JSON.stringify({ items }), "utf8");
    } else if (request.params.name !== "list_items") {
      send({ id: request.id, error: { code: -32601, message: "unknown tool" } });
      return;
    }
    const result = { items, count: items.length };
    send({ id: request.id, result: { content: [{ type: "text", text: JSON.stringify(result) }], structuredContent: result } });
  } else {
    send({ id: request.id, error: { code: -32601, message: "unknown method" } });
  }
});
"#;

const LIFECYCLE_UI: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"></head><body>Lifecycle fixture</body></html>\n";
const LIFECYCLE_MIGRATION: &str = r#"import { createInterface } from "node:readline";
const lines = createInterface({ input: process.stdin });
lines.on("line", (line) => {
  const request = JSON.parse(line);
  if (request.method !== "kestral/app-data/migrate") {
    process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id: request.id, error: { code: -32601, message: "unknown method" } })}\n`);
    return;
  }
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id: request.id, result: { protocol_version: 1, format_version: request.params.to_format_version } })}\n`);
});
"#;

pub fn write_mcp_lifecycle_package(root: &Path, consumer_holder: Option<&str>) {
    let backend_path = root.join("backend/server.mjs");
    let migration_path = root.join("backend/migrate.mjs");
    let ui_path = root.join("ui/index.html");
    std::fs::create_dir_all(backend_path.parent().unwrap()).unwrap();
    std::fs::create_dir_all(ui_path.parent().unwrap()).unwrap();
    std::fs::write(&backend_path, LIFECYCLE_BACKEND).unwrap();
    std::fs::write(&migration_path, LIFECYCLE_MIGRATION).unwrap();
    std::fs::write(&ui_path, LIFECYCLE_UI).unwrap();

    let consumer_grants = consumer_holder
        .map(|holder| {
            json!([{
                "holder": holder,
                "request": {
                    "scope": {"kind": "exact-capability", "provider": LIFECYCLE_FIXTURE_ID, "capability": "list_items"},
                    "data_scope": {"kind": "none"},
                    "condition": "silent",
                    "reason": "Allow the test consumer to list fixture items.",
                    "duration": {"kind": "non-expiring"}
                }
            }])
        })
        .unwrap_or_else(|| json!([]));
    let digest = |bytes: &[u8]| format!("sha256-{:x}", Sha256::digest(bytes));
    let document = json!({
        "format_version": 1,
        "id": LIFECYCLE_FIXTURE_ID,
        "version": "1.0.0",
        "display_name": "Lifecycle fixture",
        "description": "Host-owned package lifecycle fixture.",
        "min_host_version": "0.0.1",
        "manifest": {
            "capabilities": [
                {
                    "name": "add_item",
                    "description": "Add an item",
                    "input_schema": {
                        "type": "object",
                        "properties": {"title": {"type": "string", "minLength": 1}},
                        "required": ["title"],
                        "additionalProperties": false
                    },
                    "effect": "local-write"
                },
                {
                    "name": "list_items",
                    "description": "List items",
                    "input_schema": {"type": "object", "properties": {}, "additionalProperties": false},
                    "effect": "read-only"
                }
            ],
            "surfaces": [{
                "name": "inventory",
                "kind": "panel",
                "title": "Inventory",
                "description": "Package lifecycle fixture.",
                "intents": [
                    {"provider": LIFECYCLE_FIXTURE_ID, "capability": "add_item"},
                    {"provider": LIFECYCLE_FIXTURE_ID, "capability": "list_items"}
                ],
                "ui": {"entry": "ui/index.html"}
            }],
            "config_declarations": [{
                "name": "settings",
                "title": "Fixture settings",
                "description": "Host-rendered fixture configuration.",
                "json_schema": {"type": "object", "properties": {"enabled": {"type": "boolean"}}, "additionalProperties": false},
                "default": {"enabled": true}
            }],
            "artifact_types": [{
                "name": "item-snapshot",
                "description": "Fixture backend result.",
                "json_schema": {"type": "object"}
            }],
            "grant_requests": [
                {
                    "scope": {"kind": "exact-capability", "provider": LIFECYCLE_FIXTURE_ID, "capability": "add_item"},
                    "data_scope": {"kind": "none"},
                    "condition": "notify",
                    "reason": "Add fixture items.",
                    "duration": {"kind": "non-expiring"}
                },
                {
                    "scope": {"kind": "exact-capability", "provider": LIFECYCLE_FIXTURE_ID, "capability": "list_items"},
                    "data_scope": {"kind": "none"},
                    "condition": "silent",
                    "reason": "List fixture items.",
                    "duration": {"kind": "non-expiring"}
                }
            ]
        },
        "consumer_grant_requests": consumer_grants,
        "backend": {
            "kind": "mcp-stdio",
            "authority_mode": "unsandboxed",
            "command": "node",
            "args": ["backend/server.mjs"]
        },
        "data": {"kind": "versioned", "format_version": 1, "migration": {
            "protocol_version": 1,
            "command": "node",
            "entry": "backend/migrate.mjs",
            "args": ["backend/migrate.mjs"],
            "transitions": []
        }},
        "integrity": {
            "algorithm": "sha256",
            "assets": {
                "backend/server.mjs": digest(LIFECYCLE_BACKEND.as_bytes()),
                "backend/migrate.mjs": digest(LIFECYCLE_MIGRATION.as_bytes()),
                "ui/index.html": digest(LIFECYCLE_UI.as_bytes())
            }
        }
    });
    std::fs::write(
        root.join("app.json"),
        serde_json::to_vec_pretty(&document).unwrap(),
    )
    .unwrap();
}

pub fn drive_chat_message(
    kernel: &mut Kernel,
    message: &str,
) -> Result<host_lib::chat_app::ChatReply, String> {
    match host_lib::chat_app::prepare_chat_message(
        kernel,
        &[],
        message,
        "test-thread",
        host_lib::chat_app::DEFAULT_MAX_LLM_ITERATIONS,
        Duration::from_secs(600),
        None,
    )? {
        host_lib::chat_app::ChatStart::Immediate(reply) => Ok(reply),
        host_lib::chat_app::ChatStart::Active(mut session) => {
            let parent = session.parent_run_id().clone();
            let reply = loop {
                match session.prepare_next(kernel)? {
                    host_lib::chat_app::ChatStep::Complete(reply) => break reply,
                    host_lib::chat_app::ChatStep::Continue => continue,
                    host_lib::chat_app::ChatStep::Execute(mut invocation) => {
                        let prepared = invocation
                            .prepared
                            .take()
                            .ok_or_else(|| "chat invocation was already consumed".to_string())?;
                        let result = match kernel
                            .authorize_invocation(prepared.await_approval())
                            .map_err(|error| error.to_string())?
                        {
                            AuthorizeInvocation::Authorized(authorized) => kernel
                                .finalize_invocation(authorized.execute())
                                .map_err(|error| error.to_string())?,
                            AuthorizeInvocation::Refused(result) => result,
                        };
                        if let Some(reply) = session.finalize_next(kernel, *invocation, result)? {
                            break reply;
                        }
                    }
                }
            };
            let terminal = if session.failed() {
                RunTerminalState::Failed
            } else {
                RunTerminalState::Completed
            };
            let _ = kernel.end_run(&parent, terminal);
            Ok(reply)
        }
    }
}

pub trait KernelTestExt {
    fn install(
        &mut self,
        manifest: SealedManifest,
        handlers: BTreeMap<CapabilityName, CapabilityHandler>,
    ) -> KernelResult<Vec<IssueResult>>;
    fn invoke(
        &mut self,
        run_id: &RunId,
        capability: &CapabilityRef,
        input: JsonObject,
    ) -> KernelResult<InvocationResult>;
    fn submit_action(
        &mut self,
        binding: &SurfaceBinding,
        intent: ActionIntent,
    ) -> KernelResult<SurfaceActionOutcome>;
}

impl KernelTestExt for Kernel {
    fn install(
        &mut self,
        manifest: SealedManifest,
        handlers: BTreeMap<CapabilityName, CapabilityHandler>,
    ) -> KernelResult<Vec<IssueResult>> {
        let prepared = self.prepare_install(manifest, handlers)?;
        self.commit_install(prepared.await_approval())
    }

    fn invoke(
        &mut self,
        run_id: &RunId,
        capability: &CapabilityRef,
        input: JsonObject,
    ) -> KernelResult<InvocationResult> {
        let prepared = match self.prepare_invocation(
            run_id,
            capability,
            app_host_kernel::invocation::InvocationRequest {
                input,
                data_scope: app_host_kernel::primitives::grant::DataScope::None,
            },
        )? {
            PrepareInvocation::Prepared(prepared) => prepared,
            PrepareInvocation::Refused(result) => return Ok(result),
        };
        match self.authorize_invocation(prepared.await_approval())? {
            AuthorizeInvocation::Authorized(authorized) => {
                self.finalize_invocation(authorized.execute())
            }
            AuthorizeInvocation::Refused(result) => Ok(result),
        }
    }

    fn submit_action(
        &mut self,
        binding: &SurfaceBinding,
        intent: ActionIntent,
    ) -> KernelResult<SurfaceActionOutcome> {
        let (run_id, prepared) = self.prepare_surface_action(binding, intent)?;
        let phases = (|| {
            let result = match prepared {
                PrepareInvocation::Prepared(prepared) => {
                    match self.authorize_invocation(prepared.await_approval())? {
                        AuthorizeInvocation::Authorized(authorized) => {
                            self.finalize_invocation(authorized.execute())?
                        }
                        AuthorizeInvocation::Refused(result) => result,
                    }
                }
                PrepareInvocation::Refused(result) => result,
            };
            Ok(result)
        })();
        match phases {
            Ok(result) => {
                let terminal = match result {
                    InvocationResult::Completed { .. } => RunTerminalState::Completed,
                    InvocationResult::Failed { .. } => RunTerminalState::Failed,
                    InvocationResult::Refused { .. } => RunTerminalState::Cancelled,
                };
                self.end_run(&run_id, terminal)?;
                Ok(SurfaceActionOutcome { run_id, result })
            }
            Err(error) => {
                let _ = self.end_run(&run_id, RunTerminalState::Failed);
                Err(error)
            }
        }
    }
}

fn activate(
    manager: &mut AppManager,
    kernel: &mut Kernel,
    surface_ui: &mut SurfaceUiRegistry,
    id: &str,
    prepared: PreparedActivation,
) -> Result<(), String> {
    let activation = manager
        .prepare_kernel_activation(kernel, id, prepared)
        .map_err(|error| error.reason)?;
    let continuation = manager
        .commit_kernel_activation(
            kernel,
            id,
            activation.install.await_approval(),
            activation.continuation,
        )
        .map_err(|error| error.reason)?;
    let grants =
        manager.prepare_consumer_grants(kernel, continuation.consumer_grant_requests.clone())?;
    let approvals = grants
        .into_iter()
        .map(|grant| grant.await_approval())
        .collect();
    manager
        .finish_kernel_activation(kernel, surface_ui, id, continuation, approvals)
        .map_err(|error| error.reason)
}

pub trait AppManagerTestExt {
    fn install(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        staged_id: &str,
        approved_digest: &str,
        installed_at: &str,
    ) -> Result<InstallRecord, String>;
    fn set_enabled(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        id: &str,
        enabled: bool,
    ) -> Result<(), String>;
    fn reactivate_enabled(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
    ) -> Vec<(String, String)>;
    fn uninstall(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        config: &mut HostConfigService,
        id: &str,
        purge_secrets: bool,
        purge_data: bool,
    ) -> Result<(), String>;
}

impl AppManagerTestExt for AppManager {
    fn install(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        staged_id: &str,
        approved_digest: &str,
        installed_at: &str,
    ) -> Result<InstallRecord, String> {
        let record = self.install_record(staged_id, approved_digest, installed_at)?;
        match self.prepare_activation(&record.id) {
            Ok(prepared) => {
                if let Err(reason) = activate(self, kernel, surface_ui, &record.id, prepared) {
                    self.record_failure(&record.id, reason);
                }
            }
            Err(reason) => self.record_failure(&record.id, reason),
        }
        Ok(record)
    }

    fn set_enabled(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        id: &str,
        enabled: bool,
    ) -> Result<(), String> {
        self.set_enabled_state(id, enabled)?;
        if enabled {
            let prepared = self.prepare_activation(id)?;
            activate(self, kernel, surface_ui, id, prepared)
        } else {
            if let Some(client) = self.remove_runtime(kernel, surface_ui, id)? {
                client.shutdown();
            }
            Ok(())
        }
    }

    fn reactivate_enabled(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
    ) -> Vec<(String, String)> {
        let mut failures = Vec::new();
        for (id, prepared) in self.prepare_enabled_activations() {
            let result =
                prepared.and_then(|prepared| activate(self, kernel, surface_ui, &id, prepared));
            if let Err(reason) = result {
                self.record_failure(&id, reason.clone());
                failures.push((id, reason));
            }
        }
        failures
    }

    fn uninstall(
        &mut self,
        kernel: &mut Kernel,
        surface_ui: &mut SurfaceUiRegistry,
        config: &mut HostConfigService,
        id: &str,
        purge_secrets: bool,
        purge_data: bool,
    ) -> Result<(), String> {
        self.begin_uninstall(id, purge_secrets, purge_data)?;
        if let Some(client) = self.remove_runtime(kernel, surface_ui, id)? {
            client.shutdown();
        }
        self.finish_uninstall(config, id)
    }
}

pub fn install_parts(
    kernel: &mut Kernel,
    manifest: SealedManifest,
    handlers: BTreeMap<CapabilityName, CapabilityHandler>,
    origin: app_host_kernel::primitives::grant::GrantOrigin,
) -> KernelResult<()> {
    let prepared = kernel.prepare_install_with_grant_origin(manifest, handlers, origin)?;
    kernel.commit_install(prepared.await_approval()).map(|_| ())
}
