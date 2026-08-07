<script lang="ts">
  import {
    applyManagedAppTransition,
    inspectGitPackage,
    inspectPackage,
    planManagedAppTransition,
    trustPublisherKey,
    type AppStatusView,
    type ManagedAppOperation,
    type ManagedAppTransitionPlan,
    type PackageInspection,
  } from "$lib/api";
  import { isRemoteTransport } from "$lib/hostTransport";
  import ManagedAppDiffView from "$lib/apps/ManagedAppDiffView.svelte";
  import PackageInspectionView from "$lib/apps/PackageInspectionView.svelte";
  import {
    managedAppOperationLabel,
    plannedOperationForPackage,
  } from "$lib/apps/managedAppLifecycle";
  import { apps as installedApps } from "$lib/stores/apps";

  interface Props {
    onInstalled: (apps: AppStatusView[]) => Promise<void>;
  }

  let { onInstalled }: Props = $props();

  let path = $state("");
  let source = $state<"folder" | "git">("folder");
  let inspection = $state<PackageInspection | null>(null);
  let plan = $state<ManagedAppTransitionPlan | null>(null);
  let reviewOperation = $state<ManagedAppOperation | null>(null);
  let busy = $state(false);
  let status = $state<string | null>(null);
  let error = $state<string | null>(null);
  let downgradeAcknowledged = $state(false);

  const reviewLabel = $derived(
    plan
      ? managedAppOperationLabel(plan.operation)
      : reviewOperation
        ? managedAppOperationLabel(reviewOperation)
        : "Review app",
  );

  async function browse() {
    error = null;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, multiple: false, title: "Choose an app package" });
      if (selected) {
        path = selected;
        // The review on screen describes the previous folder. Leaving it up
        // beside the new path reads as though it already applies to the newly
        // chosen package — including its permission list.
        resetReview();
        inspection = null;
        status = null;
      }
    } catch (failure) {
      error = String(failure);
    }
  }

  function resetReview() {
    plan = null;
    reviewOperation = null;
    downgradeAcknowledged = false;
  }

  async function inspect() {
    const trimmed = path.trim();
    if (!trimmed) {
      error = source === "git"
        ? "Enter a public HTTPS Git URL."
        : "Enter the path to a package directory.";
      return;
    }
    busy = true;
    error = null;
    status = source === "git" ? "Reviewing Git app…" : "Reviewing app…";
    inspection = null;
    resetReview();
    try {
      inspection = source === "git"
        ? await inspectGitPackage(trimmed)
        : await inspectPackage(trimmed);
      status = `${inspection.display_name} is ready to install.`;
      if (inspection.signature.kind === "valid-unknown-key") {
        status = `${inspection.display_name} verifies, but its key is not trusted yet.`;
      }
      await prepareReview();
    } catch (failure) {
      error = String(failure);
      status = null;
    } finally {
      busy = false;
    }
  }

  async function prepareReview() {
    const currentInspection = inspection;
    if (!currentInspection) return;
    if (!currentInspection.installable) {
      plan = null;
      reviewOperation = null;
      status = currentInspection.blocking_error ?? currentInspection.integrity_error ?? "package is not installable";
      return;
    }
    const installed = $installedApps.find((app) => app.manifest.app_id === currentInspection.id);
    const operation = plannedOperationForPackage(currentInspection, installed);
    reviewOperation = operation;
    if (operation === "version-conflict") {
      plan = null;
      status = "Version conflict: this package uses the same version as the installed app but a different digest.";
      return;
    }
    if (operation === "downgrade" && !downgradeAcknowledged) {
      plan = null;
      status = "Check the downgrade acknowledgement to review the diff.";
      return;
    }
    busy = true;
    error = null;
    status = `Preparing ${managedAppOperationLabel(operation).toLowerCase()} review…`;
    try {
      plan = await planManagedAppTransition({
        operation,
        staged_id: currentInspection.staged_id,
        package_digest: currentInspection.package_digest,
        app_id: null,
        revision_id: null,
        acknowledge_downgrade: operation === "downgrade" ? downgradeAcknowledged : false,
        acknowledge_revert_data_caveat: false,
      });
      status = `${managedAppOperationLabel(plan.operation)} review ready.`;
    } catch (failure) {
      plan = null;
      error = String(failure);
      status = null;
    } finally {
      busy = false;
    }
  }

  async function trustUnknownKey() {
    if (!inspection || inspection.signature.kind !== "valid-unknown-key" || !inspection.signature_public_key) return;
    busy = true;
    error = null;
    status = "Saving trusted key…";
    try {
      await trustPublisherKey({
        key_id: inspection.signature.key_id,
        public_key: inspection.signature_public_key,
        scope: { kind: "app-id", app_id: inspection.id },
      });
      status = "Trusted key saved. Refreshing inspection…";
      inspection = source === "git"
        ? await inspectGitPackage(path.trim())
        : await inspectPackage(path.trim());
      await prepareReview();
    } catch (failure) {
      error = String(failure);
      status = null;
    } finally {
      busy = false;
    }
  }

  async function confirmTransition() {
    if (!plan) return;
    busy = true;
    error = null;
    status = `${managedAppOperationLabel(plan.operation)} in progress…`;
    try {
      const apps = await applyManagedAppTransition(plan);
      await onInstalled(apps);
      status = `${managedAppOperationLabel(plan.operation)} finished.`;
      inspection = null;
      resetReview();
    } catch (failure) {
      error = String(failure);
      status = null;
    } finally {
      busy = false;
    }
  }
</script>

<section class="install">
  <h2>Install an app</h2>
  <p class="muted">
    Choose a local folder or public Git repository. The host looks for <code>app.json</code> directly
    or under <code>dist</code>. Kestral checks it without running app code, then shows what the app
    needs before installation.
  </p>
  <div class="source-picker" role="group" aria-label="App source">
    <button class:active={source === "folder"} aria-pressed={source === "folder"} type="button" onclick={() => { source = "folder"; resetReview(); inspection = null; status = null; error = null; }} disabled={busy}>
      Local folder
    </button>
    <button class:active={source === "git"} aria-pressed={source === "git"} type="button" onclick={() => { source = "git"; resetReview(); inspection = null; status = null; error = null; }} disabled={busy}>
      Public Git URL
    </button>
  </div>
  <div class="row">
    <input
      type={source === "git" ? "url" : "text"}
      bind:value={path}
      placeholder={source === "git" ? "https://github.com/example/my-app.git" : "/path/to/my-app"}
      aria-label={source === "git" ? "Public Git URL" : "Package directory path"}
      disabled={busy}
      onkeydown={(event) => event.key === "Enter" && inspect()}
    />
    {#if source === "folder" && !isRemoteTransport()}
      <button type="button" onclick={browse} disabled={busy}>Browse…</button>
    {/if}
    <button class="primary" onclick={inspect} disabled={busy}>Review app</button>
  </div>

  {#if status}
    <p class="status" role="status">{status}</p>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if inspection}
    <PackageInspectionView
      {inspection}
      showPermissions={!plan || plan.operation === "install"}
      trustAction={
        inspection.signature.kind === "valid-unknown-key" && inspection.signature_public_key
          ? { label: `Trust key for ${inspection.id}`, onClick: () => void trustUnknownKey() }
          : null
      }
    />

    {#if plan}
      <div class="review">
        {#if plan.operation !== "install"}
          <ManagedAppDiffView {plan} />
        {/if}
        <p class="review-note">Trusted chrome still reviews permissions before the kernel commits.</p>
        <div class="actions">
          <button class="primary" onclick={confirmTransition} disabled={busy}>
            {reviewLabel}
          </button>
          <button onclick={() => { resetReview(); status = null; }} disabled={busy}>Cancel review</button>
        </div>
      </div>
    {:else if reviewOperation === "downgrade"}
      <div class="warning" role="group" aria-label="Downgrade acknowledgement">
        <label>
          <input type="checkbox" bind:checked={downgradeAcknowledged} onchange={() => void prepareReview()} disabled={busy} />
          I understand this is a downgrade and I want to review it.
        </label>
      </div>
    {/if}

    {#if !plan && inspection.installable}
      <div class="actions">
        <button class="primary" onclick={() => void prepareReview()} disabled={busy || !inspection || !inspection.installable || reviewOperation === "version-conflict" || (reviewOperation === "downgrade" && !downgradeAcknowledged)}>
          {reviewLabel}
        </button>
        <button onclick={() => { inspection = null; resetReview(); status = null; error = null; }} disabled={busy}>Clear review</button>
      </div>
    {/if}
  {/if}
</section>

<style>
  .install {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 16px;
    padding: 1rem 1.1rem;
    display: grid;
    gap: 0.7rem;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: 1.05rem;
  }
  .muted,
  .status {
    margin: 0;
    color: var(--color-text-muted);
  }
  code {
    font-family: ui-monospace, monospace;
  }
  .row,
  .actions,
  .source-picker {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .row {
    min-width: 0;
  }
  .source-picker button.active {
    border-color: var(--color-accent);
    color: var(--color-accent);
  }
  input {
    flex: 1 1 16rem;
    min-width: 0;
    border: 1px solid var(--color-border-strong);
    border-radius: 8px;
    padding: 0.5rem 0.6rem;
    background: var(--color-surface);
    color: var(--color-text);
  }
  .review {
    display: grid;
    gap: 0.7rem;
  }
  .review-note {
    margin: 0;
    color: var(--color-text-muted);
    font-size: 0.82rem;
  }
  .warning {
    border: 1px solid var(--color-warning-border);
    background: var(--color-warning-soft);
    border-radius: 12px;
    padding: 0.8rem;
    display: grid;
    gap: 0.5rem;
  }
  .warning label {
    display: flex;
    gap: 0.5rem;
    align-items: start;
    color: var(--color-warning-text);
  }
  button {
    border: 1px solid var(--color-border-strong);
    border-radius: 8px;
    background: var(--color-surface);
    color: var(--color-text);
    padding: 0.5rem 0.9rem;
    cursor: pointer;
  }
  button.primary {
    border: none;
    background: var(--color-accent);
    color: var(--color-accent-contrast);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .error {
    margin: 0;
    color: var(--color-danger-text);
    font-size: 0.85rem;
  }
</style>
