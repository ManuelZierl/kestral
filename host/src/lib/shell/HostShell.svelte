<script lang="ts">
  import type { Snippet } from "svelte";
  // Side-effect import: applies the persisted theme's CSS variables to the
  // document root before first paint. All colors below come from there.
  import "$lib/stores/theme";
  import TrustedChrome from "$lib/chrome/TrustedChrome.svelte";
  import AppSidebar from "$lib/shell/AppSidebar.svelte";
  import MainSurface from "$lib/shell/MainSurface.svelte";
  import TopBar from "$lib/shell/TopBar.svelte";
  import type { Tab } from "$lib/stores/hostState";
  import { activeAppId } from "$lib/stores/hostState";

  interface Props {
    tab: Tab;
    error: string | null;
    /** Present only while startup bootstrap has failed and can be re-run. */
    onRetry: (() => void) | null;
    onSelectTab: (tab: Tab) => void;
    onReady: () => void;
    children: Snippet;
  }

  let { tab, error, onRetry, onSelectTab, onReady, children }: Props = $props();
</script>

<TrustedChrome {onReady} />

<div class="shell">
  <AppSidebar current={tab} onSelect={onSelectTab} />
  <section class="workspace">
    <TopBar {tab} />
    <MainSurface flush={tab === "apps" && $activeAppId !== null}>
      {#if error}
        <div class="error" role="alert">
          <p>{error}</p>
          {#if onRetry}
            <button type="button" onclick={onRetry}>Retry startup</button>
          {/if}
        </div>
      {/if}
      {@render children()}
    </MainSurface>
  </section>
</div>

<style>
  /* Border-box everywhere: "width: 100%" plus padding must never exceed
     the parent, otherwise scroll containers grow a horizontal scrollbar. */
  :global(*),
  :global(*::before),
  :global(*::after) {
    box-sizing: border-box;
  }
  :global(body) {
    margin: 0;
    font-family: Inter, "Segoe UI", system-ui, sans-serif;
    color: var(--color-text);
    background: var(--color-bg-gradient-b);
    overflow: hidden;
  }
  /* App-wide button affordances: every button reacts to hover and press,
     and disabled buttons look disabled. Component styles keep their own
     colors; these rules only layer interaction feedback on top. */
  :global(button) {
    cursor: pointer;
    transition:
      filter 120ms ease,
      transform 60ms ease,
      opacity 120ms ease;
  }
  :global(button:hover:not(:disabled)) {
    filter: brightness(0.95);
  }
  :global(button:active:not(:disabled)) {
    filter: brightness(0.9);
    transform: translateY(1px);
  }
  :global(button:disabled) {
    cursor: not-allowed;
    opacity: 0.55;
  }
  @media (prefers-reduced-motion: reduce) {
    :global(*),
    :global(*::before),
    :global(*::after) {
      animation: none !important;
      transition: none !important;
    }
  }
  .shell {
    height: 100vh;
    /* dvh tracks the real visible height when browser UI overlays shrink
       the viewport; the vh line above is the fallback for older engines. */
    height: 100dvh;
    display: flex;
    /* Shrinks with the window; 20rem is the WCAG 1.4.10 reflow floor
       (320 CSS px) — everything must stay usable down to here. */
    min-width: 20rem;
  }
  .workspace {
    min-width: 0;
    min-height: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
  }
  .error {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.75rem;
    margin: 0 0 1rem;
  }
  /* In the edge-to-edge app view the banner keeps its own breathing room. */
  :global(.surface.flush) .error {
    margin: 1rem;
  }
  .error p {
    color: var(--color-danger-text);
    margin: 0;
  }
  .error button {
    border: 1px solid var(--color-danger-border);
    border-radius: 0.375rem;
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
    padding: 0.375rem 0.75rem;
    font: inherit;
  }
  @media (max-width: 30em) {
    .shell {
      flex-direction: column;
    }
  }
</style>
