<script lang="ts">
  import {
    cssVariableName,
    themeColorGroups,
    themeColorLabel,
    themes,
    type ThemeColors,
    type ThemeColorToken,
    type ThemeId,
  } from "$lib/design/colors";
  import { apps } from "$lib/stores/apps";
  import ActionIcon from "$lib/settings/ActionIcon.svelte";
  import {
    appCssVariableName,
    createCustomThemeProfile,
    customThemePreference,
    customThemeProfiles,
    customThemeStorageError,
    defaultAppThemeColors,
    deleteCustomThemeProfile,
    exportCustomThemeProfile,
    importCustomThemeProfile,
    invalidThemeColorTokens,
    isThemeColorValue,
    themePreference,
    updateCustomThemeProfile,
    type AppThemeColors,
    type CustomThemeProfile,
    type ThemePreference,
  } from "$lib/stores/theme";

  const previewTokens: ThemeColorToken[] = ["bgGradientB", "surfaceRaised", "text", "accent", "successText", "dangerText"];

  let editingId = $state<string | null>(null);
  let draftName = $state("");
  let draftBaseTheme = $state<ThemeId>("light");
  let draftColors = $state<ThemeColors>({ ...themes.light });
  let draftAppColors = $state<AppThemeColors>({});
  let invalidTokens = $state<ThemeColorToken[]>([]);
  let invalidAppColors = $state<string[]>([]);
  let editedTokens = $state<ThemeColorToken[]>([]);
  let editedAppColors = $state<string[]>([]);
  let error = $state<string | null>(null);
  let status = $state<string | null>(null);
  let confirmingDeleteId = $state<string | null>(null);
  let importInput = $state<HTMLInputElement | null>(null);

  const appsWithThemeColors = $derived($apps
    .filter((app) => (app.theme_colors?.length ?? 0) > 0)
    .map((app) => ({
      id: app.manifest.app_id,
      name: app.manifest.display_name,
      colors: app.theme_colors ?? [],
    })));

  function appColorsForDraft(baseTheme: ThemeId, existing: AppThemeColors = {}): AppThemeColors {
    const next = Object.fromEntries(Object.entries(existing).map(([appId, colors]) => [appId, { ...colors }]));
    for (const app of appsWithThemeColors) {
      next[app.id] = {
        ...defaultAppThemeColors(app.colors, baseTheme),
        ...(existing[app.id] ?? {}),
      };
    }
    return next;
  }

  function beginCreate() {
    editingId = "new";
    draftName = "";
    draftBaseTheme = "light";
    draftColors = { ...themes.light };
    draftAppColors = appColorsForDraft("light");
    invalidTokens = [];
    invalidAppColors = [];
    editedTokens = [];
    editedAppColors = [];
    error = null;
    status = null;
  }

  function beginEdit(profile: CustomThemeProfile) {
    editingId = profile.id;
    draftName = profile.name;
    draftBaseTheme = profile.baseTheme;
    draftColors = { ...profile.colors };
    draftAppColors = appColorsForDraft(profile.baseTheme, profile.appColors);
    invalidTokens = [];
    invalidAppColors = [];
    editedTokens = [];
    editedAppColors = [];
    error = null;
    status = null;
  }

  function cancelEdit() {
    editingId = null;
    invalidTokens = [];
    invalidAppColors = [];
    error = null;
  }

  function changeBaseTheme(baseTheme: ThemeId) {
    draftBaseTheme = baseTheme;
    draftColors = {
      ...themes[baseTheme],
      ...Object.fromEntries(editedTokens.map((token) => [token, draftColors[token]])),
    };
    const editedValues: AppThemeColors = {};
    for (const key of editedAppColors) {
      const separator = key.lastIndexOf(":");
      const appId = key.slice(0, separator);
      const name = key.slice(separator + 1);
      editedValues[appId] = {
        ...(editedValues[appId] ?? {}),
        [name]: draftAppColors[appId][name],
      };
    }
    draftAppColors = appColorsForDraft(baseTheme, editedValues);
    invalidTokens = [];
    invalidAppColors = [];
  }

  function updateColor(token: ThemeColorToken, value: string) {
    draftColors[token] = value;
    invalidTokens = invalidTokens.filter((candidate) => candidate !== token);
    if (!editedTokens.includes(token)) editedTokens = [...editedTokens, token];
    error = null;
    status = null;
  }

  function appColorKey(appId: string, name: string): string {
    return `${appId}:${name}`;
  }

  function updateAppColor(appId: string, name: string, value: string) {
    draftAppColors[appId] = { ...(draftAppColors[appId] ?? {}), [name]: value };
    invalidAppColors = invalidAppColors.filter((candidate) => candidate !== appColorKey(appId, name));
    const key = appColorKey(appId, name);
    if (!editedAppColors.includes(key)) editedAppColors = [...editedAppColors, key];
    error = null;
    status = null;
  }

  function saveProfile(event: SubmitEvent) {
    event.preventDefault();
    invalidTokens = invalidThemeColorTokens(draftColors);
    invalidAppColors = appsWithThemeColors.flatMap((app) => app.colors
      .filter((declaration) => !isThemeColorValue(draftAppColors[app.id]?.[declaration.name] ?? ""))
      .map((declaration) => appColorKey(app.id, declaration.name)));
    if (invalidTokens.length > 0 || invalidAppColors.length > 0) {
      error = "Correct the highlighted color values before saving.";
      return;
    }
    try {
      if (editingId === "new") {
        const profile = createCustomThemeProfile(draftName, draftBaseTheme, draftAppColors);
        updateCustomThemeProfile(profile.id, draftName, draftColors, draftAppColors);
        themePreference.set(customThemePreference(profile.id));
        status = `${profile.name} created and selected.`;
      } else if (editingId) {
        updateCustomThemeProfile(editingId, draftName, draftColors, draftAppColors);
        status = `${draftName.trim()} saved.`;
      }
      editingId = null;
      error = null;
    } catch (failure) {
      error = String((failure as Error).message);
    }
  }

  function selectTheme(preference: ThemePreference) {
    themePreference.set(preference);
    const profileId = preference.startsWith("custom:") ? preference.slice("custom:".length) : null;
    const profileName = $customThemeProfiles.find((profile) => profile.id === profileId)?.name;
    status = preference === "system"
      ? "Following the system color theme."
      : `${profileName ?? (preference === "light" ? "Light" : "Dark")} selected.`;
  }

  function confirmDelete(profile: CustomThemeProfile) {
    deleteCustomThemeProfile(profile.id);
    confirmingDeleteId = null;
    if (editingId === profile.id) editingId = null;
    status = `${profile.name} deleted.${$themePreference === "system" ? " System theme selected." : ""}`;
  }

  function cancelDeleteOnEscape(event: KeyboardEvent) {
    if (event.key === "Escape") confirmingDeleteId = null;
  }

  async function importProfile(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    status = null;
    try {
      const profile = importCustomThemeProfile(await file.text());
      themePreference.set(customThemePreference(profile.id));
      status = `${profile.name} imported and selected.`;
      error = null;
    } catch (failure) {
      error = `Could not import this color profile: ${String((failure as Error).message)}`;
    } finally {
      input.value = "";
    }
  }

  function exportProfile(profile: CustomThemeProfile) {
    const blob = new Blob([exportCustomThemeProfile(profile)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `${profile.name.toLocaleLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "kestral-theme"}.json`;
    document.body.append(link);
    link.click();
    link.remove();
    // Webviews may resolve the object URL after the click handler returns.
    setTimeout(() => URL.revokeObjectURL(url), 1_000);
    status = `${profile.name} exported as JSON.`;
    error = null;
  }

  function focusOnMount(node: HTMLElement) {
    node.focus();
  }

  function colorPickerValue(value: string): string {
    const trimmed = value.trim();
    if (/^#[0-9a-f]{3,4}$/i.test(trimmed)) {
      return `#${[...trimmed.slice(1, 4)].map((digit) => digit + digit).join("")}`;
    }
    if (/^#[0-9a-f]{6,8}$/i.test(trimmed)) return trimmed.slice(0, 7);
    const channels = trimmed.match(/^rgba?\(\s*([^,]+),\s*([^,]+),\s*([^,)]+)/i);
    if (!channels) return colorPickerValue(themes.light.text);
    const toByte = (channel: string) => {
      const numeric = Number.parseFloat(channel);
      const value = channel.includes("%") ? numeric * 2.55 : numeric;
      return Math.max(0, Math.min(255, Math.round(value))).toString(16).padStart(2, "0");
    };
    return `#${toByte(channels[1])}${toByte(channels[2])}${toByte(channels[3])}`;
  }
</script>

<div class="theme-settings">
  <div class="selection">
    <label>
      <span>Color theme</span>
      <select
        value={$themePreference}
        onchange={(event) => selectTheme(event.currentTarget.value as ThemePreference)}
      >
        <option value="system">System (default)</option>
        <option value="light">Light</option>
        <option value="dark">Dark</option>
        {#if $customThemeProfiles.length > 0}
          <optgroup label="Custom profiles">
            {#each $customThemeProfiles as profile (profile.id)}
              <option value={customThemePreference(profile.id)}>{profile.name}</option>
            {/each}
          </optgroup>
        {/if}
      </select>
    </label>
    <p>Custom profiles stay on this device.</p>
  </div>

  <div class="profiles-heading">
    <div>
      <h4>Custom color profiles</h4>
      <p>Create, share, or fine-tune a palette.</p>
    </div>
    <div class="actions">
      <input bind:this={importInput} class="file-input" type="file" accept="application/json,.json" onchange={importProfile} aria-label="Import color profile JSON" />
      <button type="button" class="icon-button" aria-label="Import color profile" title="Import color profile" onclick={() => importInput?.click()}><ActionIcon name="import" /></button>
      {#if editingId === null}
        <button type="button" class="primary icon-button" aria-label="Create profile" title="Create profile" onclick={beginCreate}><ActionIcon name="add" /></button>
      {/if}
    </div>
  </div>

  {#if $customThemeStorageError}
    <p class="error notice" role="alert">
      Kestral could not load saved custom profiles: {$customThemeStorageError}
    </p>
  {/if}

  {#if status}
    <p class="success" role="status">{status}</p>
  {/if}

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if editingId !== null}
    <form class="editor" onsubmit={saveProfile}>
      <header>
        <div>
          <h4>{editingId === "new" ? "Create color profile" : "Edit color profile"}</h4>
          <p>Use the picker for a solid color or enter a HEX, rgb(), or rgba() value for precise control.</p>
        </div>
      </header>

      <div class="profile-fields">
        <label>
          <span>Profile name</span>
          <input bind:value={draftName} maxlength="40" autocomplete="off" required />
        </label>
        {#if editingId === "new"}
          <label>
            <span>Start from</span>
            <select value={draftBaseTheme} onchange={(event) => changeBaseTheme(event.currentTarget.value as ThemeId)}>
              <option value="light">Light</option>
              <option value="dark">Dark</option>
            </select>
          </label>
        {:else}
          <p class="base-theme">Based on {draftBaseTheme === "light" ? "Light" : "Dark"}</p>
        {/if}
      </div>

      <p class="accessibility-note">
        Keep text readable against its surfaces and make focus indicators easy to see. Built-in profiles remain available if a custom combination is hard to use.
      </p>

      <div class="color-groups">
        {#each themeColorGroups as group, index (group.id)}
          <details open={index === 0}>
            <summary>
              <span>{group.label}</span>
              <small>{group.description} {group.tokens.length} colors</small>
            </summary>
            <div class="color-grid">
              {#each group.tokens as token (token)}
                {@const label = themeColorLabel(token)}
                <label class="color-field" class:invalid={invalidTokens.includes(token)}>
                  <span class="color-label">{label}</span>
                  <code>{cssVariableName(token)}</code>
                  <span class="color-controls">
                    <input
                      class="color-picker"
                      type="color"
                      aria-label={`${label} picker`}
                      value={colorPickerValue(draftColors[token])}
                      oninput={(event) => updateColor(token, event.currentTarget.value)}
                    />
                    <input
                      class="color-value"
                      aria-label={`${label} color value`}
                      value={draftColors[token]}
                      oninput={(event) => updateColor(token, event.currentTarget.value)}
                      spellcheck="false"
                      autocomplete="off"
                    />
                  </span>
                  {#if invalidTokens.includes(token)}
                    <small class="field-error">Enter a HEX, rgb(), or rgba() color.</small>
                  {/if}
                </label>
              {/each}
            </div>
          </details>
        {/each}
        {#each appsWithThemeColors as app (app.id)}
          <details>
            <summary>
              <span>{app.name}</span>
              <small>Colors declared by {app.name}. {app.colors.length} colors</small>
            </summary>
            <div class="color-grid">
              {#each app.colors as declaration (declaration.name)}
                {@const key = appColorKey(app.id, declaration.name)}
                {@const value = draftAppColors[app.id]?.[declaration.name] ?? declaration[draftBaseTheme]}
                <label class="color-field" class:invalid={invalidAppColors.includes(key)}>
                  <span class="color-label">{declaration.title}</span>
                  <code>{appCssVariableName(declaration.name)}</code>
                  <small>{declaration.description}</small>
                  <span class="color-controls">
                    <input
                      class="color-picker"
                      type="color"
                      aria-label={`${app.name} ${declaration.title} picker`}
                      value={colorPickerValue(value)}
                      oninput={(event) => updateAppColor(app.id, declaration.name, event.currentTarget.value)}
                    />
                    <input
                      class="color-value"
                      aria-label={`${app.name} ${declaration.title} color value`}
                      value={value}
                      oninput={(event) => updateAppColor(app.id, declaration.name, event.currentTarget.value)}
                      spellcheck="false"
                      autocomplete="off"
                    />
                  </span>
                  {#if invalidAppColors.includes(key)}
                    <small class="field-error">Enter a HEX, rgb(), or rgba() color.</small>
                  {/if}
                </label>
              {/each}
            </div>
          </details>
        {/each}
      </div>

      <div class="actions">
        <button type="submit" class="primary">{editingId === "new" ? "Create and use profile" : "Save changes"}</button>
        <button type="button" onclick={cancelEdit}>Cancel</button>
      </div>
    </form>
  {:else if $customThemeProfiles.length === 0}
    <div class="empty-state">
      <strong>No custom profiles yet</strong>
      <p>Create one when Light or Dark does not fit your workspace.</p>
    </div>
  {:else}
    <div class="profile-grid">
      {#each $customThemeProfiles as profile (profile.id)}
        {@const preference = customThemePreference(profile.id)}
        <article class="profile-card" class:active={$themePreference === preference}>
          <div class="profile-title">
            <div>
              <h5>{profile.name}</h5>
              <p>Based on {profile.baseTheme === "light" ? "Light" : "Dark"}</p>
            </div>
            {#if $themePreference === preference}<span class="active-label">In use</span>{/if}
          </div>
          <div class="swatches" aria-label={`${profile.name} color preview`}>
            {#each previewTokens as token}
              <span style={`background: ${profile.colors[token]}`} title={themeColorLabel(token)}></span>
            {/each}
          </div>
          <div class="actions">
            {#if $themePreference !== preference}
              <button type="button" class="primary" onclick={() => selectTheme(preference)}>Use profile</button>
            {/if}
            <button type="button" class="icon-button" aria-label={`Edit ${profile.name}`} title="Edit profile" onclick={() => beginEdit(profile)}><ActionIcon name="edit" /></button>
            <button type="button" class="icon-button" aria-label={`Export ${profile.name}`} title="Export profile" onclick={() => exportProfile(profile)}><ActionIcon name="export" /></button>
            {#if confirmingDeleteId === profile.id}
              <span class="confirm-delete">
                Delete {profile.name}?
                <button type="button" class="danger" onclick={() => confirmDelete(profile)} onkeydown={cancelDeleteOnEscape}>Delete</button>
                <button type="button" use:focusOnMount onclick={() => (confirmingDeleteId = null)} onkeydown={cancelDeleteOnEscape}>Keep</button>
              </span>
            {:else}
              <button type="button" class="danger icon-button" aria-label={`Delete ${profile.name}`} title="Delete profile" onclick={() => (confirmingDeleteId = profile.id)}><ActionIcon name="delete" /></button>
            {/if}
          </div>
        </article>
      {/each}
    </div>
  {/if}
</div>

<style>
  .theme-settings { min-width: 0; display: grid; gap: 1.25rem; }
  .selection { display: grid; gap: 0.35rem; }
  .selection label, .profile-fields label { min-width: 0; display: grid; gap: 0.3rem; }
  .selection p, .profiles-heading p, .editor header p, .empty-state p, .profile-title p, .base-theme { margin: 0; color: var(--color-text-muted); font-size: 0.88rem; }
  select, input { min-width: 0; border: 1px solid var(--color-border-strong); border-radius: 0.65rem; padding: 0.6rem 0.7rem; background: var(--color-surface-raised); color: var(--color-text); font: inherit; }
  .selection select { width: min(100%, 28rem); }
  .profiles-heading { display: flex; flex-wrap: wrap; align-items: end; justify-content: space-between; gap: 0.75rem; padding-top: 0.25rem; border-top: 1px solid var(--color-border-subtle); }
  h4, h5 { margin: 0; color: var(--color-text); }
  h4 { font-size: 1rem; }
  h5 { font-size: 0.95rem; }
  button { min-height: 2.5rem; border: 1px solid var(--color-border-strong); border-radius: 0.65rem; padding: 0.55rem 0.8rem; background: var(--color-surface-raised); color: var(--color-text); font: inherit; }
  button.primary { border-color: var(--color-accent); background: var(--color-accent); color: var(--color-accent-contrast); }
  button.danger { border-color: var(--color-danger-border); color: var(--color-danger-text); }
  button.icon-button { width: 2.5rem; min-width: 2.5rem; padding: 0; display: inline-grid; place-items: center; }
  button:focus-visible, input:focus-visible, select:focus-visible, summary:focus-visible { outline: 2px solid var(--color-focus-ring); outline-offset: 2px; }
  .success, .error { margin: 0; font-size: 0.9rem; font-weight: 600; }
  .success { color: var(--color-success-text); }
  .error { color: var(--color-danger-text); }
  .notice { border: 1px solid var(--color-danger-border); border-radius: 0.65rem; padding: 0.75rem; background: var(--color-danger-soft); }
  .empty-state { border: 1px dashed var(--color-border-strong); border-radius: 0.8rem; padding: 1rem; display: grid; gap: 0.25rem; }
  .profile-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 17rem), 1fr)); gap: 0.75rem; }
  .profile-card { min-width: 0; border: 1px solid var(--color-border); border-radius: 0.8rem; padding: 0.85rem; background: var(--color-surface-muted); display: grid; gap: 0.75rem; }
  .profile-card.active { border-color: var(--color-accent-border); }
  .profile-title { min-width: 0; display: flex; align-items: start; justify-content: space-between; gap: 0.5rem; }
  .profile-title h5, .profile-title p { overflow-wrap: anywhere; }
  .active-label { flex: 0 0 auto; border: 1px solid var(--color-accent-border); border-radius: 999px; padding: 0.2rem 0.5rem; background: var(--color-accent-soft); color: var(--color-accent-text); font-size: 0.75rem; font-weight: 700; }
  .swatches { display: grid; grid-template-columns: repeat(6, minmax(0, 1fr)); min-height: 2rem; overflow: hidden; border: 1px solid var(--color-border); border-radius: 0.55rem; }
  .actions { display: flex; flex-wrap: wrap; align-items: center; gap: 0.5rem; }
  .file-input { display: none; }
  .confirm-delete { min-width: min(100%, 16rem); display: flex; flex-wrap: wrap; align-items: center; gap: 0.45rem; color: var(--color-danger-text); font-size: 0.85rem; }
  .editor { min-width: 0; border: 1px solid var(--color-border); border-radius: 0.9rem; padding: 1rem; background: var(--color-surface-muted); display: grid; gap: 1rem; }
  .editor header { display: grid; gap: 0.25rem; }
  .profile-fields { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 14rem), 1fr)); align-items: end; gap: 0.75rem; }
  .base-theme { padding-bottom: 0.65rem; }
  .accessibility-note { margin: 0; border-left: 3px solid var(--color-warning-border); padding: 0.65rem 0.75rem; background: var(--color-warning-soft); color: var(--color-warning-text); font-size: 0.85rem; }
  .color-groups { min-width: 0; display: grid; gap: 0.55rem; }
  details { min-width: 0; border: 1px solid var(--color-border-subtle); border-radius: 0.7rem; background: var(--color-surface-raised); }
  summary { cursor: pointer; min-height: 2.75rem; padding: 0.7rem 0.8rem; display: grid; gap: 0.15rem; }
  summary span { font-weight: 650; }
  summary small { color: var(--color-text-muted); }
  .color-grid { min-width: 0; display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 17rem), 1fr)); gap: 0.75rem; padding: 0.8rem; border-top: 1px solid var(--color-border-subtle); }
  .color-field { min-width: 0; display: grid; gap: 0.25rem; align-content: start; }
  .color-label { font-size: 0.88rem; font-weight: 600; }
  .color-field code { color: var(--color-text-faint); font-size: 0.72rem; overflow-wrap: anywhere; }
  .color-controls { min-width: 0; display: grid; grid-template-columns: 2.75rem minmax(0, 1fr); gap: 0.45rem; }
  .color-picker { width: 2.75rem; min-height: 2.75rem; padding: 0.2rem; cursor: pointer; }
  .color-value { width: 100%; }
  .color-field.invalid .color-value { border-color: var(--color-danger-border); }
  .field-error { color: var(--color-danger-text); }
  @media (max-width: 30em) {
    .actions > button:not(.icon-button) { flex: 1 1 auto; }
    .editor { padding: 0.75rem; }
    .confirm-delete { width: 100%; }
  }
</style>
