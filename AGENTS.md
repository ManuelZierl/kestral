# AGENTS.md

Reference guide for developers and AI agents working in this repository.

## What this project is

This project is **Kestral**, a personal-first, open-source AI workspace and lean
local host for user-chosen AI apps.

Kestral starts with Chat so a new workspace is useful immediately, then becomes
more capable as the owner installs or builds focused apps. Chat is the entrance
and a cross-domain coordinator, not the presumed ideal interface for structured,
visual, stateful, or repeated work. It is to AI applications what the browser is
to websites: a host that loads, configures, mediates, and displays apps without
defining every experience itself.

Internally, it behaves like a microkernel for agentic apps: the trusted core owns
capabilities, surfaces, runs, grants, artifacts, secrets, trusted chrome, and
provenance. Apps live in userland.

Users choose which apps to install. Installation is not blanket mediated
authority: the host enforces manifests, grants, trusted chrome, sandboxed
intent-only surfaces, Runs, and provenance for actions routed through Kestral.
Native backends remain OS-powerful until process, filesystem, and network
isolation exists, so their direct behavior lies outside that grant boundary.

The self-contained product and architecture documentation lives under `docs/`.
Read `docs/architecture.md` and `docs/trust-model.md` before changing system
boundaries; the codebase is structured around their five kernel services, five
primitives, one action path, and five acceptance criteria.

## Product direction

Product priorities are ordered:

1. Make Kestral a useful personal workspace for recurring work.
2. Make that model available to similar technical users and independent app
   developers without private host coupling.
3. Improve general-user access through simple defaults, progressive disclosure,
   and curated apps without turning the host into an opinionated all-in-one
   suite.

Kestral itself is not being built as a commercial product. Revenue, growth,
marketplace control, and enterprise feature breadth do not drive the roadmap.
The MIT license still permits commercial use and forks.

The product is designed to stay lean, but Tauri or a small kernel does not prove
that outcome. Release evidence must measure startup, idle resource use,
installed footprint, worker cost, and time to first useful result. Feature work
must also account for conceptual, interaction, support, and compatibility
burden.

Before adding host behavior, ask:

- Does this improve the personal workspace or a demonstrated shared app need?
- Can a focused app own it instead?
- Does it preserve Chat as ordinary userland rather than making Chat the product
  ontology?
- Is the new maintenance and compatibility surface proportionate to the result?

## Product Invariants

- Chat is default-installed for first-run usefulness and remains ordinary
  userland.
- Chat is not the canonical interface for all AI work. Focused app surfaces,
  contextual AI actions, and grant-mediated composition are first-class.
- Notes is userland.
- LLM Provider is userland.
- Agent Engine (`com.ma-zierl.kestral-pi`) is an external installable app,
  headless, and holds zero grants.
- External app repositories own their dependencies, tests, notices, builds, and
  releases. Core source, tests, CI, and release tooling never resolve app source
  paths or package artifacts. Apps may consume versioned Kestral crates,
  schemas, or SDK packages through normal external dependency mechanisms.
- MCP servers are bridged into userland apps.
- MCP is an adapter protocol, not the internal ontology: the kernel is
  protocol-agnostic and receives only generic manifests, schemas, handlers,
  and artifacts. All MCP-specific code lives in `crates/mcp-adapter` and the
  host, never in `crates/kernel`.
- Configured MCP servers are never auto-installed or auto-granted; they
  connect only on explicit user action, and imported tool grants default to
  requires-approval.
- The host is not a chat runtime.
- The host is not an LLM provider.
- The host is not a workflow engine.
- Every capability invocation goes through `prepare_invocation`,
  `authorize_invocation`, and `finalize_invocation`.
- Every app capability action becomes a Run. Private presentation state and
  host-owner administration are not capability actions.
- Apps never call each other directly.
- Cross-app work happens through capability invocation under grants.
- Grant interaction conditions mediate delegated authority, not a person's
  direct use of the provider app. A declared same-app `SurfaceAction` must not
  show a `notify` notice or require per-use approval. Cross-app, LLM, agent,
  automation, and other programmatic calls remain fully grant-conditioned.
- Event subscriptions are a limited host event feed, not cross-app RPC.
- Raw ledger records are trusted audit data, not general plugin feed data.
- Trusted chrome is host-owned and cannot be rendered by apps.
- App config is declared by apps but stored and rendered by the host.
- Secrets are never readable by frontend/app surfaces.
- Grants constrain actions mediated by Kestral. They do not constrain direct OS
  filesystem or network access by an unsandboxed native backend.
- Development data before the first public release has no compatibility path.
  From `v0.1.0-alpha.1` onward, valid released user data must migrate forward or
  remain untouched behind a visible refusal; migrations never widen authority.

The product's canonical name is `Kestral` (with an "a" — not the bird's
spelling). Use `Kestral` in user-facing contexts, optionally described as a
`personal AI workspace`, `AI app workspace`, or `local host for AI apps`. Keep `kernel` wording for
system/debug/developer context, where the architectural distinction matters.
Internal identifiers (crate names, the `com.ma-zierl.host` bundle identifier,
binary names) intentionally keep their historical names; renaming them would
orphan user data for no user-visible gain.

## Documentation contract

The Just the Docs site under `docs/` is the public, self-contained source of
truth for the Kestral 0.1 release series. Do not require readers to consult a separate design
specification, ADR, source file, test, or release report to understand supported
product behavior.

- Keep `README.md` short: product statement, status, quick start, and links into
  `docs/`. Do not duplicate guides there.
- Update the relevant docs in the same change whenever behavior, UI labels,
  commands, environment variables, package schema, trust boundaries, supported
  platforms, persistence, deployment, or known limitations change.
- `docs/architecture.md` owns the normative product intent and priority order,
  design discipline, services, primitives, action path, userland boundary, and
  acceptance criteria.
- `docs/trust-model.md` owns security boundaries and residual authority.
- `docs/writing-apps.md` must match `schemas/app.schema.json` and the actual app
  manager/runtime behavior.
- `docs/getting-started.md` and pages under **Using Kestral** must use current,
  visible UI labels and describe only release-supported workflows.
- `docs/versioning.md` owns every persisted format version and migration rule.
  `docs/honest-gaps.md` must stay aligned with the supported boundary and current
  located TODOs.
- Preserve Just the Docs front matter and hierarchy. Add new pages to an
  existing parent unless a new top-level audience path is justified.
- Use `{% link file.md %}` for links between site pages. Do not use `../` links
  to repository-root files because those targets are not published with the
  site.
- Before finishing documentation changes, run `git diff --check`, build the
  Jekyll site, and inspect generated internal links. If local Ruby is
  unavailable, use the repository's documented container build path rather
  than claiming the site builds without verification.
- Treat examples as tested interfaces. Validate commands and data shapes
  against the code/schema; never preserve a stale example for continuity.

## Repository layout

```
kernel/
├── Cargo.toml              # workspace root: members = ["crates/kernel", "crates/mcp-adapter", "host/src-tauri"]
├── README.md               # concise front page: what it is, quick start, links into docs/
├── docs/                    # self-contained Just the Docs product, user, architecture, operations, and contributor documentation
├── .github/workflows/      # ci.yml (develop/main test matrix), release.yml (v* tag → GitHub Release)
│
├── crates/kernel/          # app-host-kernel crate (the trusted core)
│   ├── Cargo.toml          # zero infra deps: serde, chrono, uuid, jsonschema, sha2, thiserror
│   ├── src/
│   │   ├── lib.rs          # public re-exports, JsonObject type alias
│   │   ├── kernel.rs       # Kernel facade: install, invoke, uninstall, runs, surfaces, read views
│   │   ├── manifest.rs     # AppManifest, SealedManifest, seal(), require_consistent()
│   │   ├── invocation.rs   # CapabilityHandler trait, InvocationContext, InvocationResult
│   │   ├── ids.rs          # typed newtype IDs: AppId, RunId, GrantId, etc.
│   │   ├── errors.rs       # KernelError (thiserror), KernelResult<T>
│   │   ├── clock.rs        # Clock trait, SystemClock, FixedClock (test)
│   │   ├── schema.rs       # JSON Schema validation at the kernel boundary
│   │   ├── durable.rs      # persistence port + complete durable kernel projection (docs/versioning.md)
│   │   ├── primitives/     # 5 primitives: capability, surface, run, artifact, grant
│   │   └── services/       # five kernel services plus artifacts and chrome seams
│   └── tests/behavior/     # kernel behavioral suite, including architecture acceptance criteria 1-4
│
├── crates/mcp-adapter/     # MCP consumer adapter (MCP never enters the kernel)
│   ├── src/
│   │   ├── protocol.rs     # versions, tool defs (input+output schemas), result extraction
│   │   ├── transport.rs    # McpTransport trait: request/notify/shutdown, timeout+cancel
│   │   ├── stdio.rs        # child-process stdio transport (newline JSON-RPC)
│   │   ├── http.rs         # MCP Streamable HTTP: sessions, version headers, JSON+SSE
│   │   ├── client.rs       # session: initialize handshake, paginated tools/list, calls
│   │   ├── bridge.rs       # degraded-mode bridge: tools → sealed manifest + handlers
│   │   └── errors.rs       # McpError; all remote failures contained as invocation failures
│   └── tests/              # bridge tests (criterion 5) + stdio/HTTP conformance suites
│
├── host/                   # Tauri 2 + SvelteKit shell
│   ├── package.json        # npm scripts: dev, build, check, test (vitest)
│   ├── src/                # Svelte frontend
│   │   ├── lib/
│   │   │   ├── api.ts      # typed kernel wire shapes + Tauri invoke wrappers
│   │   │   ├── shell/      # HostShell, AppSidebar, TopBar, StatusBar, MainSurface
│   │   │   ├── chrome/     # TrustedChrome.svelte (approval modals, kernel-owned)
│   │   │   ├── chat/       # ChatSurface, ChatMessage, chat threads model
│   │   │   ├── apps/       # AppsPage, SurfaceRenderer, GenericFormSurface
│   │   │   ├── surfaces/   # sandboxed custom app UI: iframe host, bridge, in-frame SDK
│   │   │   ├── stuff/      # artifact browser, provenance display ("Artifacts" tab)
│   │   │   ├── settings/   # connector config, LLM provider settings, permissions editor
│   │   │   ├── system/     # run ledger + trusted notices (read-only inspector, not product)
│   │   │   ├── grants/     # GrantTable (embedded in Settings → Permissions)
│   │   │   ├── provenance/ # session-reference helpers linking chat to artifacts
│   │   │   ├── design/     # colors.ts: the single source of truth for all frontend colors
│   │   │   ├── hostTransport.ts  # Tauri-vs-remote transport switch behind api.ts
│   │   │   └── stores/     # Svelte stores: config, chatThreads, apps, grants, etc.
│   │   └── routes/+page.svelte  # thin: renders HostShell
│   ├── src-tauri/          # Rust shell
│   │   ├── Cargo.toml      # depends on app-host-kernel, tauri, reqwest
│   │   ├── tauri.conf.json # CSP, window config, bundled resources
│   │   ├── src/
│   │   │   ├── main.rs     # fn main() { host_lib::run() }
│   │   │   ├── bin/host-server.rs  # backend-only binary: remote_api::run_from_env()
│   │   │   ├── lib.rs      # Host struct, Tauri commands, run() setup, lock discipline, phased bundled-app startup
│   │   │   ├── remote_api.rs    # authenticated HTTP transport over the same Host commands (dispatch mirrors generate_handler!)
│   │   │   ├── host_paths.rs    # startup path resolution: --profile/--data-dir/env selection over the profile registry
│   │   │   ├── profiles.rs      # Kestral profile registry + per-root profile identity (kestral-profiles.json)
│   │   │   ├── chrome.rs   # ShellChrome: TrustedChrome impl, PendingApprovals, 5-min timeout; chrome/notices.rs persists trusted notices
│   │   │   ├── config.rs   # HostConfigService: on-disk config + secrets, connector profiles
│   │   │   ├── kernel_state.rs  # lifetime profile/registry locks + durable checksummed kernel snapshot
│   │   │   ├── profile_migration.rs  # pre-open whole-profile migration coordinator, journal, and authority checks
│   │   │   ├── atomic_json.rs   # atomic write-then-rename JSON persistence helper
│   │   │   ├── surface_ui.rs    # sandboxed custom-surface UI bundle registry
│   │   │   ├── package.rs       # installable app package: parse/inspect/translate, no code run
│   │   │   ├── git_source.rs    # safe app-package acquisition from public Git repos (bare store, no hooks)
│   │   │   ├── publisher_trust.rs  # ed25519 package-signature verification against the local trust store
│   │   │   ├── app_manager.rs   # third-party app lifecycle: install/update/enable/disable/uninstall, status, records
│   │   │   ├── app_data.rs      # host-indexed app-data revisions, staged migration protocol, backups, retention
│   │   │   ├── file_resources.rs   # host file-broker app: user-registered file/dir resources behind data-scoped grants
│   │   │   ├── chat_app.rs      # Chat app: delegates to selected agent workers, with plain-LLM fallback
│   │   │   ├── chat_runtime.rs  # drives a chat send through the phased action path off the kernel lock
│   │   │   ├── chat_store.rs    # persistent chat threads (JSON on disk)
│   │   │   ├── test_app.rs      # generic capability-provider fixture; test-only
│   │   │   ├── llm_provider.rs  # LLM Provider app: llm.generate capability, manifest
│   │   │   ├── llm_client.rs    # strict Rust ↔ bundled pi-ai worker bridge + LLM wire types
│   │   │   ├── node_worker.rs   # generic bounded invocation-scoped Node worker transport
│   │   │   ├── agent_worker_protocol.rs  # strict credential-free agent-worker bridge
│   │   │   ├── agent_worker.rs  # agent.run adapter + phased child-invocation dispatcher
│   │   │   ├── mcp.rs           # live MCP consumer connections; explicit connect/disconnect flow
│   │   │   ├── mcp_export.rs    # outbound MCP virtual-principal grants
│   │   │   ├── mcp_gateway.rs   # authenticated outbound Streamable HTTP MCP provider
│   │   │   └── tool_mapping.rs  # CapabilityRef ↔ LLM tool name mapping (sanitized)
│   │   └── tests/           # integration suites: chat.rs (E2E fake-LLM chat), app_manager.rs,
│   │                        # mcp_gateway.rs, os_credentials.rs, packaged_app_lifecycle.rs, support/
│   ├── provider-worker/    # bundled pi-ai Node worker: checksum-verified runtime, no ambient auth
│   └── demo-mcp-server/server.mjs  # ~100-line zero-dep Node MCP server, test-only for mcp-adapter stdio conformance
│
├── schemas/app.schema.json # JSON Schema for the installable app package manifest (docs/writing-apps.md)
├── scripts/check-release-version.mjs  # read-only version/tag consistency gate used by release.yml
│
└── .claude/skills/         # project skills: code-style (Manuel's preference profile), good-code,
                            # human-centered-interface-design, minimal-clean-code, responsive-design
```

## Build and verify commands

```sh
# Kernel behavioral suite (no external deps)
cargo test -p app-host-kernel

# MCP adapter: unit, bridge (criterion 5), and stdio/HTTP conformance tests
# (stdio conformance requires `node` in PATH)
cargo test -p mcp-adapter

# Host crate check + unit/integration tests
# (first generate THIRD-PARTY-NOTICES.txt — tauri.conf.json bundles it:
#  node scripts/generate-third-party-notices.mjs)
cargo check -p host
cargo test -p host

# Frontend type-check and unit tests (Vitest)
cd host && npm run check
cd host && npm test

# Full desktop app (dev mode — requires Node.js + Rust toolchain)
cd host && npm install && npm run tauri dev
```

## Tauri MCP automation

The user owns the Tauri development process. Agents must not start, restart,
stop, or kill it. When UI automation is needed, ask the user to run this from
`host/` and wait for them to confirm that the app is running:

```sh
npm run tauri:dev:mcp
```

This command uses `src-tauri/tauri.mcp.conf.json` and the `dev-mcp` feature.
The expected bridge log is:

```text
[MCP][WS_SERVER][INFO] WebSocket server listening on: 127.0.0.1:9223
```

Once the user confirms startup:

1. Connect with `tauri_driver_session` action `start`, port `9223`.
2. Verify the connection with `tauri_ipc_get_backend_state`. The expected app
   identifier is `com.ma-zierl.host`, with the main window at
   `http://localhost:1420/`.
3. Use `tauri_webview_dom_snapshot` with the accessibility tree to locate UI
   controls. Element refs are ephemeral; take a new snapshot after navigation,
   modal changes, or rerenders instead of reusing stale refs.
4. Use `tauri_webview_interact` and `tauri_webview_keyboard` for normal user
   flows. Use `tauri_webview_screenshot` for visual verification, especially
   for sandboxed custom-surface iframes whose inner DOM is intentionally opaque
   to the host tree.
5. Prefer public UI flows. For focused diagnostics, call registered Tauri
   commands from `tauri_webview_execute_js` via
   `window.__TAURI__.core.invoke(...)`. `tauri_ipc_execute_command` exposes only
   bridge-supported commands and may report an application command as
   unsupported. Tauri argument names passed from JavaScript use camelCase.
6. Read webview diagnostics with `tauri_read_logs` source `console`. A single
   short webview execution timeout does not prove the app crashed; retry a DOM
   snapshot and verify backend/window state first.

Do not launch a second Tauri/Vite process as a workaround for a failed bridge
connection. Ask the user to restart `npm run tauri:dev:mcp` and reconnect.
Likewise, if verification must rebuild `target/debug/host.exe`, tell the user
that the running app locks that executable on Windows and ask them to stop it;
never terminate their process yourself.

CI branch model: `develop` is the default integration branch, `main` the
release branch. PRs against `develop` run the Linux matrix (frontend
check/tests, Tasks dist reproducibility, `cargo test --all-features`); PRs
against `main` and pushes to `main` add the Windows matrix (native credential
integration + installer build). A `v*` tag on `main` publishes a GitHub
Release with the per-user NSIS installer (`.github/workflows/ci.yml`, `release.yml`).

Rust unit tests live beside each module as a child file: `src/foo.rs`
declares `#[cfg(test)] mod tests;` resolved from `src/foo/tests.rs`. Never
put a `mod tests { ... }` block inside an application source file.

## Architecture: the trusted core, adapter, and host boundary

### crates/kernel (app-host-kernel)

The trusted computing base. Zero infrastructure dependencies — no Tauri, no
HTTP, no filesystem, no async runtime. Pure Rust with serde, chrono, uuid,
jsonschema, sha2, thiserror. This makes it fully unit-testable without mocks.

**The five services** (`src/services/`):

| Service | File | Role |
|---------|------|------|
| Registry & Identity | `registry.rs` | Installed app catalog, manifest validation, content-hash seal verification |
| Permission Broker | `broker.rs` | Grant issue/check/revoke, secret storage, SecretResolver (snapshot) |
| Run Ledger | `ledger.rs` | Append-only event log, run view aggregation, event topics |
| Surface Manager | `surfaces.rs` | Open/close surface bindings, intent validation |
| Message Router & Lease Manager | `router.rs` | Event subscription delivery, advisory time-bounded leases |

Plus the artifact store (`artifacts.rs`) — not a sixth service, but the
ledger's durable-object substrate.

**The five primitives** (`src/primitives/`):

| Primitive | File | Key types |
|-----------|------|-----------|
| Capability | `capability.rs` | `CapabilityRef`, `CapabilityDeclaration` |
| Surface | `surface.rs` | `SurfaceDeclaration`, `SurfaceKind`, `ActionIntent` |
| Run | `run.rs` | `Initiator` (closed set), `RunView`, `RunTerminalState` |
| Artifact | `artifact.rs` | `ArtifactDraft` (app-proposed), `Artifact` (kernel-stamped `Provenance`) |
| Grant | `grant.rs` | `GrantScope`, `DataScope`, `GrantCondition`, `GrantDuration`, `Grant`, `DenialReason` |

**The single action path** (`kernel.rs`):

```
prepare (schema + grant) → authorize (trusted chrome if required) → execute
→ finalize (revalidate + output/artifact validation + provenance + ledger)
```

This is the only way work happens. The host uses phased execution so approvals
and handlers run outside its kernel mutex. Apps enter through
`prepare_install`/`commit_install`. `Kernel::uninstall` revokes grants, closes surfaces,
discards inboxes, cancels active runs, and releases leases. All service
fields are private; the public methods are the entire API.

**The handler contract** (`invocation.rs`):

Handlers are `Box<dyn Fn(&JsonObject, &InvocationContext) -> Result<CapabilityOutcome, HandlerFailure>>`.
They receive validated input and a `SecretResolver` (broker-scoped, snapshot).
They return a result value plus `ArtifactDraft`s (content only — the kernel
stamps provenance). Panicking handlers are caught via `catch_unwind` and
recorded as invocation failures, never crashing the kernel.

**The clock seam** (`clock.rs`):

All kernel time reads go through a `Clock` trait. Production uses
`SystemClock`; tests use `FixedClock` (only moves when told). Grant expiry,
lease expiry, and ledger timestamps are deterministic under test.

### crates/mcp-adapter (mcp-adapter)

The MCP consumer adapter. MCP is an adapter protocol, not the internal
ontology: this crate owns every MCP-specific type; the kernel receives only
generic manifests, schemas, handlers, and artifacts.

- **Transports** (`transport.rs`, `stdio.rs`, `http.rs`): the `McpTransport`
  trait (request/notify/shutdown with per-request timeout and cooperative
  cancel probe); stdio spawns a child process, Streamable HTTP does POST
  JSON-RPC with `Mcp-Session-Id` sessions, `MCP-Protocol-Version` headers,
  JSON *and* SSE response bodies, best-effort `notifications/cancelled`, and
  HTTP DELETE on shutdown. Both are synchronous — the host runs MCP work on
  blocking workers, never under the kernel lock.
- **Client** (`client.rs`): one session over any transport — initialize
  handshake with protocol-version negotiation, paginated `tools/list` with
  schema validation *before* anything can install, `tools/call` with all
  remote failures mapped to `McpError`.
- **Degraded-mode bridge** (`bridge.rs`): advertised tools become a
  sealed manifest — capabilities (input + imported output schemas), form
  surfaces, result-card artifacts, requires-approval grants — plus bound
  handlers that contain every remote error as an invocation failure. The
  bridge never installs anything itself.
- **Conformance tests** (`tests/`): stdio against the bundled Node demo
  server; Streamable HTTP against an in-process tiny_http test server
  (sessions, headers, SSE, error mapping, timeout, cancellation, DELETE).
- MCP resources, prompts, and MCP Apps UI are deliberately unmodeled for
  now; they belong in this crate later, not as kernel primitives.

### host/src-tauri (host)

The Tauri 2 shell. Depends on `app-host-kernel` and `mcp-adapter`. Contributes:

- **Trusted chrome** (`chrome.rs`): `ShellChrome` implements `TrustedChrome`
  via Tauri events + mpsc channels. 5-minute timeout, deny-by-default. The
  frontend `TrustedChrome.svelte` renders approval modals in shell-owned UI.

- **A window**: Svelte frontend renders views over the public kernel API.
  Surfaces emit `ActionIntent`s; the kernel drives them through the action
  path.

- **Bundled userland apps**: installed through phased startup orchestration in
  `lib.rs`: LLM Provider, Artifacts, Permissions, File Broker, and Chat. Chat is
  the default starting app and delegates when an optional agent is installed;
  bundled origin creates no privileged capability class. Notes is an external
  installable app. Host tests use generic fixtures and carry no Notes product.

- **Provider engine**: the LLM Provider keeps the kernel capability, profile,
  secret, Run, and ledger boundary. Provider protocol work runs in the bundled
  `host/provider-worker/` process using pinned `@earendil-works/pi-ai`. The
  worker receives only invocation-scoped broker-authorized credentials, has no
  ambient environment/file auth, and is packaged with a checksum-verified Node
  runtime. See `docs/architecture.md` and `docs/trust-model.md`.

- **Agent engine**: `kestral-pi` builds an ordinary external package with the
  `com.ma-zierl.kestral-pi` identity and a versioned `agent-worker` backend. The
  credential-free invocation worker round-trips model and tool requests over a
  channel to `KernelInvokerClient`; the dispatcher creates caller-attributed
  child Runs through the full phased action path. It is never installed at
  startup. See `docs/architecture.md` and `docs/trust-model.md`.

- **MCP servers** (`mcp.rs` + `config.rs`): user-configured servers persist
  in host config (`mcp_servers`, stdio or streamable-http). Nothing dials or
  installs at startup: `connect_mcp_server` dials and discovers tools OFF
  the kernel lock, then installs under it behind trusted-chrome grant
  prompts; `disconnect_mcp_server` uninstalls and shuts the transport down.
  Managed under *Settings → Tool servers*.

**Lock discipline** (`lib.rs`):

The `Host` struct holds `kernel: Arc<Mutex<Kernel>>` and
`config: Arc<Mutex<HostConfigService>>`. Lock ordering matters:

- `with_kernel_now` (sync commands): uses `try_lock` — never waits. If the
  kernel is busy (trusted-chrome prompt or blocking handler), returns
  "kernel busy" so the frontend can poll again.
- `with_kernel_blocking` (async commands that may block on chrome): uses
  `spawn_blocking` + `lock()` on a blocking thread.
- Production chat and surface actions use prepare/authorize/execute/finalize
  orchestration so trusted-chrome prompts and handler execution happen outside
  the kernel mutex. Finalization revalidates app, grant, cancellation, and
  deadline state before committing results.

**Configuration** (`config.rs`):

`HostConfigService` owns host config (default LLM profile, startup profile,
app settings, connector configs) and secret values. Persists to
`host-config.json` via atomic write-then-rename. Validates at load time:
version, non-empty defaults, active profile exists, connector shapes.

Secret lifecycle: `bootstrap_secrets` pushes all persisted secrets into the
kernel broker at startup. `put_secret`/`clear_secret` write both the
on-disk store and the kernel broker. The active LLM API key is mapped to a
synthetic `active_api_key` secret name via `sync_active_llm_secret`.

**Startup flow**:

1. `run()` (`lib.rs`) acquires the global registry lock and selected profile
   lock, runs the migration coordinator and pending-reset recovery, then opens
   operational stores and builds the `Kernel`. The kernel state store retains
   the profile lock for the host lifetime.
2. Frontend calls `bootstrap_startup_apps` once its trusted-chrome listeners
   are ready (installation triggers grant approval prompts).
3. The phased startup orchestration installs apps in order, then calls
   `bootstrap_secrets` to hydrate the kernel broker.

### host/src (Svelte frontend)

SvelteKit + TypeScript. Talks to the kernel only through typed Tauri
`invoke` wrappers in `api.ts`. The shell is a fixed desktop-style layout
(`HostShell.svelte`) with tabs: chat, apps, stuff, settings, system. Standalone
installed apps contribute their own sidebar destinations.

- `chrome/TrustedChrome.svelte` — the only place approval modals render
- `shell/` — layout components (sidebar, top bar, status bar, main surface)
- `stores/` — Svelte stores wrapping kernel state (apps, grants, config, etc.)
- `settings/` — connector config UI, secret input, LLM provider settings
- `system/` — run ledger table, grants table (inspector view, not the product)

Frontend tests use Vitest (jsdom environment). Pure helper tests exist for:
`chatThreadsModel`, `surfaceIntents`, `capabilityAccess`,
`connectorProfiles`, `configValidation`, `jsonSchemaFormModel`,
`artifactRenderer`, `LedgerEventSummary`, `scopeLabel`. Mounted component
tests (@testing-library/svelte) cover `ChatSurface` and `ChatMessage`; they mock
`$lib/api` / `$lib/stores/hostState` via `vi.mock` and seed writable stores.

## Key conventions

### Rust

- **Serde discipline**: all domain types use `#[serde(deny_unknown_fields)]`
  to forbid unknown keys at the boundary. Tagged enums use
  `#[serde(tag = "kind", rename_all = "kebab-case")]`.
- **Typed IDs**: `ids.rs` defines distinct newtypes (`AppId`, `RunId`, etc.)
  via a macro. Generated IDs are created at construction time — no placeholder
  identities.
- **Fail-fast at boundaries**: external data is validated against declared
  JSON Schemas once, on the way in. Malformed input is rejected with located
  errors before entering the trusted core.
- **No silent fallbacks**: missing grants, user denials, and handler failures
  are modeled as result variants (`InvocationResult::Refused`/`Failed`), not
  errors. Programming errors (unknown app, undeclared capability) are
  `KernelError`s.
- **Closed sets**: primitives, services, event kinds, and outcomes are all
  closed enums. Adding a case requires updating the match arms (compiler
  enforced).
- **Kernel-written provenance**: apps propose `ArtifactDraft` (content only);
  the kernel stamps `Provenance` (run, capability, grant, producer, time).
- **Handler containment**: panicking handlers are caught via
  `catch_unwind`; they fail the invocation, never the kernel.
- **Honest TODOs**: located design markers that say what is wrong and why.
  Keep the release-facing list aligned in `docs/honest-gaps.md`.

### Frontend (TypeScript/Svelte)

- **Typed wire shapes**: `api.ts` mirrors the Rust serde shapes as
  TypeScript interfaces. The frontend never reconstructs types from raw JSON.
- **Secrets are status-only on the frontend**: use `put_secret`,
  `clear_secret`, `has_secret`. Never read secret values into UI state.
- **No direct execution**: surfaces emit `ActionIntent`s; the kernel drives
  them through the action path.

### Design system: colors and theming (frontend)

`host/src/lib/design/colors.ts` is the single source of truth for every
color in the frontend. Rules:

- **Never write a color literal in a component.** No hex, no `rgb()/rgba()`,
  no named colors in `<style>` blocks. Reference semantic tokens as CSS
  custom properties: `var(--color-text)`, `var(--color-warning-soft)`, etc.
  (`currentColor` in SVGs is fine.)
- **Tokens are semantic, not descriptive**: `--color-danger-text`, not
  `--color-red`. Pick the token by role (surface/border/text/accent/status),
  not by which hex looks closest.
- **Need a color no token covers?** Add a field to the `ThemeColors`
  interface and give it a value in *every* theme in the `themes` registry —
  the compiler enforces completeness. Do not invent one-off variables
  elsewhere.
- **Theming**: `host/src/lib/stores/theme.ts` resolves the user preference
  (`system` | `light` | `dark`, persisted in localStorage, picked under
  Settings → Appearance) and writes the active theme's variables onto
  `document.documentElement`, plus `data-theme` and `color-scheme`. Adding a
  future theme = one new `ThemeColors` value registered in `themes`.
- **Custom themes**: a device-local custom profile is a complete validated
  `ThemeColors` snapshot with namespaced app-color overrides. Import and export
  validate the full portable shape before applying it.
- **Trusted chrome remains host-owned**: apps never receive or modify protected
  `--color-chrome-*` tokens. Built-in themes use the stable amber-on-dark
  signature; the owner may customize those tokens through host Appearance.
- **The app sidebar follows the active theme**: built-in and custom themes may
  define distinct `--color-sidebar-*` palettes so navigation fits the light or
  dark workspace while remaining visually separate from trusted chrome.
- When building anything visual, check both themes before calling it done
  (Settings → Appearance, or toggle the OS scheme with preference System).

### Responsive design (frontend)

The shell reflows from full desktop down to the WCAG 1.4.10 floor of 20rem
(320 CSS px). Keep it that way — every fixed pixel width removes
responsiveness. Rules for any HTML/CSS change:

- **Intrinsic first, breakpoints last**: prefer layouts that flex on their
  own — `flex-wrap`, `grid-template-columns: repeat(auto-fit, minmax(min(100%, Xrem), 1fr))`,
  `clamp()`/`min()`/`max()`. Add a media query only where the content
  actually breaks, and write it in `em` (e.g. `@media (max-width: 48em)`),
  never px or device widths.
- **Units**: `rem` for sizing and fonts, `em` for text-local spacing, `px`
  only for hairline borders. Fluid headings use `clamp()` with a `rem` term
  in the preferred value (never bare `vw` — it breaks browser zoom).
- **Box model**: `HostShell.svelte` applies a global `border-box` reset.
  Never rely on content-box; `width: 100%` + padding must not exceed the
  parent (that combination is what caused horizontal scrollbars in chat).
- **No page-level horizontal scroll**: wide content (tables in
  `RunLedgerTable`/`GrantTable`) scrolls inside its own `overflow-x: auto`
  wrapper with a `min-width` on the table itself.
- **Viewport height**: `100dvh` with a `100vh` fallback (see `.shell` in
  `HostShell.svelte`); never bare `100vh` for full-height regions.
- **Fixed overlays** (`chrome/TrustedChrome.svelte` dialogs/notices) are
  capped against the viewport (`max-width` + backdrop padding /
  `calc(100vw - 2rem)`) so they can never exceed a small window.
- **Established breakpoints**: 60em — sidebar narrows, top bar stacks;
  48em — sidebar collapses to an icon rail (nav labels stay in the
  accessibility tree via clipping, not `display: none`); chat: 69em/56em
  shrink the thread column, 40em stacks it into a horizontal strip above
  the conversation.
- **Accessibility is part of responsive**: the global
  `prefers-reduced-motion` gate lives in `HostShell.svelte`; keep touch
  targets ≥ 24×24 CSS px; verify changes at 200%/400% zoom and at a
  ~360px-wide window before considering layout work done.

## Testing strategy

### Kernel tests (`crates/kernel/tests/behavior/`)

Integration tests against the public `Kernel` API, one file per area:
`manifest_and_registry.rs`, `broker.rs`, `ledger.rs`, `leases_and_router.rs`,
`action_path.rs`, `phased_execution.rs`, `durable_state.rs`,
`success_criteria.rs`, with shared fixtures in `helpers.rs`. Uses
`FakeChrome` (scriptable, records prompts/notices) and `FixedClock`
(deterministic time). No mocks for network or filesystem — the kernel has
none.

The architecture acceptance criteria are tests:
1. `criterion_1_chat_remains_ordinary` — chat uses only public API
2. Primitive count stays at five — module-level design constraint
3. `criterion_3_third_party_parity` — clone with same grants → identical result
4. `criterion_4_every_action_is_attributable` — artifact → provenance → run → grant → initiator
5. `criterion_5_degraded_mode_does_real_work_safely` — bare MCP tool → installable
   app; lives in `crates/mcp-adapter/tests/bridge.rs` because the kernel is
   protocol-agnostic and the MCP adapter owns the degraded-mode story

### Host tests (`host/src-tauri/`)

- **Unit tests** (child-module files, `src/<module>/tests.rs`): config
  validation, secret sync, chat store, Notes fixture, chat app agent loop,
  phased bundled-app startup, LLM client serialization, tool name
  sanitization.
- **Integration tests** (`tests/`): E2E chat with fake LLM through public
  API; E2E MCP transport against the bundled Node demo server (requires
  `node` in PATH).

### Frontend tests (`host/src/`)

Vitest with a jsdom environment. Pure helper/logic tests for models,
validation, and rendering helpers, plus mounted component tests
(@testing-library/svelte) for Chat (`ChatSurface`, `ChatMessage`), app surfaces,
settings, trusted chrome, and shell behavior.

## Architecture → code map

| Architecture area | Code |
|---|---|
| Registry & Identity | `crates/kernel/src/services/registry.rs` |
| Permission Broker & Trusted Chrome | `services/broker.rs`, `services/chrome.rs`, `host/src-tauri/src/chrome.rs` |
| Run Ledger (+ provenance, artifact store) | `services/ledger.rs`, `services/artifacts.rs` |
| Surface Manager | `services/surfaces.rs` |
| Message Router & Lease Manager | `services/router.rs` |
| Five primitives | `crates/kernel/src/primitives/` |
| Single action path | `crates/kernel/src/kernel.rs` (`prepare_invocation`, `authorize_invocation`, `finalize_invocation`, `prepare_surface_action`) |
| App model (manifests) | `crates/kernel/src/manifest.rs` |
| Degraded-mode MCP bridge | `crates/mcp-adapter/` (bridge, client, stdio + Streamable HTTP transports), `host/src-tauri/src/mcp.rs` |
| Shell (Tauri) | `host/src-tauri/` (chrome port: `src/chrome.rs`) |
| Acceptance criteria | `crates/kernel/tests/behavior/success_criteria.rs` and `crates/mcp-adapter/tests/bridge.rs` |

## Known limitations (located TODOs in code)

- Manifest seals are content hashes (tamper evidence), not publisher-key
  signatures (`manifest.rs`).
- Event subscribers receive minimized `AppEventView` records after
  trusted-chrome install consent; subscriptions are a bounded, lossy event feed
  and the host does not yet expose an inbox-drain API (`services/router.rs`).
- Kernel declaration-only app upgrades preserve authority and history only when
  version/top-level presentation metadata changed and every behavioral contract
  is identical (`services/registry.rs`). Managed package updates remain a
  separate journaled host-level transition (`app_manager.rs`,
  `app_manager/update_journal.rs`).
- The ledger's ended-run guard is O(total ledger) per append; needs a
  run-status index before large durable histories (`services/ledger.rs`). Schemas are
  re-compiled per validation (`schema.rs`).
- Durable kernel state is a full checksummed snapshot per transition; this is
  O(total state), and Windows sudden-power-loss behavior still needs clean-VM
  verification (`kernel_state.rs`, `docs/versioning.md`).
- Native credential integration is not release-tested on macOS/Linux in this
  Windows workspace. Headless Linux has no plaintext fallback (`config.rs`).
- Package-layer ed25519 signatures verify against a local trust store
  (`publisher_trust.rs`): invalid/revoked signatures refuse install, unsigned
  packages install with a visible "unsigned" verdict. Kernel manifest seals
  remain content hashes.
- Trusted notices persist across restarts in `trusted-notices.json`
  (`chrome/notices.rs`).
- Custom app surfaces render in per-app **sandboxed iframes** (`allow-scripts`
  and `allow-forms`, with an opaque origin and host-enforced
  `form-action 'none'` → no Tauri/kernel/filesystem/secret/form-navigation
  access) that talk to
  the host solely through a versioned message bridge with a deny-by-default,
  per-app CSP (`host/src/lib/surfaces/`, `host/src-tauri/src/surface_ui.rs`).
  Bundled Svelte screens and MCP-derived generic forms/cards are unchanged.
  The **app manager** (`app_manager.rs`, Apps page) installs third-party
  packages (`docs/writing-apps.md`): inspection runs no package code, install confirms every
  permission through trusted chrome, and enable/disable/uninstall route through
  phased kernel install / `uninstall` (bidirectional grant revocation, run
  cancellation, surface removal) plus host cleanup (stop backend, drop UI
  bundles, purge secrets/data on explicit choice). A package's `surfaces[].ui`
  bundles register into the sandboxed-surface registry at activation. Managed
  lifecycle writes use a host transition guard; package verification, backend
  startup, MCP discovery, and approval waits run outside kernel, app-manager,
  and surface-registry locks. OS-level process isolation per frame remains open.

## Things to check before making changes

- **Adding a kernel primitive**: the set is closed by design.
  Adding a sixth requires demonstrating it cannot be expressed as composition
  of the existing five. Update `primitives/mod.rs` and the success criteria.
- **Adding a `LedgerEvent` variant**: update `kind()`, `ALL_KINDS` (both
  reference named constants in the `kinds` module), and the `run_view`
  aggregation match in `ledger.rs`.
- **Adding a Tauri command**: register it in `tauri::generate_handler!` in
  `lib.rs:run()`, **and** add a matching arm in `remote_api.rs::dispatch` so
  the command also works in backend-only mode (the
  `dispatch_covers_every_tauri_command` test fails if you forget). Follow the
  lock discipline: `with_kernel_now` for sync reads (try_lock),
  `with_kernel_blocking` for operations that may block on chrome.
- **Adding a host-side store**: use the atomic write-then-rename pattern
  (`write` to `*.json.tmp`, then `rename`). See `config.rs::persist()`.
- **Changing config schema**: `validate_host_config` must catch invalid
  states at load time. `HostConfig` uses `#[serde(deny_unknown_fields)]`.
- **Adding a bundled app**: follow the pattern in `chat_app.rs`: build an
  `AppManifest`, bind `CapabilityHandler`s, install
  via `prepare_install`/`commit_install`. Apps hold ordinary grants.
- **Changing the handler contract**: handlers are synchronous
  (`Box<dyn Fn...>`). The kernel calls them under its lock. An async handler
  path is the documented next milestone for streaming LLM calls.

## General rules

- Before the first public release, do not preserve obsolete development formats
  or APIs. Use the no-compatibility window to simplify the foundation.
- From `v0.1.0-alpha.1` onward, preserve user-owned durable data through explicit
  forward migrations. API and app-contract stability follow their documented
  version policy and are distinct from data continuity.
- Choose the simplest implementation that fully meets the current requirements.
- Prefer established, well maintained libraries over custom implementations.
