<script lang="ts">
  import { onMount } from "svelte";
  import { listenHostStateScope } from "$lib/hostTransport";
  import {
    createKestralProfile,
    deleteKestralProfile,
    getActiveKestralProfile,
    listKestralProfiles,
    type KestralProfileView,
  } from "$lib/api";
  import ActionIcon from "$lib/settings/ActionIcon.svelte";

  let profiles = $state<KestralProfileView[]>([]);
  let activeProfile = $state<KestralProfileView | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let restartInstructions = $state<string | null>(null);
  let createOpen = $state(false);
  let busy = $state(false);
  let confirmDelete = $state<string | null>(null);
  let confirmText = $state("");
  let displayName = $state("");
  let slug = $state("");
  let slugEdited = $state(false);

  onMount(() => {
    void load();
    const unlisten = listenHostStateScope("profiles", () => void load());
    return () => void unlisten.then((stop) => stop());
  });

  async function load() {
    loading = true;
    try {
      const [active, managed] = await Promise.all([getActiveKestralProfile(), listKestralProfiles()]);
      activeProfile = active;
      profiles = managed;
      error = null;
    } catch (failure) {
      error = String(failure);
    } finally {
      loading = false;
    }
  }

  function beginCreate() {
    createOpen = true;
    error = null;
    confirmDelete = null;
    confirmText = "";
    displayName = "";
    slug = "";
    slugEdited = false;
  }

  function cancelCreate() {
    createOpen = false;
  }

  async function saveCreate() {
    busy = true;
    try {
      const created = await createKestralProfile({ display_name: displayName.trim(), slug: slug.trim() });
      restartInstructions = created.restart_instructions;
      createOpen = false;
      await load();
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }

  function startDelete(profileId: string) {
    confirmDelete = profileId;
    confirmText = "";
    error = null;
  }

  async function remove(profile: KestralProfileView) {
    if (confirmText !== profile.display_name) {
      error = "Type the exact profile name to delete this profile.";
      return;
    }
    busy = true;
    try {
      await deleteKestralProfile(profile.profile_id);
      confirmDelete = null;
      await load();
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }

  function isProtected(profile: KestralProfileView) {
    return profile.current_runtime || profile.selected_for_next_launch;
  }

  function suggestedSlug(name: string): string {
    return name
      .toLocaleLowerCase()
      .trim()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "");
  }

  function updateDisplayName(value: string) {
    displayName = value;
    if (!slugEdited) slug = suggestedSlug(value);
  }

  function deleteUnavailableReason(profile: KestralProfileView): string {
    if (profile.current_runtime) return "The profile in use cannot be deleted.";
    if (profile.selected_for_next_launch) return "The profile selected for the next launch cannot be deleted.";
    return `Delete ${profile.display_name}`;
  }
</script>

<div class="stack">
  <div class="profiles-heading">
    <div>
      <h3>Your profiles</h3>
      <p class="hint">Separate spaces for chats, apps, settings, and permissions.</p>
    </div>
    {#if !createOpen}
      <button type="button" class="primary icon-button" aria-label="Create profile" title="Create profile" onclick={beginCreate}><ActionIcon name="add" /></button>
    {/if}
  </div>

  {#if loading}
    <p class="hint">Loading profiles…</p>
  {/if}

  {#if activeProfile && activeProfile.source !== "managed"}
    <section class="card active-card">
      <div class="card-head">
        <div>
          <h3>{activeProfile.display_name}</h3>
          <p>Custom data directory</p>
        </div>
        <span class="badge active">Current</span>
      </div>
      <details>
        <summary>Profile details</summary>
        <dl class="meta">
          <div><dt>Profile ID</dt><dd><code>{activeProfile.profile_id}</code></dd></div>
          <div><dt>Data location</dt><dd><code>{activeProfile.root}</code></dd></div>
          <div><dt>Start command</dt><dd><code>{activeProfile.restart_instructions}</code></dd></div>
        </dl>
      </details>
    </section>
  {/if}

  {#if restartInstructions}
    <section class="notice" aria-live="polite">
      <strong>Restart required</strong>
      <p>{restartInstructions}</p>
    </section>
  {/if}

  <div class="profile-grid">
    {#each profiles as profile (profile.profile_id)}
      <section class="card profile-card">
        <div class="card-head">
          <div>
            <h3>{profile.display_name}</h3>
            <p><code>{profile.slug}</code></p>
          </div>
          <div class="badges">
            {#if profile.current_runtime}
              <span class="badge active">Current</span>
            {/if}
            {#if profile.selected_for_next_launch}
              <span class="badge">Next launch</span>
            {/if}
          </div>
        </div>
        <details>
          <summary>Profile details</summary>
          <dl class="meta">
            <div><dt>Profile ID</dt><dd><code>{profile.profile_id}</code></dd></div>
            <div><dt>Data location</dt><dd><code>{profile.root}</code></dd></div>
            <div><dt>Start command</dt><dd><code>{profile.restart_instructions}</code></dd></div>
          </dl>
        </details>
        <div class="actions">
          <button
            type="button"
            class="danger icon-button"
            disabled={busy || isProtected(profile)}
            aria-label={`Delete ${profile.display_name}`}
            title={deleteUnavailableReason(profile)}
            onclick={() => startDelete(profile.profile_id)}
          ><ActionIcon name="delete" /></button>
        </div>
        {#if confirmDelete === profile.profile_id}
          <div class="delete-confirm">
            <label>
              Type <strong>{profile.display_name}</strong> to confirm
              <input bind:value={confirmText} placeholder={profile.display_name} />
            </label>
            <div class="actions">
              <button type="button" class="danger" disabled={busy} onclick={() => void remove(profile)}>Delete profile</button>
              <button type="button" disabled={busy} onclick={() => (confirmDelete = null)}>Cancel</button>
            </div>
          </div>
        {/if}
      </section>
    {/each}
  </div>

  {#if createOpen}
    <form class="card create-form" onsubmit={(event) => { event.preventDefault(); void saveCreate(); }}>
      <h3>Create profile</h3>
      <div class="form-grid">
        <label>
          Name
          <input value={displayName} oninput={(event) => updateDisplayName(event.currentTarget.value)} placeholder="Work" required />
        </label>
        <label>
          Short name
          <input bind:value={slug} oninput={() => (slugEdited = true)} placeholder="work" pattern="[a-z0-9][a-z0-9-]*" required />
        </label>
      </div>
      <p class="hint">The new profile opens after you restart Kestral.</p>
      <div class="actions">
        <button type="submit" class="primary" disabled={busy}>Create profile</button>
        <button type="button" disabled={busy} onclick={cancelCreate}>Cancel</button>
      </div>
    </form>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</div>

<style>
  .stack {
    display: grid;
    gap: 0.85rem;
  }
  .profiles-heading {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
  }
  .profiles-heading h3,
  .profiles-heading p {
    margin: 0;
  }
  .hint,
  .meta,
  .notice p {
    margin: 0;
    color: var(--color-text-muted);
  }
  .profile-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 18rem), 1fr));
    gap: 0.75rem;
  }
  .card {
    border: 1px solid var(--color-border);
    border-radius: 1rem;
    background: var(--color-surface);
    padding: 0.95rem;
    min-width: 0;
  }
  .active-card {
    border-color: var(--color-accent);
    background: var(--color-surface-muted);
  }
  .profile-card,
  .create-form,
  .notice {
    display: grid;
    gap: 0.75rem;
  }
  .card-head,
  .actions,
  .delete-confirm {
    display: flex;
    gap: 0.6rem;
    align-items: start;
    justify-content: space-between;
    flex-wrap: wrap;
  }
  .badges {
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
  }
  .card h3 {
    margin: 0;
  }
  .card p {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .meta {
    margin-top: 0.65rem;
    display: grid;
    gap: 0.55rem;
  }
  .meta > div {
    display: grid;
    gap: 0.15rem;
  }
  dt {
    font-size: 0.78rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  code {
    white-space: pre-wrap;
    word-break: break-word;
  }
  .badge {
    border-radius: 999px;
    border: 1px solid var(--color-border-strong);
    padding: 0.18rem 0.55rem;
    font-size: 0.75rem;
  }
  .badge.active {
    border-color: var(--color-accent);
    color: var(--color-accent-text);
  }
  .notice {
    border: 1px solid var(--color-warning-border);
    border-radius: 0.9rem;
    background: var(--color-warning-soft);
    padding: 0.85rem;
  }
  .form-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 14rem), 1fr));
    gap: 0.75rem;
  }
  label {
    display: grid;
    gap: 0.25rem;
  }
  input {
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    padding: 0.55rem 0.7rem;
    font: inherit;
    min-width: 0;
  }
  button {
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    background: var(--color-surface-raised);
    color: var(--color-text);
    padding: 0.45rem 0.7rem;
    font: inherit;
  }
  button.primary {
    border-color: var(--color-accent);
    background: var(--color-accent);
    color: var(--color-accent-contrast);
  }
  button.icon-button {
    width: 2.5rem;
    min-width: 2.5rem;
    min-height: 2.5rem;
    padding: 0;
    display: inline-grid;
    place-items: center;
  }
  button.danger {
    border-color: var(--color-danger-border);
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }
  .error {
    margin: 0;
    color: var(--color-danger-text);
  }
  details summary {
    width: fit-content;
    cursor: pointer;
    color: var(--color-text-muted);
    font-size: 0.85rem;
  }
  button:focus-visible,
  input:focus-visible,
  summary:focus-visible {
    outline: 2px solid var(--color-focus-ring);
    outline-offset: 2px;
  }
</style>
