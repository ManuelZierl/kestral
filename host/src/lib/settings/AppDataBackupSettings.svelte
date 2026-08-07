<script lang="ts">
  import { hostConfig, saveHostPatch } from "$lib/stores/config";

  let retention = $state(1);
  let saving = $state(false);
  let status = $state<string | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    if ($hostConfig) retention = $hostConfig.host.app_data_backup_retention;
  });

  async function save() {
    const value = Number(retention);
    if (!Number.isInteger(value) || value < 1) {
      error = "Keep at least one backup per stateful app.";
      status = null;
      return;
    }
    saving = true;
    error = null;
    status = null;
    try {
      await saveHostPatch({ host: { app_data_backup_retention: value } });
      status = `Keeping ${value} app-data backup${value === 1 ? "" : "s"} per app.`;
    } catch (failure) {
      error = String(failure);
    } finally {
      saving = false;
    }
  }
</script>

<div class="backup-settings">
  <div>
    <h3>App-data backups</h3>
    <p>
      Kestral keeps the newest pre-migration backups for each stateful app. A backup is removed only
      after its replacement migration commits successfully.
    </p>
  </div>
  <label>
    Backups per app
    <input type="number" min="1" step="1" bind:value={retention} disabled={saving || !$hostConfig} />
  </label>
  <button type="button" onclick={() => void save()} disabled={saving || !$hostConfig}>Save retention</button>
  {#if status}<p role="status" class="status">{status}</p>{/if}
  {#if error}<p role="alert" class="error">{error}</p>{/if}
</div>

<style>
  .backup-settings {
    display: grid;
    gap: 0.75rem;
  }
  h3,
  p {
    margin: 0;
  }
  p,
  label {
    color: var(--color-text-muted);
  }
  label {
    display: grid;
    gap: 0.35rem;
    max-width: 14rem;
  }
  input {
    min-width: 0;
    border: 1px solid var(--color-border-strong);
    border-radius: 8px;
    padding: 0.5rem 0.6rem;
    background: var(--color-surface);
    color: var(--color-text);
    font: inherit;
  }
  button {
    justify-self: start;
  }
  .status {
    color: var(--color-success-text);
  }
  .error {
    color: var(--color-danger-text);
  }
</style>
