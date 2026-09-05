<script lang="ts">
  import { onDestroy, tick } from "svelte";
  import type {
    GrantCondition,
    GrantDuration,
    GrantEditorRequest,
    GrantRequest,
    GrantScope,
    GrantView,
  } from "$lib/api";
  import GrantTable from "$lib/grants/GrantTable.svelte";
  import LoadingIndicator from "$lib/shell/LoadingIndicator.svelte";
  import { apps, appsLoaded } from "$lib/stores/apps";
  import { grants, grantsLoaded } from "$lib/stores/grants";
  import {
    issueEditorGrantAndRefresh,
    replaceGrantAndRefresh,
    requestManifestGrantAndRefresh,
    revokeGrantAndRefresh,
  } from "$lib/stores/grants";
  import { dataScopeLabel } from "$lib/system/dataScopeLabel";
  import {
    conditionLabel,
    groupPermissions,
    scopeKey,
    type PermissionEntry,
  } from "$lib/settings/permissionsModel";
  import { parseExpiry, secondsToExpiry, type ExpiryUnit } from "$lib/settings/grantExpiry";
  import { permissionTarget } from "$lib/stores/navigation";
  import { scrollTargetIntoView } from "$lib/a11y/scroll";
  import ActionIcon from "$lib/settings/ActionIcon.svelte";

  const ARTIFACTS_APP_ID = "com.ma-zierl.kestral-artifacts";

  const groups = $derived(groupPermissions($grants, $apps));
  let handledTargetRequest = 0;
  let highlightedGrantId = $state<string | null>(null);
  let highlightedAppId = $state<string | null>(null);
  let highlightTimer: ReturnType<typeof setTimeout> | null = null;
  const grantRows = new Map<string, HTMLElement>();
  const appGroups = new Map<string, HTMLDetailsElement>();

  function entryGrantIds(entry: PermissionEntry): string[] {
    return [entry.current.grant_id, ...entry.history.map((grant) => grant.grant_id)];
  }

  function registerGrantRow(node: HTMLElement, grantIds: string[]) {
    let registered = grantIds;
    const add = (ids: string[]) => ids.forEach((grantId) => grantRows.set(grantId, node));
    const remove = (ids: string[]) => ids.forEach((grantId) => {
      if (grantRows.get(grantId) === node) grantRows.delete(grantId);
    });
    add(registered);
    return {
      update(next: string[]) {
        remove(registered);
        registered = next;
        add(registered);
      },
      destroy() {
        remove(registered);
      },
    };
  }

  function registerAppGroup(node: HTMLDetailsElement, appId: string) {
    appGroups.set(appId, node);
    return {
      update(nextAppId: string) {
        if (appGroups.get(appId) === node) appGroups.delete(appId);
        appId = nextAppId;
        appGroups.set(appId, node);
      },
      destroy() {
        if (appGroups.get(appId) === node) appGroups.delete(appId);
      },
    };
  }

  $effect(() => {
    const target = $permissionTarget;
    if (!target || target.request === handledTargetRequest) return;
    if (target.kind === "grant") {
      const match = groups
        .flatMap((group) => group.entries)
        .find((entry) => entryGrantIds(entry).includes(target.grantId));
      if (!match) return;
    } else if (!groups.some((group) => group.appId === target.appId)) {
      return;
    }

    handledTargetRequest = target.request;
    highlightedGrantId = target.kind === "grant" ? target.grantId : null;
    highlightedAppId = target.kind === "app" ? target.appId : null;
    if (highlightTimer) clearTimeout(highlightTimer);
    void tick().then(() => {
      const row = target.kind === "grant" ? grantRows.get(target.grantId) : undefined;
      const group = target.kind === "app"
        ? appGroups.get(target.appId)
        : row?.closest("details");
      if (group) group.open = true;
      const focusTarget = row ?? group ?? null;
      focusTarget?.focus({ preventScroll: true });
      scrollTargetIntoView(focusTarget);
    });
    highlightTimer = setTimeout(() => {
      highlightedGrantId = null;
      highlightedAppId = null;
      permissionTarget.update((current) => current?.request === target.request ? null : current);
    }, 3000);
  });

  onDestroy(() => {
    if (highlightTimer) clearTimeout(highlightTimer);
  });

  function providerName(scope: GrantScope): string {
    const app = $apps.find((candidate) => candidate.manifest.app_id === scope.provider);
    return app?.manifest.display_name ?? scope.provider;
  }

  function actionTitle(scope: GrantScope): string {
    return scope.kind === "exact-capability"
      ? `${providerName(scope)}: ${scope.capability}`
      : `All ${providerName(scope)} capabilities`;
  }

  function permissionActionTitle(entry: PermissionEntry): string {
    const title = actionTitle(entry.scope);
    return entry.current.data_scope.kind === "resources"
      ? `${title} for ${dataAccessLabel(entry.scope.provider, entry.current.data_scope)}`
      : title;
  }

  function dataAccessLabel(provider: string, scope: GrantView["data_scope"]): string {
    if (provider !== ARTIFACTS_APP_ID) return dataScopeLabel(scope);
    if (scope.kind === "none") return "No artifact access";
    if (scope.kind === "all-resources") return "All current and future artifacts";
    return `${scope.resource_ids.length} selected ${scope.resource_ids.length === 1 ? "artifact" : "artifacts"}`;
  }

  function expiryText(grant: GrantView): string {
    if (grant.status !== "active") {
      return `Issued ${new Date(grant.issued_at).toLocaleString()}`;
    }
    return grant.expires_at
      ? `Expires ${new Date(grant.expires_at).toLocaleString()}`
      : "Never expires";
  }

  function entryKey(appId: string, entry: PermissionEntry): string {
    return `${appId}::${scopeKey(entry.scope, entry.current.data_scope)}`;
  }

  // -- Row actions (edit in place / revoke / grant again) --------------------

  // Busy state is tracked per row (by entry key), not globally, so acting on
  // one permission never disables or freezes the others. The custom-permission
  // builder has its own flag for the same reason.
  let busyKeys = $state(new Set<string>());
  let customBusy = $state(false);
  let rowError = $state<{ key: string; message: string } | null>(null);

  // The inline editor: opened for exactly one row, either replacing an active
  // grant or re-granting an inactive one.
  let editingKey = $state<string | null>(null);
  let editingMode = $state<"replace" | "regrant">("replace");
  let editCondition = $state<GrantCondition>("requires-approval");
  let editMcpAcknowledged = $state(false);
  let editExpiryValue = $state("");
  let editExpiryUnit = $state<ExpiryUnit>("hours");
  let editAllArtifacts = $state(false);

  // Revoke is destructive (the app loses access immediately), so it goes
  // through the same lightweight inline confirm as other one-click destructive
  // actions in Settings: only one row confirms at a time, the safe choice
  // ("Keep") gets default focus, and Escape cancels.
  let confirmingRevokeKey = $state<string | null>(null);

  // Moves focus into dynamically revealed controls (the inline revoke
  // confirm's safe "Keep" choice, the inline editor's first field) without
  // the `autofocus` attribute (flagged by svelte-check's a11y rule).
  function focusOnMount(node: HTMLElement) {
    node.focus();
  }

  function cancelRevokeOnEscape(event: KeyboardEvent) {
    if (event.key === "Escape") {
      confirmingRevokeKey = null;
    }
  }

  function openEditor(appId: string, entry: PermissionEntry, mode: "replace" | "regrant") {
    editingKey = entryKey(appId, entry);
    editingMode = mode;
    editCondition = entry.current.condition;
    editMcpAcknowledged = false;
    editAllArtifacts = entry.current.data_scope.kind === "all-resources";
    if (entry.current.status === "active" && entry.current.expires_at) {
      const seconds = Math.max(
        1,
        Math.round((Date.parse(entry.current.expires_at) - Date.now()) / 1000),
      );
      const expiry = secondsToExpiry(seconds);
      editExpiryValue = expiry.value;
      editExpiryUnit = expiry.unit;
    } else {
      editExpiryValue = "";
      editExpiryUnit = "hours";
    }
    rowError = null;
  }

  function closeEditor() {
    editingKey = null;
    rowError = null;
  }

  // Escape closes the inline editor from anywhere inside it (listening on the
  // form itself trips svelte-check's noninteractive-interaction a11y rule).
  $effect(() => {
    if (editingKey === null) return;
    const onKeydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeEditor();
    };
    window.addEventListener("keydown", onKeydown);
    return () => window.removeEventListener("keydown", onKeydown);
  });

  function editorDuration(): GrantDuration | null {
    const parsed = parseExpiry(editExpiryValue, editExpiryUnit);
    if (parsed.kind === "never") return { kind: "non-expiring" };
    if (parsed.kind === "invalid") return null;
    return { kind: "expires-after", seconds: parsed.seconds };
  }

  async function runRowAction(key: string, action: () => Promise<void>) {
    busyKeys.add(key);
    rowError = null;
    try {
      await action();
    } catch (caught) {
      rowError = { key, message: caught instanceof Error ? caught.message : String(caught) };
    } finally {
      busyKeys.delete(key);
    }
  }

  async function applyEditor(appId: string, entry: PermissionEntry) {
    const key = entryKey(appId, entry);
    const duration = editorDuration();
    if (!duration) {
      rowError = { key, message: "Enter a whole number of minutes, hours, or days — or leave empty for never." };
      return;
    }
    const dataScope = entry.scope.provider === ARTIFACTS_APP_ID && editAllArtifacts
      ? { kind: "all-resources" as const }
      : entry.current.data_scope;
    if (entry.scope.provider === ARTIFACTS_APP_ID && dataScope.kind === "none") {
      rowError = {
        key,
        message: "Choose all artifacts, or open Artifacts and allow individual items.",
      };
      return;
    }
    const request: GrantEditorRequest = {
      holder: appId,
      scope: entry.scope,
      condition: editCondition,
      duration,
      reason:
        editingMode === "replace"
          ? "Adjusted from the permissions page"
          : "Granted again from the permissions page",
      allow_all_provider_scope: entry.scope.kind === "all-provider-capabilities",
      acknowledge_less_interactive_mcp: editMcpAcknowledged,
      data_scope: dataScope,
    };
    await runRowAction(key, async () => {
      if (editingMode === "replace") {
        await replaceGrantAndRefresh(entry.current.grant_id, request);
      } else {
        await issueEditorGrantAndRefresh(request);
      }
      editingKey = null;
    });
  }

  async function revoke(appId: string, entry: PermissionEntry) {
    await runRowAction(entryKey(appId, entry), () => revokeGrantAndRefresh(entry.current.grant_id));
  }

  async function grantAgain(appId: string, entry: PermissionEntry) {
    if (entry.declared) {
      await runRowAction(entryKey(appId, entry), () =>
        requestManifestGrantAndRefresh(appId, entry.declared as GrantRequest),
      );
    } else {
      openEditor(appId, entry, "regrant");
    }
  }

  async function grantDeclared(appId: string, request: GrantRequest) {
    await runRowAction(`${appId}::declared::${scopeKey(request.scope, request.data_scope)}`, () =>
      requestManifestGrantAndRefresh(appId, request),
    );
  }

  // -- Custom permission builder (collapsed; for permissions no manifest asks for)

  let customHolder = $state("");
  let customProvider = $state("");
  let customCapability = $state("");
  let customCondition = $state<GrantCondition>("requires-approval");
  let customMcpAcknowledged = $state(false);
  let customExpiryValue = $state("");
  let customExpiryUnit = $state<ExpiryUnit>("hours");
  let customReason = $state("");
  let customAllProvider = $state(false);
  let customAllArtifacts = $state(false);
  let customError = $state("");

  $effect(() => {
    if (!customHolder && $apps[0]) customHolder = $apps[0].manifest.app_id;
    if (!customProvider && $apps[0]) customProvider = $apps[0].manifest.app_id;
  });
  const customCapabilities = $derived(
    $apps.find((app) => app.manifest.app_id === customProvider)?.manifest.capabilities ?? [],
  );
  $effect(() => {
    if (!customCapabilities.some((item) => item.name === customCapability)) {
      customCapability = customCapabilities[0]?.name ?? "";
    }
  });
  $effect(() => {
    if (customProvider !== ARTIFACTS_APP_ID) customAllArtifacts = false;
  });

  async function submitCustom() {
    const parsed = parseExpiry(customExpiryValue, customExpiryUnit);
    const duration: GrantDuration | null =
      parsed.kind === "never"
        ? { kind: "non-expiring" }
        : parsed.kind === "seconds"
          ? { kind: "expires-after", seconds: parsed.seconds }
          : null;
    if (customProvider === ARTIFACTS_APP_ID && !customAllArtifacts) {
      customError = "Choose access to all artifacts, or use Artifacts to allow individual items.";
      return;
    }
    if (
      !customHolder ||
      !customProvider ||
      (!customAllProvider && !customCapability) ||
      !duration
    ) {
      customError = "Choose installed apps, an exact capability, and a valid expiry.";
      return;
    }
    const scope: GrantScope = customAllProvider
      ? { kind: "all-provider-capabilities", provider: customProvider }
      : { kind: "exact-capability", provider: customProvider, capability: customCapability };
    customBusy = true;
    customError = "";
    try {
      await issueEditorGrantAndRefresh({
        holder: customHolder,
        scope,
        data_scope: customAllArtifacts ? { kind: "all-resources" } : { kind: "none" },
        condition: customCondition,
        duration,
        reason: customReason.trim(),
        allow_all_provider_scope: customAllProvider,
        acknowledge_less_interactive_mcp: customMcpAcknowledged,
      });
      customReason = "";
      customExpiryValue = "";
      customAllProvider = false;
      customAllArtifacts = false;
      customCondition = "requires-approval";
      customMcpAcknowledged = false;
    } catch (caught) {
      customError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      customBusy = false;
    }
  }
</script>

<section class="permissions" aria-label="Permissions">
  {#if !$appsLoaded || !$grantsLoaded}
    <LoadingIndicator fill label="Loading permissions…" />
  {:else}
  {#each groups as group (group.appId)}
    {@const activeCount = group.entries.filter((entry) => entry.current.status === "active").length}
    {@const totalCount = group.entries.length + group.neverGranted.length}
    <details
      class="app-group"
      class:highlighted={highlightedAppId === group.appId}
      aria-label={`Permissions held by ${group.displayName}`}
      tabindex="-1"
      use:registerAppGroup={group.appId}
    >
      <summary>
        <span class="summary-main">
          <span class="app-name">{group.displayName}</span>
          <span class="app-count">{activeCount} active{totalCount !== activeCount ? ` · ${totalCount - activeCount} inactive` : ""}</span>
        </span>
      </summary>
      <ul class="entries">
        {#each group.entries as entry (entryKey(group.appId, entry))}
          {@const key = entryKey(group.appId, entry)}
          {@const active = entry.current.status === "active"}
          <li
            class="entry"
            class:inactive={!active}
            class:highlighted={entryGrantIds(entry).includes(highlightedGrantId ?? "")}
            use:registerGrantRow={entryGrantIds(entry)}
            tabindex="-1"
          >
            <div class="entry-row">
              <div class="what">
                <strong>{actionTitle(entry.scope)}</strong>
                <span class="muted">{dataAccessLabel(entry.scope.provider, entry.current.data_scope)}</span>
              </div>
              <div class="how">
                {#if active}
                  <span class="badge condition">{conditionLabel(entry.current.condition)}</span>
                {:else}
                  <span class="badge off">{entry.current.status === "expired" ? "Expired" : "Revoked"}</span>
                {/if}
                <span class="muted">{expiryText(entry.current)}</span>
              </div>
              <div class="entry-actions">
                {#if active}
                  {#if confirmingRevokeKey === key}
                    <span class="confirm-inline">
                      Revoke access?
                      <button
                        type="button"
                        class="danger"
                        disabled={busyKeys.has(key)}
                        onclick={() => {
                          confirmingRevokeKey = null;
                          void revoke(group.appId, entry);
                        }}
                        onkeydown={cancelRevokeOnEscape}
                      >Revoke</button>
                      <button
                        type="button"
                        use:focusOnMount
                        onclick={() => (confirmingRevokeKey = null)}
                        onkeydown={cancelRevokeOnEscape}
                      >
                        Keep
                      </button>
                    </span>
                  {:else}
                    <button
                      type="button"
                      class="icon-button"
                      disabled={busyKeys.has(key)}
                      aria-expanded={editingKey === key}
                      onclick={() =>
                        editingKey === key ? closeEditor() : openEditor(group.appId, entry, "replace")}
                      aria-label={`Edit ${permissionActionTitle(entry)} for ${group.displayName}`}
                      title="Edit permission"
                    ><ActionIcon name="edit" /></button>
                    <button
                      type="button"
                      class="danger"
                      disabled={busyKeys.has(key)}
                      onclick={() => (confirmingRevokeKey = key)}
                      aria-label={`Revoke ${permissionActionTitle(entry)} for ${group.displayName}`}
                    >Revoke</button>
                  {/if}
                {:else}
                  <button
                    type="button"
                    disabled={busyKeys.has(key)}
                    onclick={() => void grantAgain(group.appId, entry)}
                    aria-label={`Grant ${permissionActionTitle(entry)} for ${group.displayName} again`}
                  >Grant again</button>
                {/if}
              </div>
            </div>

            {#if editingKey === key}
              <form
                class="inline-editor"
                aria-label={`Edit ${permissionActionTitle(entry)}`}
                onsubmit={(event) => {
                  event.preventDefault();
                  void applyEditor(group.appId, entry);
                }}
              >
                {#if entry.scope.provider === ARTIFACTS_APP_ID}
                  <label class="artifact-access">
                    <input bind:checked={editAllArtifacts} type="checkbox" disabled={busyKeys.has(key)} />
                    <span>
                      <strong>All current and future artifacts</strong><br />
                      {#if entry.current.data_scope.kind === "resources"}
                        Leave unchecked to keep access to the selected artifacts only.
                      {:else if entry.current.data_scope.kind === "none"}
                        This permission currently allows actions but no artifacts. Select this option, or use Artifacts to choose individual items.
                      {:else}
                        Chat or another app may list and read artifacts created later too.
                      {/if}
                    </span>
                  </label>
                {/if}
                <label>
                  Approval
                  <select bind:value={editCondition} disabled={busyKeys.has(key)} use:focusOnMount>
                    <option value="silent">Runs silently</option>
                    <option value="notify">Notifies on delegated use</option>
                    <option value="requires-approval">Approval for delegated/high-impact use</option>
                  </select>
                </label>
                {#if entry.scope.provider.startsWith("mcp-") && editCondition !== "requires-approval"}
                  <label class="mcp-warning">
                    <input bind:checked={editMcpAcknowledged} type="checkbox" disabled={busyKeys.has(key)} />
                    <span>
                      Future Chat and LLM-driven calls may proceed without asking first. Tool descriptions,
                      conversation content, and external data can influence those calls.
                    </span>
                  </label>
                {/if}
                <label>
                  Expires after
                  <span class="expiry">
                    <input
                      bind:value={editExpiryValue}
                      type="number"
                      min="1"
                      step="1"
                      placeholder="Never"
                      disabled={busyKeys.has(key)}
                      aria-label="expiry amount"
                    />
                    <select bind:value={editExpiryUnit} disabled={busyKeys.has(key)} aria-label="expiry unit">
                      <option value="minutes">minutes</option>
                      <option value="hours">hours</option>
                      <option value="days">days</option>
                    </select>
                  </span>
                </label>
                <span class="editor-actions">
                  <button
                    type="submit"
                    class="primary"
                    disabled={busyKeys.has(key) ||
                      (entry.scope.provider.startsWith("mcp-") &&
                        editCondition !== "requires-approval" &&
                        !editMcpAcknowledged)}
                  >
                    {editingMode === "replace" ? "Apply change" : "Grant permission"}
                  </button>
                  <button type="button" disabled={busyKeys.has(key)} onclick={closeEditor}>Cancel</button>
                </span>
              </form>
            {/if}

            {#if rowError?.key === key}
              <p class="error" role="alert">{rowError.message}</p>
            {/if}

            {#if entry.history.length > 0}
              <details class="history">
                <summary>{entry.history.length} earlier audit {entry.history.length === 1 ? "record" : "records"}</summary>
                <ul>
                  {#each entry.history as fact (fact.grant_id)}
                    <li>
                      <span class="badge off small">{fact.status}</span>
                      {conditionLabel(fact.condition)} · issued {new Date(fact.issued_at).toLocaleString()}
                    </li>
                  {/each}
                </ul>
              </details>
            {/if}
          </li>
        {/each}

        {#each group.neverGranted as request (scopeKey(request.scope, request.data_scope))}
          {@const key = `${group.appId}::declared::${scopeKey(request.scope, request.data_scope)}`}
          <li class="entry inactive">
            <div class="entry-row">
              <div class="what">
                <strong>{actionTitle(request.scope)}</strong>
                <span class="muted">{request.reason}</span>
              </div>
              <div class="how">
                <span class="badge off">Not granted</span>
                <span class="muted">Would be: {conditionLabel(request.condition)}</span>
                <span class="muted">{dataAccessLabel(request.scope.provider, request.data_scope)}</span>
              </div>
              <div class="entry-actions">
                <button
                  type="button"
                  disabled={busyKeys.has(key)}
                  onclick={() => void grantDeclared(group.appId, request)}
                  aria-label={`Grant ${actionTitle(request.scope)} for ${group.displayName}`}
                >Grant</button>
              </div>
            </div>
            {#if rowError?.key === key}
              <p class="error" role="alert">{rowError.message}</p>
            {/if}
          </li>
        {/each}
      </ul>
    </details>
  {:else}
    <p class="muted">No apps are installed yet, so there are no permissions to manage.</p>
  {/each}
  {/if}

  <details class="expander">
    <summary>Add permission</summary>
    <form
      class="custom-form"
      onsubmit={(event) => {
        event.preventDefault();
        void submitCustom();
      }}
    >
      <div class="form-grid">
        <label>App that gets access
          <select bind:value={customHolder} disabled={customBusy}>
            {#each $apps as app}<option value={app.manifest.app_id}>{app.manifest.display_name}</option>{/each}
          </select>
        </label>
        <label>App that provides the action
          <select bind:value={customProvider} disabled={customBusy}>
            {#each $apps as app}<option value={app.manifest.app_id}>{app.manifest.display_name}</option>{/each}
          </select>
        </label>
        {#if !customAllProvider}
          <label>Capability
            <select bind:value={customCapability} disabled={customBusy}>
              {#each customCapabilities as item}<option value={item.name}>{item.name}</option>{/each}
            </select>
          </label>
        {:else}
          <div class="scope-summary">
            <span>Capability scope</span>
            <strong>All current and future capabilities</strong>
          </div>
        {/if}
        {#if customProvider === ARTIFACTS_APP_ID}
          <label class="artifact-access">
            <input bind:checked={customAllArtifacts} type="checkbox" disabled={customBusy} />
            <span>
              <strong>All current and future artifacts</strong><br />
              Selected-artifact shortcuts are available for Chat on the Artifacts page.
            </span>
          </label>
        {/if}
        <label>Approval
          <select bind:value={customCondition} disabled={customBusy}>
            <option value="silent">Runs silently</option>
            <option value="notify">Notifies on delegated use</option>
            <option value="requires-approval">Approval for delegated/high-impact use</option>
          </select>
        </label>
        {#if customProvider.startsWith("mcp-") && customCondition !== "requires-approval"}
          <label class="mcp-warning">
            <input bind:checked={customMcpAcknowledged} type="checkbox" disabled={customBusy} />
            <span>
              Future Chat and LLM-driven calls may proceed without asking first. Tool descriptions,
              conversation content, and external data can influence those calls.
            </span>
          </label>
        {/if}
        <label>Expires after
          <span class="expiry">
            <input
              bind:value={customExpiryValue}
              type="number"
              min="1"
              step="1"
              placeholder="Never"
              disabled={customBusy}
              aria-label="expiry amount"
            />
            <select bind:value={customExpiryUnit} disabled={customBusy} aria-label="expiry unit">
              <option value="minutes">minutes</option>
              <option value="hours">hours</option>
              <option value="days">days</option>
            </select>
          </span>
        </label>
        <label>Reason (optional)
          <input
            bind:value={customReason}
            maxlength="500"
            placeholder="Added from the permissions page"
            disabled={customBusy}
          />
        </label>
      </div>
      <label class="advanced">
        <input bind:checked={customAllProvider} type="checkbox" disabled={customBusy} />
        <span>
          <strong>Advanced: all provider capabilities</strong><br />
          This grants access to every current and future capability from the selected provider.
        </span>
      </label>
      {#if customError}<p class="error" role="alert">{customError}</p>{/if}
      <div class="actions">
        <button
          type="submit"
          class="primary"
          disabled={customBusy ||
            (customProvider.startsWith("mcp-") &&
              customCondition !== "requires-approval" &&
              !customMcpAcknowledged)}
        >Issue grant</button>
      </div>
    </form>
  </details>

  <details class="expander">
    <summary>Audit history</summary>
    <GrantTable />
  </details>
</section>

<style>
  .permissions {
    display: grid;
    gap: 1rem;
  }
  .muted {
    color: var(--color-text-muted);
  }
  .mcp-warning {
    grid-column: 1 / -1;
    display: flex;
    align-items: flex-start;
    gap: 0.55rem;
    padding: 0.65rem;
    border: 1px solid var(--color-warning-border);
    border-radius: 8px;
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
    font-size: 0.85rem;
    line-height: 1.4;
  }
  .mcp-warning input {
    flex: 0 0 auto;
    margin-top: 0.15em;
  }
  .artifact-access {
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    align-items: start;
    gap: 0.55rem;
    padding: 0.65rem;
    border: 1px solid var(--color-warning-border);
    border-radius: 0.65rem;
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
    font-size: 0.85rem;
    line-height: 1.4;
  }
  .artifact-access input {
    margin-top: 0.15em;
  }
  .app-group > summary {
    cursor: pointer;
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.35rem 0.6rem;
    justify-content: space-between;
    min-height: 2.75rem;
    padding: 0.45rem 0;
  }
  .app-group:focus-visible,
  .app-group.highlighted {
    border-radius: 12px;
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  .summary-main {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.35rem 0.6rem;
  }
  .app-name {
    font-weight: 700;
    font-size: 1rem;
  }
  .app-count {
    font-size: 0.84rem;
    color: var(--color-text-muted);
  }
  .entries {
    list-style: none;
    margin: 0.5rem 0 0;
    padding: 0;
    display: grid;
    gap: 0.5rem;
  }
  .entry {
    border: 1px solid var(--color-border-subtle);
    border-radius: 12px;
    padding: 0.7rem 0.8rem;
    display: grid;
    gap: 0.55rem;
    transition: border-color 160ms ease, background-color 160ms ease, box-shadow 160ms ease;
  }
  .entry.highlighted {
    border-color: var(--color-accent-border);
    background: var(--color-accent-soft);
    box-shadow: 0 0 0 2px var(--color-accent-border);
  }
  .entry:focus {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  .entry.inactive {
    background: var(--color-surface-muted);
  }
  .entry-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.6rem 1rem;
  }
  .what {
    flex: 1 1 14rem;
    min-width: 0;
    display: grid;
    gap: 0.15rem;
  }
  .what .muted {
    font-size: 0.84rem;
    overflow-wrap: anywhere;
  }
  .how {
    flex: 1 1 12rem;
    min-width: 0;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem 0.6rem;
    font-size: 0.88rem;
  }
  .entry-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin-left: auto;
  }
  .confirm-inline {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem;
    color: var(--color-warning-text);
    font-size: 0.85rem;
  }
  .badge {
    display: inline-flex;
    border-radius: 999px;
    padding: 0.2rem 0.55rem;
    font-size: 0.76rem;
    font-weight: 700;
    white-space: nowrap;
  }
  .badge.condition {
    background: var(--color-success-soft);
    color: var(--color-success-text);
  }
  .badge.off {
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .badge.small {
    font-size: 0.7rem;
    text-transform: uppercase;
  }
  .inline-editor {
    display: flex;
    flex-wrap: wrap;
    align-items: end;
    gap: 0.6rem 0.75rem;
    padding: 0.65rem;
    border: 1px solid var(--color-border);
    border-radius: 10px;
    background: var(--color-surface-muted);
  }
  .inline-editor label {
    flex: 1 1 10rem;
    min-width: 0;
  }
  .inline-editor .artifact-access {
    flex-basis: 100%;
  }
  .editor-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }
  .history summary {
    cursor: pointer;
    font-size: 0.84rem;
    color: var(--color-text-muted);
  }
  .history ul {
    list-style: none;
    margin: 0.4rem 0 0;
    padding: 0;
    display: grid;
    gap: 0.3rem;
    font-size: 0.84rem;
    color: var(--color-text-muted);
  }
  .history li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem;
  }
  .expander {
    border: 1px solid var(--color-border-subtle);
    border-radius: 12px;
    padding: 0.7rem 0.8rem;
  }
  .expander > summary {
    cursor: pointer;
    font-weight: 600;
  }
  .custom-form {
    margin-top: 0.75rem;
    display: grid;
    gap: 0.75rem;
  }
  .custom-form p {
    margin: 0;
  }
  .form-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 14rem), 1fr));
    gap: 0.75rem;
  }
  label {
    min-width: 0;
    display: grid;
    gap: 0.35rem;
    font-weight: 650;
    font-size: 0.9rem;
  }
  .scope-summary {
    min-width: 0;
    display: grid;
    align-content: start;
    gap: 0.35rem;
    font-size: 0.9rem;
  }
  .scope-summary span {
    font-weight: 650;
  }
  .scope-summary strong {
    color: var(--color-text-muted);
  }
  select,
  input {
    min-width: 0;
    border: 1px solid var(--color-border);
    border-radius: 8px;
    background: var(--color-surface);
    color: var(--color-text);
    padding: 0.55em 0.65em;
    font: inherit;
  }
  /* The value+unit pair must never overflow its grid column: both halves may
     shrink, and the unit drops to its own line when the column is too tight. */
  .expiry {
    min-width: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .expiry input {
    flex: 1 1 4rem;
    min-width: 0;
  }
  .expiry select {
    flex: 0 1 auto;
    min-width: 0;
  }
  .advanced {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: start;
    gap: 0.5rem;
    padding: 0.75rem;
    border: 1px solid var(--color-warning-border);
    border-radius: 10px;
    background: var(--color-warning-soft);
  }
  .advanced input {
    margin-top: 0.2em;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  button {
    min-height: 2rem;
    border: 1px solid var(--color-border);
    border-radius: 8px;
    background: var(--color-surface);
    color: var(--color-text);
    padding: 0.4rem 0.75rem;
    font: inherit;
    cursor: pointer;
  }
  button.primary {
    background: var(--color-accent);
    color: var(--color-accent-contrast);
  }
  button.icon-button {
    width: 2.25rem;
    min-width: 2.25rem;
    min-height: 2.25rem;
    padding: 0;
    display: inline-grid;
    place-items: center;
  }
  button.danger {
    border-color: var(--color-danger-border);
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }
  button:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .error {
    margin: 0;
    color: var(--color-danger-text);
  }
</style>
