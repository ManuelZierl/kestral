<script lang="ts">
  import type { Tab } from "$lib/stores/hostState";
  import { activeAppId } from "$lib/stores/hostState";
  import { apps } from "$lib/stores/apps";
  import { openAppPermissions, openAppSettings } from "$lib/stores/navigation";

  interface Props {
    tab: Tab;
  }

  let { tab }: Props = $props();

  const titles: Record<Tab, { title: string; subtitle: string }> = {
    chat: {
      title: "Chat",
      subtitle: "Talk to your apps. Actions routed through Kestral are checked and recorded.",
    },
    apps: {
      title: "Apps",
      subtitle: "Install and manage the apps available in this host.",
    },
    stuff: {
      title: "Artifacts",
      subtitle: "Everything your apps produced, with provenance.",
    },
    settings: {
      title: "Settings",
      subtitle: "Providers, tool servers, apps, and permissions.",
    },
    system: {
      title: "System",
      subtitle: "Run history, trusted notices, and storage.",
    },
  };
  const displayedAppId = $derived(tab === "chat" ? "chat" : tab === "apps" ? $activeAppId : null);
  const activeApp = $derived(
    displayedAppId ? $apps.find((app) => app.manifest.app_id === displayedAppId) : undefined,
  );
  const heading = $derived(
    tab === "apps" && activeApp
      ? { title: activeApp.manifest.display_name, subtitle: activeApp.manifest.description }
      : titles[tab],
  );
</script>

<header class="topbar">
  <div class="heading">
    <h2>{heading.title}</h2>
    <p class="subtitle">{heading.subtitle}</p>
  </div>
  {#if activeApp}
    <div class="app-actions">
      <button
        class="app-action"
        type="button"
        aria-label={`Permissions for ${activeApp.manifest.display_name}`}
        title={`Permissions for ${activeApp.manifest.display_name}`}
        onclick={() => openAppPermissions(activeApp.manifest.app_id)}
      >
        <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
          <path
            d="M12 3 5 6v5c0 4.6 2.9 8.8 7 10 4.1-1.2 7-5.4 7-10V6l-7-3Z"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
          <path
            d="m9.2 12 1.8 1.8 3.8-4"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
      </button>
      <button
        class="app-action"
        type="button"
        aria-label={`Settings for ${activeApp.manifest.display_name}`}
        title={`Settings for ${activeApp.manifest.display_name}`}
        onclick={() => openAppSettings(activeApp.manifest.app_id, activeApp.manifest.display_name)}
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
  {/if}
</header>

<style>
  .topbar {
    min-height: 4.25rem;
    box-sizing: border-box;
    padding: 0.9rem 1.4rem;
    border-bottom: 1px solid var(--color-bar-border);
    background: var(--color-bar-bg);
    backdrop-filter: blur(18px);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
  }
  .heading {
    min-width: 0;
  }
  h2,
  .subtitle {
    margin: 0;
  }
  h2 {
    font-size: 1.05rem;
    color: var(--color-text);
    font-weight: 700;
  }
  .subtitle {
    margin-top: 0.2rem;
    font-size: 0.85rem;
    color: var(--color-text-muted);
  }
  .app-actions {
    flex: 0 0 auto;
    display: flex;
    gap: 0.5rem;
  }
  .app-action {
    width: 2.5rem;
    height: 2.5rem;
    display: grid;
    place-items: center;
    border: 1px solid var(--color-border);
    border-radius: 10px;
    background: var(--color-surface);
    color: var(--color-text-muted);
  }
  .app-action:hover {
    color: var(--color-text);
    border-color: var(--color-border-strong);
  }
  .app-action:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  @media (max-width: 60em) {
    .topbar {
      padding: 0.75rem 1rem;
    }
  }
  /* At the reflow floor every vertical rem counts: the title alone still
     names the view; the description returns on wider screens. */
  @media (max-width: 30em) {
    .topbar {
      min-height: 0;
    }
    .subtitle {
      display: none;
    }
  }
</style>
