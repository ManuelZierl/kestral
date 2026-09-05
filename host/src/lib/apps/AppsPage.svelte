<script lang="ts">
  import { onMount } from "svelte";
  import { listInstalledApps, type AppStatusView, type InstalledApp } from "$lib/api";
  import { apps as installedApps, refreshApps } from "$lib/stores/apps";
  import { refreshGrants } from "$lib/stores/grants";
  import { refreshHost } from "$lib/stores/hostState";
  import AppManagerCard from "$lib/apps/AppManagerCard.svelte";
  import InstallPanel from "$lib/apps/InstallPanel.svelte";
  import LoadingIndicator from "$lib/shell/LoadingIndicator.svelte";
  import SurfaceRenderer from "$lib/apps/SurfaceRenderer.svelte";
  import { missingRequestedCapabilities } from "$lib/apps/appMetadata";
  import { standaloneSurfaces } from "$lib/apps/standaloneSurfaces";
  import { grants } from "$lib/stores/grants";
  import { activeAppId } from "$lib/stores/hostState";

  const developerGuideUrl = "https://github.com/ManuelZierl/kestral/blob/develop/docs/writing-apps.md";

  let managedApps = $state<AppStatusView[]>([]);
  let loaded = $state(false);
  let busy = $state(false);
  let waitingForKernel = $state(false);
  let busyMessage = $state("Loading apps…");
  let error = $state<string | null>(null);
  let showInstaller = $state(false);
  let retryTimer: ReturnType<typeof setTimeout> | null = null;
  let mounted = false;

  function scheduleBusyRetry() {
    if (!mounted || retryTimer !== null) return;
    retryTimer = setTimeout(() => {
      retryTimer = null;
      void load();
    }, 1000);
  }

  async function load() {
    if (busy) {
      scheduleBusyRetry();
      return;
    }
    const keepVisible = loaded;
    busy = true;
    waitingForKernel = false;
    busyMessage = keepVisible ? "Refreshing apps…" : "Loading apps…";
    if (!keepVisible) {
      loaded = false;
    }
    error = null;
    try {
      const statuses = await listInstalledApps();
      await refreshApps();
      managedApps = statuses;
      loaded = true;
    } catch (failure) {
      const message = String(failure);
      if (message.includes("kernel busy")) {
        error = null;
        waitingForKernel = true;
        busyMessage = "Waiting for the host…";
        scheduleBusyRetry();
      } else {
        error = message;
      }
      if (!keepVisible) {
        loaded = false;
      }
    } finally {
      busy = false;
    }
  }

  function installedFor(id: string): InstalledApp | undefined {
    return $installedApps.find((record) => record.manifest.app_id === id);
  }

  async function openDeveloperGuide() {
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(developerGuideUrl);
    } catch {
      window.open(developerGuideUrl, "_blank", "noopener,noreferrer");
    }
  }

  async function onChanged(next: AppStatusView[]) {
    managedApps = next;
    busy = true;
    waitingForKernel = false;
    busyMessage = "Updating apps…";
    error = null;
    try {
      await refreshApps();
      await refreshGrants(true);
      showInstaller = false;
    } catch (failure) {
      const message = String(failure);
      if (message.includes("kernel busy")) {
        waitingForKernel = true;
        busyMessage = "Waiting for the host…";
        scheduleBusyRetry();
      } else {
        error = message;
      }
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    mounted = true;
    void load();
    return () => {
      mounted = false;
      if (retryTimer !== null) clearTimeout(retryTimer);
    };
  });

  // The sidebar can only select an app already present in this authoritative
  // shared store. Keep that app mounted while the separate management view is
  // loading or the kernel is briefly busy; hide it only for a real load error.
  const activeApp = $derived(
    error === null
      ? $installedApps.find((app) => app.manifest.app_id === $activeAppId)
      : undefined,
  );
  const activeSurfaces = $derived(activeApp ? standaloneSurfaces(activeApp.manifest) : []);
  const missing = $derived(activeApp ? missingRequestedCapabilities(activeApp.manifest, $grants) : []);
</script>

{#if $activeAppId && activeApp && activeSurfaces.length > 0}
  <!-- Standalone app view: the app owns the workspace. Host chrome around it
       stays minimal — identity lives in the top bar, status in the status
       bar; management actions live on the Apps tab. -->
  <section class="app-screen">
    {#if busy || waitingForKernel}
      <p class="status" role="status">{busyMessage}</p>
    {/if}
    {#if error}
      <p class="load-error" role="alert">Could not load apps: {error}</p>
    {/if}
    {#if missing.length > 0}
      <p class="permissions-warning" role="status">
        {missing.length} permission{missing.length === 1 ? "" : "s"} needed — review this app under Apps.
      </p>
    {/if}
    {#each activeSurfaces as surface (surface.name)}
      <SurfaceRenderer app={activeApp} {surface} fill onOutcome={refreshHost} />
    {/each}
  </section>
{:else}
  <section class="stack">
    <header class="page-header">
      <div>
        <p class="eyebrow">Your workspace</p>
        <h2>Apps</h2>
        <p class="intro">Install focused AI apps for the work you actually do. Each app keeps its own interface and only gets the permissions you approve.</p>
      </div>
      <div class="header-actions">
        <button type="button" onclick={() => { showInstaller = !showInstaller; }} aria-expanded={showInstaller}>
          {showInstaller ? "Close installer" : "Add app"}
        </button>
        <button class="secondary" type="button" onclick={() => void openDeveloperGuide()}>Build your own</button>
      </div>
    </header>

    {#if showInstaller}
      <InstallPanel onInstalled={onChanged} />
    {/if}

    {#if (busy || waitingForKernel) && !loaded}
      <LoadingIndicator fill label={busyMessage} />
    {/if}

    {#if error}
      <div class="load-error" role="alert">
        <p class="error">Could not load apps: {error}</p>
        <button type="button" onclick={() => void load()} disabled={busy}>Retry</button>
      </div>
    {/if}

    {#if loaded && managedApps.length === 0}
      <section class="first-app" aria-labelledby="first-app-title">
        <div>
          <p class="eyebrow">Start here</p>
          <h3 id="first-app-title">Make Kestral useful for one real job</h3>
          <p>
            Add an existing app, or scaffold a small app around a workflow you repeat. Kestral is designed for purpose-built screens and actions, not for forcing every task through chat.
          </p>
        </div>
        <div class="first-app-actions">
          <button class="primary" type="button" onclick={() => { showInstaller = true; }}>Install an app</button>
          <button type="button" onclick={() => void openDeveloperGuide()}>Create a focused app</button>
        </div>
        <p class="trust-note">Before installation, Kestral inspects the package without running it and shows the permissions it requests.</p>
      </section>
    {:else if loaded}
      <div class="app-list" aria-label="Installed third-party apps">
        {#each managedApps as app (app.id)}
          <AppManagerCard
            {app}
            installedApp={installedFor(app.id)}
            {onChanged}
          />
        {/each}
      </div>
    {/if}
  </section>
{/if}

<style>
  .stack {
    margin-top: 1rem;
    display: grid;
    gap: 1rem;
  }
  .page-header,
  .first-app {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 16px;
    padding: 1.1rem;
  }
  .page-header {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    align-items: flex-start;
    flex-wrap: wrap;
  }
  .page-header > div:first-child,
  .first-app > div:first-child {
    min-width: min(100%, 20rem);
    max-width: 48rem;
  }
  h2,
  h3,
  .intro,
  .eyebrow,
  .first-app p {
    margin: 0;
  }
  h2 {
    font-size: 1.35rem;
  }
  h3 {
    margin-top: 0.15rem;
    font-size: 1.05rem;
  }
  .intro,
  .first-app p,
  .status {
    color: var(--color-text-muted);
  }
  .intro,
  .first-app > div:first-child p:last-child {
    margin-top: 0.4rem;
    line-height: 1.5;
  }
  .eyebrow {
    color: var(--color-accent);
    font-size: 0.75rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .header-actions,
  .first-app-actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .first-app {
    display: grid;
    gap: 1rem;
  }
  .trust-note {
    padding-top: 0.8rem;
    border-top: 1px solid var(--color-border);
    font-size: 0.82rem;
  }
  .app-list {
    display: grid;
    gap: 1rem;
  }
  button {
    border: 1px solid var(--color-border-strong);
    border-radius: 8px;
    background: var(--color-surface);
    color: var(--color-text);
    padding: 0.55rem 0.9rem;
    cursor: pointer;
  }
  button.primary {
    border-color: transparent;
    background: var(--color-accent);
    color: var(--color-accent-contrast);
  }
  button.secondary {
    color: var(--color-accent);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .error {
    margin: 0;
    color: var(--color-danger-text);
  }
  .status {
    margin: 0;
  }
  .load-error {
    margin: 0;
    color: var(--color-danger-text);
  }
  .app-screen {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .permissions-warning {
    margin: 0.75rem 1rem;
    padding: 0.5rem 0.65rem;
    border: 1px solid var(--color-warning-border);
    border-radius: 10px;
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
    font-size: 0.85rem;
  }

  @media (max-width: 40em) {
    .header-actions,
    .first-app-actions {
      width: 100%;
    }
    .header-actions button,
    .first-app-actions button {
      flex: 1 1 10rem;
    }
  }
</style>
