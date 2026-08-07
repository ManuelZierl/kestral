<script lang="ts">
  import { onMount } from "svelte";
  import {
    listPublisherTrust,
    revokePublisherKey,
    trustPublisherKey,
    type TrustRecord,
  } from "$lib/api";
  import { listenHostStateScope } from "$lib/hostTransport";

  let records = $state<TrustRecord[]>([]);
  let loading = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let status = $state<string | null>(null);
  let appId = $state("");
  let keyId = $state("");
  let publicKey = $state("");

  onMount(() => {
    void load();
    const unlisten = listenHostStateScope("publisher-trust", () => void load());
    return () => void unlisten.then((stop) => stop());
  });

  async function load() {
    loading = true;
    error = null;
    try {
      records = await listPublisherTrust();
    } catch (failure) {
      error = String(failure);
    } finally {
      loading = false;
    }
  }

  function scopeLabel(scope: TrustRecord["scope"]) {
    return scope.kind === "app-id" ? scope.app_id : `${scope.namespace_prefix}*`;
  }

  async function trustExactAppId() {
    const trimmedAppId = appId.trim();
    const trimmedKeyId = keyId.trim();
    const trimmedPublicKey = publicKey.trim();
    if (!trimmedAppId || !trimmedKeyId || !trimmedPublicKey) {
      error = "Enter an app id, key id, and public key.";
      return;
    }
    busy = true;
    error = null;
    status = "Saving trusted key…";
    try {
      records = await trustPublisherKey({
        key_id: trimmedKeyId,
        public_key: trimmedPublicKey,
        scope: { kind: "app-id", app_id: trimmedAppId },
      });
      appId = "";
      keyId = "";
      publicKey = "";
      status = "Trusted key saved.";
    } catch (failure) {
      error = String(failure);
      status = null;
    } finally {
      busy = false;
    }
  }

  async function revoke(record: TrustRecord) {
    busy = true;
    error = null;
    status = `Revoking ${record.key_id}…`;
    try {
      records = await revokePublisherKey({ key_id: record.key_id, scope: record.scope });
      status = `Revoked ${record.key_id}.`;
    } catch (failure) {
      error = String(failure);
      status = null;
    } finally {
      busy = false;
    }
  }

  // Revoking a publisher key affects future installs from that key, so it
  // goes through the same lightweight inline confirm as other one-click
  // destructive actions in Settings: the safe choice ("Keep") gets default
  // focus, Escape cancels.
  let confirmingKey = $state<string | null>(null);

  function focusOnMount(node: HTMLElement) {
    node.focus();
  }

  function cancelRevokeOnEscape(event: KeyboardEvent) {
    if (event.key === "Escape") {
      confirmingKey = null;
    }
  }
</script>

<div class="stack">
  <p class="hint">
    Signature verification proves publisher-key continuity for a package, not safety.
    This screen only stores exact app-id trust entries before install. You can revoke keys here later.
  </p>

  <section class="card trust-form" aria-label="Trust a publisher key for an exact app id">
    <h3>Trust exact app id</h3>
    <div class="form-grid">
      <label>
        App id
        <input bind:value={appId} placeholder="com.example.app" autocomplete="off" />
      </label>
      <label>
        Key id
        <input bind:value={keyId} placeholder="ed25519:…" autocomplete="off" />
      </label>
    </div>
    <label>
      Public key
      <textarea bind:value={publicKey} rows="4" placeholder="Base64-encoded ed25519 public key"></textarea>
    </label>
    <div class="actions">
      <button type="button" onclick={trustExactAppId} disabled={busy}>Trust key</button>
      <button type="button" class="secondary" onclick={() => void load()} disabled={busy}>Refresh</button>
    </div>
    <p class="muted">This never skips the install permission review.</p>
  </section>

  {#if loading}
    <p class="muted" role="status">Loading trusted keys…</p>
  {/if}

  {#if status}
    <p class="status" role="status">{status}</p>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <section class="card" aria-label="Trusted and revoked keys">
    <h3>Trusted and revoked keys</h3>
    {#if records.length === 0}
      <p class="muted">No trusted keys yet.</p>
    {:else}
      <div class="records">
        {#each records as record (record.key_id + scopeLabel(record.scope))}
          {@const recordKey = record.key_id + scopeLabel(record.scope)}
          <article class="record">
            <div class="record-head">
              <div>
                <h4><code>{record.key_id}</code></h4>
                <p>{scopeLabel(record.scope)}</p>
              </div>
              <span class={`badge badge-${record.status}`}>{record.status}</span>
            </div>
            <p class="key"><code>{record.public_key}</code></p>
            <div class="actions">
              {#if confirmingKey === recordKey}
                <span class="confirm-inline">
                  Revoke this key?
                  <button
                    type="button"
                    class="danger"
                    disabled={busy}
                    onclick={() => {
                      confirmingKey = null;
                      void revoke(record);
                    }}
                    onkeydown={cancelRevokeOnEscape}
                  >
                    Revoke
                  </button>
                  <button
                    type="button"
                    use:focusOnMount
                    onclick={() => (confirmingKey = null)}
                    onkeydown={cancelRevokeOnEscape}
                  >
                    Keep
                  </button>
                </span>
              {:else}
                <button
                  type="button"
                  class="danger"
                  disabled={busy || record.status === "revoked"}
                  onclick={() => (confirmingKey = recordKey)}
                >
                  Revoke
                </button>
              {/if}
            </div>
          </article>
        {/each}
      </div>
    {/if}
  </section>
</div>

<style>
  .stack {
    display: grid;
    gap: 0.85rem;
  }
  .hint,
  .muted,
  .status,
  .error {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .hint,
  .muted {
    color: var(--color-text-muted);
  }
  .status {
    color: var(--color-text-muted);
  }
  .error {
    color: var(--color-danger-text);
  }
  .card {
    border: 1px solid var(--color-border);
    border-radius: 16px;
    background: var(--color-surface);
    padding: 1rem;
    display: grid;
    gap: 0.75rem;
    min-width: 0;
  }
  h3,
  h4 {
    margin: 0;
  }
  .form-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 14rem), 1fr));
    gap: 0.75rem;
  }
  label {
    display: grid;
    gap: 0.25rem;
  }
  input,
  textarea {
    width: 100%;
    min-width: 0;
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    padding: 0.55rem 0.7rem;
    font: inherit;
    background: var(--color-surface);
    color: var(--color-text);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
  }
  .confirm-inline {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem;
    color: var(--color-warning-text);
    font-size: 0.85rem;
  }
  button {
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    background: var(--color-surface-raised);
    color: var(--color-text);
    padding: 0.45rem 0.7rem;
    font: inherit;
  }
  button.secondary {
    background: var(--color-surface);
  }
  button.danger {
    border-color: var(--color-danger-border);
    color: var(--color-danger-text);
  }
  button:disabled {
    opacity: 0.5;
  }
  .records {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 18rem), 1fr));
    gap: 0.75rem;
  }
  .record {
    border: 1px solid var(--color-border-subtle);
    border-radius: 12px;
    padding: 0.85rem;
    background: var(--color-surface-muted);
    display: grid;
    gap: 0.55rem;
    min-width: 0;
  }
  .record-head {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    align-items: start;
    flex-wrap: wrap;
  }
  .record p {
    margin: 0;
  }
  .key {
    font-size: 0.8rem;
    overflow-wrap: anywhere;
  }
  .badge {
    border-radius: 999px;
    padding: 0.15rem 0.5rem;
    font-size: 0.74rem;
    font-weight: 700;
  }
  .badge-trusted {
    background: var(--color-success-soft);
    color: var(--color-success-text);
  }
  .badge-revoked {
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }
</style>
