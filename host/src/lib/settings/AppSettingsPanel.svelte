<script lang="ts">
  import type { InstalledApp } from "$lib/api";
  import { standaloneSurfaces } from "$lib/apps/standaloneSurfaces";
  import JsonSchemaForm from "$lib/settings/JsonSchemaForm.svelte";
  import { supportsJsonSchemaForm } from "$lib/settings/jsonSchemaFormModel";
  import { appConfigEntry, hostConfig, saveAppConfig } from "$lib/stores/config";
  import { activeAppId, currentTab } from "$lib/stores/hostState";

  interface Props {
    app: InstalledApp;
  }

  let { app }: Props = $props();
  let entry = $derived(appConfigEntry($hostConfig, app.manifest.app_id));
  let hasStandaloneSurface = $derived(standaloneSurfaces(app.manifest).length > 0);

  function openApp() {
    activeAppId.set(app.manifest.app_id);
    currentTab.set("apps");
  }
</script>

<section class="panel">
  <div class="topline">
    <div>
      <h3>{app.manifest.display_name}</h3>
      <p>{app.manifest.description}</p>
    </div>
  </div>
  {#each app.manifest.config_declarations as declaration}
    <div class="config-block">
      <h4>{declaration.title}</h4>
      <p>{declaration.description}</p>
      {#if supportsJsonSchemaForm(declaration.json_schema)}
        <JsonSchemaForm
          schema={declaration.json_schema}
          initialValue={Object.keys(entry.settings).length === 0 && declaration.default && typeof declaration.default === "object" ? declaration.default as Record<string, never> : entry.settings}
          onSubmit={(value) => saveAppConfig(app.manifest.app_id, value)}
        />
      {:else}
        <div class="structured-settings">
          {#if hasStandaloneSurface}
            <p>Use the {app.manifest.display_name} app screen to create and edit these structured settings.</p>
            <button type="button" onclick={openApp}>Open {app.manifest.display_name}</button>
          {:else}
            <p>This app declares structured settings that Kestral's generic settings editor cannot edit.</p>
          {/if}
        </div>
      {/if}
    </div>
  {:else}
    <p class="empty">This app has no configurable settings.</p>
  {/each}
</section>

<style>
  .panel {
    border: 1px solid var(--color-border);
    border-radius: 16px;
    padding: 1rem;
    background: var(--color-surface-muted);
  }
  .topline {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    align-items: flex-start;
  }
  h3,
  h4,
  p {
    margin: 0;
  }
  .topline p,
  .config-block p {
    color: var(--color-text-muted);
  }
  .config-block {
    margin-top: 0.9rem;
  }
  .structured-settings {
    display: grid;
    gap: 0.65rem;
    margin-top: 0.75rem;
    padding: 0.85rem;
    border: 1px solid var(--color-border);
    border-radius: 12px;
    background: var(--color-surface);
  }
  .structured-settings button {
    width: fit-content;
    min-height: 2.25rem;
    border: 1px solid var(--color-border-strong);
    border-radius: 8px;
    padding: 0.4rem 0.8rem;
    background: var(--color-surface);
    color: var(--color-text);
    cursor: pointer;
  }
  .structured-settings button:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  .empty {
    margin-top: 0.9rem;
    color: var(--color-text-faint);
  }
</style>
