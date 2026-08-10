<script lang="ts">
  import { onMount } from "svelte";
  import { isRemoteTransport } from "$lib/hostTransport";
  import {
    exportPortableProfile,
    getActiveKestralProfile,
    getPortableRecoveryStatus,
    importPortableProfile,
    type KestralProfileView,
    type PortableExportResult,
    type PortableImportResult,
    type PortableRecoveryStatus,
  } from "$lib/api";

  let activeProfile = $state<KestralProfileView | null>(null);
  let exportPath = $state("");
  let importPath = $state("");
  let exportResult = $state<PortableExportResult | null>(null);
  let preview = $state<PortableImportResult | null>(null);
  let importResult = $state<PortableImportResult | null>(null);
  let recovery = $state<PortableRecoveryStatus | null>(null);
  let target = $state<"fresh" | "overwrite-current">("fresh");
  let displayName = $state("Imported workspace");
  let slug = $state("imported-workspace");
  let confirmation = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  const remote = isRemoteTransport();
  const overwritePhrase = $derived(activeProfile ? `RESTORE ${activeProfile.slug}` : "");

  onMount(() => {
    void Promise.all([getActiveKestralProfile(), getPortableRecoveryStatus()])
      .then(([profile, status]) => {
        activeProfile = profile;
        recovery = status;
      })
      .catch((failure) => (error = String(failure)));
  });

  async function chooseExport() {
    error = null;
    if (remote) return;
    const { save } = await import("@tauri-apps/plugin-dialog");
    const selected = await save({
      title: "Export portable workspace",
      defaultPath: `${activeProfile?.slug ?? "kestral"}.kestral-portable.zip`,
      filters: [{ name: "Kestral portable workspace", extensions: ["zip"] }],
    });
    if (selected) exportPath = selected;
  }

  async function chooseImport() {
    error = null;
    if (remote) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      title: "Import portable workspace",
      multiple: false,
      directory: false,
      filters: [{ name: "Kestral portable workspace", extensions: ["zip"] }],
    });
    if (typeof selected === "string") {
      importPath = selected;
      await inspectImport();
    }
  }

  async function runExport() {
    if (!exportPath) return;
    busy = true;
    error = null;
    exportResult = null;
    try {
      exportResult = await exportPortableProfile(exportPath);
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }

  async function inspectImport() {
    if (!importPath) return;
    busy = true;
    error = null;
    preview = null;
    importResult = null;
    try {
      preview = await importPortableProfile(importPath, { kind: "preview" });
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }

  async function runImport() {
    if (!preview) return;
    busy = true;
    error = null;
    try {
      importResult = await importPortableProfile(
        importPath,
        target === "fresh"
          ? { kind: "fresh", display_name: displayName.trim(), slug: slug.trim() }
          : { kind: "overwrite-current", confirmation },
      );
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }
</script>

<div class="transfer">
  <header>
    <h3>Portable workspace</h3>
    <p>Back up or transfer this profile without secret values or third-party app binaries.</p>
  </header>

  {#if recovery}
    <section class="review" aria-labelledby="portable-recovery-heading">
      <div>
        <h4 id="portable-recovery-heading">Finish imported workspace setup</h4>
        <p class="hint">Imported {new Date(recovery.imported_at).toLocaleString()}.</p>
      </div>
      {#if recovery.secrets.length}
        <p><strong>Re-enter credentials:</strong> {recovery.secrets.map((secret) => `${secret.owner}/${secret.name}`).join(", ")}</p>
      {/if}
      {#if recovery.apps.length}
        <p><strong>Reinstall matching app packages:</strong> {recovery.apps.map((app) => `${app.display_name} ${app.version} (${app.package_digest})`).join(", ")}</p>
      {/if}
      {#if recovery.file_resources.length}
        <p><strong>Re-register file resources:</strong> {recovery.file_resources.map((resource) => resource.display_name).join(", ")}</p>
      {/if}
    </section>
  {/if}

  <section>
    <div>
      <h4>Export</h4>
      <p class="hint">Includes durable workspace state and app data. Credentials stay in this device's OS vault.</p>
    </div>
    {#if remote}
      <label>Host save path <input bind:value={exportPath} placeholder="/srv/backups/workspace.kestral-portable.zip" /></label>
    {:else}
      <button type="button" onclick={() => void chooseExport()}>Choose save location</button>
      {#if exportPath}<code>{exportPath}</code>{/if}
    {/if}
    <button type="button" class="primary" disabled={busy || !exportPath} onclick={() => void runExport()}>Export workspace</button>
    {#if exportResult}
      <div class="success" role="status">
        <strong>Portable workspace created</strong>
        <span>{exportResult.files} files · SHA-256 <code>{exportResult.sha256}</code></span>
        <span>{exportResult.excluded_secrets} credentials excluded · {exportResult.reinstall_apps} apps require reinstallation</span>
      </div>
    {/if}
  </section>

  <section>
    <div>
      <h4>Import</h4>
      <p class="hint">Passkeys and external file paths do not transfer. The archive is verified before a target can be chosen.</p>
    </div>
    {#if remote}
      <label>Host archive path <input bind:value={importPath} placeholder="/srv/backups/workspace.kestral-portable.zip" /></label>
      <button type="button" disabled={busy || !importPath} onclick={() => void inspectImport()}>Validate archive</button>
    {:else}
      <button type="button" disabled={busy} onclick={() => void chooseImport()}>Choose archive</button>
      {#if importPath}<code>{importPath}</code>{/if}
    {/if}

    {#if preview}
      <div class="review">
        <strong>Archive verified</strong>
        <dl>
          <div><dt>App data</dt><dd>Kept</dd></div>
          <div><dt>Credentials</dt><dd>{preview.secrets.length} to re-enter</dd></div>
          <div><dt>Third-party apps</dt><dd>{preview.apps.length} to reinstall</dd></div>
          <div><dt>File resources</dt><dd>{preview.file_resources.length} to re-register</dd></div>
          <div><dt>Passkeys</dt><dd>Excluded</dd></div>
        </dl>
      </div>
      <fieldset>
        <legend>Import target</legend>
        <label class="choice"><input type="radio" bind:group={target} value="fresh" /> Create a fresh profile</label>
        <label class="choice"><input type="radio" bind:group={target} value="overwrite-current" /> Overwrite the current profile after restart</label>
      </fieldset>
      {#if target === "fresh"}
        <div class="fields">
          <label>Profile name <input bind:value={displayName} required /></label>
          <label>Short name <input bind:value={slug} pattern="[a-z0-9][a-z0-9-]*" required /></label>
        </div>
      {:else}
        <div class="danger-review">
          <strong>The current profile will be retained as a rollback backup.</strong>
          <label>Type <code>{overwritePhrase}</code> to confirm <input bind:value={confirmation} /></label>
        </div>
      {/if}
      <button
        type="button"
        class:danger={target === "overwrite-current"}
        class:primary={target === "fresh"}
        disabled={busy || (target === "overwrite-current" ? confirmation !== overwritePhrase : !displayName.trim() || !slug.trim())}
        onclick={() => void runImport()}
      >{target === "fresh" ? "Import as new profile" : "Schedule overwrite and restart"}</button>
    {/if}

    {#if importResult}
      <div class="success" role="status">
        <strong>Import prepared</strong>
        <span>{importResult.restart_instructions}</span>
        {#if importResult.secrets.length}<span>Re-enter credentials for: {importResult.secrets.map((secret) => `${secret.owner}/${secret.name}`).join(", ")}</span>{/if}
      </div>
    {/if}
  </section>

  {#if busy}<p role="status">Working…</p>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  .transfer, section, header, .review, .success, .danger-review { display: grid; gap: 0.7rem; }
  section { border-top: 1px solid var(--color-border-subtle); padding-top: 0.9rem; }
  h3, h4, p { margin: 0; }
  .hint, header p { color: var(--color-text-muted); }
  label { display: grid; gap: 0.3rem; min-width: 0; }
  input { min-width: 0; border: 1px solid var(--color-border-strong); border-radius: 0.65rem; padding: 0.6rem 0.7rem; font: inherit; }
  button { width: fit-content; min-height: 2.5rem; border: 1px solid var(--color-border-strong); border-radius: 0.65rem; background: var(--color-surface-raised); color: var(--color-text); padding: 0.5rem 0.8rem; font: inherit; }
  button.primary { border-color: var(--color-accent); background: var(--color-accent); color: var(--color-accent-contrast); }
  button.danger { border-color: var(--color-danger-border); background: var(--color-danger-soft); color: var(--color-danger-text); }
  button:focus-visible, input:focus-visible { outline: 2px solid var(--color-focus-ring); outline-offset: 2px; }
  code { overflow-wrap: anywhere; }
  fieldset { display: grid; gap: 0.5rem; margin: 0; border: 1px solid var(--color-border); border-radius: 0.8rem; padding: 0.75rem; }
  .choice { display: flex; align-items: center; gap: 0.5rem; }
  .fields { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 14rem), 1fr)); gap: 0.7rem; }
  dl { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 9rem), 1fr)); gap: 0.6rem; margin: 0; }
  dl div { display: grid; gap: 0.15rem; }
  dt { color: var(--color-text-muted); font-size: 0.8rem; }
  dd { margin: 0; }
  .review, .success { border: 1px solid var(--color-success-border); background: var(--color-success-soft); border-radius: 0.8rem; padding: 0.8rem; }
  .danger-review { border: 1px solid var(--color-danger-border); background: var(--color-danger-soft); border-radius: 0.8rem; padding: 0.8rem; }
  .error { color: var(--color-danger-text); overflow-wrap: anywhere; }
  @media (max-width: 30em) { button { width: 100%; } }
</style>
