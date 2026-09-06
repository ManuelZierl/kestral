<script lang="ts">
  import { onDestroy, onMount } from "svelte";

  import {
    appSurfaceEvents,
    closeSurface,
    getAppConfig,
    getSurfaceState,
    listAppArtifacts,
    listGrants,
    openSurface,
    putSurfaceState,
    requestManagedData,
    submitAction,
    cancelSurfaceAction,
    updateAppConfig,
    type ActionIntent,
    type CapabilityRef,
    type InstalledApp,
    type JsonObject,
    type SurfaceActionOutcome,
    type SurfaceBinding,
    type SurfaceDeclaration,
    type SurfaceUiBundle,
  } from "$lib/api";
  import {
    createSurfaceBridge,
    trustedSourceGuard,
    type SurfaceBridge,
  } from "$lib/surfaces/hostSurfaceBridge";
  import {
    SURFACE_BRIDGE_VERSION,
    extensionEventMessage,
    initMessage,
    themeMessage,
    type HostToAppMessage,
    type SurfaceInitMessage,
  } from "$lib/surfaces/surfaceBridgeProtocol";
  import { needsHostGestureAttestation } from "$lib/surfaces/surfaceActionAttestation";
  import { loadSurfaceHostContext } from "$lib/surfaces/modelProfileEditorContext";
  import { resolvedAppearance, surfaceThemeVariables } from "$lib/stores/theme";
  import LoadingIndicator from "$lib/shell/LoadingIndicator.svelte";

  interface Props {
    app: InstalledApp;
    surface: SurfaceDeclaration;
    bundle: SurfaceUiBundle;
    /** Context supplied by an extension slot, not the untrusted frame. */
    extensionContext?: JsonObject;
    /// Standalone mode: the surface owns the whole workspace, so the iframe
    /// stretches to the available height (the app scrolls internally) instead
    /// of sizing to content. Requires host-owned chrome around it (the top
    /// bar) to carry the app's identity — the in-flow identity strip only
    /// shows while loading.
    fill?: boolean;
    onOutcome?: (outcome: SurfaceActionOutcome) => void;
    /// The frame published slot-specific state for its extension point owner.
    /// Untrusted: validate against the extension point's contract before use.
    onExtensionState?: (payload: JsonObject) => void;
    /// How long to wait for the frame's `ready` before declaring it hung. A
    /// frame that never responds is isolated as an error — it never blocks or
    /// crashes the host.
    handshakeTimeoutMs?: number;
  }

  let { app, surface, bundle, extensionContext = {}, fill = false, onOutcome, onExtensionState, handshakeTimeoutMs = 10000 }: Props = $props();

  let status = $state<"loading" | "ready" | "error">("loading");
  let errorMessage = $state<string | null>(null);
  let appNotice = $state<string | null>(null);
  let documentUrl = $state<string>("");
  let iframeEl = $state<HTMLIFrameElement | null>(null);
  let pendingAttestation = $state<ActionIntent | null>(null);
  let attestationCancelButton = $state<HTMLButtonElement | null>(null);
  let pendingAttestationResolve: ((approved: boolean) => void) | null = null;
  // Content height the frame reports over the bridge, so the iframe fits its
  // content instead of stranding a small control in a large fixed box. Clamped
  // against an untrusted frame reporting an absurd value.
  let contentHeight = $state<number | null>(null);
  const MAX_SURFACE_HEIGHT = 20000;

  const appId = $derived(app.manifest.app_id);
  const appThemeColors = $derived(app.theme_colors ?? []);
  const declaredIntents = $derived<CapabilityRef[]>(surface.intents);
  const pendingDeclaration = $derived(
    pendingAttestation
      ? app.manifest.capabilities.find((capability) => capability.name === pendingAttestation?.capability.capability) ?? null
      : null,
  );
  // Host storage supports a single config section per app; expose its schema
  // for the frame to render a settings form.
  const configSchema = $derived<JsonObject | null>(
    app.manifest.config_declarations[0]?.json_schema ?? null,
  );

  let binding: SurfaceBinding | null = null;
  let bridge: SurfaceBridge | null = null;
  let listener: ((event: MessageEvent) => void) | null = null;
  let handshakeTimer: ReturnType<typeof setTimeout> | null = null;
  let startRetryTimer: ReturnType<typeof setTimeout> | null = null;
  let initPayload: SurfaceInitMessage | null = null;
  let destroyed = false;
  // A protocol-version mismatch cannot heal by retrying; every other failure
  // (kernel busy, slow frame boot, transient open failure) may.
  let permanentError = $state(false);
  // Invalidates an older in-flight start() when retry() begins a new attempt,
  // so a stale continuation can never claim the component after teardown.
  let attempt = 0;
  const KERNEL_BUSY_RETRY_MS = 1000;
  const KERNEL_BUSY_WRITE_ATTEMPTS = 3;

  async function retryKernelBusy<T>(operation: () => Promise<T>): Promise<T> {
    for (let attempt = 1; ; attempt += 1) {
      try {
        return await operation();
      } catch (failure) {
        if (attempt >= KERNEL_BUSY_WRITE_ATTEMPTS || !String(failure).includes("kernel busy")) {
          throw failure;
        }
        await new Promise((resolve) => setTimeout(resolve, KERNEL_BUSY_RETRY_MS));
      }
    }
  }

  function scheduleStartRetry(myAttempt: number): void {
    if (destroyed || myAttempt !== attempt || startRetryTimer !== null) return;
    startRetryTimer = setTimeout(() => {
      startRetryTimer = null;
      if (!destroyed && myAttempt === attempt) void start();
    }, KERNEL_BUSY_RETRY_MS);
  }

  function postToFrame(message: HostToAppMessage): void {
    // Opaque-origin sandbox frames can only be reached with "*". We always
    // post to this frame's own window, so it reaches nowhere else. Svelte
    // state may wrap nested arrays in proxies, which structured clone rejects;
    // the bridge contract is JSON, so materialize a plain wire value first.
    const wireMessage = JSON.parse(JSON.stringify(message)) as HostToAppMessage;
    iframeEl?.contentWindow?.postMessage(wireMessage, "*");
  }

  function releaseBinding(): void {
    if (!binding) return;
    const currentBinding = binding;
    binding = null;
    // Teardown cannot repair a failed surface start, but it must release the
    // kernel binding without producing an unhandled rejection.
    void closeSurface(currentBinding).catch(() => {});
  }

  function finishAttestation(approved: boolean): void {
    const resolve = pendingAttestationResolve;
    pendingAttestationResolve = null;
    pendingAttestation = null;
    resolve?.(approved);
  }

  async function attestOwnApprovalAction(intent: ActionIntent): Promise<boolean> {
    const grants = await retryKernelBusy(() => listGrants());
    if (!needsHostGestureAttestation(
      appId,
      app.manifest.capabilities,
      grants,
      intent.capability,
      intent.data_scope,
    )) {
      return true;
    }
    if (pendingAttestation !== null || pendingAttestationResolve !== null) {
      throw new Error("Another app action is already waiting for host confirmation.");
    }
    return await new Promise<boolean>((resolve) => {
      pendingAttestationResolve = resolve;
      pendingAttestation = intent;
    });
  }

  function handleAttestationKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      finishAttestation(false);
    }
  }

  $effect(() => {
    if (pendingAttestation && attestationCancelButton) {
      queueMicrotask(() => attestationCancelButton?.focus());
    }
  });

  onMount(() => {
    void start();
  });

  async function start(): Promise<void> {
    const myAttempt = ++attempt;
    const stale = () => destroyed || myAttempt !== attempt;
    // Refuse a bundle written for a bridge version this host doesn't speak.
    if (bundle.protocol_version !== SURFACE_BRIDGE_VERSION) {
      permanentError = true;
      status = "error";
      errorMessage = `This app surface needs a different host (bridge v${bundle.protocol_version}, host v${SURFACE_BRIDGE_VERSION}).`;
      return;
    }

    // This attempt's binding stays in a local until the attempt commits: when
    // a stale continuation resumes, the shared `binding` may already belong to
    // a newer attempt and must not be released from here.
    let openedBinding: SurfaceBinding;
    try {
      openedBinding = await openSurface(appId, surface.name);
    } catch (failure) {
      if (stale()) return;
      // Synchronous kernel commands deliberately fail fast instead of waiting.
      // Contention is transient, so keep loading and retry without user action.
      if (String(failure).includes("kernel busy")) {
        scheduleStartRetry(myAttempt);
        return;
      }
      // App uninstalled / surface gone: fail visibly, no crash.
      status = "error";
      errorMessage = String(failure);
      return;
    }
    if (stale()) {
      void closeSurface(openedBinding).catch(() => {});
      return;
    }
    binding = openedBinding;

    let config: JsonObject;
    let hostContext: JsonObject;
    try {
      [config, hostContext] = await Promise.all([
        getAppConfig(appId),
        loadSurfaceHostContext(app, surface.name),
      ]);
    } catch (failure) {
      if (stale()) {
        void closeSurface(openedBinding).catch(() => {});
        if (binding === openedBinding) binding = null;
        return;
      }
      releaseBinding();
      if (String(failure).includes("kernel busy")) {
        scheduleStartRetry(myAttempt);
        return;
      }
      status = "error";
      errorMessage = `Unable to load this app surface's configuration or host context: ${String(failure)}`;
      return;
    }
    if (stale()) {
      void closeSurface(openedBinding).catch(() => {});
      if (binding === openedBinding) binding = null;
      return;
    }

    const appearance = $resolvedAppearance;
    initPayload = initMessage({
      instanceId: binding.instance_id,
      appId,
      surface: surface.name,
      capabilities: declaredIntents,
      configSchema,
      config,
      theme: appearance.theme,
      variables: surfaceThemeVariables(appId, appThemeColors, appearance),
      extensionContext,
      hostContext,
    });

    bridge = createSurfaceBridge({
      binding,
      declaredIntents,
      actions: {
        invoke: async (intent, onProgress) => {
          // The kernel historically lets a provider's own read/local-write
          // surface action stand in for a per-use approval. A sandboxed frame
          // cannot prove that a human actually clicked, so host chrome obtains
          // that physical gesture before forwarding exactly those actions.
          // The kernel still re-checks the grant and remains the authority.
          if (!(await attestOwnApprovalAction(intent))) {
            throw new Error("Action cancelled before execution.");
          }
          const outcome = await submitAction(binding!, intent, onProgress);
          onOutcome?.(outcome);
          // The frame receives this action's result as the invoke return value
          // and updates from it directly. Do NOT echo an `event` back here: a
          // frame that refreshes on `onEvent` (e.g. the Tasks surface) would
          // re-invoke, and that invoke would echo again — a self-sustaining
          // loop that keeps the surface perpetually "busy" and its inputs
          // disabled. External-change events belong to a real feed, not to a
          // frame's own action.
          return outcome;
        },
        cancelRun: (runId) => cancelSurfaceAction(binding!, runId),
        getConfig: () => getAppConfig(appId),
        updateConfig: (next) => retryKernelBusy(() => updateAppConfig(appId, next)),
        getState: (key) => getSurfaceState(binding!, key),
        putState: (key, expectedRevision, value) =>
          putSurfaceState(binding!, key, expectedRevision, value),
        managedData: (request) => retryKernelBusy(() => requestManagedData(binding!, request)),
        listArtifacts: () => listAppArtifacts(appId),
        listEvents: () => appSurfaceEvents(appId),
      },
      post: postToFrame,
      // Origin + source gate: only messages from THIS frame's window with an
      // opaque (sandbox) origin are trusted.
      isTrustedSource: trustedSourceGuard(() => iframeEl?.contentWindow ?? null),
      onReady: () => {
        status = "ready";
        clearHandshakeTimer();
      },
      onAppError: (message) => {
        appNotice = message;
      },
      onResize: (height) => {
        contentHeight = Math.min(Math.max(Math.ceil(height), 0), MAX_SURFACE_HEIGHT);
      },
      onExtensionState: (payload) => {
        onExtensionState?.(payload);
      },
    });

    listener = (event: MessageEvent) => bridge?.handleMessage(event);
    window.addEventListener("message", listener);

    // Render the frame (its `load` posts init) and start the hang guard.
    documentUrl = bundle.document_url;
    handshakeTimer = setTimeout(() => {
      if (status === "loading") {
        status = "error";
        errorMessage = "This app surface didn't respond and was stopped.";
        // A hung frame stays mounted showing the error, so release its
        // resources now rather than stranding the kernel binding and message
        // listener until the whole component unmounts.
        teardown();
      }
    }, handshakeTimeoutMs);
  }

  // Recovery path for transient failures: release whatever the failed attempt
  // left behind, reset to a clean loading state, and open the surface again.
  function retry(): void {
    if (destroyed || permanentError) return;
    teardown();
    initPayload = null;
    documentUrl = "";
    contentHeight = null;
    appNotice = null;
    errorMessage = null;
    status = "loading";
    void start();
  }

  function handleFrameLoad(): void {
    const appearance = $resolvedAppearance;
    if (initPayload) postToFrame({
      ...initPayload,
      theme: appearance.theme,
      variables: surfaceThemeVariables(appId, appThemeColors, appearance),
    });
  }

  /// Send a slot-specific message to the frame (the extension point owner's
  /// half of the extension contract). A message posted before the frame is
  /// ready is dropped — owners react to state the frame published, so the
  /// frame is live by the time they have anything to send.
  export function sendExtensionEvent(payload: JsonObject): void {
    postToFrame(extensionEventMessage(payload));
  }

  $effect(() => {
    const appearance = $resolvedAppearance;
    if (initPayload && iframeEl) {
      postToFrame(themeMessage(
        appearance.theme,
        surfaceThemeVariables(appId, appThemeColors, appearance),
      ));
    }
  });

  function clearHandshakeTimer(): void {
    if (handshakeTimer !== null) {
      clearTimeout(handshakeTimer);
      handshakeTimer = null;
    }
  }

  function clearStartRetryTimer(): void {
    if (startRetryTimer !== null) {
      clearTimeout(startRetryTimer);
      startRetryTimer = null;
    }
  }

  // Idempotent: safe to run from both the hang guard and unmount.
  function teardown(): void {
    if (listener) {
      window.removeEventListener("message", listener);
      listener = null;
    }
    finishAttestation(false);
    clearHandshakeTimer();
    clearStartRetryTimer();
    bridge = null;
    releaseBinding();
  }

  onDestroy(() => {
    destroyed = true;
    teardown();
  });
</script>

{#if pendingAttestation}
  <div class="attestation-backdrop">
    <div
      class="attestation-dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="attestation-title"
      aria-describedby="attestation-detail"
      tabindex="-1"
      onkeydown={handleAttestationKeydown}
    >
      <div class="attestation-badge">Kestral host confirmation</div>
      <h2 id="attestation-title">Confirm app action</h2>
      <p id="attestation-detail">
        <strong>{app.manifest.display_name}</strong> wants to use
        <code>{pendingAttestation.capability.provider}/{pendingAttestation.capability.capability}</code>.
        This confirmation is outside the app frame so scripted app UI cannot count as your approval.
      </p>
      {#if pendingDeclaration}
        <p class="attestation-description">{pendingDeclaration.description}</p>
      {/if}
      {#if pendingAttestation.goal}
        <div class="attestation-goal">
          <span>Purpose</span>
          <strong>{pendingAttestation.goal}</strong>
        </div>
      {/if}
      <div class="attestation-actions">
        <button
          type="button"
          class="attestation-deny"
          bind:this={attestationCancelButton}
          onclick={() => finishAttestation(false)}
        >Cancel</button>
        <button type="button" class="attestation-approve" onclick={() => finishAttestation(true)}>
          Continue
        </button>
      </div>
    </div>
  </div>
{/if}

{#if status === "error"}
  <div class="surface-error" role="alert">
    <h3>{surface.title}</h3>
    <p>{errorMessage}</p>
    {#if !permanentError}
      <button type="button" class="retry" onclick={retry}>Try again</button>
    {/if}
  </div>
{:else}
  <div class="surface-frame" class:fill>
    <!-- Host-rendered identity strip. Trusted chrome (identity, approvals)
         always lives OUTSIDE the app frame; the app cannot draw it. In fill
         mode the host top bar already names the app, so the strip only
         doubles as a loading indicator and leaves once the frame is ready. -->
    {#if !fill || status !== "ready"}
      <div class="identity" data-testid="surface-identity">
        <span class="dot" class:ready={status === "ready"} aria-hidden="true"></span>
        <span class="name">{app.manifest.display_name}</span>
        <span class="sep">·</span>
        <span class="title">{surface.title}</span>
        {#if status !== "ready"}<span class="loading">· loading…</span>{/if}
      </div>
    {/if}
    {#if appNotice}
      <p class="app-notice" role="status">{appNotice}</p>
    {/if}
    {#if status === "loading"}
      <LoadingIndicator
        fill={fill}
        size={fill ? 2.5 : 1.5}
        label={`Loading ${app.manifest.display_name}…`}
      />
    {/if}
    {#if documentUrl}
      <!-- Hidden zero-size frames are not eligible for lazy loading in WebKitGTK,
           but the host cannot reveal this frame until its ready handshake. -->
      <iframe
        bind:this={iframeEl}
        title={`${app.manifest.display_name}: ${surface.title}`}
        sandbox="allow-scripts allow-forms allow-downloads"
        src={documentUrl}
        onload={handleFrameLoad}
        referrerpolicy="no-referrer"
        allow=""
        loading="eager"
        class:loading={status === "loading"}
        style={!fill && contentHeight !== null ? `height: ${contentHeight}px` : undefined}
      ></iframe>
    {/if}
  </div>
{/if}

<style>
  .attestation-backdrop {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    box-sizing: border-box;
    padding: 1rem;
    background: var(--color-scrim);
  }
  .attestation-dialog {
    box-sizing: border-box;
    width: min(100%, 30rem);
    max-height: 100%;
    overflow-y: auto;
    padding: 1.25rem 1.5rem;
    border: 3px solid var(--color-chrome-accent);
    border-radius: 14px;
    background: var(--color-chrome-bg);
    color: var(--color-chrome-text);
    box-shadow: 0 12px 48px var(--color-shadow-strong);
  }
  .attestation-badge {
    display: inline-block;
    margin-bottom: 0.75rem;
    padding: 0.25rem 0.55rem;
    border-radius: 6px;
    background: var(--color-chrome-accent);
    color: var(--color-chrome-accent-contrast);
    font-size: 0.75rem;
    font-weight: 700;
  }
  .attestation-dialog h2 {
    margin: 0 0 0.5rem;
    font-size: 1.15rem;
  }
  .attestation-dialog p {
    margin: 0 0 0.65rem;
    line-height: 1.45;
  }
  .attestation-dialog code {
    color: var(--color-chrome-text-muted);
  }
  .attestation-description {
    color: var(--color-chrome-text-muted);
  }
  .attestation-goal {
    display: grid;
    gap: 0.2rem;
    margin: 0.8rem 0;
    padding: 0.6rem 0.7rem;
    border: 1px solid var(--color-chrome-panel-border);
    border-radius: 8px;
    background: var(--color-chrome-panel-bg);
  }
  .attestation-goal span {
    color: var(--color-chrome-text-muted);
    font-size: 0.75rem;
  }
  .attestation-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 1rem;
  }
  .attestation-actions button {
    min-height: 2rem;
    padding: 0.4rem 0.8rem;
    border-radius: 8px;
    cursor: pointer;
  }
  .attestation-deny {
    border: 1px solid var(--color-chrome-panel-border);
    background: var(--color-chrome-panel-bg);
    color: var(--color-chrome-text);
  }
  .attestation-approve {
    border: 1px solid var(--color-chrome-accent);
    background: var(--color-chrome-accent);
    color: var(--color-chrome-accent-contrast);
    font-weight: 700;
  }
  .surface-frame {
    margin-top: 0.9rem;
    display: grid;
    gap: 0.5rem;
  }
  /* Standalone mode: the frame is the workspace, edge to edge. It takes all
     remaining height and the app scrolls inside its own sandbox. */
  .surface-frame.fill {
    margin-top: 0;
    display: flex;
    flex-direction: column;
    gap: 0;
    flex: 1;
    min-height: 0;
    position: relative;
  }
  .surface-frame.fill iframe {
    flex: 1;
    min-height: 0;
    height: auto;
    max-height: none;
    border: none;
    border-radius: 0;
  }
  .surface-frame iframe.loading {
    visibility: hidden;
    min-height: 0;
    height: 0;
    border: 0;
  }
  .surface-frame.fill iframe.loading {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
  .surface-frame.fill .identity {
    padding: 0.4rem 1rem;
  }
  .surface-frame.fill .app-notice {
    margin: 0.5rem 1rem;
  }
  .identity {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.8rem;
    color: var(--color-text-muted);
  }
  .identity .name {
    font-weight: 700;
    color: var(--color-text);
  }
  .dot {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 999px;
    background: var(--color-warning-text);
  }
  .dot.ready {
    background: var(--color-success-text);
  }
  .app-notice {
    margin: 0;
    font-size: 0.82rem;
    color: var(--color-warning-text);
    background: var(--color-warning-soft);
    border: 1px solid var(--color-warning-border);
    border-radius: 10px;
    padding: 0.5rem 0.65rem;
  }
  iframe {
    width: 100%;
    display: block;
    /* Start compact (a bare <iframe> otherwise defaults to 150px, which strands
       a small control in an empty box); an inline `height` from `onResize` then
       sizes it to the frame's real content. Capped so a large or hostile
       surface scrolls internally rather than dominating the view. */
    height: 2.75rem;
    min-height: 2.75rem;
    max-height: 80vh;
    max-height: 80dvh;
    border: 1px solid var(--color-border);
    border-radius: 12px;
    background: var(--color-surface);
    color-scheme: light dark;
  }
  .surface-error {
    margin-top: 0.9rem;
    padding: 0.75rem 0.85rem;
    border: 1px solid var(--color-warning-border);
    background: var(--color-warning-soft);
    border-radius: 12px;
    color: var(--color-warning-text);
  }
  .surface-error h3 {
    margin: 0 0 0.25rem;
    font-size: 0.95rem;
  }
  .surface-error .retry {
    margin-top: 0.4rem;
    min-height: 1.75rem;
    padding: 0.2rem 0.7rem;
    border: 1px solid var(--color-warning-border);
    border-radius: 8px;
    background: var(--color-surface);
    color: var(--color-warning-text);
    cursor: pointer;
  }
</style>
