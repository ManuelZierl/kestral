<script lang="ts">
  import { onMount } from "svelte";
  import {
    arrangeSidebarDestinations,
    resetSidebarLayout,
    setSidebarDestinationHidden,
    setSidebarDestinationOrder,
    sidebarLayout,
    sidebarLayoutStorageError,
    type SidebarDestination,
  } from "$lib/stores/sidebarLayout";

  interface Props {
    destinations: SidebarDestination[];
    onClose: () => void;
  }

  let { destinations, onClose }: Props = $props();
  let dialog: HTMLElement;
  let closeButton: HTMLButtonElement;
  const orderedDestinations = $derived(arrangeSidebarDestinations(destinations, $sidebarLayout));

  onMount(() => closeButton.focus());

  function move(destinationId: string, offset: -1 | 1): void {
    const ids = orderedDestinations.map((destination) => destination.id);
    const currentIndex = ids.indexOf(destinationId);
    const nextIndex = currentIndex + offset;
    if (currentIndex < 0 || nextIndex < 0 || nextIndex >= ids.length) return;
    [ids[currentIndex], ids[nextIndex]] = [ids[nextIndex], ids[currentIndex]];
    setSidebarDestinationOrder(ids);
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = dialog.querySelectorAll<HTMLElement>(
      "button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex='-1'])",
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

<div class="backdrop">
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="sidebar-customization-title"
    aria-describedby="sidebar-customization-description"
    bind:this={dialog}
    onkeydown={handleKeydown}
    tabindex="-1"
  >
    <header>
      <div>
        <h2 id="sidebar-customization-title">Customize navigation</h2>
        <p id="sidebar-customization-description">Choose what appears and arrange it in the order you use.</p>
      </div>
      <button bind:this={closeButton} class="icon-button" type="button" aria-label="Close navigation customization" onclick={onClose}>
        <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
          <path d="m6 6 12 12M18 6 6 18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
        </svg>
      </button>
    </header>

    {#if $sidebarLayoutStorageError}
      <p class="error" role="alert">{$sidebarLayoutStorageError} Defaults are shown instead.</p>
    {/if}

    <ol class="destination-list">
      {#each orderedDestinations as destination, index (destination.id)}
        <li>
          <label class="visibility">
            <input
              type="checkbox"
              aria-label={`Show ${destination.label}`}
              checked={!$sidebarLayout.hidden.includes(destination.id)}
              onchange={(event) => setSidebarDestinationHidden(destination.id, !event.currentTarget.checked)}
            />
            <span>
              <strong>{destination.label}</strong>
              <small>{destination.kind === "host" ? "Kestral screen" : "Installed app"}</small>
            </span>
          </label>
          <div class="move-actions">
            <button
              class="icon-button"
              type="button"
              aria-label={`Move ${destination.label} up`}
              title="Move up"
              disabled={index === 0}
              onclick={() => move(destination.id, -1)}
            >
              <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
                <path d="m6 14 6-6 6 6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            </button>
            <button
              class="icon-button"
              type="button"
              aria-label={`Move ${destination.label} down`}
              title="Move down"
              disabled={index === orderedDestinations.length - 1}
              onclick={() => move(destination.id, 1)}
            >
              <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
                <path d="m6 10 6 6 6-6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            </button>
          </div>
        </li>
      {/each}
    </ol>

    <footer>
      <p>Changes are saved on this device.</p>
      <div class="footer-actions">
        <button type="button" class="secondary" onclick={resetSidebarLayout}>Reset to default</button>
        <button type="button" class="primary" onclick={onClose}>Close</button>
      </div>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 800;
    display: grid;
    place-items: center;
    padding: 1rem;
    background: var(--color-scrim);
  }
  .dialog {
    width: min(100%, 36rem);
    max-height: calc(100vh - 2rem);
    max-height: calc(100dvh - 2rem);
    overflow-y: auto;
    display: grid;
    gap: 1rem;
    padding: 1.25rem;
    border: 1px solid var(--color-border);
    border-radius: 1rem;
    background: var(--color-surface-raised);
    color: var(--color-text);
    box-shadow: 0 1rem 3rem var(--color-shadow-strong);
  }
  header,
  footer,
  .footer-actions,
  .move-actions {
    display: flex;
    align-items: center;
  }
  header,
  footer {
    justify-content: space-between;
    gap: 1rem;
  }
  h2,
  p {
    margin: 0;
  }
  header p,
  footer p,
  small {
    color: var(--color-text-muted);
  }
  header p {
    margin-top: 0.25rem;
  }
  .destination-list {
    display: grid;
    gap: 0.5rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .destination-list li {
    min-width: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 0.75rem;
    align-items: center;
    padding: 0.65rem 0.75rem;
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.75rem;
    background: var(--color-surface);
  }
  .visibility {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 0.75rem;
    cursor: pointer;
  }
  .visibility input {
    width: 1.15rem;
    height: 1.15rem;
    flex-shrink: 0;
    accent-color: var(--color-accent);
  }
  .visibility span {
    min-width: 0;
    display: grid;
    gap: 0.15rem;
  }
  .visibility strong,
  .visibility small {
    overflow-wrap: anywhere;
  }
  .move-actions,
  .footer-actions {
    gap: 0.4rem;
  }
  button {
    min-height: 2.5rem;
    padding: 0.55rem 0.8rem;
    border: 1px solid var(--color-border-strong);
    border-radius: 0.6rem;
    background: var(--color-surface);
    color: var(--color-text);
    font: inherit;
  }
  button:focus-visible,
  input:focus-visible {
    outline: 3px solid var(--color-focus-ring);
    outline-offset: 2px;
  }
  .icon-button {
    width: 2.5rem;
    padding: 0;
    display: inline-grid;
    place-items: center;
    flex-shrink: 0;
  }
  .primary {
    border-color: var(--color-accent-border);
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
  }
  .secondary {
    background: transparent;
  }
  .error {
    padding: 0.75rem;
    border: 1px solid var(--color-danger-border);
    border-radius: 0.65rem;
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }
  @media (max-width: 30em) {
    .backdrop {
      padding: 0.5rem;
    }
    .dialog {
      max-height: calc(100vh - 1rem);
      max-height: calc(100dvh - 1rem);
      padding: 1rem;
    }
    footer {
      align-items: flex-start;
      flex-direction: column;
    }
    .footer-actions {
      width: 100%;
      flex-wrap: wrap;
    }
    .footer-actions button {
      flex: 1;
    }
  }
</style>
