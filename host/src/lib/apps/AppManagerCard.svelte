<script lang="ts">
  import {
    applyManagedAppTransition,
    listManagedAppRevisions,
    planManagedAppTransition,
    setAppEnabled,
    uninstallApp,
    type AppStatusView,
    type ManagedAppRevisionView,
    type ManagedAppTransitionPlan,
    type InstalledApp,
  } from "$lib/api";
  import ManagedAppDiffView from "$lib/apps/ManagedAppDiffView.svelte";
  import { activeAppId } from "$lib/stores/hostState";
  import { openAppSettings } from "$lib/stores/navigation";
  import { managedAppOperationLabel } from "$lib/apps/managedAppLifecycle";
  import { standaloneSurfaces } from "$lib/apps/standaloneSurfaces";

  interface Props {
    app: AppStatusView;
    /// Full installed record, present when the app is active in the kernel.
    /// Only active apps can open a standalone screen.
    installedApp?: InstalledApp;
    onChanged: (apps: AppStatusView[]) => Promise<void>;
  }
  let { app, installedApp, onChanged }: Props = $props();

  let busy = $state(false);
  let error = $state<string | null>(null);
  let revisionError = $state<string | null>(null);
  let revisionBusy = $state(false);
  let revisions = $state<ManagedAppRevisionView[]>([]);
  let selectedRevisionId = $state<string | null>(null);
  let appliedRevisionKey = "";
  let acknowledgeRevertDataCaveat = $state(false);
  let revertPlan = $state<ManagedAppTransitionPlan | null>(null);
  let confirmingUninstall = $state(false);
  // Both destructive extras default OFF: a plain uninstall stays reversible
  // (reinstall keeps your saved keys), and deleting secrets/data is an
  // explicit opt-in because it cannot be undone.
  let purgeSecrets = $state(false);
  let purgeData = $state(false);

  const statusLabel = $derived(
    {
      active: "Active",
      disabled: "Disabled",
      failed: "Failed to start",
      "needs-permissions": "Needs permissions",
    }[app.status] ?? app.status,
  );
  const signatureLabel = $derived(
    {
      bundled: "Bundled",
      unsigned: "Unsigned",
      "valid-unknown-key": "Valid unknown key",
      trusted: "Trusted",
      invalid: "Invalid",
      revoked: "Revoked",
    }[app.signature] ?? app.signature,
  );

  function integrationStatus(contribution: AppStatusView["extension_contributions"][number]): string {
    switch (contribution.compatibility) {
      case "exact":
        return `Compatible with target contract v${contribution.target_contract_version}`;
      case "target-missing":
        return "Dormant: target app is not active";
      case "point-missing":
        return "Dormant: target does not provide this extension point";
      case "contract-mismatch":
        return `Dormant: target provides contract v${contribution.target_contract_version}`;
    }
  }

  const selectedRevision = $derived(
    selectedRevisionId ? revisions.find((revision) => revision.revision_id === selectedRevisionId) ?? null : null,
  );

  function defaultRevisionId(values: ManagedAppRevisionView[]): string | null {
    return values.length > 1
      ? values[values.length - 2]?.revision_id ?? values[values.length - 1]?.revision_id ?? null
      : values[values.length - 1]?.revision_id ?? null;
  }

  $effect(() => {
    const next = app.revisions;
    const key = next.map((revision) => revision.revision_id).join("\u0000");
    if (key === appliedRevisionKey) return;
    appliedRevisionKey = key;
    revisions = next;
    selectedRevisionId = defaultRevisionId(next);
    acknowledgeRevertDataCaveat = false;
    revertPlan = null;
  });

  async function loadRevisions() {
    revisionBusy = true;
    revisionError = null;
    try {
      revisions = await listManagedAppRevisions(app.id);
      selectedRevisionId = defaultRevisionId(revisions);
      acknowledgeRevertDataCaveat = false;
      revertPlan = null;
    } catch (failure) {
      revisionError = String(failure);
    } finally {
      revisionBusy = false;
    }
  }

  function pickRevision(revisionId: string) {
    selectedRevisionId = revisionId;
    acknowledgeRevertDataCaveat = false;
    revertPlan = null;
  }

  async function buildRevertPlan() {
    if (!selectedRevisionId || !acknowledgeRevertDataCaveat) {
      revertPlan = null;
      return;
    }
    busy = true;
    error = null;
    try {
      revertPlan = await planManagedAppTransition({
        operation: "revert",
        app_id: app.id,
        staged_id: null,
        package_digest: null,
        revision_id: selectedRevisionId,
        acknowledge_downgrade: false,
        acknowledge_revert_data_caveat: true,
      });
    } catch (failure) {
      error = String(failure);
      revertPlan = null;
    } finally {
      busy = false;
    }
  }

  async function confirmRevert() {
    if (!revertPlan) return;
    busy = true;
    error = null;
    try {
      await onChanged(await applyManagedAppTransition(revertPlan));
      selectedRevisionId = null;
      acknowledgeRevertDataCaveat = false;
      revertPlan = null;
      await loadRevisions();
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }

  async function toggleEnabled() {
    busy = true;
    error = null;
    try {
      await onChanged(await setAppEnabled(app.id, !app.enabled));
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }

  async function confirmUninstall() {
    busy = true;
    error = null;
    try {
      await onChanged(await uninstallApp(app.id, purgeSecrets, purgeData));
      confirmingUninstall = false;
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }
</script>

<article class="card" data-testid={`app-${app.id}`}>
  <div class="head">
    <h3>{app.display_name}</h3>
    <span class="version">v{app.version}</span>
    {#if app.bundled}
      <span class="tag bundled">Bundled</span>
    {/if}
    <span class={`tag status status-${app.status}`}>{statusLabel}</span>
    <span class="spacer"></span>
    <span class={`tag sig sig-${app.signature}`}>
      {signatureLabel}
    </span>
    <button
      class="app-settings"
      type="button"
      aria-label={`Settings for ${app.display_name}`}
      title={`Settings for ${app.display_name}`}
      onclick={() => openAppSettings(app.id, app.display_name)}
    >
      <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
        <path
          d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm7.5-3a7.5 7.5 0 0 0-.1-1.2l2-1.6-2-3.4-2.4 1a7.6 7.6 0 0 0-2-1.2L14.5 3h-5l-.5 2.6a7.6 7.6 0 0 0-2 1.2l-2.4-1-2 3.4 2 1.6a7.6 7.6 0 0 0 0 2.4l-2 1.6 2 3.4 2.4-1a7.6 7.6 0 0 0 2 1.2l.5 2.6h5l.5-2.6a7.6 7.6 0 0 0 2-1.2l2.4 1 2-3.4-2-1.6c.1-.4.1-.8.1-1.2Z"
          fill="none"
          stroke="currentColor"
          stroke-width="1.7"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
    </button>
  </div>

  <p class="desc">{app.description}</p>
  <p class="meta">
    {app.backend_kind}{app.publisher ? ` · ${app.publisher}` : ""}{app.installed_at
      ? ` · installed ${new Date(app.installed_at).toLocaleDateString()}`
      : ""}
  </p>

  {#if app.status === "needs-permissions"}
    <p class="notice warn">
      {app.missing_permissions} permission{app.missing_permissions === 1 ? "" : "s"} not granted. Manage in Settings → Permissions.
    </p>
  {/if}
  {#if app.status === "failed" && app.status_detail}
    <p class="notice danger">Startup failed: {app.status_detail}</p>
  {/if}

  {#if app.signature === "valid-unknown-key"}
    <p class="notice warn">This app verifies, but its publisher key is not trusted yet.</p>
  {:else if app.signature === "revoked"}
    <p class="notice danger">This app's publisher key is revoked for this scope.</p>
  {/if}

  {#if app.extension_contributions.length > 0}
    <section class="integrations" aria-label="App integrations">
      <h4>App integrations</h4>
      <ul>
        {#each app.extension_contributions as contribution}
          <li class:dormant={contribution.compatibility !== "exact"}>
            <strong>{contribution.target_app} / {contribution.extension_point}</strong>
            <span>Requires contract v{contribution.contract_version}</span>
            <span>{integrationStatus(contribution)}</span>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  {#if !app.bundled}
  <section class="revisions" aria-label="Retained revisions">
    <div class="revisions-head">
      <div>
        <h4>Retained revisions</h4>
        <p>Use one of these to revert the app version. Incompatible data requires a declared reverse migration.</p>
      </div>
      <button type="button" onclick={() => void loadRevisions()} disabled={revisionBusy}>Refresh revisions</button>
    </div>

    {#if revisionBusy && revisions.length === 0}
      <p class="muted" role="status">Loading retained revisions…</p>
    {/if}

    {#if revisionError}
      <p class="notice danger" role="alert">Could not load revisions: {revisionError}</p>
    {/if}

    {#if revisions.length === 0}
      <p class="muted">No retained revisions are available yet.</p>
    {:else}
      <div class="revision-grid">
        {#each revisions as revision (revision.revision_id)}
          <article class:selected={revision.revision_id === selectedRevisionId} class="revision-card">
            <div class="revision-head">
              <div>
                <h5>v{revision.version}</h5>
                <p><code>{revision.revision_id}</code></p>
              </div>
              <span class="tag">{new Date(revision.installed_at).toLocaleDateString()}</span>
            </div>
            <p class="revision-meta">{revision.backend_kind}{revision.publisher ? ` · ${revision.publisher}` : ""} · {revision.signature_verdict}</p>
            <p class="revision-desc">{revision.description}</p>
            <div class="actions">
              <button type="button" onclick={() => pickRevision(revision.revision_id)} disabled={busy}>Revert app version</button>
            </div>
          </article>
        {/each}
      </div>
    {/if}

    {#if selectedRevision}
      <div class="revert-review" role="group" aria-label="Revert review">
        <label class="ack">
          <input
            type="checkbox"
            bind:checked={acknowledgeRevertDataCaveat}
            onchange={() => void buildRevertPlan()}
            disabled={busy}
          />
          I understand Kestral will preserve compatible data or require a declared reverse migration.
        </label>

        {#if revertPlan}
          <ManagedAppDiffView plan={revertPlan} />
          <div class="actions">
            <button type="button" class="danger" onclick={() => void confirmRevert()} disabled={busy}>
              Revert app version
            </button>
          </div>
        {:else if acknowledgeRevertDataCaveat}
          <p class="notice warn" role="status">Review loaded. Revert is ready to commit after you inspect the diff.</p>
        {/if}
      </div>
    {/if}
  </section>
  {/if}

  <div class="actions">
    {#if installedApp && standaloneSurfaces(installedApp.manifest).length > 0 && app.status !== "disabled"}
      <button onclick={() => activeAppId.set(app.id)} disabled={busy}>Open</button>
    {/if}
    {#if !app.bundled}
      <button onclick={toggleEnabled} disabled={busy}>
        {app.enabled ? "Disable" : "Enable"}
      </button>
    {/if}
    {#if app.removable}
      <button
        class="danger"
        onclick={() => {
          // Always open the confirmation on the safe default: a stale checked
          // state from an earlier cancelled attempt must never carry into a
          // fresh uninstall review.
          purgeSecrets = false;
          purgeData = false;
          confirmingUninstall = true;
        }}
        disabled={busy}
      >
        Uninstall
      </button>
    {/if}
    {#if app.bundled}
      <span class="bundled-note">Bundled app — managed by the host.</span>
    {/if}
  </div>

  {#if error}
    <p class="notice danger" role="alert">{error}</p>
  {/if}

  {#if confirmingUninstall}
    <div class="uninstall" role="group" aria-label="Uninstall options">
      <p>Uninstall <strong>{app.display_name}</strong>? Its grants, surfaces, and running work are removed. Your saved keys stay unless you choose to delete them below.</p>
      <label><input type="checkbox" bind:checked={purgeSecrets} /> Also delete stored secrets (can't be undone)</label>
      <label><input type="checkbox" bind:checked={purgeData} /> Also delete app data / settings (can't be undone)</label>
      <div class="actions">
        <button class="danger" onclick={confirmUninstall} disabled={busy}>
          {busy
            ? "Removing…"
            : purgeSecrets && purgeData
              ? "Uninstall and delete secrets + data"
              : purgeSecrets
                ? "Uninstall and delete secrets"
                : purgeData
                  ? "Uninstall and delete app data"
                  : "Confirm uninstall"}
        </button>
        <button
          onclick={() => {
            confirmingUninstall = false;
            purgeSecrets = false;
            purgeData = false;
          }}
          disabled={busy}
        >
          Cancel
        </button>
      </div>
    </div>
  {/if}

</article>

<style>
  .card {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 16px;
    padding: 1rem 1.1rem;
    box-shadow: 0 10px 26px var(--color-shadow-soft);
    display: grid;
    gap: 0.5rem;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  h3 {
    margin: 0;
    font-size: 1.05rem;
  }
  .version {
    color: var(--color-text-muted);
    font-size: 0.82rem;
  }
  .spacer {
    flex: 1;
  }
  .tag {
    border-radius: 999px;
    padding: 0.12rem 0.55rem;
    font-size: 0.74rem;
    font-weight: 700;
  }
  .bundled {
    background: var(--color-chip-purple-soft);
    color: var(--color-text);
  }
  .status-active {
    background: var(--color-success-soft);
    color: var(--color-success-text);
  }
  .status-disabled {
    background: var(--color-border);
    color: var(--color-text-muted);
  }
  .status-failed {
    background: var(--color-warning-soft);
    color: var(--color-danger-text);
  }
  .status-needs-permissions {
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .sig-valid-unknown-key,
  .sig-unsigned {
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .sig-trusted {
    background: var(--color-success-soft);
    color: var(--color-success-text);
  }
  .sig-invalid,
  .sig-revoked {
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }
  .desc {
    margin: 0;
  }
  .meta {
    margin: 0;
    color: var(--color-text-muted);
    font-size: 0.8rem;
  }
  .notice {
    margin: 0;
    font-size: 0.82rem;
    padding: 0.5rem 0.65rem;
    border-radius: 8px;
  }
  .notice.warn {
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .notice.danger {
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }
  .muted {
    margin: 0;
    color: var(--color-text-muted);
    font-size: 0.82rem;
  }
  .integrations {
    display: grid;
    gap: 0.4rem;
    padding: 0.7rem 0;
    border-top: 1px solid var(--color-border-subtle);
  }
  .integrations h4 {
    margin: 0;
    font-size: 0.9rem;
  }
  .integrations ul {
    display: grid;
    gap: 0.4rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .integrations li {
    display: grid;
    gap: 0.1rem;
    padding: 0.55rem 0.65rem;
    border: 1px solid var(--color-border-subtle);
    border-radius: 10px;
    background: var(--color-success-soft);
    color: var(--color-success-text);
    font-size: 0.82rem;
    overflow-wrap: anywhere;
  }
  .integrations li.dormant {
    border-color: var(--color-warning-border);
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .revisions {
    display: grid;
    gap: 0.65rem;
    padding: 0.8rem 0;
    border-top: 1px solid var(--color-border-subtle);
  }
  .revisions-head {
    display: flex;
    align-items: start;
    justify-content: space-between;
    gap: 0.75rem;
    flex-wrap: wrap;
  }
  .revisions-head p,
  .revision-desc,
  .revision-meta {
    margin: 0;
    color: var(--color-text-muted);
    font-size: 0.82rem;
    overflow-wrap: anywhere;
  }
  .revision-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 18rem), 1fr));
    gap: 0.75rem;
  }
  .revision-card {
    display: grid;
    gap: 0.5rem;
    border: 1px solid var(--color-border-subtle);
    border-radius: 12px;
    background: var(--color-surface-muted);
    padding: 0.8rem;
    min-width: 0;
  }
  .revision-card.selected {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 1px var(--color-accent-soft);
  }
  .revision-head {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    flex-wrap: wrap;
    align-items: start;
  }
  .revision-head h5 {
    margin: 0;
    font-size: 0.95rem;
  }
  .revision-head p {
    margin: 0;
    color: var(--color-text-muted);
    font-size: 0.8rem;
    overflow-wrap: anywhere;
  }
  .ack {
    display: flex;
    gap: 0.5rem;
    align-items: start;
    color: var(--color-text);
  }
  .revert-review {
    display: grid;
    gap: 0.75rem;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    align-items: center;
  }
  button {
    border: 1px solid var(--color-border-strong);
    border-radius: 8px;
    background: var(--color-surface);
    color: var(--color-text);
    padding: 0.4rem 0.8rem;
    cursor: pointer;
    font-size: 0.85rem;
  }
  button.app-settings {
    width: 2.25rem;
    height: 2.25rem;
    flex: 0 0 auto;
    display: grid;
    place-items: center;
    padding: 0;
    color: var(--color-text-muted);
  }
  button.app-settings:hover {
    color: var(--color-text);
  }
  button.app-settings:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  button.danger {
    border-color: var(--color-warning-border);
    color: var(--color-danger-text);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .bundled-note {
    color: var(--color-text-muted);
    font-size: 0.8rem;
  }
  .uninstall {
    border: 1px solid var(--color-warning-border);
    background: var(--color-warning-soft);
    border-radius: 10px;
    padding: 0.7rem 0.8rem;
    display: grid;
    gap: 0.4rem;
    font-size: 0.85rem;
  }
  .uninstall p {
    margin: 0;
  }
  .uninstall label {
    display: flex;
    gap: 0.4rem;
    align-items: center;
  }
</style>
