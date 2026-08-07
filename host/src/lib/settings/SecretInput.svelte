<script lang="ts">
  import { checkSecret, removeSecret, saveSecret } from "$lib/stores/config";
  import {
    secretInputPlaceholder,
    secretStatusAfterClear,
    secretStatusAfterSave,
    secretStatusFromPresence,
    secretStatusLabel,
    type SecretStatus,
  } from "$lib/settings/secretInputModel";

  interface Props {
    owner: string;
    secretName: string;
    label: string;
    checkStored?: () => Promise<boolean>;
    saveStored?: (value: string) => Promise<void>;
    clearStored?: () => Promise<void>;
  }

  let { owner, secretName, label, checkStored, saveStored, clearStored }: Props = $props();
  const inputId = $derived(`secret-${owner}-${secretName}`);

  let value = $state("");
  let status = $state<SecretStatus>("checking");
  let saving = $state(false);
  let clearing = $state(false);
  let error = $state<string | null>(null);
  let secretRequestId = 0;

  async function refreshStatus(mode: "check" | "save" | "clear", requestId: number) {
    const present = await (checkStored ? checkStored() : checkSecret(owner, secretName));
    if (requestId !== secretRequestId) return;
    if (mode === "save") {
      status = secretStatusAfterSave(present);
      if (!present) {
        error = "Secret was saved, but the stored status could not be confirmed.";
      }
      return;
    }
    if (mode === "clear") {
      status = secretStatusAfterClear(present);
      if (present) {
        error = "Secret was cleared, but the stored status could not be confirmed.";
      }
      return;
    }
    status = secretStatusFromPresence(present);
  }

  $effect(() => {
    secretName;
    error = null;
    status = "checking";
    const requestId = ++secretRequestId;
    refreshStatus("check", requestId)
      .catch((failure) => {
        if (requestId !== secretRequestId) return;
        status = "error";
        error = String(failure);
      });
  });

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (saving || clearing || value.trim() === "") return;
    saving = true;
    error = null;
    try {
      await (saveStored ? saveStored(value) : saveSecret(owner, secretName, value));
      value = "";
      await refreshStatus("save", ++secretRequestId);
    } catch (failure) {
      status = "error";
      error = String(failure);
    } finally {
      saving = false;
    }
  }

  async function clearStoredSecret() {
    if (saving || clearing) return;
    clearing = true;
    error = null;
    try {
      await (clearStored ? clearStored() : removeSecret(owner, secretName));
      value = "";
      await refreshStatus("clear", ++secretRequestId);
    } catch (failure) {
      status = "error";
      error = String(failure);
    } finally {
      clearing = false;
    }
  }

  // Clearing removes a stored credential outright, so it goes through the
  // same lightweight inline confirm as other one-click destructive actions in
  // Settings: the safe choice ("Keep") gets default focus, Escape cancels.
  let confirmingClear = $state(false);

  function focusOnMount(node: HTMLElement) {
    node.focus();
  }

  function cancelClearOnEscape(event: KeyboardEvent) {
    if (event.key === "Escape") {
      confirmingClear = false;
    }
  }
</script>

<form class="secret" onsubmit={submit}>
  <label class="field" for={inputId}>
    <span class="field-header">
      <span>{label}</span>
      <span class="status {status}">{secretStatusLabel(status)}</span>
    </span>
    <input id={inputId} bind:value={value} type="password" placeholder={secretInputPlaceholder(status)} />
  </label>
  <p class="note">Stored in your operating system credential vault. The host keeps status only.</p>
  <div class="actions">
    <button type="submit" disabled={saving || clearing || value.trim() === ""}>Save key</button>
    {#if confirmingClear}
      <span class="confirm-inline">
        Clear this key?
        <button
          type="button"
          class="secondary"
          disabled={saving || clearing}
          onclick={() => {
            confirmingClear = false;
            void clearStoredSecret();
          }}
          onkeydown={cancelClearOnEscape}
        >
          Clear
        </button>
        <button
          type="button"
          use:focusOnMount
          onclick={() => (confirmingClear = false)}
          onkeydown={cancelClearOnEscape}
        >
          Keep
        </button>
      </span>
    {:else}
      <button
        type="button"
        class="secondary"
        disabled={saving || clearing || status === "not-set" || status === "checking"}
        onclick={() => (confirmingClear = true)}
      >
        Clear key
      </button>
    {/if}
  </div>
  {#if error}
    <span class="error">{error}</span>
  {/if}
</form>

<style>
  .secret {
    display: grid;
    gap: 0.5rem;
  }
  .field {
    display: grid;
    gap: 0.25rem;
  }
  .field-header {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
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
  .note {
    margin: 0;
    color: var(--color-warning-text);
    font-size: 0.85rem;
  }
  input {
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    padding: 0.6rem 0.7rem;
  }
  button {
    width: fit-content;
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    background: var(--color-surface-raised);
    padding: 0.55rem 0.85rem;
  }
  .secondary {
    color: var(--color-danger-text);
    border-color: var(--color-danger-border);
    background: var(--color-danger-soft);
  }
  .status {
    width: fit-content;
    color: var(--color-text-muted);
    background: var(--color-surface-muted);
    border: 1px solid var(--color-border);
    border-radius: 999px;
    padding: 0.15rem 0.55rem;
    font-size: 0.8rem;
  }
  .set,
  .updated-now {
    color: var(--color-success-text);
    background: var(--color-success-soft);
    border-color: var(--color-success-border);
  }
  .error {
    color: var(--color-danger-text);
    font-size: 0.85rem;
  }
  .error.status {
    background: var(--color-danger-soft);
    border-color: var(--color-danger-border);
  }
  .not-set {
    color: var(--color-text-soft);
    background: var(--color-surface-muted);
    border-color: var(--color-border);
  }
</style>
