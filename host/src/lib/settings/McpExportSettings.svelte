<script lang="ts">
  import { onMount } from "svelte";
  import {
    deleteMcpExportProfile,
    hasMcpExportToken,
    listApps,
    listMcpExportProfiles,
    mcpExportRecentActivity,
    mcpGatewayStatus,
    revokeMcpExportToken,
    rotateMcpExportToken,
    setMcpExportEnabled,
    startMcpGateway,
    stopMcpGateway,
    upsertMcpExportProfile,
    type InstalledApp,
    type McpExportActivity,
    type McpExportInteraction,
    type McpExportProfileView,
    type McpGatewayStatus,
  } from "$lib/api";
  import { listenHostStateScope } from "$lib/hostTransport";

  let profiles = $state<McpExportProfileView[]>([]);
  let apps = $state<InstalledApp[]>([]);
  let gateway = $state<McpGatewayStatus>({ running: false, local_address: null });
  let activity = $state<McpExportActivity[]>([]);
  let tokenStatus = $state<Record<string, boolean>>({});
  let revealedToken = $state<string | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let editingId = $state<string | null>(null);
  let draft = $state(emptyProfile());

  // Rotating, revoking and deleting all take effect immediately against
  // whatever remote client is using the profile, and none of them can be
  // undone, so each is confirmed inline the way the rest of Settings does it.
  type PendingConfirm = { kind: "rotate" | "revoke" | "delete"; profileId: string };
  let pendingConfirm = $state<PendingConfirm | null>(null);

  function isConfirming(kind: PendingConfirm["kind"], profileId: string): boolean {
    return pendingConfirm?.kind === kind && pendingConfirm.profileId === profileId;
  }

  /// Focuses the safe choice when an inline confirm appears. Mirrors the
  /// pattern in McpServerSettings: only ever runs in direct response to the
  /// user's own click, so moving focus is expected rather than surprising.
  function focusOnMount(node: HTMLElement) {
    node.focus();
  }

  const capabilities = $derived(
    apps.flatMap((app) => app.manifest.capabilities.map((capability) => ({
      provider: app.manifest.app_id,
      providerName: app.manifest.display_name,
      capability: capability.name,
      description: capability.description,
    }))),
  );

  const interactionLabels: Record<McpExportProfileView["interaction"], string> = {
    "requires-approval": "requires local approval",
    notify: "allowed, with notice",
    silent: "allowed silently",
  };

  function emptyProfile(): McpExportProfileView {
    return {
      id: "",
      display_name: "",
      enabled: false,
      capabilities: [],
      interaction: "requires-approval",
      expires_after_seconds: null,
      rate_limit_per_minute: 30,
      expose_results: false,
      expose_artifacts: false,
    };
  }

  onMount(() => {
    void load();
    const unlisten = listenHostStateScope("mcp-export", () => void load());
    return () => void unlisten.then((stop) => stop());
  });

  async function load() {
    try {
      [profiles, apps, gateway] = await Promise.all([
        listMcpExportProfiles(),
        listApps(),
        mcpGatewayStatus(),
      ]);
      tokenStatus = Object.fromEntries(
        await Promise.all(profiles.map(async (profile) => [profile.id, await hasMcpExportToken(profile.id)])),
      );
      await refreshActivity();
    } catch (failure) {
      error = String(failure);
    }
  }

  async function refreshActivity() {
    try {
      // Newest first for the activity view.
      activity = [...(await mcpExportRecentActivity())].reverse();
    } catch (failure) {
      error = String(failure);
    }
  }

  function activitySummary(entry: McpExportActivity): string {
    const parts = [entry.event.replace(/-/g, " ")];
    if (entry.tool) parts.push(entry.tool);
    if (entry.outcome) parts.push(entry.outcome);
    if (entry.profile) parts.push(`(${entry.profile})`);
    return parts.join(" · ");
  }

  function activityTime(iso: string): string {
    const parsed = new Date(iso);
    return Number.isNaN(parsed.getTime()) ? iso : parsed.toLocaleTimeString();
  }

  function edit(profile: McpExportProfileView) {
    editingId = profile.id;
    draft = structuredClone(profile);
    revealedToken = null;
    error = null;
  }

  function create() {
    editingId = "";
    draft = emptyProfile();
    revealedToken = null;
    error = null;
  }

  function selected(provider: string, capability: string): boolean {
    return draft.capabilities.some((item) => item.provider === provider && item.capability === capability);
  }

  function toggleCapability(provider: string, capability: string) {
    if (selected(provider, capability)) {
      draft.capabilities = draft.capabilities.filter(
        (item) => item.provider !== provider || item.capability !== capability,
      );
    } else {
      draft.capabilities = [...draft.capabilities, { provider, capability }];
    }
  }

  async function save() {
    busy = true;
    error = null;
    try {
      const saved = await upsertMcpExportProfile({
        ...draft,
        id: draft.id.trim(),
        display_name: draft.display_name.trim(),
        // New profiles start disabled (they have no credential yet). Editing an
        // existing profile preserves its current on/off state, so saving a
        // detail no longer silently takes a live profile offline.
        enabled: editingId === "" ? false : draft.enabled,
      });
      editingId = null;
      await load();
      edit(saved);
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }

  async function setEnabled(profile: McpExportProfileView, enabled: boolean) {
    busy = true;
    error = null;
    try {
      if (enabled && !tokenStatus[profile.id]) {
        throw new Error("Generate a credential before enabling this profile.");
      }
      await setMcpExportEnabled(profile.id, enabled);
      await load();
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }

  async function rotate(profileId: string) {
    pendingConfirm = null;
    try {
      revealedToken = await rotateMcpExportToken(profileId);
      tokenStatus[profileId] = true;
    } catch (failure) {
      error = String(failure);
    }
  }

  async function revoke(profileId: string) {
    pendingConfirm = null;
    try {
      await revokeMcpExportToken(profileId);
      tokenStatus[profileId] = false;
      revealedToken = null;
    } catch (failure) {
      error = String(failure);
    }
  }

  async function remove(profileId: string) {
    pendingConfirm = null;
    try {
      await deleteMcpExportProfile(profileId);
      if (editingId === profileId) editingId = null;
      await load();
    } catch (failure) {
      error = String(failure);
    }
  }

  async function toggleGateway() {
    try {
      gateway = gateway.running
        ? (await stopMcpGateway(), { running: false, local_address: null })
        : await startMcpGateway();
    } catch (failure) {
      error = String(failure);
    }
  }
</script>

<div class="stack">
  <p class="explanation">
    Private/developer bearer-token access only. The gateway binds to localhost; a Cloudflare Tunnel
    may provide transport, but it does not replace authentication. OAuth is not implemented.
  </p>
  <div class="gateway-row">
    <div>
      <strong>{gateway.running ? "Gateway running" : "Gateway stopped"}</strong>
      <p>{gateway.local_address ? `http://${gateway.local_address}/mcp` : "No endpoint is listening."}</p>
    </div>
    <button type="button" onclick={toggleGateway}>{gateway.running ? "Stop gateway" : "Start gateway"}</button>
  </div>

  <div class="profile-grid">
    {#each profiles as profile (profile.id)}
      <section class="profile">
        <div class="profile-head">
          <div><strong>{profile.display_name}</strong><code>{profile.id}</code></div>
          <span>{profile.enabled ? "Enabled" : "Disabled"}</span>
        </div>
        <p>{profile.capabilities.length} exact action{profile.capabilities.length === 1 ? "" : "s"}; {interactionLabels[profile.interaction]}</p>
        {#if profile.expose_results || profile.expose_artifacts}
          <p class="warning">Remote clients may receive local result or artifact information.</p>
        {/if}
        <div class="actions">
          <button type="button" onclick={() => edit(profile)}>Edit</button>

          {#if !tokenStatus[profile.id]}
            <button type="button" onclick={() => rotate(profile.id)}>Generate credential</button>
          {:else if isConfirming("rotate", profile.id)}
            <span class="confirm-inline">
              Replace this credential? Any client still using the old one stops working immediately.
              <button type="button" class="danger" onclick={() => rotate(profile.id)}>Rotate</button>
              <button type="button" use:focusOnMount onclick={() => (pendingConfirm = null)}>Keep</button>
            </span>
          {:else}
            <button
              type="button"
              onclick={() => (pendingConfirm = { kind: "rotate", profileId: profile.id })}
            >Rotate credential</button>
          {/if}

          {#if tokenStatus[profile.id]}
            {#if isConfirming("revoke", profile.id)}
              <span class="confirm-inline">
                Revoke this credential? Remote clients lose access to {profile.display_name} at once.
                <button type="button" class="danger" onclick={() => revoke(profile.id)}>Revoke</button>
                <button type="button" use:focusOnMount onclick={() => (pendingConfirm = null)}>Keep</button>
              </span>
            {:else}
              <button
                type="button"
                onclick={() => (pendingConfirm = { kind: "revoke", profileId: profile.id })}
              >Revoke credential</button>
            {/if}
          {/if}

          <button type="button" disabled={busy} onclick={() => setEnabled(profile, !profile.enabled)}>{profile.enabled ? "Disable" : "Enable"}</button>

          {#if isConfirming("delete", profile.id)}
            <span class="confirm-inline">
              Delete {profile.display_name}? Its credential and exported actions are removed for good.
              <button type="button" class="danger" onclick={() => remove(profile.id)}>Delete</button>
              <button type="button" use:focusOnMount onclick={() => (pendingConfirm = null)}>Keep</button>
            </span>
          {:else}
            <button
              type="button"
              class="danger"
              onclick={() => (pendingConfirm = { kind: "delete", profileId: profile.id })}
            >Delete</button>
          {/if}
        </div>
      </section>
    {:else}
      <p>No export profiles. Nothing is remotely reachable.</p>
    {/each}
  </div>
  <button type="button" onclick={create}>Create export profile</button>

  {#if editingId !== null}
    <form class="editor" onsubmit={(event) => { event.preventDefault(); void save(); }}>
      <h3>{editingId === "" ? "New export profile" : "Edit export profile"}</h3>
      <label>Profile ID <input bind:value={draft.id} disabled={editingId !== ""} required pattern="[a-z0-9][a-z0-9-]*" /></label>
      <label>Display name <input bind:value={draft.display_name} required /></label>
      <label>Call policy
        <select bind:value={draft.interaction}>
          <option value="requires-approval">Require local approval</option>
          <option value="notify">Allow and notify</option>
          <option value="silent">Allow silently</option>
        </select>
      </label>
      <label>Rate limit per minute <input type="number" min="1" max="600" bind:value={draft.rate_limit_per_minute} /></label>
      <fieldset>
        <legend>Exact actions</legend>
        {#each capabilities as item (`${item.provider}/${item.capability}`)}
          <label class="check"><input type="checkbox" checked={selected(item.provider, item.capability)} onchange={() => toggleCapability(item.provider, item.capability)} /> <span><strong>{item.providerName}: {item.capability}</strong><small>{item.description}</small></span></label>
        {/each}
      </fieldset>
      <label class="check"><input type="checkbox" bind:checked={draft.expose_results} /> Return full action results</label>
      <label class="check"><input type="checkbox" bind:checked={draft.expose_artifacts} /> Return artifact references</label>
      {#if draft.expose_results || draft.expose_artifacts}<p class="warning" role="alert">These responses may expose local data to the authenticated remote client.</p>{/if}
      <div class="actions"><button type="submit" disabled={busy}>{editingId === "" ? "Save (starts disabled)" : "Save changes"}</button><button type="button" onclick={() => (editingId = null)}>Cancel</button></div>
    </form>
  {/if}

  {#if revealedToken}
    <div class="token" role="status"><strong>Credential shown once</strong><code>{revealedToken}</code><p>Store it now. The host will not display it again.</p></div>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <section class="activity">
    <div class="activity-head">
      <strong>Recent remote activity</strong>
      <button type="button" class="link" onclick={() => void refreshActivity()}>Refresh</button>
    </div>
    {#if activity.length === 0}
      <p class="telemetry">No remote calls this session. Activity appears here when a client connects.</p>
    {:else}
      <ul class="activity-list">
        {#each activity as entry, index (index)}
          <li class:failed={entry.outcome === "failed" || entry.event === "auth-failed"}>
            <span class="activity-time">{activityTime(entry.at)}</span>
            <span class="activity-detail">{activitySummary(entry)}</span>
          </li>
        {/each}
      </ul>
    {/if}
    <p class="telemetry">In-memory and session-scoped. The full audit trail is written to mcp-gateway-audit.jsonl.</p>
  </section>
</div>

<style>
  .stack, .editor, .profile, .token { display: grid; gap: 0.75rem; }
  .explanation, .profile p, .gateway-row p, .telemetry { margin: 0; color: var(--color-text-muted); }
  .gateway-row, .profile-head, .actions { display: flex; gap: 0.65rem; align-items: center; justify-content: space-between; flex-wrap: wrap; }
  .profile-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 20rem), 1fr)); gap: 0.75rem; }
  .profile, .editor, .token { border: 1px solid var(--color-border); border-radius: 12px; padding: 0.85rem; }
  .profile-head div, label { display: grid; gap: 0.25rem; }
  code { overflow-wrap: anywhere; }
  input, select { min-width: 0; border: 1px solid var(--color-border-strong); border-radius: 8px; padding: 0.55rem; }
  fieldset { display: grid; gap: 0.5rem; border: 1px solid var(--color-border); border-radius: 10px; }
  .check { grid-template-columns: auto minmax(0, 1fr); align-items: start; }
  .check span { display: grid; }
  small { color: var(--color-text-muted); }
  button { width: fit-content; min-height: 2rem; border: 1px solid var(--color-border-strong); border-radius: 8px; background: var(--color-surface-raised); color: var(--color-text); padding: 0.4rem 0.7rem; }
  .danger, .error { color: var(--color-danger-text); }
  .warning { margin: 0; padding: 0.6rem; border: 1px solid var(--color-warning-border); border-radius: 8px; background: var(--color-warning-soft); color: var(--color-warning-text); }
  /* Wraps rather than pushing the action row into horizontal overflow when the
     consequence sentence is long or the panel is narrow. */
  .confirm-inline { display: inline-flex; flex-wrap: wrap; gap: 0.4rem; align-items: center; color: var(--color-warning-text); font-size: 0.85rem; }
  .token code { user-select: all; padding: 0.5rem; background: var(--color-surface-muted); }
  .activity { display: grid; gap: 0.5rem; }
  .activity-head { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; }
  .link { border: none; background: transparent; color: var(--color-accent); padding: 0; min-height: 0; width: auto; cursor: pointer; }
  .activity-list { list-style: none; margin: 0; padding: 0; display: grid; gap: 0.2rem; max-height: 14rem; overflow-y: auto; }
  .activity-list li { display: flex; gap: 0.6rem; align-items: baseline; font-size: 0.85rem; padding: 0.3rem 0.4rem; border-radius: 6px; background: var(--color-surface-muted); }
  .activity-list li.failed { background: var(--color-warning-soft); color: var(--color-warning-text); }
  .activity-time { color: var(--color-text-faint); font-variant-numeric: tabular-nums; flex-shrink: 0; }
  .activity-detail { overflow-wrap: anywhere; }
</style>
