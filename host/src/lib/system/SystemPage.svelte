<script lang="ts">
  import { onMount } from "svelte";
  import {
    getActiveKestralProfile,
    getConfigStorageInfo,
    requestSystemReset,
    type ConfigStorageInfo,
    type KestralProfileView,
  } from "$lib/api";
  import { isRemoteTransport } from "$lib/hostTransport";
  import RunLedgerTable from "$lib/system/RunLedgerTable.svelte";
  import TrustedNoticeTable from "$lib/system/TrustedNoticeTable.svelte";
  import { refreshRecords, shellError } from "$lib/stores/hostState";
  import { resetThemeState } from "$lib/stores/theme";
  import { resetSidebarLayout } from "$lib/stores/sidebarLayout";
  import { refreshTrustedNotices } from "$lib/stores/trustedNotices";

  let storageInfo = $state<ConfigStorageInfo | null>(null);
  let storageError = $state<string | null>(null);
  let activeProfile = $state<KestralProfileView | null>(null);
  let resetReviewOpen = $state(false);
  let resetConfirmation = $state("");
  let resetBusy = $state(false);
  let resetError = $state<string | null>(null);
  let resetStatus = $state<string | null>(null);

  const confirmationPhrase = $derived(activeProfile ? `RESET ${activeProfile.slug}` : "");

  async function loadStorageInfo() {
    try {
      storageInfo = await getConfigStorageInfo();
      storageError = null;
    } catch (error) {
      storageInfo = null;
      storageError = String(error);
    }
  }

  async function loadActiveProfile() {
    try {
      activeProfile = await getActiveKestralProfile();
    } catch (error) {
      resetError = String(error);
    }
  }

  function reviewSystemReset() {
    resetReviewOpen = true;
    resetConfirmation = "";
    resetError = null;
    resetStatus = null;
  }

  function cancelSystemReset() {
    resetReviewOpen = false;
    resetConfirmation = "";
    resetError = null;
  }

  function clearResetUiState() {
    if (typeof localStorage === "undefined") return;
    resetThemeState();
    resetSidebarLayout();
    localStorage.removeItem("kernel.active-chat-thread");
    localStorage.removeItem("kernel.pending-chat-sends");
  }

  async function resetSystem() {
    if (resetConfirmation !== confirmationPhrase || resetBusy) return;
    resetBusy = true;
    resetError = null;
    try {
      const result = await requestSystemReset(resetConfirmation);
      clearResetUiState();
      resetStatus = result.restart_required
        ? "Reset scheduled. Stop and restart the Kestral backend to finish it. This console will need to pair again."
        : "Reset scheduled. Kestral is restarting to finish it.";
    } catch (error) {
      resetError = String(error);
      resetBusy = false;
    }
  }

  onMount(() => {
    void refreshTrustedNotices().catch((error) => shellError.set(String(error)));
    void refreshRecords().catch((error) => shellError.set(String(error)));
    void loadStorageInfo();
    void loadActiveProfile();
  });
</script>

<section class="stack">
  <article class="card">
    <h2>Activity</h2>
    <p class="muted">
      Capability work routed through Kestral this session, one entry per Run. Unsandboxed native
      backends can also act outside this history.
    </p>
    <RunLedgerTable />
  </article>
  <article class="card">
    <h2>Recent trusted notices</h2>
    <TrustedNoticeTable />
  </article>
  <article class="card">
    <h2>Local storage</h2>
    {#if storageInfo}
      <dl class="paths">
        <div>
          <dt>Config file</dt>
          <dd><code>{storageInfo.config_path}</code></dd>
        </div>
        <div>
          <dt>Secrets store</dt>
          <dd><code>{storageInfo.secrets_path}</code></dd>
        </div>
        <div>
          <dt>Chat store</dt>
          <dd><code>{storageInfo.chat_store_path}</code></dd>
        </div>
      </dl>
      <p class="note">This file contains credential references and status only; values stay in the OS vault.</p>
    {:else if storageError}
      <p class="error">{storageError}</p>
    {:else}
      <p class="muted">Loading storage paths...</p>
    {/if}
  </article>
  <article class="card danger-zone">
    <h2>System reset</h2>
    <p>
      Return the current profile to a fresh installation. This permanently removes its
      conversations, apps and managed app data, configuration, protected credentials,
      permissions, artifacts, Run history, trusted notices, package trust, registered
      resources, and Kestral audit and update logs.
    </p>
    <p class="muted">
      Other Kestral profiles and files outside this profile stay untouched. Files or folders
      registered as resources are unregistered, not deleted. Data held by cloud providers,
      operating-system logs, and files written outside Kestral are outside this reset.
    </p>
    {#if activeProfile}
      <p class="profile-scope">
        Current profile: <strong>{activeProfile.display_name}</strong>
        <code>{activeProfile.root}</code>
      </p>
      {#if resetReviewOpen}
        <section class="reset-review" aria-labelledby="reset-review-title">
          <h3 id="reset-review-title">Review permanent deletion</h3>
          <p>
            There is no undo. Kestral preserves this profile's identity so other profiles and
            profile selection continue to work, then restarts with empty profile data.
          </p>
          <label for="reset-confirmation">
            Type <code>{confirmationPhrase}</code> to enable reset
          </label>
          <input
            id="reset-confirmation"
            type="text"
            bind:value={resetConfirmation}
            autocomplete="off"
            spellcheck="false"
            disabled={resetBusy}
          />
          <div class="actions">
            <button
              type="button"
              class="danger"
              onclick={() => void resetSystem()}
              disabled={resetBusy || resetConfirmation !== confirmationPhrase}
            >
              {resetBusy ? "Reset scheduled" : isRemoteTransport() ? "Schedule reset" : "Reset and restart Kestral"}
            </button>
            <button type="button" class="secondary" onclick={cancelSystemReset} disabled={resetBusy}>
              Keep my data
            </button>
          </div>
        </section>
      {:else}
        <button type="button" class="secondary" onclick={reviewSystemReset}>Review system reset</button>
      {/if}
    {:else if !resetError}
      <p class="muted">Loading current profile...</p>
    {/if}
    {#if resetStatus}<p class="success" role="status">{resetStatus}</p>{/if}
    {#if resetError}<p class="error" role="alert">{resetError}</p>{/if}
  </article>
  <article class="card">
    <h2>About this host</h2>
    <p class="muted">
      Kestral is a personal-first, open-source AI workspace and lean local host for user-chosen
      apps. Chat is the default starting app, not the canonical interface for all AI work.
      Capability actions routed through Kestral follow the host's single action path, are checked
      against the permissions you granted, and are recorded as Runs above. Native app backends
      and stdio tool servers remain OS-powerful in this alpha and can act outside that path.
    </p>
    <p class="muted">
      Installing an app is not blanket trust. The host enforces boundaries through app
      manifests, grants, host-owned approval prompts, and provenance stamped on everything
      an app produces through the mediated path. Permissions are reviewed and managed under
      Settings&nbsp;→&nbsp;Permissions.
    </p>
  </article>
</section>

<style>
  .stack {
    margin-top: 1rem;
    display: grid;
    gap: 1rem;
  }
  .card {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 18px;
    padding: 1rem 1.1rem;
  }
  h2 {
    margin-top: 0;
  }
  .paths {
    display: grid;
    gap: 0.75rem;
    margin: 0;
  }
  .paths div {
    display: grid;
    gap: 0.2rem;
  }
  dt {
    font-weight: 600;
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .note,
  .muted,
  .error {
    margin-bottom: 0;
  }
  .danger-zone {
    border-color: var(--color-danger-border);
  }
  .danger-zone > p {
    max-width: 70ch;
  }
  .profile-scope {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 0.7rem;
    align-items: baseline;
  }
  .profile-scope code {
    overflow-wrap: anywhere;
  }
  .reset-review {
    display: grid;
    gap: 0.75rem;
    max-width: 44rem;
    padding: 1rem;
    border: 1px solid var(--color-danger-border);
    border-radius: 0.75rem;
    background: var(--color-danger-soft);
  }
  .reset-review h3,
  .reset-review p {
    margin: 0;
  }
  .reset-review label {
    font-weight: 600;
  }
  .reset-review input {
    width: 100%;
    min-height: 2.75rem;
    padding: 0.65rem 0.75rem;
    border: 1px solid var(--color-border-strong);
    border-radius: 0.6rem;
    background: var(--color-surface-raised);
    color: var(--color-text);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.6rem;
  }
  button {
    min-height: 2.75rem;
    padding: 0.65rem 0.9rem;
    border: 1px solid var(--color-border-strong);
    border-radius: 0.65rem;
    background: var(--color-surface-raised);
    color: var(--color-text);
  }
  button.secondary {
    background: transparent;
  }
  button.danger {
    border-color: var(--color-danger-border);
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }
  button:disabled,
  input:disabled {
    cursor: default;
    opacity: 0.65;
  }
  .success {
    color: var(--color-success-text);
    margin-bottom: 0;
  }
  .muted {
    color: var(--color-text-muted);
  }
  .error {
    color: var(--color-danger-text);
  }
</style>
