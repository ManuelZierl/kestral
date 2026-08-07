<script lang="ts">
  import { onMount } from "svelte";
  import {
    connectMcpServer,
    clearMcpHttpAuthSecret,
    deleteMcpServer,
    disconnectMcpServer,
    hasMcpHttpAuthSecret,
    listMcpServers,
    putMcpHttpAuthSecret,
    upsertMcpServer,
    type McpServerStatusView,
  } from "$lib/api";
  import {
    draftFromServer,
    draftToServer,
    emptyMcpServerDraft,
    transportSummary,
    type McpServerDraft,
  } from "$lib/settings/mcpServerSettingsModel";
  import { refreshHost } from "$lib/stores/hostState";
  import LoadingIndicator from "$lib/shell/LoadingIndicator.svelte";
  import SecretInput from "$lib/settings/SecretInput.svelte";

  let servers = $state<McpServerStatusView[]>([]);
  let loaded = $state(false);
  let draft = $state<McpServerDraft | null>(null);
  // The id of the server being edited, or null when the draft is a new one.
  // Servers are stored keyed by id, so an edit that changed the id would
  // insert a second entry rather than rename the original.
  let editingServerId = $state<string | null>(null);
  let busyServerId = $state<string | null>(null);
  let error = $state<string | null>(null);
  let connectionErrors = $state<Record<string, string>>({});
  let deleteConfirmId = $state<string | null>(null);

  onMount(() => void refreshServers());

  async function refreshServers() {
    try {
      servers = await listMcpServers();
      error = null;
      loaded = true;
    } catch (failure) {
      error = String(failure);
      loaded = true;
    }
  }

  function startAdd() {
    error = null;
    deleteConfirmId = null;
    editingServerId = null;
    draft = emptyMcpServerDraft();
  }

  function startEdit(server: McpServerStatusView) {
    error = null;
    deleteConfirmId = null;
    editingServerId = server.id;
    draft = draftFromServer(server);
  }

  function cancelDraft() {
    draft = null;
    editingServerId = null;
    error = null;
  }

  // Focuses the safe ("No") choice when an inline destructive-action confirm
  // appears, without the `autofocus` attribute (flagged by svelte-check's a11y
  // rule because it can be used to yank focus at page load). This action only
  // ever runs when the confirm UI is freshly mounted in direct response to the
  // user's own "Delete" click, so moving focus here is expected, not surprising.
  function focusOnMount(node: HTMLElement) {
    node.focus();
  }

  async function saveDraft() {
    if (!draft) return;
    const result = draftToServer(draft);
    if (!result.ok) {
      error = result.error;
      return;
    }
    // Adding is an insert keyed by id, so a colliding id would replace the
    // existing server — including its transport and any live connection —
    // without ever saying so. Refuse instead.
    if (editingServerId === null && servers.some((server) => server.id === result.server.id)) {
      error = `A tool server with the id “${result.server.id}” already exists. Choose a different id, or edit the existing server.`;
      return;
    }
    error = null;
    try {
      await upsertMcpServer(result.server);
      draft = null;
      editingServerId = null;
      await refreshServers();
    } catch (failure) {
      error = String(failure);
    }
  }

  async function connect(server: McpServerStatusView) {
    busyServerId = server.id;
    error = null;
    try {
      await connectMcpServer(server.id);
      const nextErrors = { ...connectionErrors };
      delete nextErrors[server.id];
      connectionErrors = nextErrors;
      await refreshHost();
    } catch (failure) {
      connectionErrors = { ...connectionErrors, [server.id]: connectionErrorMessage(failure) };
    } finally {
      busyServerId = null;
      await refreshServers();
    }
  }

  async function disconnect(server: McpServerStatusView) {
    busyServerId = server.id;
    error = null;
    try {
      await disconnectMcpServer(server.id);
      await refreshHost();
    } catch (failure) {
      error = String(failure);
    } finally {
      busyServerId = null;
      await refreshServers();
    }
  }

  async function remove(serverId: string) {
    error = null;
    try {
      await deleteMcpServer(serverId);
      const nextErrors = { ...connectionErrors };
      delete nextErrors[serverId];
      connectionErrors = nextErrors;
      deleteConfirmId = null;
      await refreshServers();
    } catch (failure) {
      error = String(failure);
    }
  }

  function connectionErrorMessage(failure: unknown): string {
    const detail = String(failure);
    return detail.includes("401")
      ? `Authentication failed. Update the stored HTTP credential, then retry. ${detail}`
      : detail;
  }
</script>

<div class="mcp">
  <p class="hint">
    Tool servers you add appear as apps after you connect them. Their tools always ask for
    your approval before running.
  </p>

  {#if !loaded}
    <LoadingIndicator fill label="Loading tool servers…" />
  {/if}
  {#each servers as server (server.id)}
    <div class="server-row">
      <div class="server-info">
        <span class="server-name">
          {server.display_name}
          {#if server.connected}
            <span class="chip connected">Connected</span>
          {/if}
        </span>
        <span class="server-transport">{transportSummary(server)}</span>
        {#if connectionErrors[server.id]}
          <span class="server-error" role="alert">{connectionErrors[server.id]}</span>
        {/if}
      </div>
      <div class="server-actions">
        {#if server.connected}
          <button
            type="button"
            disabled={busyServerId !== null}
            onclick={() => void disconnect(server)}
          >
            {busyServerId === server.id ? "Disconnecting…" : "Disconnect"}
          </button>
        {:else}
          <button
            type="button"
            disabled={busyServerId !== null}
            onclick={() => void connect(server)}
          >
            {busyServerId === server.id ? "Connecting…" : connectionErrors[server.id] ? "Retry connection" : "Connect"}
          </button>
          <button type="button" disabled={busyServerId !== null} onclick={() => startEdit(server)}>
            Edit
          </button>
          {#if deleteConfirmId === server.id}
            <span class="confirm-inline">
              Delete {server.display_name}?
              <button type="button" class="danger" onclick={() => void remove(server.id)}>Delete</button>
              <button type="button" use:focusOnMount onclick={() => (deleteConfirmId = null)}>
                Keep
              </button>
            </span>
          {:else}
            <button
              type="button"
              class="danger"
              disabled={busyServerId !== null}
              onclick={() => (deleteConfirmId = server.id)}
            >
              Delete
            </button>
          {/if}
        {/if}
      </div>
      {#if !server.connected && server.transport.kind === "streamable-http" && server.transport.authentication.kind === "static-header"}
        <div class="server-auth">
          <SecretInput
            owner="mcp-http"
            secretName={server.id}
            label={server.transport.authentication.header_name === "Authorization" && server.transport.authentication.value_prefix === "Bearer "
              ? "Bearer token"
              : `${server.transport.authentication.header_name} credential`}
            checkStored={() => hasMcpHttpAuthSecret(server.id)}
            saveStored={(value) => putMcpHttpAuthSecret(server.id, value)}
            clearStored={() => clearMcpHttpAuthSecret(server.id)}
          />
        </div>
      {/if}
    </div>
  {:else}
    {#if loaded}<p class="empty">No tool servers configured.</p>{/if}
  {/each}

  {#if draft}
    <form
      class="draft"
      onsubmit={(event) => {
        event.preventDefault();
        void saveDraft();
      }}
    >
      <div class="draft-grid">
        <label>
          Id
          <input
            bind:value={draft.id}
            placeholder="weather"
            disabled={editingServerId !== null}
            title={editingServerId !== null
              ? "A server's id identifies it and cannot be changed. Delete it and add a new one to use a different id."
              : undefined}
          />
        </label>
        <label>
          Name
          <input bind:value={draft.displayName} placeholder="Weather tools" />
        </label>
        <label>
          Transport
          <select bind:value={draft.transportKind}>
            <option value="stdio">Local command (stdio)</option>
            <option value="streamable-http">Remote endpoint (HTTP)</option>
          </select>
        </label>
        {#if draft.transportKind === "stdio"}
          <label>
            Command
            <input bind:value={draft.command} placeholder="node" />
          </label>
          <label>
            Arguments (one per line)
            <textarea bind:value={draft.args} rows="3" placeholder={'server.mjs\n--verbose'}></textarea>
          </label>
        {:else}
          <label>
            Endpoint URL
            <input bind:value={draft.url} placeholder="https://example.com/mcp" />
          </label>
          <label>
            HTTP authentication
            <select bind:value={draft.httpAuthKind}>
              <option value="none">None</option>
              <option value="bearer">Bearer token</option>
              <option value="custom-header">Custom secret header</option>
            </select>
          </label>
          {#if draft.httpAuthKind === "custom-header"}
            <label>
              Header name
              <input bind:value={draft.httpHeaderName} placeholder="X-API-Key" />
            </label>
            <label>
              Value prefix
              <input bind:value={draft.httpValuePrefix} placeholder="Optional, e.g. Token " />
            </label>
          {/if}
        {/if}
      </div>
      <div class="draft-actions">
        <button type="submit">Save server</button>
        <button type="button" onclick={cancelDraft}>Cancel</button>
      </div>
    </form>
  {:else}
    <button type="button" class="add" onclick={startAdd}>Add tool server</button>
  {/if}

  {#if error}
    <div class="error-row" role="alert">
      <p class="error">{error}</p>
      {#if !draft}
        <button type="button" onclick={() => void refreshServers()}>Retry</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .mcp {
    display: grid;
    gap: 0.75rem;
  }
  .hint {
    margin: 0;
    color: var(--color-text-muted);
    font-size: 0.88rem;
  }
  .server-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.75rem;
    flex-wrap: wrap;
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: 0.7rem 0.85rem;
    background: var(--color-surface-muted);
  }
  .server-auth {
    flex-basis: 100%;
    padding-top: 0.65rem;
    border-top: 1px solid var(--color-border-subtle);
  }
  .server-info {
    display: grid;
    gap: 0.2rem;
    min-width: 0;
  }
  .server-name {
    font-weight: 600;
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .server-transport {
    color: var(--color-text-muted);
    font-size: 0.82rem;
    overflow-wrap: anywhere;
  }
  .server-error {
    color: var(--color-danger-text);
    font-size: 0.82rem;
    overflow-wrap: anywhere;
  }
  .chip {
    border-radius: 999px;
    padding: 0.15rem 0.55rem;
    font-size: 0.72rem;
    font-weight: 700;
  }
  .chip.connected {
    background: var(--color-success-soft);
    color: var(--color-success-text);
  }
  .server-actions {
    display: flex;
    gap: 0.4rem;
    align-items: center;
    flex-wrap: wrap;
  }
  .confirm-inline {
    display: inline-flex;
    gap: 0.4rem;
    align-items: center;
    color: var(--color-warning-text);
    font-size: 0.85rem;
  }
  button {
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    background: var(--color-surface-raised);
    color: var(--color-text);
    padding: 0.4rem 0.7rem;
    font-size: 0.85rem;
  }
  button.danger {
    background: var(--color-danger-soft);
    border-color: var(--color-danger-border);
    color: var(--color-danger-text);
  }
  .add {
    width: fit-content;
  }
  .draft {
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: 0.85rem;
    display: grid;
    gap: 0.75rem;
  }
  .draft-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 14rem), 1fr));
    gap: 0.7rem;
  }
  label {
    display: grid;
    gap: 0.25rem;
    font-size: 0.85rem;
  }
  input,
  select,
  textarea {
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    padding: 0.5rem 0.65rem;
    font: inherit;
  }
  textarea {
    resize: vertical;
  }
  .draft-actions {
    display: flex;
    gap: 0.5rem;
  }
  .empty {
    margin: 0;
    color: var(--color-text-muted);
  }
  .error {
    margin: 0;
    color: var(--color-danger-text);
  }
  .error-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
</style>
