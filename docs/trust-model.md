---
title: Trust model
layout: default
parent: Architecture
nav_order: 1
---

# Trust model
{: .no_toc }

1. TOC
{:toc}

{: .warning }
Grants, Runs, and provenance constrain actions mediated by Kestral. Native app
backends and stdio tool servers can act directly with the backend operating-system
account's filesystem and network authority until process isolation exists. A
sandboxed app surface does not sandbox its backend.

## Trust zones

| Zone | Trusted for | Not trusted for |
|---|---|---|
| Kernel | Identity, grants, invocation validation, Runs, provenance, and durable audit invariants | UI, transport, package acquisition, or product behavior |
| Host runtime | Package lifecycle, persistence, profiles, native credentials, protocol adapters, workers, and composition enforcement | Treating unmediated native-process behavior as a granted or attributable action |
| Host trusted chrome | Approval prompts, permission dialogs, app identity badges, dangerous-action confirmations, secret entry, data-access indicators, and warnings | Acting on behalf of an app without kernel mediation |
| Remote owner console | Complete authenticated host administration and trusted approval presentation for the paired owner | Tenant isolation, capability-scoped integration, or ordinary app UI |
| App surface | Displaying app content, emitting declared intents, and requesting user-owned downloads | Trusted prompts, direct execution, secrets, Tauri, direct filesystem or path access, kernel access, choosing a native download path, or imitating/overlaying trusted chrome |
| App backend | Implementing declared capabilities | Conferring its own grants or provenance; native code is still OS-powerful in the 0.1 series |
| Remote MCP client/server | Protocol messages and advertised metadata | Authority claims; grants remain authoritative |

## Enforced properties

- **Exhaustive manifests:** undeclared capabilities, surfaces, artifacts,
  connectors, subscriptions, and grants have no code path.
- **No self-issued authority:** manifest requests describe needs; only the
  broker issues grants after host policy and trusted-chrome decisions.
- **Permission proposals are non-authoritative:** the bundled Permissions app
  can emit only a kernel-stamped proposal artifact for one exact installed
  capability with no resource scope. Submission revalidates its provenance,
  initiating app, provider, capability, and fixed approval-required policy;
  only the broker can turn it into a grant after a trusted-chrome decision.
- **Permission discovery is non-authoritative:** the bundled Permissions app
  can read a bounded host-generated list of exact installed capabilities the
  invoking app does not hold. MCP tools use the same generic catalog. Provider
  metadata is untrusted descriptive data. Listing a capability neither grants
  it nor makes it callable.
- **One action path:** input, grant, deadline, and lifecycle state are checked
  before execution and again before a result commits.
- **Direct human control:** a declared same-app action from the provider's live
  surface is already an explicit human command. It remains grant-checked and
  audited, but grant interaction conditions cannot add a notice or per-use
  approval. Delegated cross-app, LLM, agent, automation, and programmatic calls
  remain condition-gated.
- **Scoped audit:** invocation and approval records identify the exact requested
  data scope, not only the broader grant that covered it.
- **Broad resource grants stay explicit:** an `all-resources` grant covers every
  current and future resource governed by its exact provider capability. It is
  shown as broad access and starts unchecked during install review. Apps cannot
  invoke with that wildcard; each invocation must still name exact resources.
- **Kernel-written provenance:** apps propose content; the kernel stamps the
  producer, capability, Run, grant, and time after validating every artifact.
- **Artifact snapshots:** handlers receive only a read-only snapshot resolver
  scoped to the exact artifact resource IDs authorized for that invocation.
  Query can list only metadata and provenance; read returns bounded content for
  an exact authorized artifact ID. Malformed cursors and oversized content fail
  closed. An invocation with no exact artifact IDs fails explicitly; it cannot
  turn missing authority into a successful empty query. Chat and agent tool
  catalogs omit artifact capabilities that have no authorized current artifact;
  query expands live standing grants to exact current IDs and read derives its
  exact ID from validated tool input before kernel preparation.
- **Chat thread resources:** `chat.list_threads` returns only bounded metadata
  for exact resources authorized for that invocation. `chat.read_thread` returns a paginated
  public transcript for the exact authorized resource ID and omits private
  messages, reasoning, system entries, tool-status records, and hidden extension
  context. The feed is stable but lossy at the inbox layer, so repeated delivery
  is not guaranteed.
- **Visible refusals and failures:** denial, invalid output, transport failure,
  and handler panic become typed Run outcomes rather than fake success.
- **Secret mediation:** surfaces never read credentials. Apps declare secret
  names, and broker-filtered snapshots expose values only to the authorized
  invocation boundary. Clearing a secret applies to later invocations; it does
  not revoke a value already copied into an in-flight invocation snapshot.
- **MCP client credentials:** static HTTP header values are host-adapter state
  in the OS credential vault. They are resolved only for an explicit connect,
  marked sensitive in the HTTP client, applied to every request, and never
  copied into the kernel broker, frontend stores, config JSON, ledger, or logs.
- **Teardown:** uninstall revokes grants in both directions, closes surfaces,
  cancels active Runs, releases leases, and drops event inboxes. Reinstalling
  the same ID inherits no kernel authority by identity alone.
- **Surface document isolation:** verified custom UI is served through an
  opaque host route with its own response CSP, never through inherited-policy
  `srcdoc` and never by allowing inline scripts in the trusted shell. The route
  is unguessable, carries no app-supplied path, uses an ordinary non-Tauri
  random-port loopback origin in native mode, permits no child frames or
  objects, refuses Tauri IPC origins as network destinations, and is invalidated
  on replacement, disable, or uninstall. The iframe remains opaque-origin and
  communicates only through the source-, instance-, schema-, and binding-checked
  surface bridge.
- **Observable native downloads:** a custom surface may request a download but
  cannot choose its path or read the resulting file. In native desktop mode the
  host resolves and creates the owner's download directory, chooses a
  collision-free filename, observes WebKit completion, and renders success or
  failure outside the app frame. Browser-host mode leaves the destination and
  completion UI to the local browser.
- **Monotonic migration authority:** profile migration cannot turn an exact
  capability into provider-wide access, fixed resources into `all-resources`,
  a visible condition into `silent`, or a finite expiry into a later or
  non-expiring grant. Issued and revoked grant facts remain present. Pending
  approvals, owner sessions, OAuth/WebAuthn ceremonies, leases, event inboxes,
  workers, and streams are never restored as migrated authority.
- **Portable transfer narrows dormant app authority:** archives contain no app
  binaries, OS-vault values, passkeys, or external file paths. Imported
  third-party app registrations are removed, grants involving those app IDs are
  revoked without deleting issued facts, and a matching package must pass the
  normal verification and approval path before it can run again.
- **Bundled declaration upgrades:** version and top-level presentation metadata
  may change without uninstalling a bundled app. Every authority and behavioral
  declaration must remain identical; otherwise the transition fails before the
  manifest or existing grants change.
- **Bounded event feed:** apps receive minimized host event views, not the raw
  trusted ledger and not cross-app RPC. `run-event` and `app-data-changed` are
  tagged envelopes with bounded, lossy delivery semantics; inbox overflow drops
  old events and records the loss.
- **Bounded progress:** transient progress requires a typed `kind`, is limited
  to 64 KiB and 120 events per second, is never persisted, and cancellation is
  requested if the host reports that its consumer disappeared.
- **Shared-state coordination:** concurrent Runs use time-bounded leases over
  shared artifacts and workspace paths. Conflicts are surfaced instead of
  silently overwriting another Run's work.
- **Agent mediation:** agent model and tool callbacks become child Runs under
  the original initiating app's grants. The agent worker has no kernel handle,
  credentials, or direct capability path.
- **Prompt transparency:** the host's Chat system prompt is assembled from a
  visible immutable protocol layer, assistant instructions, explicit skills,
  and optional runtime context. Skills are descriptive only; they never confer
  authority.
- **Model-profile narrowing:** an external app may opt into Chat's versioned
  model-profile editor contract and store model and generation choices, selected
  Chat prompt layers, bounded custom prompt text, and exact tool references. The
  protocol prompt layer cannot be removed.
  Missing selected prompt layers fail closed. Tool references are only an
  allowlist: Chat intersects them with its active grants on every plain or
  delegated turn. Missing, expired, or revoked grants remain unavailable, and
  an empty allowlist supplies no tools.
- **No credential widening through profiles:** credential-free local model
  profiles may be selected directly. A credential-bearing model profile is
  usable only while it is the active Chat default, because each invocation
  receives only that profile's broker-authorized synthetic credential alias.
  Neither Chat nor Model Profiles reads a different saved credential.

Trusted chrome is technically outside app frames at all times. Apps cannot
render approvals, identity, secret prompts, permission warnings, or data-access
indicators, even when their surface visually resembles the surrounding host.
Built-in themes keep a stable amber-on-dark signature; the owner can customize
trusted-chrome colors through host-owned Appearance settings.

The host may send a sandboxed surface its resolved semantic color variables and
that package's validated, namespaced app-color declarations. This is
presentation data only; protected trusted-chrome tokens are not included.
Frames cannot register colors at runtime, overwrite
host or trusted-chrome tokens, edit Appearance, or read another app's color
namespace.

Backend-only mode uses SSH-created one-time pairing codes to register WebAuthn
passkeys. A successful passkey assertion creates an opaque, short-lived,
server-side owner session represented by an `HttpOnly`, `SameSite=Strict`
cookie. Pairing codes are stored only as digests, consumed before registration,
and expire after ten minutes; ceremony state never goes to the client. The
session still confers complete owner authority over the remote command surface
and is suitable only for a paired trusted owner console. Apps and integrations
use grants and capability-scoped MCP exports; owner authentication is never an
app authority mechanism.

Serving the frontend, HMR, and `/api` through one development origin changes
only the transport topology. Vite proxies API requests to the loopback backend;
it does not receive host filesystem, config, secret-store, worker, grant, or
kernel authority. Those remain inside `host-server`, as they do behind the
single-origin HTTPS proxy used for a split deployment.

## Package integrity and publisher identity

Package inspection validates the format, app namespace, embedded JSON Schemas,
complete asset allowlist, SHA-256 checksums, host-version floor, and manifest
consistency without executing package code. Missing, extra, traversing,
symlinked, non-regular, duplicate-normalized, case-colliding, or mismatched
payload entries fail inspection.

Package app icons are host metadata, not executable UI. Custom image bytes are
integrity-covered, limited to 256 KiB, checked against an allowed image type,
and returned to the shell as image data rather than a package filesystem path.
SVG icons with active or external content are refused, and the shell renders
accepted SVG only as a passive image resource (`img` or a CSS mask for
`currentColor` artwork), never as trusted-chrome markup.

Inspection copies only declared files into randomized host-owned staging. The
package digest is deterministic over each normalized path, byte length, and
exact staged bytes in path order. Installation consumes the opaque staging ID
and approved digest, copies into a temporary content-addressed location,
verifies before and after atomic replacement, and never re-reads the mutable
source path. The separately sealed kernel manifest deliberately has a different
hash because it excludes host-only package and UI data.

An optional detached Ed25519 signature binds the package digest and app ID to a
publisher key. Invalid or revoked signatures fail installation. A valid unknown
key requires an explicit trust decision; unsigned packages remain installable
with a visible verdict. Publisher trust helps identify code origin but does not
grant runtime authority.

Kernel manifest seals are separate content hashes. They detect manifest
tampering but are not publisher signatures. Grant preparation binds both the
holder and provider manifest hashes, so code replacement invalidates a decision
collected against an earlier provider declaration.

The durable ledger stores invocation input/result digests rather than their
full bodies. This avoids duplicating cloud prompts, conversations, and tool
results in audit history. Artifact content and app-owned stores retain their own
product data under their documented lifecycle.

System reset is host-owner administration, not an app capability. Sandboxed app
surfaces cannot call Tauri or the remote owner API; a paired remote console has
full owner authority and can schedule the same reset. The host requires an exact
profile-specific confirmation phrase at the backend boundary, applies deletion
before profile stores open on restart, clears indexed OS-vault credentials, and
fails visibly while retaining the reset request if complete deletion cannot be
verified. The operation covers Kestral-owned current-profile state only. Other
profiles, external resource files, provider-held data, operating-system logs,
and data written outside the profile root remain outside its authority.

## Provider credential boundary

The bundled provider backend, pinned provider SDK graph, and bundled Node
runtime are trusted provider infrastructure. They are not available to
third-party package backends. Each model or catalog operation receives only its
selected profile configuration and broker-authorized credential over bounded,
correlated NDJSON stdin. Credentials never travel in process arguments,
environment variables, capability input, progress, or ledger data. The worker's
authentication context cannot read ambient environment credentials or files and
registers only the selected provider.

OAuth is host-mediated: trusted chrome displays URLs, device codes, progress,
and prompts. Opaque credential state returns directly to OS-backed profile
storage, including token rotation on failed operations, and never passes through
frontend state. Worker messages reject unknown fields and fail closed.
Cancellation requests graceful worker cancellation, then terminates the process
after a bounded grace period; late output still cannot pass kernel finalization.

ChatGPT Codex profiles use pi-ai's OpenAI OAuth client and the fixed ChatGPT
subscription backend. The resulting access token and account identifier are
credential material: only the selected invocation worker receives them, and
Settings receives presence and terminal sign-in status only. This authority can
consume the connected account's Codex quota. OpenAI remains responsible for
account eligibility, model availability, quota enforcement, and revocation;
Kestral does not infer remaining quota or fall back to API-key billing.

Profiles remain explicit authority and policy choices. Cloud profiles require
the data-egress acknowledgement. Ambient AWS profiles, shared credential files,
Vertex application-default credentials, and arbitrary provider environment
lookup are disabled. A new multi-field credential form requires an explicit
host config and secret schema before support is enabled.

No provider is selected in a fresh profile. An unconfigured `llm.generate`
invocation fails visibly without reading a credential, starting the provider
worker, probing local ports, or contacting a network endpoint. Chat translates
that specific failure into fixed host-authored setup guidance. The invocation
produces no provider-response artifact or provider metrics and is never recorded
as a successful model call.

Provider text verbosity is separate from reasoning effort. The host exposes it
only when the pinned adapter advertises and enforces the control for the selected
model. Unsupported combinations fail rather than being silently ignored.

A newly created profile contains an unauthenticated remote MCP configuration for
the public Kestral GitMCP documentation endpoint and attempts one connection on
that first startup. This is a disclosed network request to `gitmcp.io`; it sends
the normal MCP handshake and tool requests but no Kestral credential. The remote
service and its tool metadata remain untrusted. Trusted chrome separately
controls installation grants and exact Chat grants, and Chat grants default to
per-use approval. Rejection grants no authority, endpoint failure does not block
startup, and the owner can disconnect or delete the server like any other MCP
configuration.

## Residual native-code authority

{: .warning }
Native backends and stdio tool servers are trusted backend code in the 0.1
series. They run as the backend operating-system account. Kernel grants
constrain actions sent through the host, not direct OS filesystem or network
access by that process. Sandboxed app UI does not make its backend sandboxed.

Package-declared app-data migration commands are native backend code with the
same authority mode as their app backend. They are not kernel capabilities and
receive no grants or secrets. The host passes a verified read-only payload and a
staged candidate as `APP_HOST_DATA_DIR`, never the active source revision or the
host-owned surface-state envelope. This prevents an honest migrator from
modifying the source through the supplied path, but an unsandboxed process still
retains its operating-system account's ambient filesystem and network authority.
Install and update such packages only when that residual authority is acceptable.

Chat message extensions can publish validated text ranges for host-rendered
marks, but extension-state messages carry no model authority. An app that needs
to influence a later response must invoke `chat.inject_user_context` with an
exact thread resource through a covering grant. The action is a Run, and the
handler derives source identity, version, content hash, and Run ID from the
kernel invocation context.

Before each send, Chat requires the source Run to have completed that capability
under its original grant, requires that exact grant to remain active and cover
the thread, and requires the installed source hash to remain unchanged. Failed
or cancelled Runs never become model-visible. Revocation, expiry, uninstall, or
replacement makes stored text inert, and a later grant cannot revive it. This is
the security boundary; the model is not asked to classify an untrusted block
with trusted exceptions.

Authorized text is supplemental user-level input in a late, attributed message.
The next visible user message wins conflicts. It cannot override the immutable
host protocol, grant tools or permissions, or prove side effects, but it can
influence Chat to use tools Chat already holds. Trusted chrome therefore labels
an all-conversation silent grant as broad standing authority. The exact text is
not part of the visible transcript. Chat always exposes currently stored entries
in its host-owned model-context inspector. When exact recording is enabled, the
host stores the exact effective message in the per-send receipt; otherwise it
stores only source, Run, grant, revision, and digest metadata.

Chat prompt composition is review-aware but not authority-bearing: the selected
assistant profile receipt pins the reviewed skill digests and profile digest at
send time, while future sends fall back to Standard if the selected profile or
reviewed skill content is no longer available.

Model profile receipts follow the same fail-closed change rule. A future turn
uses a selected profile only while the external source app, source version,
profile digest, and configured provider profile remain available. Otherwise
Chat clears the selection and uses its default. Historical composition receipts
retain the exact selected model-profile snapshot and effective capability list.

Chat's prompt layers are transparent but not expansive: runtime identity is on
by default, while app inventory and connector/profile identifiers are off by
default. The host does not include secrets, base URLs, file paths, tool outputs,
or conversation history in the system prompt. There is no `llm.alter_system_prompt`
grant; prompt composition is host-owned configuration, not a capability.

Reading-opportunity observation is opt-in telemetry with a deliberately narrow
ceiling. Chat observes nothing until an installed extension asks. What crosses the
sandbox boundary is bounded aggregates — cumulative qualified-visible
milliseconds and a 32-band exposure bitset per session. Raw geometry is converted
at the host boundary and discarded, so no scroll offset, viewport size,
intersection ratio, focus-event log, pointer path, keystroke, window title, or
DOM content outside the owning response is ever persisted or sent. Observation
requires no grant because it produces no cross-app or external authority: an app
receives aggregates about its own extension's response and nothing else.

The resulting estimate is an upper bound on what was *possible*, not a
measurement of what happened. It is never authority: it cannot mark text read,
cannot override or weaken an explicit mark, and is not an input to any grant,
approval, or audit decision. Kestral does not detect attention, comprehension,
actual reading, or reading speed, and the estimate must not be presented as if it
did. Concurrent windows are capped by elapsed wall time so aggregates stay
physically possible, preserving uncertainty rather than inventing an allocation.
Crash or forced shutdown may lose the unflushed interval; this is not
crash-complete telemetry.

Model-facing extension state has no direct path into a request. Only text
accepted through `chat.inject_user_context` is actionable, and its authority is
revalidated as described above. What a mark or derived estimate means remains
app-owned guidance; an enabled app skill can explain it but cannot grant the
injection capability.

Sandboxed surfaces may persist bounded JSON in a host-owned namespace scoped by
their live app and surface binding. This private state path has no cross-app or
external authority and is therefore not a capability or grant use. It cannot
name another app, read files, or access secrets. Cross-app reads of the same
state remain ordinary grant-mediated capabilities. Native backend code for the
owning app can read its app data directory, so the store does not isolate a
surface from its own backend.

Backend-free apps may separately declare bounded host-managed domain data. The
owning live surface can use only its own declared record and document
collections through the closed `window.appHost.data.v1` or `data.v2` bridge; it
cannot supply an app identity or path. This private owning-app access is not
delegated authority and creates no grant or Run. Generated cross-app reads are
ordinary capabilities: they require
a grant, an exact `app-data:<provider>:<collection>` invocation resource, and a
Run through the full action path. Contract v2 proposal capabilities are also
ordinary grants and Runs. They require an exact collection, record, or document
resource scope derived from the host-generated schema, even when a standing
grant says all resources. The handler revalidates package/contract bytes and
the current target generation or revision before returning an artifact draft;
it cannot mutate managed data. The kernel validates and stamps that artifact in
the normal finalization transaction. A proposal artifact is reviewable data,
not an authorization decision. Applying it is a frontend-owned CAS operation.
Contract v1 and v2 deliberately have no delegated managed-data writes because
current handlers cannot defer a data commit until finalization.
Host-managed persistence removes the owning app's need for native code but is
not OS isolation from some other unsandboxed process running as the same account.

The host uses bounded process handling, strips ambient credentials from worker
protocols, verifies packaged payloads, and surfaces the authority mode. Those
controls contain failures and reduce accidental authority; they are not an OS
sandbox.

Host-owned JSON files are owner-only on Unix. This prevents other local OS
accounts from reading profile metadata through permissive default umasks, but
does not protect stores from native backend code running as the Kestral account.
Atomic replacement, checksums where present, and strict document validation are
crash/corruption controls, not authentication against a process with that same
account's filesystem authority.

## Design success criteria

The behavioral suites enforce the central claims:

1. Chat uses the public app API and remains ordinary userland.
2. The primitive count remains five.
3. A third-party app with the same grants receives the same capability access.
4. Every artifact can be traced through provenance to a Run, capability, grant,
   producer, and initiator.
5. A bare MCP tool becomes useful through generic forms, result artifacts, and
   approval-required grants without entering the kernel ontology.
