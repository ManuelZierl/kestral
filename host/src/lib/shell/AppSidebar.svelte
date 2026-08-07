<script lang="ts">
  import type { KestralIconName } from "$lib/api";
  import AppIconGraphic from "$lib/apps/AppIconGraphic.svelte";
  import type { Tab } from "$lib/stores/hostState";
  import { activeAppId } from "$lib/stores/hostState";
  import { apps } from "$lib/stores/apps";
  import { standaloneSurfaces } from "$lib/apps/standaloneSurfaces";
  import KestralMark from "$lib/shell/KestralMark.svelte";
  import SidebarStatus from "$lib/shell/SidebarStatus.svelte";
  import CustomizeSidebar from "$lib/shell/CustomizeSidebar.svelte";
  import {
    arrangeSidebarDestinations,
    setSidebarCollapsed,
    sidebarLayout,
    type SidebarDestination,
  } from "$lib/stores/sidebarLayout";

  interface HostNavItem extends SidebarDestination {
    kind: "host";
    tab: Tab;
    label: string;
    icon: KestralIconName;
    secondary?: boolean;
  }

  interface AppNavItem extends SidebarDestination {
    kind: "app";
    appId: string;
    icon: import("$lib/api").AppIcon | null | undefined;
  }

  type NavItem = HostNavItem | AppNavItem;

  interface Props {
    current: Tab;
    onSelect: (tab: Tab) => void;
  }

  let { current, onSelect }: Props = $props();
  let customizing = $state(false);
  let customizeButton: HTMLButtonElement;

  const hostItems: HostNavItem[] = [
    {
      id: "host:chat",
      kind: "host",
      tab: "chat",
      label: "Chat",
      icon: "chat-bubble",
    },
    {
      id: "host:apps",
      kind: "host",
      tab: "apps",
      label: "Apps",
      icon: "app-grid",
    },
    {
      id: "host:stuff",
      kind: "host",
      tab: "stuff",
      label: "Artifacts",
      icon: "artifact-box",
    },
    {
      id: "host:settings",
      kind: "host",
      tab: "settings",
      label: "Settings",
      icon: "settings",
    },
    {
      id: "host:system",
      kind: "host",
      tab: "system",
      label: "System",
      icon: "activity",
      secondary: true,
    },
  ];

  const standaloneApps = $derived(
    $apps.filter((app) => {
      if (["chat", "llm-provider"].includes(app.manifest.app_id)) return false;
      return standaloneSurfaces(app.manifest).length > 0;
    }),
  );

  const destinations = $derived<NavItem[]>([
    ...hostItems,
    ...standaloneApps.map((app): AppNavItem => ({
      id: `app:${app.manifest.app_id}`,
      kind: "app",
      appId: app.manifest.app_id,
      label: app.manifest.display_name,
      icon: app.icon,
    })),
  ]);
  const orderedDestinations = $derived(arrangeSidebarDestinations(destinations, $sidebarLayout));
  const visibleDestinations = $derived(
    orderedDestinations.filter((destination) => !$sidebarLayout.hidden.includes(destination.id)),
  );

  function selectHostTab(tab: Tab) {
    if (tab === "apps") activeAppId.set(null);
    onSelect(tab);
  }

  function selectApp(appId: string) {
    activeAppId.set(appId);
    onSelect("apps");
  }

  function closeCustomization(): void {
    customizing = false;
    requestAnimationFrame(() => customizeButton?.focus());
  }
</script>

<aside class="sidebar" class:collapsed={$sidebarLayout.collapsed} aria-label="App navigation">
  <div class="brand">
    <div class="glyph"><KestralMark size="2rem" /></div>
    <div class="brand-copy">
      <h1>Kestral</h1>
    </div>
    <button
      class="collapse-toggle"
      type="button"
      aria-label={$sidebarLayout.collapsed ? "Expand navigation" : "Collapse navigation"}
      title={$sidebarLayout.collapsed ? "Expand navigation" : "Collapse navigation"}
      onclick={() => setSidebarCollapsed(!$sidebarLayout.collapsed)}
    >
      <svg class:reversed={$sidebarLayout.collapsed} viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
        <path
          d="m14 18-6-6 6-6"
          fill="none"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
    </button>
  </div>

  <nav class="nav" aria-label="Primary">
    {#each visibleDestinations as destination (destination.id)}
      {#if destination.kind === "host"}
        <button
          class:active={current === destination.tab && (destination.tab !== "apps" || $activeAppId === null)}
          class:secondary={destination.secondary}
          title={destination.label}
          aria-current={current === destination.tab && (destination.tab !== "apps" || $activeAppId === null) ? "page" : undefined}
          onclick={() => selectHostTab(destination.tab)}
        >
          <span class="icon" aria-hidden="true">
            <AppIconGraphic icon={{ kind: "kestral", name: destination.icon }} fallback={destination.label} />
          </span>
          <span class="label">{destination.label}</span>
        </button>
      {:else}
        <button
          class:active={current === "apps" && $activeAppId === destination.appId}
          title={destination.label}
          aria-current={current === "apps" && $activeAppId === destination.appId ? "page" : undefined}
          onclick={() => selectApp(destination.appId)}
        >
          <span class="icon app-glyph" class:asset-icon={destination.icon?.kind === "asset"} aria-hidden="true">
            <AppIconGraphic icon={destination.icon} fallback={destination.label} />
          </span>
          <span class="label">{destination.label}</span>
        </button>
      {/if}
    {/each}
  </nav>

  <button
    bind:this={customizeButton}
    class="customize-toggle"
    type="button"
    aria-label="Customize navigation"
    title="Customize navigation"
    onclick={() => (customizing = true)}
  >
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
      <path d="M4 7h10M18 7h2M4 17h2M10 17h10M14 4v6M6 14v6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
    </svg>
    <span class="label">Customize</span>
  </button>
  <SidebarStatus compact={$sidebarLayout.collapsed} />
</aside>

{#if customizing}
  <CustomizeSidebar destinations={destinations} onClose={closeCustomization} />
{/if}

<style>
  .sidebar {
    width: 18.5rem;
    flex-shrink: 0;
    box-sizing: border-box;
    overflow-y: auto;
    padding: 1rem 0.9rem 1rem 1rem;
    border-right: 1px solid var(--color-sidebar-border);
    background: linear-gradient(180deg, var(--color-sidebar-bg-a), var(--color-sidebar-bg-b));
    color: var(--color-sidebar-text);
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  .brand {
    display: grid;
    grid-template-columns: 2.5rem minmax(0, 1fr) 2.25rem;
    gap: 0.75rem;
    align-items: center;
    padding: 0.4rem;
  }
  .glyph {
    width: 2.5rem;
    height: 2.5rem;
    border-radius: 0.9rem;
    display: grid;
    place-items: center;
    background: var(--color-brand-mark-bg);
    color: var(--color-brand-mark);
    /* Keeps the brand tile distinct from the surrounding sidebar. */
    border: 1px solid var(--color-sidebar-card-border);
    font-weight: 800;
  }
  .brand-copy {
    min-width: 0;
  }
  .collapse-toggle {
    width: 2.25rem;
    height: 2.25rem;
    justify-content: center;
    flex-shrink: 0;
    padding: 0;
    border-color: var(--color-sidebar-card-border);
    border-radius: 0.75rem;
    color: var(--color-sidebar-text-muted);
  }
  .collapse-toggle svg {
    transition: transform 120ms ease;
  }
  .collapse-toggle svg.reversed {
    transform: rotate(180deg);
  }
  .brand h1 {
    margin: 0;
    font-size: 0.98rem;
    line-height: 1.35;
    overflow-wrap: anywhere;
  }
  .nav {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    flex: 1;
    min-height: 0;
  }
  button {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    width: 100%;
    border: 1px solid transparent;
    border-radius: 0.95rem;
    background: transparent;
    color: var(--color-sidebar-text);
    padding: 0.8rem 0.9rem;
    cursor: pointer;
    text-align: left;
    font-size: 0.96rem;
  }
  button:hover {
    background: var(--color-sidebar-hover);
  }
  button:focus-visible {
    outline: 3px solid var(--color-brand-gradient-b);
    outline-offset: 2px;
  }
  button.active {
    background: linear-gradient(135deg, var(--color-sidebar-active-a), var(--color-sidebar-active-b));
    border-color: var(--color-sidebar-active-border);
    box-shadow: inset 0 0 0 1px var(--color-sidebar-card-bg);
  }
  button.secondary {
    color: var(--color-sidebar-text-muted);
  }
  .icon {
    display: inline-grid;
    place-items: center;
    width: 1.5rem;
    color: var(--color-sidebar-text-faint);
    font-size: 0.8rem;
    font-weight: 700;
  }
  .app-glyph {
    width: 1.5rem;
    height: 1.5rem;
    border-radius: 0.45rem;
    background: var(--color-sidebar-card-bg);
    color: var(--color-sidebar-text);
  }
  .app-glyph.asset-icon {
    overflow: hidden;
    background: transparent;
  }
  .customize-toggle {
    flex-shrink: 0;
    color: var(--color-sidebar-text-muted);
    border-color: var(--color-sidebar-card-border);
  }
  .sidebar.collapsed {
    width: 4rem;
    padding: 0.75rem 0.5rem;
  }
  .sidebar.collapsed .brand {
    grid-template-columns: 2.25rem;
    justify-content: center;
    padding-inline: 0;
  }
  .sidebar.collapsed .glyph,
  .sidebar.collapsed .brand-copy {
    display: none;
  }
  .sidebar.collapsed button {
    justify-content: center;
    padding: 0.7rem 0;
  }
  .sidebar.collapsed .collapse-toggle {
    display: flex;
    padding: 0;
  }
  .sidebar.collapsed .label {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  @media (max-width: 60em) {
    .sidebar {
      width: 14rem;
    }
    button {
      padding-inline: 0.7rem;
    }
  }
  /* Below tablet width the sidebar collapses to an icon rail so the
     workspace keeps most of the window. Labels stay in the accessibility
     tree (clipped, not display:none) so buttons keep their names. */
  @media (max-width: 48em) {
    .sidebar {
      width: 4rem;
      padding: 0.75rem 0.5rem;
    }
    .brand {
      grid-template-columns: 2.5rem;
      justify-content: center;
    }
    .brand-copy,
    .sidebar.collapsed .brand-copy {
      display: none;
    }
    .collapse-toggle,
    .sidebar.collapsed .collapse-toggle {
      display: none;
    }
    .sidebar.collapsed .brand {
      grid-template-columns: 2.5rem;
    }
    .sidebar.collapsed .glyph {
      display: grid;
    }
    button {
      justify-content: center;
      padding: 0.7rem 0;
    }
    .label {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip-path: inset(50%);
      white-space: nowrap;
    }
  }
  /* At the reflow floor, a top navigation bar gives the workspace the full
     viewport width instead of permanently reserving a 4rem icon rail. */
  @media (max-width: 30em) {
    .sidebar,
    .sidebar.collapsed {
      width: 100%;
      height: 3.75rem;
      overflow: hidden;
      padding: 0.4rem 0.5rem;
      border-right: none;
      border-bottom: 1px solid var(--color-sidebar-border);
      flex-direction: row;
    }
    .brand,
    .sidebar.collapsed .brand {
      display: none;
    }
    .nav {
      min-width: 0;
      flex-direction: row;
      /* flex-start (not space-around) keeps the leading buttons reachable
         once the strip overflows and scrolls. */
      justify-content: flex-start;
      gap: 0.2rem;
      /* Installed-app buttons stay in the rail and the accessibility tree;
         when they overflow, the strip scrolls rather than dropping them. */
      overflow-x: auto;
    }
    .nav button {
      width: auto;
      min-width: 2.75rem;
      min-height: 2.75rem;
      flex: 1;
      padding: 0.5rem;
    }
    .customize-toggle,
    .sidebar.collapsed .customize-toggle {
      width: 2.75rem;
      min-width: 2.75rem;
      min-height: 2.75rem;
      align-self: center;
      justify-content: center;
      padding: 0;
    }
  }
</style>
