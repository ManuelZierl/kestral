<script lang="ts">
  import { artifacts } from "$lib/stores/artifacts";
  import { apps } from "$lib/stores/apps";
  import { chatThreads } from "$lib/stores/chatThreads";
  import { isRemoteTransport, signOutRemoteConnection } from "$lib/hostTransport";
  import { shellError } from "$lib/stores/hostState";

  interface Props {
    compact?: boolean;
  }

  let { compact = false }: Props = $props();
  let signOutError = $state<string | null>(null);

  async function disconnectRemoteHost() {
    signOutError = null;
    try {
      await signOutRemoteConnection();
    } catch (failure) {
      signOutError = failure instanceof Error ? failure.message : String(failure);
    }
  }
</script>

<footer class="status" class:compact aria-label="Host status">
  <div class="counts">
    <span>{ $chatThreads.length } chats</span>
    <span>{ $apps.length } apps</span>
    <span>{ $artifacts.length } artifacts</span>
  </div>
  <div
    class="connection"
    class:warning={Boolean($shellError || signOutError)}
    title={signOutError ?? $shellError ?? "Host connected"}
  >
    <span class="indicator" aria-hidden="true"></span>
    <span class="connection-label">{ signOutError ?? $shellError ?? "Host connected" }</span>
    {#if isRemoteTransport()}
      <button type="button" aria-label="Sign out" title="Sign out" onclick={disconnectRemoteHost}>
        <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
          <path
            d="M10 5H5v14h5M14 8l4 4-4 4M18 12H9"
            fill="none"
            stroke="currentColor"
            stroke-width="1.8"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
        <span class="action-label">Sign out</span>
      </button>
    {/if}
  </div>
</footer>

<style>
  .status {
    flex-shrink: 0;
    display: grid;
    gap: 0.65rem;
    padding: 0.8rem 0.45rem 0.15rem;
    border-top: 1px solid var(--color-sidebar-card-border);
    color: var(--color-sidebar-text-muted);
    font-size: 0.8rem;
  }
  .counts {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem 0.75rem;
  }
  .connection {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .connection-label {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .indicator {
    width: 0.5rem;
    height: 0.5rem;
    flex-shrink: 0;
    border-radius: 50%;
    background: currentColor;
  }
  .warning {
    color: var(--color-sidebar-text);
    font-weight: 600;
  }
  button {
    min-width: 1.75rem;
    min-height: 1.75rem;
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.35rem;
    padding: 0.2em 0.55em;
    border: 1px solid var(--color-sidebar-card-border);
    border-radius: 0.45rem;
    background: var(--color-sidebar-card-bg);
    color: var(--color-sidebar-text-muted);
    font: inherit;
  }
  button:focus-visible {
    outline: 3px solid var(--color-brand-gradient-b);
    outline-offset: 2px;
  }
  .status.compact {
    justify-items: center;
    padding-inline: 0;
  }
  .status.compact .counts,
  .status.compact .connection-label,
  .status.compact .action-label {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .status.compact .connection {
    width: 100%;
    flex-direction: column;
    justify-content: center;
  }
  .status.compact button {
    margin-left: 0;
    padding: 0;
  }
  @media (max-width: 48em) {
    .status {
      justify-items: center;
      padding-inline: 0;
    }
    .counts,
    .connection-label,
    .action-label {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip-path: inset(50%);
      white-space: nowrap;
    }
    .connection {
      width: 100%;
      flex-direction: column;
      justify-content: center;
    }
    button {
      margin-left: 0;
      padding: 0;
    }
  }
  @media (max-width: 30em) {
    .status {
      align-self: stretch;
      align-content: center;
      padding: 0 0 0 0.5rem;
      border-top: 0;
      border-left: 1px solid var(--color-sidebar-card-border);
    }
    .connection {
      flex-direction: row;
    }
  }
</style>
