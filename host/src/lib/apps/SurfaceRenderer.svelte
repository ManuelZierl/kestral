<script lang="ts">
  import { getSurfaceUi, type InstalledApp, type SurfaceDeclaration, type SurfaceUiBundle } from "$lib/api";
  import GenericFormSurface from "$lib/apps/GenericFormSurface.svelte";
  import { capabilityForFormSurface } from "$lib/apps/surfaceIntents";
  import LoadingIndicator from "$lib/shell/LoadingIndicator.svelte";
  import AppSurfaceFrame from "$lib/surfaces/AppSurfaceFrame.svelte";

  interface Props {
    app: InstalledApp;
    surface: SurfaceDeclaration;
    /** Standalone mode: the surface owns the workspace and fills its height. */
    fill?: boolean;
    onOutcome: () => void;
  }

  let { app, surface, fill = false, onOutcome }: Props = $props();

  // A surface may ship a custom, sandboxed UI bundle (host-owned; served into
  // an isolated frame). If it does, it takes precedence — this is the path
  // that lets a third-party app match a bundled Svelte screen's quality.
  // Otherwise we fall back to the generic form or a degraded placeholder, so
  // built-in and bare-MCP surfaces are unchanged.
  let bundle = $state<SurfaceUiBundle | null>(null);
  let resolved = $state(false);
  let loadError = $state<string | null>(null);
  let loadAttempt = $state(0);
  const appId = $derived(app.manifest.app_id);
  const surfaceName = $derived(surface.name);
  const contentHash = $derived(app.content_hash);

  $effect(() => {
    const currentAppId = appId;
    const currentSurfaceName = surfaceName;
    contentHash;
    loadAttempt;
    let cancelled = false;
    resolved = false;
    bundle = null;
    loadError = null;
    void (async () => {
      try {
        const next = await getSurfaceUi(currentAppId, currentSurfaceName);
        if (cancelled) return;
        bundle = next;
      } catch (failure) {
        if (!cancelled) loadError = String(failure);
      } finally {
        if (!cancelled) resolved = true;
      }
    })();
    return () => {
      cancelled = true;
    };
  });

  const formCapability = $derived(capabilityForFormSurface(app, surface));
</script>

{#if bundle}
  <AppSurfaceFrame {app} {surface} {bundle} {fill} onOutcome={() => onOutcome()} />
{:else if !resolved}
  <div class="surface pending" class:fill>
    <LoadingIndicator {fill} label="Loading surface…" />
  </div>
{:else if loadError}
  <div class="surface disabled" class:fill role="alert">
    <h3>{surface.title}</h3>
    <p>Unable to load this app surface: {loadError}</p>
    <button type="button" onclick={() => (loadAttempt += 1)}>Try again</button>
  </div>
{:else if surface.kind === "form"}
  {#if formCapability}
    <div class="surface" class:fill>
      <h3>{surface.title}</h3>
      <GenericFormSurface
        appId={app.manifest.app_id}
        surface={surface.name}
        capability={formCapability}
        onOutcome={() => onOutcome()}
      />
    </div>
  {:else}
    <div class="surface disabled" class:fill>
      <h3>{surface.title}</h3>
      <p>This form cannot run because it does not declare exactly one local capability intent.</p>
    </div>
  {/if}
{:else}
  <div class="surface degraded" class:fill>
    <h3>{surface.title}</h3>
    <p>{surface.kind} surfaces are reserved here; richer renderers can slot in later.</p>
  </div>
{/if}

<style>
  .surface {
    margin-top: 0.9rem;
    padding-top: 0.9rem;
    border-top: 1px dashed var(--color-border);
  }
  /* Standalone mode: no separator against surrounding card content — the
     surface is the whole workspace. */
  .surface.fill {
    margin-top: 0;
    padding-top: 0;
    border-top: none;
    flex: 1;
    min-height: 0;
  }
  h3 {
    margin: 0 0 0.5rem;
  }
  .pending {
    min-height: 1rem;
  }
  .degraded {
    color: var(--color-text-muted);
  }
  .disabled {
    color: var(--color-warning-text);
  }
  button {
    width: fit-content;
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    background: var(--color-surface-raised);
    color: var(--color-text);
    padding: 0.45rem 0.75rem;
  }
</style>
