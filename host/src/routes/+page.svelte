<script lang="ts">
  import { onMount } from "svelte";
  import AppsPage from "$lib/apps/AppsPage.svelte";
  import ChatSurface from "$lib/chat/ChatSurface.svelte";
  import HostShell from "$lib/shell/HostShell.svelte";
  import RemoteConnection from "$lib/shell/RemoteConnection.svelte";
  import {
    needsRemoteConnection,
    remoteConnectionAuthenticated,
    restoreRemoteConnection,
  } from "$lib/hostTransport";
  import SettingsPage from "$lib/settings/SettingsPage.svelte";
  import StuffPage from "$lib/stuff/StuffPage.svelte";
  import SystemPage from "$lib/system/SystemPage.svelte";
  import {
    bootstrapFailed,
    currentTab,
    initializeHost,
    shellError,
    startPolling,
    stopPolling,
    type Tab,
  } from "$lib/stores/hostState";

  let mounted = $state(false);
  const connected = $derived(!needsRemoteConnection() || $remoteConnectionAuthenticated);

  onMount(() => {
    let active = true;
    void (async () => {
      if (needsRemoteConnection()) await restoreRemoteConnection();
      if (!active) return;
      mounted = true;
    })();
    return () => {
      active = false;
      stopPolling();
    };
  });

  function remoteConnected() {
    // Authentication state is owned by hostTransport and updates `connected`.
  }

  async function hostReady() {
    await initializeHost();
    startPolling();
  }

  function selectTab(tab: Tab) {
    currentTab.set(tab);
  }
</script>

{#if !mounted}
  <div class="startup" aria-hidden="true"></div>
{:else if connected}
  <HostShell
    tab={$currentTab}
    error={$shellError}
    onRetry={$bootstrapFailed ? initializeHost : null}
    onSelectTab={selectTab}
    onReady={hostReady}
  >
    {#if $currentTab === "chat"}
      <ChatSurface />
    {:else if $currentTab === "apps"}
      <AppsPage />
    {:else if $currentTab === "stuff"}
      <StuffPage />
    {:else if $currentTab === "settings"}
      <SettingsPage />
    {:else}
      <SystemPage />
    {/if}
  </HostShell>
{:else}
  <RemoteConnection onConnected={remoteConnected} />
{/if}

<style>
  .startup {
    min-height: 100vh;
    min-height: 100dvh;
    background: var(--color-bg-gradient-b);
  }
</style>
