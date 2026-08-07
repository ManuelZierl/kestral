<script lang="ts">
  import { onMount } from "svelte";
  import { listInstalledApps, type AppStatusView, type InstalledApp } from "$lib/api";
  import { apps as installedApps, refreshApps } from "$lib/stores/apps";
  import { refreshGrants } from "$lib/stores/grants";
  import { refreshHost } from "$lib/stores/hostState";
  import AppManagerCard from "$lib/apps/AppManagerCard.svelte";
  import InstallPanel from "$lib/apps/InstallPanel.svelte";
  import EmptyState from "$lib/shell/EmptyState.svelte";
  import LoadingIndicator from "$lib/shell/LoadingIndicator.svelte";
  import SurfaceRenderer from "$lib/apps/SurfaceRenderer.svelte";
  import { missingRequestedCapabilities } from "$lib/apps/appMetadata";
  import { standaloneSurfaces } from "$lib/apps/standaloneSurfaces";
  import { grants } from "$lib/stores/grants";
  import { activeAppId } from "$lib/stores/hostState";

  let managedApps = $state<AppStatusView[]>([]);
  let loaded = $state(false);
  let busy = $state(false);
  let waitingForKernel = $state(false);
  let busyMessage = $state("Loading apps…");
  let error = $state<string | null>(null);
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

  async function onChanged(next: AppStatusView[]) {
    managedApps = next;
    busy = true;
    waitingForKernel = false;
    busyMessage = "Updating apps…";
    error = null;
    try {
      await refreshApps();
      await refreshGrants(true);
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
  <InstallPanel onInstalled={onChanged} />

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
    <EmptyState title="No apps yet" message="Install an app above to get started." />
  {:else if loaded}
    {#each managedApps as app (app.id)}
      <AppManagerCard
        {app}
        installedApp={installedFor(app.id)}
        {onChanged}
      />
    {/each}
  {/if}
</section>
{/if}

<style>
  .stack {
    margin-top: 1rem;
    display: grid;
    gap: 1rem;
  }
  .error {
    margin: 0;
    color: var(--color-danger-text);
  }
  .status {
    margin: 0;
    color: var(--color-text-muted);
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
</style>
