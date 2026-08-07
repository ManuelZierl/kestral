<script lang="ts">
  import { onDestroy, tick } from "svelte";
  import { missingRequestedCapabilities } from "$lib/apps/appMetadata";
  import { apps } from "$lib/stores/apps";
  import { grants, requestAppGrantsAndRefresh } from "$lib/stores/grants";
  import AppSettingsPanel from "$lib/settings/AppSettingsPanel.svelte";
  import AppDataBackupSettings from "$lib/settings/AppDataBackupSettings.svelte";
  import GrantPolicyEditor from "$lib/settings/GrantPolicyEditor.svelte";
  import FileResourcesSettings from "$lib/settings/FileResourcesSettings.svelte";
  import ChatPromptSettings from "$lib/settings/ChatPromptSettings.svelte";
  import PackageTrustSettings from "$lib/settings/PackageTrustSettings.svelte";
  import KestralProfileSettings from "$lib/settings/KestralProfileSettings.svelte";
  import LlmProviderSettings from "$lib/settings/LlmProviderSettings.svelte";
  import McpServerSettings from "$lib/settings/McpServerSettings.svelte";
  import McpExportSettings from "$lib/settings/McpExportSettings.svelte";
  import ThemeSettings from "$lib/settings/ThemeSettings.svelte";
  import EmptyState from "$lib/shell/EmptyState.svelte";
  import { appSettingsTarget, permissionTarget, type AppSettingsTarget } from "$lib/stores/navigation";
  import { currentTab } from "$lib/stores/hostState";
  import { scrollTargetIntoView } from "$lib/a11y/scroll";

  type SettingsSection =
    | "general"
    | "chat"
    | "profiles"
    | "providers"
    | "servers"
    | "files"
    | "trust"
    | "apps"
    | "permissions"
    | "advanced";
  const dedicatedAppSettings: Partial<Record<string, SettingsSection>> = {
    chat: "chat",
    "llm-provider": "providers",
    "com.ma-zierl.host.file-broker": "files",
  };
  let activeSection = $state<SettingsSection>("general");
  let handledAppSettingsRequest = 0;
  let highlightedAppId = $state<string | null>(null);
  let unavailableAppSettings = $state<AppSettingsTarget | null>(null);
  let restoringPermissions = $state(false);
  let permissionsError = $state<string | null>(null);
  let appSettingsHighlightTimer: ReturnType<typeof setTimeout> | null = null;
  const appSettingsPanels = new Map<string, HTMLElement>();

  async function restoreChatPermissions() {
    if (!chatApp || restoringPermissions) return;
    restoringPermissions = true;
    permissionsError = null;
    try {
      await requestAppGrantsAndRefresh(chatApp.manifest.app_id);
    } catch (failure) {
      permissionsError = String(failure);
    } finally {
      restoringPermissions = false;
    }
  }

  function registerAppSettingsPanel(node: HTMLElement, appId: string) {
    appSettingsPanels.set(appId, node);
    return {
      update(nextAppId: string) {
        if (appSettingsPanels.get(appId) === node) appSettingsPanels.delete(appId);
        appId = nextAppId;
        appSettingsPanels.set(appId, node);
      },
      destroy() {
        if (appSettingsPanels.get(appId) === node) appSettingsPanels.delete(appId);
      },
    };
  }

  $effect(() => {
    if ($permissionTarget) activeSection = "permissions";
  });

  $effect(() => {
    const target = $appSettingsTarget;
    if (!target || target.request === handledAppSettingsRequest) return;

    handledAppSettingsRequest = target.request;
    if (!$apps.some((app) => app.manifest.app_id === target.appId)) {
      activeSection = "apps";
      highlightedAppId = null;
      unavailableAppSettings = target;
      appSettingsTarget.update((current) => current?.request === target.request ? null : current);
      return;
    }

    activeSection = dedicatedAppSettings[target.appId] ?? "apps";
    unavailableAppSettings = null;
    highlightedAppId = target.appId;
    if (appSettingsHighlightTimer) clearTimeout(appSettingsHighlightTimer);
    void tick().then(() => {
      const panel = appSettingsPanels.get(target.appId);
      panel?.focus({ preventScroll: true });
      scrollTargetIntoView(panel ?? null);
    });
    appSettingsHighlightTimer = setTimeout(() => {
      highlightedAppId = null;
      appSettingsTarget.update((current) => current?.request === target.request ? null : current);
    }, 3000);
  });

  onDestroy(() => {
    if (appSettingsHighlightTimer) clearTimeout(appSettingsHighlightTimer);
  });

  const settingsGroups: {
    label: string;
    sections: { id: SettingsSection; label: string }[];
  }[] = [
    {
      label: "Personal",
      sections: [
        { id: "general", label: "Appearance" },
        { id: "chat", label: "Chat" },
        { id: "profiles", label: "Kestral profiles" },
      ],
    },
    {
      label: "Connections",
      sections: [
        { id: "providers", label: "Model providers" },
        { id: "servers", label: "Tool servers" },
        { id: "files", label: "File resources" },
      ],
    },
    {
      label: "Apps & access",
      sections: [
        { id: "apps", label: "App settings" },
        { id: "permissions", label: "Permissions" },
        { id: "trust", label: "Package trust" },
      ],
    },
    {
      label: "System",
      sections: [{ id: "advanced", label: "Advanced" }],
    },
  ];

  const chatApp = $derived($apps.find((app) => app.manifest.app_id === "chat") ?? null);
  const missingChatCapabilities = $derived(
    chatApp ? missingRequestedCapabilities(chatApp.manifest, $grants) : [],
  );
</script>

<div class="settings-layout">
  <nav class="settings-nav" aria-label="Settings sections">
    {#each settingsGroups as group}
      <div class="settings-nav-group" role="group" aria-label={group.label}>
        <span class="settings-nav-label">{group.label}</span>
        {#each group.sections as section}
          <button
            type="button"
            class:active={activeSection === section.id}
            aria-current={activeSection === section.id ? "page" : undefined}
            onclick={() => (activeSection = section.id)}
          >
            {section.label}
          </button>
        {/each}
      </div>
    {/each}
  </nav>

  <div class="settings">
  {#if activeSection === "general"}
  <section class="group">
    <header class="group-header">
      <h2>Appearance</h2>
      <p>Choose how Kestral looks on this device.</p>
    </header>

    <article class="card">
      <h3>Theme</h3>
      <ThemeSettings />
    </article>

  </section>
  {:else if activeSection === "chat"}
  <section class="group">
    <header class="group-header">
      <h2>Chat</h2>
      <p>Set the default prompt behavior, app guidance, and runtime privacy controls for Chat.</p>
    </header>
    <div
      class="app-settings-panel"
      class:highlighted={highlightedAppId === "chat"}
      tabindex="-1"
      use:registerAppSettingsPanel={"chat"}
    >
      <ChatPromptSettings />
    </div>
  </section>
  {:else if activeSection === "profiles"}
  <section class="group">
    <header class="group-header">
      <h2>Kestral profiles</h2>
      <p>Keep different workspaces and their data separate.</p>
    </header>
    <article class="card">
      <AppDataBackupSettings />
    </article>
    <article class="card">
      <KestralProfileSettings />
    </article>
  </section>
  {:else if activeSection === "providers"}
  <section class="group">
    <header class="group-header">
      <h2>Model providers</h2>
      <p>Connect a model and choose the default for Chat.</p>
    </header>
    <article
      class="card app-settings-panel"
      class:highlighted={highlightedAppId === "llm-provider"}
      tabindex="-1"
      use:registerAppSettingsPanel={"llm-provider"}
    >
      <LlmProviderSettings />
    </article>
  </section>
  {:else if activeSection === "servers"}
  <section class="group">
    <header class="group-header">
      <h2>Tool servers</h2>
      <p>Connect MCP servers so their tools become apps you can use. Nothing connects until you say so.</p>
    </header>
    <article class="card">
      <McpServerSettings />
    </article>
  </section>
  {:else if activeSection === "files"}
  <section class="group">
    <header class="group-header">
      <h2>File resources</h2>
      <p>Pick a local file or folder to broker, then grant selected apps scoped access.</p>
    </header>
    <article
      class="card app-settings-panel"
      class:highlighted={highlightedAppId === "com.ma-zierl.host.file-broker"}
      tabindex="-1"
      use:registerAppSettingsPanel={"com.ma-zierl.host.file-broker"}
    >
      <FileResourcesSettings />
    </article>
  </section>
  {:else if activeSection === "trust"}
  <section class="group">
    <header class="group-header">
      <h2>Package trust</h2>
      <p>Trust or revoke exact app-id publisher keys. Signatures verify continuity, not safety.</p>
    </header>
    <article class="card">
      <PackageTrustSettings />
    </article>
  </section>
  {:else if activeSection === "apps"}
  <section class="group">
    <header class="group-header">
      <h2>Apps</h2>
      <p>Settings declared by installed apps appear here. Built-in settings stay in their task sections.</p>
    </header>
    <article class="card">
      {#if unavailableAppSettings}
        <div class="warning" role="status">
          <strong>Settings for {unavailableAppSettings.displayName} are unavailable.</strong>
          <p>This app must be active before Kestral can show its settings. Return to Apps, resolve its status, then try again.</p>
          <button type="button" onclick={() => currentTab.set("apps")}>Open Apps</button>
        </div>
      {/if}
      <div class="stack">
        {#each $apps.filter((app) => !dedicatedAppSettings[app.manifest.app_id]) as app}
          <div
            use:registerAppSettingsPanel={app.manifest.app_id}
            class="app-settings-panel"
            class:highlighted={highlightedAppId === app.manifest.app_id}
            tabindex="-1"
          >
            <AppSettingsPanel {app} />
          </div>
        {:else}
          <EmptyState title="No other app settings yet" message="Settings for other apps appear here after installation." />
        {/each}
      </div>
    </article>
  </section>
  {:else if activeSection === "permissions"}
  <section class="group">
    <header class="group-header">
      <h2>Permissions</h2>
      <p>Review and adjust what each app is allowed to do.</p>
    </header>
    <article class="card">
      {#if chatApp && missingChatCapabilities.length > 0}
        <div class="warning">
          <strong>Chat is missing permissions.</strong>
          <p>
            Missing: {missingChatCapabilities.map((capability) => capability.label).join(", ")}
          </p>
          <button type="button" disabled={restoringPermissions} onclick={() => void restoreChatPermissions()}>
            Restore permissions
          </button>
          {#if permissionsError}
            <p role="alert">{permissionsError}</p>
          {/if}
        </div>
      {/if}
      <GrantPolicyEditor />
    </article>
  </section>
  {:else}
  <section class="group advanced">
    <header class="group-header">
      <h2>Advanced</h2>
      <p>
        Developer features that can expose this device to other programs. Leave these alone
        unless you understand what they do.
      </p>
    </header>
    <article class="card">
      <details>
        <summary>
          <span class="summary-title">Share actions over MCP</span>
          <span class="summary-sub">Let an authenticated remote client call local actions</span>
        </summary>
        <div class="advanced-body">
          <McpExportSettings />
        </div>
      </details>
    </article>
  </section>
  {/if}
  </div>
</div>

<style>
  .settings-layout {
    min-width: 0;
    margin-top: 1rem;
    display: grid;
    grid-template-columns: minmax(9rem, 12rem) minmax(0, 1fr);
    gap: 1rem;
    align-items: start;
  }
  .settings-nav {
    min-width: 0;
    display: grid;
    gap: 0.25rem;
    position: sticky;
    top: 0;
  }
  .settings-nav-group {
    min-width: 0;
    display: grid;
    gap: 0.25rem;
  }
  .settings-nav-group + .settings-nav-group {
    margin-top: 0.85rem;
  }
  .settings-nav-label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0 0.75rem 0.2rem;
    color: var(--color-text);
    font-size: 0.72rem;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .settings-nav-label::after {
    content: "";
    height: 1px;
    flex: 1;
    background: var(--color-border);
  }
  .settings-nav button {
    width: 100%;
    min-width: 0;
    border: 1px solid transparent;
    border-radius: 10px;
    background: transparent;
    color: var(--color-text-muted);
    padding: 0.65rem 0.75rem;
    text-align: left;
    font: inherit;
  }
  .settings-nav button:hover,
  .settings-nav button.active {
    border-color: var(--color-border);
    background: var(--color-surface);
    color: var(--color-text);
  }
  .settings-nav button:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  .settings {
    min-width: 0;
    display: grid;
    gap: 1.75rem;
  }
  .group {
    min-width: 0;
    display: grid;
    gap: 0.7rem;
  }
  .group-header {
    display: grid;
    gap: 0.15rem;
    padding-bottom: 0.15rem;
    border-bottom: 1px solid var(--color-border-subtle);
  }
  .group-header h2 {
    margin: 0;
    font-size: 0.8rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--color-text-muted);
  }
  .group-header p {
    margin: 0;
    font-size: 0.85rem;
    color: var(--color-text-faint);
    overflow-wrap: anywhere;
  }
  .card {
    min-width: 0;
    max-width: 100%;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 18px;
    padding: 1rem 1.1rem;
  }
  .card h3 {
    margin-top: 0;
    font-size: 1.02rem;
  }
  /* The Advanced group carries a caution accent and stays collapsed so its
     reach (exposing local actions to remote clients) is a deliberate opt-in. */
  .advanced .group-header {
    border-bottom-color: var(--color-warning-border);
  }
  .advanced .card {
    border-color: var(--color-warning-border);
  }
  details > summary {
    cursor: pointer;
    display: grid;
    gap: 0.15rem;
  }
  .summary-title {
    font-weight: 600;
    color: var(--color-text);
  }
  .summary-sub {
    font-size: 0.82rem;
    color: var(--color-text-muted);
  }
  .advanced-body {
    margin-top: 0.9rem;
  }
  .stack {
    display: grid;
    gap: 0.8rem;
  }
  .app-settings-panel {
    min-width: 0;
    border-radius: 16px;
  }
  .app-settings-panel:focus-visible,
  .app-settings-panel.highlighted {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  .warning {
    min-width: 0;
    margin-bottom: 1rem;
    border: 1px solid var(--color-warning-border);
    background: var(--color-warning-soft);
    border-radius: 12px;
    padding: 0.85rem;
    display: grid;
    gap: 0.5rem;
  }
  .warning p {
    margin: 0;
    color: var(--color-warning-text);
    overflow-wrap: anywhere;
  }
  .warning button {
    width: fit-content;
    border: 1px solid var(--color-warning-border);
    border-radius: 8px;
    background: var(--color-surface-raised);
    padding: 0.45rem 0.75rem;
  }
  @media (max-width: 48em) {
    .settings-layout {
      grid-template-columns: minmax(0, 1fr);
    }
    .settings-nav {
      position: static;
      display: flex;
      overflow-x: auto;
      padding-bottom: 0.25rem;
    }
    .settings-nav-group {
      display: flex;
      flex: 0 0 auto;
    }
    .settings-nav-group + .settings-nav-group {
      margin-top: 0;
      margin-left: 0.35rem;
      padding-left: 0.35rem;
      border-left: 1px solid var(--color-border);
    }
    .settings-nav-label {
      display: none;
    }
    .settings-nav button {
      width: auto;
      flex: 0 0 auto;
      white-space: nowrap;
    }
  }
  @media (max-width: 30em) {
    .settings-nav {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(min(100%, 9rem), 1fr));
      align-items: start;
      gap: 0.75rem 0.5rem;
      overflow-x: visible;
    }
    .settings-nav-group {
      min-width: 0;
      display: grid;
    }
    .settings-nav-group + .settings-nav-group {
      margin-left: 0;
      padding-left: 0;
      border-left: 0;
    }
    .settings-nav-label {
      display: block;
    }
    .settings-nav button {
      flex: 1 1 7rem;
      white-space: normal;
      text-align: center;
    }
    .card {
      border-radius: 14px;
      padding: 0.8rem;
    }
    .warning button {
      width: 100%;
      max-width: 100%;
    }
  }
</style>
