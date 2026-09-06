<script lang="ts">
  import { onMount } from "svelte";
  import {
    listInstalledApps,
    setAppEnabled,
    type AppStatusView,
    type InstalledApp,
  } from "$lib/api";
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
  import { openAppPermissions } from "$lib/stores/navigation";

  const developerGuideUrl = "https://manuelzierl.github.io/kestral/writing-apps.html";
  const curatedAppsUrl = "https://manuelzierl.github.io/kestral/curated-apps.html";

  let managedApps = $state<AppStatusView[]>([]);
  let loaded = $state(false);
  let busy = $state(false);
  let waitingForKernel = $state(false);
  let busyMessage = $state("Loading apps…");
  let error = $state<string | null>(null);
  let showInstaller = $state(false);
  let recoveringAppId = $state<string | null>(null);
  let retryTimer: ReturnType<typeof setTimeout> | null = null;
  let mounted = false;

  const failedApps = $derived(managedApps.filter((app) => !app.bundled && app.status === "failed"));
  const permissionBlockedApps = $derived(managedApps.filter((app) => app.status === "needs-permissions"));

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

  async function openExternal(url: string) {
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
    } catch {
      window.open(url, "_blank", "noopener,noreferrer");
    }
  }

  async function retryFailedApp(app: AppStatusView) {
    if (recoveringAppId !== null) return;
    recoveringAppId = app.id;
    busy = true;
    waitingForKernel = false;
    busyMessage = `Restarting ${app.display_name}…`;
    error = null;
    try {
      // A failed managed app is still enabled durably. Cycling its lifecycle is
      // the existing authoritative recovery path: disable tears down stale
      // runtime state, then enable performs the normal inspected activation
      // path again rather than introducing a privileged restart side door.
      await setAppEnabled(app.id, false);
      await onChanged(await setAppEnabled(app.id, true));
    } catch (failure) {
      error = `Could not restart ${app.display_name}: ${String(failure)}`;
    } finally {
      busy = false;
      recoveringAppId = null;
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
        <button class="secondary" type="button" onclick={() => void openExternal(curatedAppsUrl)}>Browse curated apps</button>
        <button class="secondary" type="button" onclick={() => void openExternal(developerGuideUrl)}>Build your own</button>
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
        <p class="error">{error}</p>
        <button type="button" onclick={() => void load()} disabled={busy}>Refresh state</button>
      </div>
    {/if}

    {#if loaded && failedApps.length > 0}
      <section class="health-card danger-card" aria-labelledby="failed-apps-title">
        <div>
          <p class="eyebrow danger-eyebrow">Needs attention</p>
          <h3 id="failed-apps-title">{failedApps.length} app{failedApps.length === 1 ? "" : "s"} failed to start</h3>
          <p>Retry uses the normal app lifecycle: Kestral tears down the failed runtime and activates the same inspected revision again.</p>
        </div>
        <div class="health-actions">
          {#each failedApps as app (app.id)}
            <button
              type="button"
              onclick={() => void retryFailedApp(app)}
              disabled={recoveringAppId !== null}
            >
              {recoveringAppId === app.id ? `Restarting ${app.display_name}…` : `Retry ${app.display_name}`}
            </button>
          {/each}
        </div>
      </section>
    {/if}

    {#if loaded && permissionBlockedApps.length > 0}
      <section class="health-card" aria-labelledby="permissions-title">
        <div>
          <p class="eyebrow">Permissions</p>
          <h3 id="permissions-title">Some apps are waiting for authority</h3>
          <p>Missing permissions keep those apps from becoming active. Review the exact app rather than broadening permissions globally.</p>
        </div>
        <div class="health-actions">
          {#each permissionBlockedApps as app (app.id)}
            <button type="button" onclick={() => openAppPermissions(app.id)}>
              Review {app.display_name}
            </button>
          {/each}
        </div>
      </section>
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
          <button type="button" onclick={() => void openExternal(curatedAppsUrl)}>Browse curated apps</button>
          <button type="button" onclick={() => void openExternal(developerGuideUrl)}>Create a focused app</button>
        </div>
        <p class="trust-note">Before installation, Kestral inspects the package without running it and shows the permissions it requests.</p>
      </section>
    {:else if loaded}
      <div class="app-list" aria-label="Installed apps">
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
  .first-app,
  .health-card {
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
  .first-app > div:first-child,
  .health-card > div:first-child {
    min-width: min(100%, 20rem);
    max-width: 48rem;
  }
  h2,
  h3,
  .intro,
  .eyebrow,
  .first-app p,
  .health-card p {
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
  .health-card p,
  .status {
    color: var(--color-text-muted);
  }
  .intro,
  .first-app > div:first-child p:last-child,
  .health-card > div:first-child p:last-child {
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
  .danger-eyebrow {
    color: var(--color-danger-text);
  }
  .header-actions,
  .first-app-actions,
  .health-actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .first-app,
  .health-card {
    display: grid;
    gap: 1rem;
  }
  .danger-card {
    border-color: var(--color-danger-border);
    background: var(--color-danger-soft);
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
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    flex-wrap: wrap;
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
    .first-app-actions,
    .health-actions {
      width: 100%;
    }
    .header-actions button,
    .first-app-actions button,
    .health-actions button {
      flex: 1 1 10rem;
    }
  }
</style>
