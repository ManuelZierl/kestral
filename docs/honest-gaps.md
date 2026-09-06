---
title: Alpha limitations
layout: default
parent: Operations
nav_order: 3
---

# Alpha limitations

The following limits are part of the `0.1.0-alpha.1` testing boundary, not
hidden fallback behavior.

## Platform and distribution

- Windows and Linux packages are built by CI, but this first alpha has limited
  clean-machine and distribution coverage. macOS artifacts are not produced.
- Every native alpha artifact is unsigned. Windows SmartScreen can show
  **Unknown publisher**, and managed application-control policy can block the
  installer, portable executable, application, or uninstaller completely.
  Signing trust and administrator rights are separate; the current-user NSIS
  and portable ZIP are not intended to require elevation.
- Linux AppImage and server archives normally run without root; `.deb`
  installation normally requires root or `sudo`. Managed Linux policy and
  package-source warnings can still apply.
- No MSI is published for the alpha because the current Tauri/WiX bundler
  rejects a non-numeric SemVer prerelease identifier.
- The frontend dependency audit reports moderate instances of
  `GHSA-frvp-7c67-39w9` through the MCP SDK's Hono dependency. No compatible
  fix is available; Kestral does not use the affected inbound static-file
  server path. High-severity npm audit findings remain a release failure.
- App packages install from directories or public HTTPS Git repositories.
  `.ahpkg` archive ingestion and private Git authentication are not supported.
- External source-built apps can require their own runtimes; normal product
  startup does not.
- The base installer bundles the provider worker, its pinned provider SDK graph,
  and a Node runtime. External apps are independently built, tested, and
  released. CI now enforces release-artifact size ceilings for the Linux
  AppImage, Debian package, backend archive, browser client, Windows portable
  archive, and NSIS installer. Cold and warm startup, idle CPU and memory,
  per-worker resources, app startup, and time to first useful result still lack
  published baselines and regression ceilings. "Lean" is therefore partly
  release-gated, not yet a complete runtime performance contract.

## Isolation and app runtime

- Grants, Runs, and provenance cover actions mediated by Kestral. They do not
  observe or constrain direct actions by an unsandboxed native process.
- Native app backends and stdio tool servers run as the backend OS user.
  Filesystem and network access are not OS-sandboxed in the 0.1 series.
- Release builds require a user-level opt-in before any unsandboxed native app
  can activate. The opt-in is host-wide rather than scoped to one package.
- Custom app UI is sandboxed in an opaque-origin iframe, but OS-level per-frame
  process isolation is not guaranteed.
- A sandboxed custom surface's own `read-only`/`local-write` actions require a
  host-owned physical confirmation before the frame may forward the request,
  regardless of the frontend's current grant snapshot. This is deliberately
  stricter than `silent`/`notify` standing permissions: using frontend grant
  state as the gate would create a race if authority changed before kernel
  preparation. The frame cannot synthesize the host confirmation. Capability
  effects remain provider-declared, so the confirmation attests a human gesture
  rather than independently proving that the declared effect matches the app's
  implementation. Cross-app, external-write, destructive, and unspecified
  effects continue through normal kernel-owned trusted chrome. A future cleaner
  design is to make the kernel consume a single-use host gesture attestation, or
  remove the direct-surface approval shortcut entirely.
- App backends have no general crash-loop or automatic restart policy. The Apps
  screen exposes failed startup state and a manual retry that tears down the
  failed lifecycle and reactivates the same inspected revision through the
  ordinary enable path.
- The file broker resolves a requested path, proves the result is inside the
  granted resource root, and then re-opens it by path. The opened handle is
  checked to be the same file the containment check saw, but a directory
  component replaced between resolution and open can still be followed
  consistently by both steps. Closing that window needs component-wise
  `openat`/`O_NOFOLLOW` resolution. This is defence in depth rather than a live
  escape: app backends are not OS-sandboxed (above), so a process able to win
  the race can already read the file directly.
- Packaged MCP backends do not yet receive broker-mediated own secrets,
  initiate child Runs, consume minimized events, or propose multiple typed
  artifacts through the child protocol.

## Protocols and remote access

- MCP consumption imports tools only, not MCP resources, prompts, or MCP Apps
  UI.
- MCP consumers support explicitly configured static Bearer or custom secret
  headers. MCP OAuth protected-resource discovery, PKCE/DCR, token refresh,
  mTLS, cookies, and signed per-request authentication are not implemented.
- Outbound MCP uses bearer-token profiles. OAuth protected-resource and
  audience validation are not implemented.
- Backend/client split mode is single-owner and has no tenant isolation.
- A passkey-authenticated browser session authorizes the full owner command
  surface. There is no narrower read-only remote role; capability-scoped
  integrations must use MCP export profiles. Sessions are in-memory and a
  backend restart signs every browser out.
- Remote HTTP carries request-correlated custom-surface progress and trusted
  events through one authenticated SSE connection backed by a bounded replay
  feed. Under sustained event pressure the browser detects a sequence gap and
  refreshes authoritative state, but evicted transient progress cannot be
  reconstructed.
- Event delivery remains lossy by design for the minimized host event feed and
  does not guarantee every `run-event` or `app-data-changed` envelope reaches
  every subscriber.
- The browser client cannot browse or directly access the client machine's local
  filesystem. Browser-managed downloads remain local to that client; all File
  Broker resources and app filesystem access are server-side.

## Execution and storage

- Host-managed data contracts v1/v2 support owning-surface CRUD, equality
  queries, optional unique indexes, CAS, fixed transactions, documents, and
  generated delegated `get`/`list` capabilities. V2 also supports fixed typed
  proposal artifacts for exact collection generations, record revisions, and
  document revisions. It does not yet support delegated mutations, a general change
  feed, export/restore UI, automatic incompatible schema transformations, or
  automatic restoration from its preserved contract snapshot. Proposal replay
  and application remain frontend-owned CAS workflows; Kestral does not yet
  provide a generic host replay UI.

- Cancellation is cooperative around generic blocking handlers and approval
  operations. Provider, agent, and MCP adapters add transport or process
  deadlines, but cancellation cannot undo an external effect already started.
  A cancelled or unauthorized late result cannot commit to kernel state.
- A handler receives an invocation-scoped snapshot of its declared secrets.
  Clearing a secret prevents later invocations from resolving it but cannot
  claw the value back from work already in flight.
- Durable kernel state rewrites the complete projection, making commits
  O(total state).
- Invocation input/result bodies are digest-only in the ledger, but artifacts
  and app-owned stores still retain product content. There is no selective
  artifact payload purge policy or encrypted per-payload audit store.
- JSON Schemas are compiled per validation. Large long-lived profiles need
  future validator caching; durable commits still rewrite the complete state
  projection as described above.
- Failed app updates restore their retained source data revision automatically,
  and portable workspace import/export can restore a complete profile snapshot.
  There is still no browser for selecting and restoring one app-data revision. A code downgrade after
  an incompatible migration requires an exact publisher-declared and tested
  reverse edge; otherwise it is refused.
- Development state created before the first public release remains disposable.
  The immutable `alpha.1` fixture corpus and locked migration coordinator form
  the first public forward-migration baseline; each later release still needs an
  explicit fixture-tested step. Do not use irreplaceable data in unpublished
  development profiles.
- Full kernel-state replacement and lock behavior are tested on Windows, but a
  clean-VM sudden-power-loss fault test remains outstanding.
- macOS native credentials are not release-tested. Linux native credentials
  require an unlocked Secret Service. Credential deletion cannot
  guarantee physical erasure from storage snapshots, backups, swap, or media.
- Headless Linux without an unlocked Secret Service fails closed; there is no
  plaintext credential fallback.
- A single host kernel mutex remains a throughput limit. Background projection
  reads fail fast as `kernel busy` and reconcile later. User-triggered Chat,
  configuration, and surface operations wait on blocking workers instead of
  blocking the webview or exposing normal lock contention as an action failure.
  Managed-app backend startup and approval waits release the mutex, and
  split-mode event-driven and safety reconciliations sequence kernel projections
  to avoid manufacturing contention between their own read requests.
- Built-in provider model catalogs are snapshots of the pinned provider worker.
  **Discover models** is live only for providers whose adapters implement
  discovery, including custom Ollama and OpenAI-compatible profiles. Model
  variants are shown only when that catalog or live adapter declares them.
  Text verbosity is shown only for models whose pinned adapter exposes a real
  payload mapping; it is not inferred from model names.
- Kestral does not display remaining ChatGPT Codex subscription quota. OpenAI
  controls account eligibility, model access, quota limits, and revocation;
  exhausted or ineligible accounts fail as provider errors.
- Kestral does not currently provide microphone capture, transcription, speech
  synthesis, or playback. These belong in external apps, but sandboxed app
  surfaces intentionally have no microphone authority today. A future device
  contract must preserve visible activation, cancellation, attribution, bounded
  retention, and explicit provider egress rather than restoring a privileged
  Chat path.
- Reading-opportunity observation is a coarse upper bound and has real blind
  spots. It cannot observe a response reached through a screen reader, a
  reader-mode view, or any path that does not scroll it through Chat's own log,
  so an accessibility user may accumulate no estimate at all while reading
  normally. Explicit marks are unaffected, and an absent estimate never implies
  unread text. Multiple windows or the remote HTTP surface showing the same
  thread each observe independently; merged time is capped by elapsed wall time
  rather than attributed to one window, so overlap stays visible as uncertainty
  instead of being split by guesswork. A crash or forced shutdown loses the
  unflushed interval — this is not crash-complete telemetry. Time is credited
  only for intervals bounded by an observer tick, so a suspended or frozen tab
  contributes nothing rather than hours.
## Intentionally absent

Kestral is not an LLM provider, agent engine, chat runtime, workflow engine, or
general multi-user server. It is also not an IDE, hosted service, or model
runtime. Default-installed Chat remains an ordinary app. Those behaviors remain
in apps or external services rather than becoming privileged kernel features.
