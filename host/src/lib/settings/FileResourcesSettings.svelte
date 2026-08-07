<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { isRemoteTransport } from "$lib/hostTransport";
  import { apps } from "$lib/stores/apps";
  import { grants, refreshGrants } from "$lib/stores/grants";
  import {
    grantFileResourceAccessAndRefresh,
    fileResources,
    fileResourcesLoaded,
    registerFileResourceAndRefresh,
    removeFileResourceAndRefresh,
    refreshFileResources,
  } from "$lib/stores/fileResources";
  import type { FileResourceGrantOperation, GrantScope, TrustedFileResourceView } from "$lib/api";

  const FILE_BROKER_APP_ID = "com.ma-zierl.host.file-broker";
  const OPERATION_LABELS: Record<FileResourceGrantOperation, string> = {
    "list": "List",
    "read": "Read",
    "create-or-replace": "Create or replace",
    "delete": "Delete",
  };
  const CAPABILITY_LABELS: Record<string, string> = {
    "file.list": "List",
    "file.read": "Read",
    "file.create-or-replace": "Create or replace",
    "file.delete": "Delete",
  };

  let error = $state<string | null>(null);
  let busyResourceId = $state<string | null>(null);
  let deleteConfirmId = $state<string | null>(null);
  let selectedHolders = $state<Record<string, string>>({});
  let hostResourcePath = $state("");
  let initialLoadFailed = $state(false);

  onMount(() => void loadResources());

  async function loadResources() {
    error = null;
    initialLoadFailed = false;
    try {
      await refreshFileResources();
    } catch (failure) {
      initialLoadFailed = true;
      error = String(failure);
    }
  }

  function resourceGrants(resource: TrustedFileResourceView) {
    return $grants.filter(
      (grant) =>
        grant.scope.kind === "exact-capability"
        && grant.scope.provider === FILE_BROKER_APP_ID
        && grant.data_scope.kind === "resources"
        && grant.data_scope.resource_ids.includes(resource.resource_id),
    );
  }

  function holderName(holder: string) {
    return $apps.find((app) => app.manifest.app_id === holder)?.manifest.display_name ?? holder;
  }

  function grantOperationLabel(scope: GrantScope) {
    if (scope.kind !== "exact-capability" || scope.provider !== FILE_BROKER_APP_ID) {
      return scope.kind;
    }
    return CAPABILITY_LABELS[scope.capability] ?? scope.capability;
  }

  async function pickResourcePath(directory: boolean) {
    error = null;
    try {
      const selected = await open({
        directory,
        multiple: false,
        title: directory ? "Choose a folder to broker" : "Choose a file to broker",
      });
      if (typeof selected !== "string" || selected.trim().length === 0) return;
      await registerResourcePath(selected);
    } catch (failure) {
      error = String(failure);
    }
  }

  async function registerResourcePath(path: string) {
    const trimmed = path.trim();
    if (!trimmed || busyResourceId !== null) return;
    error = null;
    busyResourceId = "registering";
    try {
      await registerFileResourceAndRefresh(trimmed);
      await refreshGrants(true);
      hostResourcePath = "";
    } catch (failure) {
      error = String(failure);
    } finally {
      busyResourceId = null;
    }
  }

  async function removeResource(resourceId: string) {
    error = null;
    busyResourceId = resourceId;
    try {
      await removeFileResourceAndRefresh(resourceId);
      deleteConfirmId = null;
      await refreshGrants(true);
    } catch (failure) {
      error = String(failure);
    } finally {
      busyResourceId = null;
    }
  }

  async function grantAccess(resourceId: string, operation: FileResourceGrantOperation) {
    const holder = selectedHolders[resourceId] ?? $apps[0]?.manifest.app_id;
    if (!holder) {
      error = "Install an app before granting access.";
      return;
    }
    error = null;
    busyResourceId = resourceId;
    try {
      await grantFileResourceAccessAndRefresh(holder, resourceId, [operation]);
      await refreshGrants(true);
    } catch (failure) {
      error = String(failure);
    } finally {
      busyResourceId = null;
    }
  }
</script>

<div class="file-resources">
  <p class="hint">
    Register a file or folder here. Apps only see safe resource views; the canonical path stays in this trusted settings screen.
  </p>

  {#if isRemoteTransport()}
    <form class="host-path" onsubmit={(event) => { event.preventDefault(); void registerResourcePath(hostResourcePath); }}>
      <label for="host-resource-path">File or folder path on the Kestral host</label>
      <div class="toolbar">
        <input
          id="host-resource-path"
          bind:value={hostResourcePath}
          placeholder="/home/user/Documents"
          autocomplete="off"
          disabled={busyResourceId === "registering"}
          required
        />
        <button type="submit" disabled={busyResourceId === "registering"}>Register host path</button>
      </div>
    </form>
  {:else}
    <div class="toolbar">
      <button type="button" onclick={() => void pickResourcePath(false)}>Add file</button>
      <button type="button" onclick={() => void pickResourcePath(true)}>Add folder</button>
    </div>
  {/if}

  {#if !fileResourcesLoaded && !initialLoadFailed}
    <p class="loading" role="status">Loading file resources…</p>
  {/if}

  <div class="resource-grid">
    {#each $fileResources as resource (resource.resource_id)}
      <article class="resource-card" aria-labelledby={`resource-${resource.resource_id}`}>
        <header class="resource-header">
          <div>
            <h3 id={`resource-${resource.resource_id}`}>{resource.display_name}</h3>
            <p>
              {resource.kind}
              <span class:removing={resource.status === "removing"} class="status">
                {resource.status}
              </span>
            </p>
          </div>
          <div class="resource-actions">
            {#if deleteConfirmId === resource.resource_id}
              <span class="confirm-inline">
                Remove this resource?
                <button type="button" class="danger" onclick={() => void removeResource(resource.resource_id)}>
                  Yes
                </button>
                <button type="button" onclick={() => (deleteConfirmId = null)}>No</button>
              </span>
            {:else}
              <button
                type="button"
                class="danger"
                disabled={busyResourceId === resource.resource_id}
                onclick={() => (deleteConfirmId = resource.resource_id)}
              >
                Remove
              </button>
            {/if}
          </div>
        </header>

        <details class="trusted-path">
          <summary>Trusted path</summary>
          <p>{resource.canonical_path}</p>
        </details>

        <div class="grants">
          <div class="grant-heading">
            <strong>App access</strong>
            <span>{resourceGrants(resource).length} grant(s)</span>
          </div>
          {#each resourceGrants(resource) as grant (grant.grant_id)}
            <div class="grant-row">
              <span>{holderName(grant.holder)}</span>
              <span>{grantOperationLabel(grant.scope)}</span>
            </div>
          {:else}
            <p class="empty">No app has access yet.</p>
          {/each}
        </div>

        <div class="grant-actions">
          <label class="grant-label" for={`holder-${resource.resource_id}`}>
            Grant access to an app
          </label>
          <select
            id={`holder-${resource.resource_id}`}
            value={selectedHolders[resource.resource_id] ?? $apps[0]?.manifest.app_id ?? ""}
            onchange={(event) =>
              (selectedHolders = {
                ...selectedHolders,
                [resource.resource_id]: (event.currentTarget as HTMLSelectElement).value,
              })}
          >
            {#each $apps as app (app.manifest.app_id)}
              <option value={app.manifest.app_id}>{app.manifest.display_name}</option>
            {/each}
          </select>
          <div class="grant-buttons">
            {#each Object.entries(OPERATION_LABELS) as [operation, label]}
              <button
                type="button"
                disabled={$apps.length === 0 || busyResourceId === resource.resource_id}
                onclick={() => void grantAccess(resource.resource_id, operation as FileResourceGrantOperation)}
              >
                {label}
              </button>
            {/each}
          </div>
        </div>
      </article>
    {:else}
      {#if fileResourcesLoaded && !initialLoadFailed}
        <p class="empty">No file resources registered yet.</p>
      {/if}
    {/each}
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
    {#if initialLoadFailed}
      <button type="button" onclick={() => void loadResources()}>Retry file resources</button>
    {/if}
  {/if}
</div>

<style>
  .file-resources {
    display: grid;
    gap: 0.9rem;
  }
  .hint {
    margin: 0;
    color: var(--color-text-muted);
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 0.6rem;
  }
  .host-path {
    display: grid;
    gap: 0.4rem;
  }
  .host-path label {
    font-weight: 600;
  }
  .host-path input {
    flex: 1 1 18rem;
    min-width: min(100%, 18rem);
    min-height: 2.5rem;
    padding: 0.55em 0.7em;
    border: 1px solid var(--color-border);
    border-radius: 0.4rem;
    color: var(--color-text);
    background: var(--color-surface-raised);
    font: inherit;
  }
  .toolbar button,
  .grant-buttons button,
  .resource-actions button,
  .confirm-inline button {
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    background: var(--color-surface-raised);
    color: var(--color-text);
    padding: 0.55rem 0.8rem;
  }
  .danger {
    border-color: var(--color-danger-border);
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }
  .loading,
  .empty {
    margin: 0;
    color: var(--color-text-muted);
  }
  .resource-grid {
    display: grid;
    gap: 0.9rem;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 18rem), 1fr));
  }
  .resource-card {
    min-width: 0;
    border: 1px solid var(--color-border);
    border-radius: 16px;
    padding: 0.95rem;
    background: var(--color-surface);
    display: grid;
    gap: 0.85rem;
  }
  .resource-header {
    display: flex;
    justify-content: space-between;
    gap: 0.75rem;
    flex-wrap: wrap;
  }
  .resource-header h3 {
    margin: 0;
    font-size: 1rem;
  }
  .resource-header p {
    margin: 0.15rem 0 0;
    color: var(--color-text-muted);
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    align-items: center;
  }
  .status {
    border-radius: 999px;
    padding: 0.1rem 0.5rem;
    background: var(--color-success-soft);
    color: var(--color-success-text);
    font-size: 0.72rem;
    font-weight: 700;
  }
  .status.removing {
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .resource-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .trusted-path {
    border: 1px solid var(--color-border-subtle);
    border-radius: 12px;
    padding: 0.6rem 0.7rem;
  }
  .trusted-path summary {
    cursor: pointer;
    color: var(--color-text-muted);
  }
  .trusted-path p {
    margin: 0.5rem 0 0;
    overflow-wrap: anywhere;
    color: var(--color-text);
  }
  .grants {
    display: grid;
    gap: 0.45rem;
  }
  .grant-heading {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    color: var(--color-text-muted);
    flex-wrap: wrap;
  }
  .grant-row {
    display: flex;
    justify-content: space-between;
    gap: 0.75rem;
    flex-wrap: wrap;
    border: 1px solid var(--color-border-subtle);
    border-radius: 10px;
    padding: 0.45rem 0.6rem;
  }
  .grant-row span:last-child {
    color: var(--color-text-muted);
  }
  .grant-actions {
    display: grid;
    gap: 0.5rem;
  }
  .grant-label {
    color: var(--color-text-muted);
    font-size: 0.86rem;
  }
  select {
    min-width: 0;
    max-width: 100%;
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    padding: 0.55rem 0.7rem;
    background: var(--color-surface);
    color: var(--color-text);
  }
  .grant-buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 0.45rem;
  }
  .confirm-inline {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    flex-wrap: wrap;
  }
  .error {
    margin: 0;
    color: var(--color-danger-text);
  }
</style>
