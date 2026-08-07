<script lang="ts">
  import { onDestroy } from "svelte";
  import type { ChatPromptPreview, ChatPromptSkillView, JsonObject } from "$lib/api";
  import { getChatPromptPreview } from "$lib/api";
  import { appConfigEntry, hostConfig, saveAppConfig } from "$lib/stores/config";

  const MAX_CUSTOM_INSTRUCTIONS = 16 * 1024;
  let entry = $derived(appConfigEntry($hostConfig, "chat"));
  type EnabledSkill = { app_id: string; skill_name: string; content_hash: string };
  let draft = $state(readDraft());
  let preview = $state<ChatPromptPreview | null>(null);
  let previewStatus = $state<"idle" | "loading" | "ready" | "error">("idle");
  let previewError = $state<string | null>(null);
  let saveStatus = $state<"idle" | "saving" | "saved" | "error">("idle");
  let saveError = $state<string | null>(null);
  let requestVersion = 0;
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;
  let confirmReset = $state(false);

  function readDraft() {
    const settings = entry.settings ?? {};
    return {
      use_default_instructions: settings.use_default_instructions !== false,
      custom_instructions: typeof settings.custom_instructions === "string" ? settings.custom_instructions : "",
      enabled_skills: Array.isArray(settings.enabled_skills) ? (settings.enabled_skills as EnabledSkill[]) : [],
      show_runtime_identity: settings.show_runtime_identity !== false,
      show_app_inventory: settings.show_app_inventory === true,
      show_connection_details: settings.show_connection_details === true,
      max_iterations: typeof settings.max_iterations === "number" ? settings.max_iterations : 10,
      show_metadata: settings.show_metadata === true,
      show_thinking: settings.show_thinking === true,
      record_injected_context: settings.record_injected_context === true,
    };
  }

  // The host state poller replaces `$hostConfig` with a fresh object every
  // 1.5s, so `entry` changes identity constantly even when nothing was edited
  // elsewhere. Reload the draft only when the saved *content* actually
  // changed, otherwise a poll tick would discard whatever is being typed.
  let loadedSettings: string | null = null;

  $effect(() => {
    const nextSettings = JSON.stringify(entry.settings ?? {});
    if (nextSettings !== loadedSettings) {
      loadedSettings = nextSettings;
      draft = readDraft();
    }
  });

  function mergedSettings(): JsonObject {
    return {
      ...entry.settings,
      max_iterations: draft.max_iterations,
      show_metadata: draft.show_metadata,
      show_thinking: draft.show_thinking,
      record_injected_context: draft.record_injected_context,
      use_default_instructions: draft.use_default_instructions,
      custom_instructions: draft.custom_instructions,
      enabled_skills: draft.enabled_skills,
      show_runtime_identity: draft.show_runtime_identity,
      show_app_inventory: draft.show_app_inventory,
      show_connection_details: draft.show_connection_details,
    };
  }

  function schedulePreview() {
    if (refreshTimer) clearTimeout(refreshTimer);
    refreshTimer = setTimeout(() => void refreshPreview(), 250);
  }

  $effect(() => {
    draft;
    schedulePreview();
  });

  async function refreshPreview() {
    const version = ++requestVersion;
    previewStatus = "loading";
    previewError = null;
    try {
      const result = await getChatPromptPreview(mergedSettings());
      if (version !== requestVersion) return;
      preview = result;
      previewStatus = "ready";
    } catch (error) {
      if (version !== requestVersion) return;
      previewStatus = "error";
      previewError = String(error);
    }
  }

  async function save() {
    saveStatus = "saving";
    saveError = null;
    try {
      await saveAppConfig("chat", mergedSettings());
      saveStatus = "saved";
    } catch (error) {
      saveStatus = "error";
      saveError = String(error);
    }
  }

  function resetToDefault() {
    draft = {
      ...draft,
      use_default_instructions: true,
      custom_instructions: "",
    };
    confirmReset = false;
  }

  function toggleSkill(skill: ChatPromptSkillView) {
    const sameIdentity = (value: EnabledSkill) =>
      value.app_id === skill.app_id && value.skill_name === skill.skill_name;
    const present = draft.enabled_skills.some((value: EnabledSkill) =>
      sameIdentity(value) && value.content_hash === skill.content_hash);
    const withoutSkill = draft.enabled_skills.filter((value: EnabledSkill) => !sameIdentity(value));
    draft = {
      ...draft,
      enabled_skills: present
        ? withoutSkill
        : [...withoutSkill, { app_id: skill.app_id, skill_name: skill.skill_name, content_hash: skill.content_hash }],
    };
  }

  function characterCount(value: string): number {
    return Array.from(value).length;
  }

  function updateMaxIterations(input: HTMLInputElement) {
    const value = input.valueAsNumber;
    if (Number.isInteger(value) && value >= 1 && value <= 50) {
      draft.max_iterations = value;
    }
  }

  onDestroy(() => {
    if (refreshTimer) clearTimeout(refreshTimer);
  });
</script>

<form
  class="prompt-settings"
  onsubmit={(event) => {
    event.preventDefault();
    void save();
  }}
>
    <article class="card primary-card">
      <header class="card-header">
        <h3>Assistant behavior</h3>
        <p>Choose the instructions that guide every new Chat response.</p>
      </header>
      <fieldset class="instruction-mode">
        <legend>Assistant instructions</legend>
        <label class="choice-row">
          <input
            type="radio"
            name="assistant-instruction-mode"
            checked={draft.use_default_instructions}
            onchange={() => (draft.use_default_instructions = true)}
          />
          <span><strong>Kestral default</strong><small>Use Kestral's standard assistant behavior.</small></span>
        </label>
        <label class="choice-row">
          <input
            type="radio"
            name="assistant-instruction-mode"
            checked={!draft.use_default_instructions}
            onchange={() => (draft.use_default_instructions = false)}
          />
          <span><strong>Custom</strong><small>Replace the default with instructions you write below.</small></span>
        </label>
      </fieldset>
      {#if !draft.use_default_instructions}
        <div class="custom-editor">
          <label for="custom-instructions">Custom instructions</label>
          <textarea
            id="custom-instructions"
            value={draft.custom_instructions}
            maxlength={MAX_CUSTOM_INSTRUCTIONS}
            oninput={(event) => (draft.custom_instructions = event.currentTarget.value)}
            aria-describedby="custom-instructions-help"
          ></textarea>
          <p id="custom-instructions-help" class="help">
            Replaces the Kestral default. {characterCount(draft.custom_instructions)} / {MAX_CUSTOM_INSTRUCTIONS} characters
          </p>
        </div>
      {/if}
      {#if !draft.use_default_instructions || draft.custom_instructions.length > 0}
        <div class="actions">
          {#if confirmReset}
            <button type="button" class="danger" onclick={resetToDefault}>Confirm instruction reset</button>
            <button type="button" class="secondary" onclick={() => (confirmReset = false)}>Keep my changes</button>
          {:else}
            <button type="button" class="secondary" onclick={() => (confirmReset = true)}>Reset assistant instructions</button>
          {/if}
        </div>
      {/if}
      {#if characterCount(draft.custom_instructions) > MAX_CUSTOM_INSTRUCTIONS}
        <p class="error" role="alert">Custom instructions must stay within 16 KiB.</p>
      {/if}
    </article>

    <details class="card disclosure">
      <summary>
        <span class="summary-title">Conversation details</span>
        <span class="summary-description">Activity, thinking, and agent-loop limits</span>
      </summary>
      <div class="disclosure-body">
        <label class="toggle-row"><input type="checkbox" checked={draft.show_metadata} onchange={(event) => (draft.show_metadata = event.currentTarget.checked)} /> <span><strong>Show activity details</strong><small>Show tool status, compact MCP result cards, and run details in conversations.</small></span></label>
        <label class="toggle-row"><input type="checkbox" checked={draft.show_thinking} onchange={(event) => (draft.show_thinking = event.currentTarget.checked)} /> <span><strong>Show provider thinking</strong><small>Keep provider thinking in a collapsed section below replies.</small></span></label>
        <label class="toggle-row"><input type="checkbox" checked={draft.record_injected_context} onchange={(event) => (draft.record_injected_context = event.currentTarget.checked)} /> <span><strong>Record app context sent to the model</strong><small>Store the exact host-final injected text on disk with each future chat request. It may contain sensitive app or user content and remains until the thread is deleted; turning this off does not remove existing records.</small></span></label>
        <label class="number-setting" for="max-iterations">
          <span><strong>Maximum iterations</strong><small>Stop an agent loop after this many model turns.</small></span>
          <input id="max-iterations" type="number" min="1" max="50" required value={draft.max_iterations} oninput={(event) => updateMaxIterations(event.currentTarget)} />
        </label>
      </div>
    </details>

    <details class="card disclosure">
      <summary>
        <span class="summary-title">Context shared with the model</span>
        <span class="summary-description">{draft.show_runtime_identity ? "Runtime identity is included" : "Runtime identity is not included"}</span>
      </summary>
      <div class="disclosure-body">
        <label class="toggle-row"><input type="checkbox" checked={draft.show_runtime_identity} onchange={(event) => (draft.show_runtime_identity = event.currentTarget.checked)} /> <span><strong>Runtime identity</strong><small>Include the Kestral version, execution mode, model, and connector kind.</small></span></label>
        <div class="dependent-options">
          <label class="toggle-row"><input type="checkbox" disabled={!draft.show_runtime_identity} checked={draft.show_app_inventory} onchange={(event) => (draft.show_app_inventory = event.currentTarget.checked)} /> <span><strong>App inventory</strong><small>Include installed app identities and versions.</small></span></label>
          <label class="toggle-row"><input type="checkbox" disabled={!draft.show_runtime_identity} checked={draft.show_connection_details} onchange={(event) => (draft.show_connection_details = event.currentTarget.checked)} /> <span><strong>Connection details</strong><small>Include connector and profile identifiers.</small></span></label>
        </div>
        <p class="help">Secrets, base URLs, and filesystem paths are never included.</p>
      </div>
    </details>

    <details class="card disclosure skills">
      <summary>
        <span class="summary-title">App guidance</span>
        <span class="summary-description">Optional instructions from installed apps</span>
      </summary>
      <div class="disclosure-body">
        <p class="help">App guidance can shape responses, but it cannot grant permissions. Changed guidance must be reviewed before it can be re-enabled.</p>
        {#if preview}
          {#each preview.available_skills as skill}
            <section class="skill-card">
              <header>
                <strong>{skill.app_display_name} {skill.app_version}</strong>
                <span class={`status ${skill.status}`}>{skill.status.replace("-", " ")}</span>
              </header>
              <p>{skill.description}</p>
              <details class="nested-disclosure">
                <summary>Review full instructions</summary>
                <pre>{skill.instructions}</pre>
              </details>
              <div class="actions">
                <button type="button" class="secondary" onclick={() => toggleSkill(skill)}>
                  {draft.enabled_skills.some((value: EnabledSkill) => value.app_id === skill.app_id && value.skill_name === skill.skill_name && value.content_hash === skill.content_hash) ? "Disable" : skill.status === "review-required" ? "Re-enable" : "Enable"}
                </button>
                {#if skill.status_reason}
                  <span class="help">{skill.status_reason}</span>
                {/if}
              </div>
            </section>
          {:else}
            <p class="help">No app guidance is available.</p>
          {/each}
        {:else}
          <p class="help">App guidance is unavailable until the prompt preview loads.</p>
        {/if}
      </div>
    </details>

    <details class="card disclosure preview">
      <summary>
        <span class="summary-title">Prompt preview</span>
        <span class="summary-description">Inspect the exact candidate system prompt before saving</span>
      </summary>
      <div class="disclosure-body">
        <p class="help">This read-only preview excludes tools, live app context, and conversation history.</p>
        <div class="actions">
          <button type="button" class="secondary" onclick={() => void refreshPreview()}>Refresh preview</button>
        </div>
        {#if previewStatus === "loading"}<p role="status">Updating preview…</p>{/if}
        {#if previewStatus === "error"}<p class="error" role="alert">{previewError}</p>{/if}
        {#if preview}
          <p class="digest"><strong>Digest:</strong> {preview.digest}</p>
          <pre class="system-prompt">{preview.system_prompt}</pre>
          <section class="composition" aria-labelledby="prompt-composition-title">
            <h4 id="prompt-composition-title">Prompt composition</h4>
            <ul>
              {#each preview.layers as layer}
                <li>
                  <span><strong>{layer.title}</strong><small>{layer.source ?? "No source"}</small></span>
                  <span class:included={layer.included} class="layer-status">{layer.included ? "Included" : "Not included"}</span>
                </li>
              {/each}
            </ul>
          </section>
        {/if}
      </div>
    </details>

    <footer class="form-actions">
      <button type="submit" class="primary" disabled={saveStatus === "saving" || characterCount(draft.custom_instructions) > MAX_CUSTOM_INSTRUCTIONS}>{saveStatus === "saving" ? "Saving…" : "Save Chat settings"}</button>
      {#if saveStatus === "saved"}<p class="success" role="status">Chat settings saved.</p>{/if}
      {#if saveStatus === "error"}<p class="error" role="alert">{saveError}</p>{/if}
    </footer>
</form>

<style>
  .prompt-settings { min-width: 0; width: 100%; display: grid; gap: 0.75rem; }
  .card { min-width: 0; border: 1px solid var(--color-border); border-radius: 1rem; padding: 1rem; background: var(--color-surface); }
  .primary-card, .disclosure-body { display: grid; gap: 0.85rem; }
  .card-header { display: grid; gap: 0.2rem; }
  .card-header h3, .card-header p, .composition h4 { margin: 0; }
  .card-header h3 { font-size: 1.05rem; }
  .card-header p { color: var(--color-text-muted); }
  .toggle-row { min-width: 0; min-height: 2.75rem; display: flex; gap: 0.65rem; align-items: flex-start; }
  .toggle-row input { flex: 0 0 auto; margin-top: 0.25rem; }
  .toggle-row span, .number-setting span { min-width: 0; display: grid; gap: 0.15rem; }
  .toggle-row small, .number-setting small { color: var(--color-text-muted); }
  .instruction-mode { min-width: 0; margin: 0; padding: 0; border: 0; display: grid; gap: 0.5rem; }
  .instruction-mode legend { margin-bottom: 0.25rem; font-size: 0.9rem; color: var(--color-text-muted); }
  .choice-row { min-height: 2.75rem; border: 1px solid var(--color-border-subtle); border-radius: 0.65rem; padding: 0.65rem 0.75rem; display: flex; gap: 0.65rem; align-items: flex-start; background: var(--color-surface-raised); }
  .choice-row:has(input:checked) { border-color: var(--color-accent); }
  .choice-row input { margin-top: 0.2rem; }
  .choice-row span { min-width: 0; display: grid; gap: 0.15rem; }
  .choice-row small { color: var(--color-text-muted); }
  .custom-editor { min-width: 0; display: grid; gap: 0.35rem; }
  textarea { box-sizing: border-box; min-width: 0; min-height: 12rem; width: 100%; max-width: 100%; resize: vertical; border: 1px solid var(--color-border-strong); border-radius: 0.65rem; padding: 0.75rem; background: var(--color-surface-raised); color: var(--color-text); font: inherit; line-height: 1.5; }
  .help { color: var(--color-text-muted); margin: 0; }
  .actions { display: flex; flex-wrap: wrap; gap: 0.5rem; }
  .form-actions { display: flex; flex-wrap: wrap; gap: 0.75rem; align-items: center; padding-top: 0.25rem; }
  button { min-height: 2.75rem; border-radius: 0.65rem; border: 1px solid var(--color-border); background: var(--color-surface-raised); color: var(--color-text); padding: 0.6rem 0.9rem; font: inherit; }
  button.primary { border-color: var(--color-accent); background: var(--color-accent); color: var(--color-accent-contrast); }
  button:disabled, input:disabled { cursor: default; opacity: 0.65; }
  button.secondary { background: transparent; }
  button.danger { background: var(--color-danger-soft); color: var(--color-danger-text); }
  button:focus-visible, input:focus-visible, textarea:focus-visible, summary:focus-visible { outline: 2px solid var(--color-focus-ring); outline-offset: 2px; }
  .disclosure > summary { min-height: 2.75rem; cursor: pointer; display: grid; align-content: center; gap: 0.15rem; }
  .summary-title { font-weight: 650; }
  .summary-description { color: var(--color-text-muted); font-size: 0.9rem; }
  .disclosure-body { margin-top: 1rem; }
  .dependent-options { margin-inline-start: 1.75rem; display: grid; gap: 0.25rem; }
  .number-setting { min-width: 0; display: flex; flex-wrap: wrap; justify-content: space-between; gap: 0.75rem; align-items: center; }
  .number-setting input { width: min(100%, 7rem); border: 1px solid var(--color-border-strong); border-radius: 0.65rem; padding: 0.6rem 0.7rem; background: var(--color-surface-raised); color: var(--color-text); font: inherit; }
  .skill-card { border: 1px solid var(--color-border-subtle); border-radius: 0.75rem; padding: 0.75rem; display: grid; gap: 0.5rem; background: var(--color-surface-muted); }
  .skill-card header { display: flex; flex-wrap: wrap; gap: 0.5rem; align-items: center; }
  .skill-card p { margin: 0; overflow-wrap: anywhere; }
  .status.disabled, .status.enabled, .status.review-required { font-size: 0.8rem; text-transform: uppercase; }
  .success { color: var(--color-success-text); margin: 0; }
  .error { color: var(--color-danger-text); margin: 0; }
  pre { min-width: 0; max-width: 100%; white-space: pre-wrap; overflow-wrap: anywhere; word-break: break-word; margin: 0; }
  .nested-disclosure summary { min-height: 2.75rem; cursor: pointer; display: flex; align-items: center; }
  .nested-disclosure pre { margin-top: 0.5rem; }
  .digest { margin: 0; overflow-wrap: anywhere; }
  .system-prompt { border: 1px solid var(--color-border-subtle); border-radius: 0.75rem; padding: 1rem; background: var(--color-surface-muted); line-height: 1.5; }
  .composition { min-width: 0; display: grid; gap: 0.5rem; }
  .composition ul { min-width: 0; margin: 0; padding: 0; list-style: none; display: grid; gap: 0.4rem; }
  .composition li { min-width: 0; display: flex; flex-wrap: wrap; justify-content: space-between; gap: 0.5rem; border-top: 1px solid var(--color-border-subtle); padding-top: 0.5rem; }
  .composition li > span:first-child { min-width: min(100%, 18rem); display: grid; gap: 0.1rem; }
  .composition small { color: var(--color-text-muted); overflow-wrap: anywhere; }
  .layer-status { color: var(--color-text-muted); }
  .layer-status.included { color: var(--color-success-text); }
  @media (max-width: 30em) {
    .card { padding: 0.8rem; }
    .actions button { width: 100%; }
    .form-actions button { width: 100%; }
    .dependent-options { margin-inline-start: 0.75rem; }
    .number-setting input { width: 100%; }
  }
</style>
