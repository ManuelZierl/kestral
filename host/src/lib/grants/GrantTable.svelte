<script lang="ts">
  // Read-only audit view: every grant fact the kernel ever issued, newest
  // first. Managing permissions (edit/revoke/grant again) happens in the
  // grouped list in Settings → Permissions; this table exists so the raw
  // immutable record stays inspectable.
  import { grants, grantsLoaded } from "$lib/stores/grants";
  import { apps } from "$lib/stores/apps";
  import { dataScopeLabel } from "$lib/system/dataScopeLabel";
  import { scopeLabel } from "$lib/system/scopeLabel";
  import LoadingIndicator from "$lib/shell/LoadingIndicator.svelte";
  import type { GrantScope, GrantStatus } from "$lib/api";

  function providerLabel(scope: GrantScope): string {
    const provider = scope.provider;
    const app = $apps.find((candidate) => candidate.manifest.app_id === provider);
    return app ? `${app.manifest.display_name} (${provider})` : provider;
  }

  function statusClass(status: GrantStatus): string {
    return `status-${status}`;
  }
</script>

{#if !$grantsLoaded}
  <LoadingIndicator fill label="Loading permissions…" />
{:else}
<div class="table-scroll">
<table>
  <thead>
    <tr>
      <th>Issued</th>
      <th>App</th>
      <th>Allowed action</th>
      <th>Data scope</th>
      <th>Approval</th>
      <th>Source</th>
      <th>Status</th>
    </tr>
  </thead>
  <tbody>
    {#each $grants as grant (grant.grant_id)}
      <tr>
        <td>{new Date(grant.issued_at).toLocaleString()}</td>
        <td>
          <strong>{grant.holder_display_name}</strong><br />
          <code>{grant.holder}</code>
        </td>
        <td>
          <strong>{providerLabel(grant.scope)}</strong><br />
          <code>{scopeLabel(grant.scope)}</code>
        </td>
        <td>{dataScopeLabel(grant.data_scope)}</td>
        <td>{grant.condition}</td>
        <td>{grant.origin.replaceAll("-", " ")}</td>
        <td>
          <span class={`status ${statusClass(grant.status)}`}>{grant.status}</span>
        </td>
      </tr>
    {:else}
      <tr>
        <td colspan="7" class="empty">No permissions have been issued.</td>
      </tr>
    {/each}
  </tbody>
</table>
</div>
{/if}

<style>
  /* Wide tables scroll inside their own container; the page never
     scrolls horizontally. */
  .table-scroll {
    overflow-x: auto;
  }
  table {
    width: 100%;
    min-width: 40rem;
    border-collapse: collapse;
  }
  th,
  td {
    padding: 0.55rem 0.45rem;
    border-bottom: 1px solid var(--color-border-subtle);
    text-align: left;
    vertical-align: top;
  }
  code {
    color: var(--color-text-muted);
  }
  .status {
    display: inline-flex;
    border-radius: 999px;
    padding: 0.2rem 0.55rem;
    font-size: 0.78rem;
    font-weight: 700;
    text-transform: uppercase;
  }
  .status-active {
    background: var(--color-success-soft);
    color: var(--color-success-text);
  }
  .status-revoked,
  .status-expired {
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .empty {
    color: var(--color-text-muted);
  }
</style>
