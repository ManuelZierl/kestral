---
title: Architecture
layout: default
nav_order: 5
has_children: true
---

{% assign internal_link_prefix = "" %}{% assign jekyll_major = jekyll.version | split: "." | first %}{% if jekyll_major == "3" %}{% assign internal_link_prefix = site.baseurl %}{% endif %}

# Architecture
{: .no_toc }

1. TOC
{:toc}

Kestral is a personal-first, open-source AI workspace and lean local host for
user-chosen apps. Chat is the default starting app, not the canonical interface
for all AI work. Internally, Kestral behaves like a microkernel: the trusted core
owns identity, authority, execution records, artifacts, surfaces, and mediation.
Product behavior such as Chat, model access, notes, canvases, and agent loops
stays in apps.

## Product intent and priorities

Kestral starts with Chat because conversation is a low-friction way to connect a
model and explore an unfamiliar workspace. Chat is not the presumed ideal
interface for every task. Repeated, structured, visual, and stateful work should
be able to move into focused app surfaces with contextual AI actions while Chat
remains available for open-ended requests and cross-app coordination.

The intended progression is:

```text
start with Chat -> install focused apps -> use AI inside those apps
                -> compose apps under grants -> customize the workspace
```

Product priorities are ordered:

1. Serve the owner's recurring work as a useful personal workspace.
2. Let similar technical users and independent developers build and distribute
   apps without private host coupling.
3. Make the same model accessible to broader general users through simple
   defaults, progressive disclosure, and curated apps.

Kestral itself is not being built as a commercial product. Revenue, growth,
marketplace control, and enterprise breadth do not determine the roadmap; the
MIT license still permits commercial use and forks.

The host owns shared trusted mechanisms: app identity and lifecycle, surface and
trusted-chrome boundaries, provider and credential mediation, grants,
capability Runs, artifacts and provenance, and composition enforcement. Apps own
task-specific workflows and interaction design. A feature belongs in the host
only when several apps need the same trusted mechanism or it cannot work
correctly as an app among equals.

"Lean" is an end-user outcome, not an architectural synonym for Tauri. Measure
cold and warm startup, idle CPU and memory, installed footprint, worker cost,
time to first useful result, and the conceptual and operational burden added by
each shared feature. The pre-publication candidate does not yet have complete
budgets or regression gates.

## Design discipline

A component belongs in the kernel only when compromising it would break the
trust model for every app, or when it cannot function correctly as one app
among equals. Everything else is userland. Chat is the permanent test: it uses
no privileged API, and any special access it needs must be expressible through
the same primitives and grants available to third parties.

Default installation is a product and onboarding choice, not kernel privilege.
Chat and the bundled support apps may be present in every new profile while
remaining replaceable in the architecture and subject to userland parity.

Agent runtimes, model providers, and protocols are reusable components and
adapters, not product identities. Kestral can host many loops and providers
without defining itself around one of them. The same rule keeps it
runtime-agnostic and MCP-compatible rather than MCP-defined.

## Major components

| Component | Responsibility |
|---|---|
| `crates/kernel` | Runtime- and protocol-agnostic trusted core. It accepts generic manifests, schemas, handlers, grants, Runs, surfaces, and artifacts. It has no filesystem, network, async runtime, or UI dependency. |
| `crates/mcp-adapter` | MCP consumer protocol, stdio and Streamable HTTP transports, session client, and translation from MCP tools to generic app declarations and handlers. |
| `host` | Host runtime plus Tauri 2/Svelte desktop shell. The desktop shell contributes trusted chrome and a window; the runtime owns persistence, native credentials, packages, profiles, app processes, protocol adapters, and local/remote owner transports outside the kernel. |
| LLM Provider | Bundled userland app with the stable `llm-provider` identity and `llm.generate`, `llm.models.list`, and `llm.models.refresh` capabilities. Provider protocol work runs in an invocation-scoped bundled worker only when a profile is selected; an unconfigured request fails visibly and Chat adds local host-authored setup guidance. Profiles, grants, secrets, Runs, artifacts, and provenance remain host/kernel owned. |
| Chat | Default-installed userland app and starting surface. It provides conversation, model/tool coordination, and public extension contracts without a privileged kernel API. |
| `kestral-pi` | Optional external headless agent app. Its worker holds no credentials and asks the host to mediate model and tool calls as attributable child Runs. |

## Five services

| Service | Responsibility |
|---|---|
| Registry and Identity | Maintain the app catalog, versions, stable identities, exhaustive manifests, and content-hash seals. Attribute every kernel interaction to an app identity. |
| Permission Broker | Issue, display, check, expire, and revoke grants; resolve only declared owner-scoped secrets. |
| Run Ledger | Append initiator, goal, grants, invocations, events, artifacts, approvals or denials, and terminal state. Derive Run views and provenance for audit, replay, and debugging. |
| Surface Manager | Bind sandboxed panels, cards, forms, pickers, and dashboards; validate intent-only actions and provide no direct tool or surface-to-surface call path. The host separately offers bounded app-and-surface-scoped presentation state with no capability authority. |
| Message Router and Lease Manager | Deliver a bounded minimized event feed and coordinate advisory, time-bounded leases over artifacts and workspace paths. Surface conflicts instead of silently overwriting shared state. |

Artifacts are the ledger's durable-object substrate, not a sixth service. Exact
artifact resource IDs are grant-scoped data targets, and ordinary handlers may
read only a bounded, read-only snapshot resolver copied into the invocation
context. That resolver cannot widen grants, re-enter the kernel, or expose
unscoped artifacts. The host contextualizes Chat and agent tools from live
artifact grants: unusable no-resource grants expose no artifact tool, read IDs
are constrained to authorized current artifacts, and query resolves a standing
exact or `all-resources` grant to the exact current IDs before preparation.

## Five primitives

- **Capability:** something the system can do, declared and invoked through the
  kernel with input/output schemas and an advisory effect. MCP tools and
  host-provided actions such as artifact access are adapter-specific forms of
  this broader primitive.
- **Surface:** a visual place an app renders. It receives data and emits only
  validated action intents; it never holds secrets, calls tools directly, or
  contacts another surface. Protocol UI resources can be adapted into surfaces
  without defining the primitive.
- **Run:** one concrete execution attempt. A chat message, surface button,
  schedule, file change, external event, or parent Run can initiate it. It
  carries an initiator, goal, context, grants, invocations, events, artifacts,
  approvals, and terminal state.
- **Artifact:** a validated durable object such as a file, document, message,
  dataset, plan, or event proposal, stamped with kernel-written provenance.
  Artifacts are the medium of composition between apps.
- **Grant:** revocable authority held by an app or Run: what it may do, over
  which data, under silent, notify, or approval-required conditions, and for
  how long. Notify grants create durable trusted notices, not only transient UI.
  Read views preserve each data-scope/condition pair; conditions are never
  flattened across resources.

The set is deliberately closed. Agents, skills, automations, connectors, and
extension points are manifest data composed over these primitives.

Derived product concepts stay outside the kernel:

- An **automation** is a userland trigger whose firing starts a Run.
- A **task** is a presentation-level grouping of related Runs.
- An **agent** is a reasoning policy and tool binding executed by a runtime
  adapter.
- A **skill** is reusable instructions or context.
- **Memory** is artifacts plus retrieval over artifacts and the ledger; memory
  apps can add indexes, embeddings, summarization, and policy.

Chat prompt composition is host/app behavior, not a kernel primitive. The host
builds one bounded system prompt from four layers:

1. a visible, immutable Kestral protocol layer;
2. default or custom user assistant instructions;
3. explicitly enabled manifest skills, each bound to an exact SHA-256 digest of
   its instruction text;
4. optional host runtime context.

Chat assistant profiles are declared in app manifests and discovered from the
installed app catalog. The host derives the profile receipt from the live
installed manifest, including the app identity, version, profile digest, and
reviewed skill digests. Per-thread profile selection is host-owned state. If a
selected profile or reviewed skill content is no longer available, Chat falls
back to Standard for future sends and keeps the historical receipt pinned to the
reviewed source that was accepted at send time.

Model profiles are separate from assistant profiles. External apps opt into
Chat's `model-profile-editor` v1 contract with a contributed surface and a
host-stored `model-profiles` config declaration. The contract carries provider
profile IDs, model IDs, generation parameters, prompt-layer choices,
profile-specific prompt text, and tool allowlists. A prompt override may select
currently available Chat-owned layers and append bounded text, but the immutable
Kestral protocol layer always remains first. Missing selected layers make the
profile unavailable rather than silently changing its prompt.
Chat stores an exact per-thread receipt including the contributing app identity
and resolves it through the host; the profile app never calls Chat or the LLM
Provider. At send time, Chat requires the source app, declared contract, and
profile digest to remain current, resolves the configured
provider profile, and intersects the profile's tool references with Chat's live
grant-aware capability catalog. The same intersection is passed through the
`agent.run` contract, so a delegated engine cannot recover tools omitted by the
profile. No profile can issue or widen a grant.

Chat exposes three conversation-level extension points. `thread-actions` v1
carries the current thread ID, exact thread resource ID, and observed revision
for small inline actions such as exporting the visible conversation. The
resource ID is bounded context, not authority: capability calls still require
an exact resource invocation covered by either an exact-resource grant or an
explicitly approved `all-resources` grant for that capability. Broad grants
cover current and future resources, but cannot be used as an invocation data
scope; the Run and ledger keep the exact requested thread ID. `composer-context`
v1 carries the current thread ID, selection, and request ID for compose-time contributions.
`composer-actions` v1 lets extensions accept, remove, or review a draft
contribution by thread and draft ID.

`message-actions` is v6. Its context includes the thread's exact resource ID,
completed assistant-message metadata, host-stamped creation and completion
times, plus canonical reading parts with part index, excerpt, and plain text so
extensions can preserve exact text ranges without re-segmenting the response.
Those values are typed, bounded context, not authority. Cross-app actions still
need a capability intent and a covering grant.

Chat owns passive reading-opportunity observation because Chat owns the
conversation DOM; a sandboxed extension surface cannot observe the response it
annotates. One observer serves the whole log and stays idle until an extension
requests it. It reduces geometry to bounded integer aggregates at the boundary —
cumulative qualified-visible time gated on visibility, focus, and a single
primary reading region, plus a 32-band exposure bitset — and discards the source
geometry. Apps own the persisted state and the derived estimate. This adds no
kernel primitive, service, grant, or capability: observation produces no
cross-app or external authority, and the aggregates describe only the response
the asking extension is already bound to.

Runtime identity defaults on. That includes host version, delegated-agent or
plain-LLM mode, model, and connector kind. App inventory and connector/profile
identifiers default off. Secrets, base URLs, filesystem paths, tool outputs,
conversation history, and grant-authorized app context remain separate from the
system prompt.

Arbitrary dynamic app influence enters Chat only through the ordinary
`chat.inject_user_context` capability. The external app is the grant holder and
Chat is the provider. Each bounded, revisioned upsert or removal names an exact
thread resource, follows the complete action path, and becomes a Run. Chat
derives source app identity, version, content hash, and Run identity from the
invocation context rather than accepting them from app input.

Stored entries remain inert unless their source Run completed this exact
capability under its original grant, that grant is still active and covers the
thread, and the installed source content hash is unchanged. Revocation, expiry,
uninstall, or replacement therefore stops future inclusion immediately. Issuing
a new grant does not resurrect text written under a revoked grant; the app must
publish it again. Accepted entries are placed in one attributed late user
message immediately before the visible user message. The protocol treats each
entry's text as authorized supplemental user-level input, while the visible
message wins conflicts. This authority cannot replace the host protocol, make a
tool available, grant a permission, or prove a side effect.

Skills do not grant authority. Changed, missing, or oversized skills require
review and do not contribute until re-enabled. Sent turns store the current
prompt-receipt shape with the exact system prompt, digest, layer set, and
attributed app-context metadata. Chat's optional transparency setting records
the exact host-final app-context message for future sends; when it is off, the
receipt retains only source Run, grant, revision, and digest metadata.

The bundled headless Permissions app is a userland inspection and proposal
adapter, not the Permission Broker. Its read-only `permissions.list_active`
capability receives a host-bound snapshot of the invoking app's standing active
grants and returns only provider, capability, data-scope, and
interaction-condition data. This grant catalog is distinct from the tools
supplied to a model after profile and contextual narrowing. The separate
read-only `permissions.list_requestable` capability receives a host-bound
snapshot of exact capabilities declared by installed providers that the
invoking app does not currently hold, including bounded descriptive metadata
and declared effects. MCP tools are ordinary entries in this provider-agnostic
catalog. It remains available with an empty result so the model can distinguish
"nothing requestable" from a missing read tool. Its
`permissions.propose_grant` capability binds the proposed holder to the invoking
app and can only produce an exact, no-resource, approval-required request for a
candidate in that same host-bound snapshot. The host supplies the proposal tool
only when its contextual input schema can enumerate at least one exact
candidate. The result is a kernel-stamped `permission-proposal` artifact. It
confers no authority. A separate host command verifies the artifact producer,
capability, type, initiating app, installed provider and capability, and fixed
policy before passing the request through the broker's existing phased
trusted-chrome issuance path.

## One action path

```text
prepare -> authorize -> execute -> finalize
```

1. **Prepare** validates input schema, app identity, capability declaration,
   grant scope, data scope, expiry, cancellation, and deadline.
2. **Authorize** obtains trusted-chrome approval when the grant requires it.
3. **Execute** calls the handler outside the host's kernel mutex.
4. **Finalize** revalidates authority and lifecycle state, validates all output
   and artifact schemas, stamps provenance, and commits ledger records.

Ending a Run or removing/replacing either side of an invocation records pending
work as cancelled in the same durable transition. Opaque phase tokens are unique
across live kernel instances, so a decision or handler result prepared against
one kernel cannot be consumed by another.

Grant interaction conditions apply to delegated authority. When a person uses
a declared action in the provider app's own live surface, that
`SurfaceAction` remains a schema-validated, grant-checked, attributable Run, but
`notify` does not create a trusted notice and `requires-approval` does not add a
second confirmation. The surface action is already the person's explicit
command. Calls to another app or LLM provider, agent and automation work, and
all other programmatic invocations retain their configured grant condition.

The ledger records the requested data scope on invocation and approval events.
Invocation input and result bodies are not duplicated into audit history; the
ledger retains their SHA-256 digests while artifacts hold durable content.
`all-resources` exists only as a standing grant scope. Trusted chrome presents
it as broad access and leaves it unchecked by default during install review.
For artifact queries, the host expands that standing authority to the exact
current artifact IDs at invocation time; both plain Chat and delegated agent
dispatch use the same resolver. The wildcard itself never enters an invocation.

No app invokes another app directly. Cross-app work is a capability invocation
under a grant, and every app capability action becomes a Run. Private surface
state and host-owner administration remain outside this capability statement.

This remains true inside an agent loop. An agent worker can request a model or
tool operation only through a host dispatcher. The dispatcher attributes a
child Run to the original initiating app and drives the complete phased action
path. The worker receives neither a kernel handle nor credentials, so it cannot
fabricate a completion, bypass a grant, or inherit the engine app's authority.

Concurrent Runs touching shared artifacts or workspace paths must coordinate
through leases. A conflict is visible through trusted chrome rather than
resolved by silently clobbering another Run's work. Run completion and app
uninstall release leases; uninstall also discards event inboxes, cancels active
Runs, closes surfaces, and revokes grants in both directions.

## Userland parity

The following are ordinary apps or app-level presentations, not kernel
services: Chat, artifact browsers, automation managers, Run inspectors, agent
runtime adapters, model providers, permission-proposal adapters, and
memory/retrieval services. Anything a bundled app can do must be reproducible by
an external app with equivalent grants. Installed origin does not create a
privileged class. Requesting a proposal is reproducible by any app with the same
grant to Permissions; interpreting it as a grant decision remains host policy
and trusted chrome, never app authority.

Focused standalone surfaces are as fundamental as conversational integration.
An app may offer a document, canvas, form, dashboard, or another purpose-built
interface and separately expose capabilities that Chat or other granted callers
can use. Extension points add contextual presentation; they do not turn Chat
into the host's product ontology.

Media capture, transcription, and speech synthesis are external app concerns.
Kestral does not bundle a Media app, expose Media-specific host commands, or put
a microphone control in Chat. A future external Media app must own its capture
and processing flow through explicit app contracts; it cannot inherit device
authority from Chat or bypass the sandbox and capability boundaries.

## Package and protocol boundary

```text
app.json -- host package reader --> generic manifest -- seal --> kernel
backend  -- host adapter ---------------------------> handlers --^
ui/theme -- sandboxed surface registry ------------> intent/theme bridge
```

The kernel never sees files, Tauri, MCP, HTTP, or process types. Bare MCP tools
and package backends are translated by adapters before installation. This keeps
MCP replaceable and prevents package concerns from expanding the trusted domain
model.

`app.json` is exhaustive package input, but it is not a serialized kernel
object. The host injects package identity into a generic app manifest, removes
host-only UI bindings, validates manifest consistency, seals that manifest, and
separately binds backend handlers and sandboxed UI. Package and manifest hashes
cover different objects and are both retained. This translation is deliberate:
embedding a raw sealed manifest would couple the public package format to
internal serialization and leak backend/UI concerns into the kernel.
Package-declared app colors remain host-only presentation metadata. The shell
namespaces and resolves them with the host palette before sending bounded CSS
variables to that app's sandbox; neither declarations nor profile overrides
enter kernel state or confer authority.

Authored packages declare capabilities and schemas statically so inspection can
remain code-free. MCP-backed activation verifies the backend's advertised tools
against those declarations. Bare MCP servers remain a distinct degraded-mode
discovery path because they have no authored package declaration. General
packaged executables speak MCP over stdio rather than a second tool protocol;
the specialized `agent-worker` adapter exists only because agent callbacks flow
from the worker back through host-mediated model and tool invocations.

Configured Streamable HTTP servers may carry one host-managed static secret
header. Header metadata remains host config; the value remains in the OS
credential vault and is resolved only during explicit connection setup. The
adapter validates the final URL and header, disables redirects, and applies the
header to the whole MCP session. Neither the credential nor HTTP authentication
becomes a kernel primitive or packaged-app backend feature.

Managed-app lifecycle writes are serialized by a host transition guard, not by
holding the kernel or app-manager mutex for the whole operation. Package
verification, process startup, MCP handshakes, tool discovery, and trusted-chrome
approval waits run without the kernel, app-manager, or surface registry locked.
Short prepare, commit, deactivate, and rollback phases reacquire the state they
own and revalidate the app lifecycle generation and package digest before
committing. This keeps unrelated reads available while an app backend starts or
a person reviews authority.

## Provider and agent runtimes

The bundled provider worker replaces provider-specific HTTP and stream parsing
in Rust; it does not replace the LLM Provider app or become an authority layer.
Each operation starts a private-process worker using the bundled,
checksum-verified Node v22.19.0 runtime and
`@earendil-works/pi-ai@0.80.7`. Installed builds never use `node` from `PATH`.
Configuration and one broker-authorized profile credential cross bounded NDJSON
stdin for that invocation only. Progress is execution state of `llm.generate`,
not another capability. Model-produced tool calls are still executed as
separate child Runs. Process startup adds latency and bundle size in exchange
for invocation-scoped credential lifetime, crash containment, and hard
cancellation. Completed `llm.generate` results and response artifacts preserve
provider-reported cache-read and cache-write token counts plus monotonic
request-to-first-token and total provider-stream latency. First-token latency is
absent when the provider emits no non-empty stream delta.

A fresh host profile has no default provider profile and no preconfigured local
endpoint. In that state `llm.generate` fails with a specific unconfigured
outcome; Chat renders explicit setup guidance. The attempt creates no
provider-response artifact, worker process, credential read, port probe, or
network request. Selecting a saved profile is always an owner action; the host
does not auto-add a provider discovered on the machine.

The `openai-codex` profile uses pi-ai's ChatGPT OAuth adapter and fixed
`https://chatgpt.com/backend-api` subscription endpoint. It is distinct from
the `openai` API-key profile: eligible ChatGPT Plus/Pro accounts consume their
provider-managed Codex quota, while OpenAI API profiles use API billing.
pi-ai's bundled Codex catalog is available before login; login, token refresh,
and generation still run in invocation-scoped provider workers.

The optional `com.ma-zierl.kestral-pi` package contributes one `agent.run`
capability, no surfaces, and no self-grants. Its version 1 `agent-worker`
protocol is a host adapter contract, not a kernel primitive. The host supplies
the runtime and callback dispatcher; the package supplies the checksum-pinned
worker built with `@earendil-works/pi-agent-core@0.80.7`. Every invocation gets
a fresh process: a 10-second ready deadline, 120-second idle deadline, 2 MiB
stdout-line cap, 16 KiB stderr cap, and one-second cancellation grace. The app's
`max_duration_secs` config is constrained to 60-3600 seconds and defaults to
600. Synchronous child invocations reset the idle deadline. The adapter excludes
the engine itself and LLM Provider from model-visible tools, caps tool results
at 32 KiB as explicitly untrusted data, and writes a kernel-stamped
`agent-transcript` artifact. Host and worker protocol changes require a
coordinated release; there is no compatibility fallback.

Agent callbacks enter a bounded host dispatcher: eight workers, a bounded
queue, and at most four outstanding requests per initiating app. Saturation is
reported as an overload failure instead of spawning another OS thread. The
language-neutral protocol is documented for third-party agent packages in
[Agent workers]({{ internal_link_prefix }}{% link agent-workers.md %}); Kestral Pi is its first external
implementation, not a privileged runtime class.

## Durability

The host persists one authoritative, versioned, checksummed kernel projection.
It contains installed manifests and seals, immutable grants and revocations,
the complete sequenced Run ledger, and artifacts with provenance. Each logical
mutation builds candidate service views, writes the full projection, and only
then swaps the in-memory views. Invocation completion commits artifacts and all
corresponding ledger events together. A definitely failed write leaves both
memory and the previous file unchanged. If an adapter cannot determine whether
a write committed, the live kernel enters recovery-required mode and refuses
further durable transitions; restart loads the authoritative file.

A definitely failed write after provider code ran or after a one-shot approval
was consumed also requires restart. The previous durable snapshot remains
authoritative, but Kestral cannot safely replay the external effect or user
decision merely to recreate its missing terminal record.

Recovery validates the entire projection and fails startup rather than silently
starting empty. Active Runs are durably ended once as `interrupted` before state
is exposed; any prepared or executing invocation first receives a cancellation
record. Recovery rejects impossible parent/child attribution, unmatched
invocation outcomes, invalid grant lifetimes, and artifact provenance that does
not agree with its Run, capability, and grant. Executable handlers are rebound
only when recovered manifest bytes and seals match. Pending invocations, leases,
surfaces, and event inboxes are session state and are not restored, so a crash
cannot preserve lease authority.

This full transactional snapshot is intentionally simpler than independent
service files or an append-only persistence log, at the cost of rewriting all
durable state per transition. Other host-owned stores use strict versioned JSON
and atomic write-then-rename. App update transitions use a journal so an
interrupted update can resume or roll back at startup. Publisher-owned app bytes
live in host-indexed revision directories. For a declared format edge, the app
manager stages a copy, runs the package's bounded migration JSON-RPC process,
validates the candidate by starting the target backend against it, and atomically
replaces the host-owned active pointer. The old revision remains a configurable
backup. App code interprets records; the host owns only revision identity,
format metadata, lifecycle, and retention.

Backend-free apps may instead declare host-managed data contracts v1 or v2. This
is a host runtime service, not a sixth kernel primitive: the host owns bounded
JSON-object persistence, schema validation, optional unique equality indexes,
CAS revisions, transactions, quotas, and lifecycle checks. An owning live sandbox surface uses
the authenticated surface bridge without a grant or Run. Fixed generated
`get`/`list` capabilities bind delegated reads to the provider app identity and
enter the normal grant-checked action path with exact
`app-data:<app-id>:<collection>` resource scope. Indexed reads may bind their
equality value from trusted current-Chat context so it is not model-controlled.
Delegated mutations remain unsupported until the action path can stage a side
effect and commit it only after finalization revalidates authority. Contract v2
also permits fixed proposal bindings. A proposal validates a typed payload and
an exact collection generation, record revision, or document revision, then
produces a reviewable artifact without changing managed data. The host derives
the target app, target kind, collection, stable resource ID, and envelope; the
artifact goes through ordinary kernel schema validation, provenance stamping,
and atomic Run/artifact finalization. Collection IDs remain
`app-data:<app-id>:<collection>`; record and document IDs append
`:record:<uuid>` or `:document:<uuid>`. Chat and agents turn the host-owned
proposal schema annotation into that exact target scope even when their grant
is all-resources. Replay/application remains a frontend CAS responsibility,
not delegated capability behavior.

Before those stores open, the host holds the global profile-registry lock and
the selected profile lock and runs one migration coordinator. A recognized
migration stages and validates a complete profile candidate, checks cross-store
identity and non-widening authority, retains the original byte-level backup,
and commits through an idempotent six-phase journal. Unknown or corrupt formats
never become an empty profile. OS-vault values and browser-local appearance and
pending-send records retain separate storage owners because they cannot be
included honestly in a filesystem transaction; the coordinator preserves and
validates their profile-scoped references instead.

Executable handlers are not durable, but bundled declarations can evolve
without destructive uninstall/reinstall. A declaration-only kernel upgrade
allows version and top-level presentation changes while requiring every
capability, schema, effect, surface, grant, connector, config, artifact,
extension, and subscription contract to remain identical. It preserves grants,
revocations, Runs, ledger history, and artifacts atomically.

## Architectural acceptance criteria

The architecture remains valid only while all five criteria hold:

1. Chat remains ordinary and uses no privileged API.
2. The primitive count stays at five; pressure for a sixth is resolved through
   composition unless it passes the strict kernel-membership test.
3. An external app can replicate a bundled app given equivalent grants.
4. Every artifact and host-mediated app side effect traces through the ledger to
   a Run, capability, grant, and initiator.
5. A bare MCP server can be connected with the short add/connect flow and do
   real mediated work through generated forms, artifact cards, and approvals.

These criteria establish architectural validity, not product success. Product
evidence must additionally show that the personal workspace improves recurring
work, focused apps outperform Chat where their interaction model fits, external
developers can use public seams, and the complete host remains measurably lean.

See [Trust model]({{ internal_link_prefix }}{% link trust-model.md %}) for the security consequences of
these boundaries.
