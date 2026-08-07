<script lang="ts">
  import type { ConnectorConfigView, ModelVariant, TextVerbosity } from "$lib/api";
  import ActionIcon from "$lib/settings/ActionIcon.svelte";
  import SecretInput from "$lib/settings/SecretInput.svelte";
  import {
    CHATGPT_CODEX_DEFAULT_MODEL,
    connectorEndpointKind,
    connectorCredentialLabel,
    connectorIsCloud,
    connectorKindSemantics,
    connectorProfileName,
    connectorUsesOAuth,
    defaultApiKeySecretName,
    defaultOAuthSecretName,
    type ConnectorEndpointKind,
  } from "$lib/settings/connectorProfiles";
  import {
    defaultBaseUrlForConnectorKind,
    modelVariantLabel,
    textVerbosityLabel,
    type ConnectorProfileCard,
  } from "$lib/settings/llmProviderSettingsModel";

  interface Props {
    card: ConnectorProfileCard;
    isDefault: boolean;
    requiresCloudAcceptance: boolean;
    onBeginEdit: () => void;
    onDraftChange: (draft: ConnectorConfigView) => void;
    onSave: (makeDefault: boolean) => void;
    onAcceptCloudSave: (makeDefault: boolean) => void;
    onCancel: () => void;
    onDiscoverModels: () => void;
    onTest: () => void;
    onSignIn: () => void;
    onDisconnect: () => void;
    onDelete: () => void;
  }

  let {
    card,
    isDefault,
    requiresCloudAcceptance,
    onBeginEdit,
    onDraftChange,
    onSave,
    onAcceptCloudSave,
    onCancel,
    onDiscoverModels,
    onTest,
    onSignIn,
    onDisconnect,
    onDelete,
  }: Props = $props();

  const endpointKindLabels: Record<ConnectorEndpointKind, string> = {
    "local-ollama": "Local",
    "local-openai-compatible": "Local",
    "cloud-openai-compatible": "Cloud",
    "cloud-provider": "Cloud",
  };

  const endpointKind = $derived(connectorEndpointKind(card.draft));
  const secretName = $derived(card.draft.secret_refs.api_key ?? defaultApiKeySecretName(card.draft.id));
  const oauthSecretName = $derived(card.draft.secret_refs.oauth ?? defaultOAuthSecretName(card.draft.id));
  const isCloudProfile = $derived(connectorIsCloud(card.draft));
  const kindSemantics = $derived(connectorKindSemantics(card.draft.kind));
  const credentialLabel = $derived(connectorCredentialLabel(card.draft.kind));
  const isPersisted = $derived(card.persistedConnector !== null);
  const usesOAuth = $derived(connectorUsesOAuth(card.draft.kind));
  const oauthAccountLabel = $derived(kindSemantics.oauthAccountLabel ?? "Model account");
  const oauthStatusLabel = $derived(
    card.oauthStatus === "connected" ? "Connected"
      : card.oauthStatus === "not-connected" ? "Not connected"
        : card.oauthStatus === "error" ? "Status unavailable"
          : "Checking status…",
  );
  const baseUrlPlaceholder = $derived(defaultBaseUrlForConnectorKind(card.draft.kind));
  const selectedDiscoveredModel = $derived(
    card.discoveredModels.some((model) => model.id === card.draft.default_model)
      ? card.draft.default_model
      : "",
  );
  const selectedModelInfo = $derived(
    card.discoveredModels.find((model) => model.id === card.draft.default_model),
  );

  function updateDraft(next: ConnectorConfigView) {
    onDraftChange({
      ...next,
      secret_refs: { ...next.secret_refs },
    });
  }

  let confirmingDelete = $state(false);
  let confirmingDisconnect = $state(false);
  let makeDefault = $state(false);

  // Focuses the safe ("Keep") choice when the inline delete confirm appears,
  // without the `autofocus` attribute (flagged by svelte-check's a11y rule).
  function focusOnMount(node: HTMLElement) {
    node.focus();
  }

  function cancelDeleteOnEscape(event: KeyboardEvent) {
    if (event.key === "Escape") {
      confirmingDelete = false;
    }
  }

  function cancelDisconnectOnEscape(event: KeyboardEvent) {
    if (event.key === "Escape") {
      confirmingDisconnect = false;
    }
  }
</script>

<section class="panel">
  <div class="panel-header">
    <div>
      <h3>{connectorProfileName(card.draft.id) || "New profile"}</h3>
      <p>{kindSemantics.label} · {endpointKindLabels[endpointKind]}{isDefault ? " · default" : ""}</p>
    </div>
    <button type="button" class="secondary icon-button" aria-label={`Edit ${connectorProfileName(card.draft.id) || "profile"}`} title="Edit profile" onclick={onBeginEdit} disabled={card.editing || card.busy}><ActionIcon name="edit" /></button>
  </div>

  {#if card.editing}
  <div class="row">
    <label>
      Profile id
      <input
        value={card.draft.id}
        oninput={(event) =>
          updateDraft({
            ...card.draft,
            id: event.currentTarget.value,
          })}
        disabled={isPersisted || !card.editing || card.busy}
      />
    </label>
      <label>
        Kind
        <select
          value={card.draft.kind}
          onchange={(event) => {
            const kind = event.currentTarget.value as ConnectorConfigView["kind"];
            // Only swap in the new kind's default URL when the user hasn't
            // customized the current one — toggling kinds must not clobber
            // a hand-entered endpoint.
            const keepBaseUrl =
              card.draft.base_url.trim() !== "" &&
              card.draft.base_url !== defaultBaseUrlForConnectorKind(card.draft.kind);
            updateDraft({
              ...card.draft,
              kind,
              base_url: kind === "openai-codex" || !keepBaseUrl
                ? defaultBaseUrlForConnectorKind(kind)
                : card.draft.base_url,
              default_model: kind === "openai-codex" ? CHATGPT_CODEX_DEFAULT_MODEL : "",
              default_variant: null,
              default_text_verbosity: null,
            });
          }}
          disabled={!card.editing || card.busy}
        >
        <option value="ollama">Ollama</option>
        <option value="open-ai-compatible">OpenAI-compatible</option>
        <option value="openai">OpenAI</option>
        <option value="anthropic">Anthropic</option>
        <option value="anthropic-oauth">Anthropic OAuth</option>
        <option value="openai-codex">ChatGPT (Codex subscription)</option>
        <option value="github-copilot">GitHub Copilot</option>
        <option value="openrouter">OpenRouter</option>
        <option value="google">Google AI</option>
        <option value="mistral">Mistral AI</option>
        <option value="amazon-bedrock">Amazon Bedrock</option>
      </select>
    </label>
  </div>

  {#if card.editing && !isPersisted && card.draft.kind !== "ollama" && !card.draft.secret_refs.api_key && !usesOAuth}
    <p class="hint">Set the profile ID before entering its credential.</p>
  {/if}

  <div class="row">
    <label>
      Base URL
      <input
        value={card.draft.base_url}
        placeholder={baseUrlPlaceholder}
        oninput={(event) =>
          updateDraft({
            ...card.draft,
            base_url: event.currentTarget.value,
          })}
        disabled={!card.editing || card.busy}
        readonly={card.draft.kind === "openai-codex"}
      />
    </label>
    <label>
      Default model
      <input
        value={card.draft.default_model}
        placeholder="llama3.1"
        oninput={(event) =>
          updateDraft({
            ...card.draft,
            default_model: event.currentTarget.value,
            default_variant: null,
            default_text_verbosity: null,
          })}
        disabled={!card.editing || card.busy}
      />
    </label>
  </div>

  <div class="model-tools">
    <button type="button" class="secondary" onclick={onDiscoverModels} disabled={card.busy}>
      Discover models
    </button>
    {#if card.discoveredModels.length > 0}
      <label class="model-picker">
        Discovered models
        <select
          value={selectedDiscoveredModel}
          onchange={(event) => {
            const model = card.discoveredModels.find((candidate) => candidate.id === event.currentTarget.value);
            if (!model) {
              return;
            }
            updateDraft({
              ...card.draft,
              default_model: model.id,
              default_variant: card.draft.default_variant && model.variants.includes(card.draft.default_variant)
                ? card.draft.default_variant
                : null,
              default_text_verbosity: card.draft.default_text_verbosity && model.text_verbosity.includes(card.draft.default_text_verbosity)
                ? card.draft.default_text_verbosity
                : null,
            });
          }}
          disabled={!card.editing || card.busy}
        >
          <option value="">Choose a discovered model</option>
          {#each card.discoveredModels as model (model.id)}
            <option value={model.id}>{model.display_name ?? model.id}</option>
          {/each}
        </select>
      </label>
      {#if selectedModelInfo && (selectedModelInfo.variants.length > 0 || card.draft.default_variant)}
        <label class="model-picker">
          Model variant
          <select
            value={card.draft.default_variant ?? ""}
            onchange={(event) => updateDraft({
              ...card.draft,
              default_variant: (event.currentTarget.value || null) as ModelVariant | null,
            })}
            disabled={!card.editing || card.busy}
          >
            <option value="">Provider default</option>
            {#if card.draft.default_variant && !selectedModelInfo.variants.includes(card.draft.default_variant)}
              <option value={card.draft.default_variant}>
                {modelVariantLabel(card.draft.default_variant)} (not advertised)
              </option>
            {/if}
            {#each selectedModelInfo.variants as variant (variant)}
              <option value={variant}>{modelVariantLabel(variant)}</option>
            {/each}
          </select>
        </label>
      {/if}
      {#if selectedModelInfo && (selectedModelInfo.text_verbosity.length > 0 || card.draft.default_text_verbosity)}
        <label class="model-picker">
          Text verbosity
          <select
            value={card.draft.default_text_verbosity ?? ""}
            onchange={(event) => updateDraft({
              ...card.draft,
              default_text_verbosity: (event.currentTarget.value || null) as TextVerbosity | null,
            })}
            disabled={!card.editing || card.busy}
          >
            <option value="">Provider default</option>
            {#if card.draft.default_text_verbosity && !selectedModelInfo.text_verbosity.includes(card.draft.default_text_verbosity)}
              <option value={card.draft.default_text_verbosity}>
                {textVerbosityLabel(card.draft.default_text_verbosity)} (not advertised)
              </option>
            {/if}
            {#each selectedModelInfo.text_verbosity as verbosity (verbosity)}
              <option value={verbosity}>{textVerbosityLabel(verbosity)}</option>
            {/each}
          </select>
        </label>
      {/if}
    {/if}
  </div>
  {#if card.draft.kind !== "ollama"}
    {#if isCloudProfile}
      <p class="warning">
        Cloud profile: chat content and tool results may leave this device.
        {#if requiresCloudAcceptance}
          To change the current default from local to cloud, explicitly accept this data sharing.
        {/if}
      </p>
    {/if}

    {#if usesOAuth}
      <div class="oauth-sign-in">
        <p>{kindSemantics.oauthDescription ?? `Connect ${oauthAccountLabel} through the verified host dialog.`}</p>
        {#if isPersisted}
          <p class="account-status" class:connected={card.oauthStatus === "connected"} role="status">
            {oauthAccountLabel}: <strong>{oauthStatusLabel}</strong>
          </p>
        {/if}
        <details class="advanced-secret">
          <summary>Credential details</summary>
          <dl class="storage-metadata">
            <div><dt>Host storage entry</dt><dd><code>{oauthSecretName}</code></dd></div>
          </dl>
        </details>
      </div>
    {:else}
      <SecretInput owner="llm-provider" secretName={secretName} label={credentialLabel} />

      <details class="advanced-secret">
        <summary>Advanced: where this credential is stored</summary>
        <label>
          Credential storage name
          <input
            value={card.draft.secret_refs.api_key ?? ""}
            placeholder={defaultApiKeySecretName(card.draft.id)}
            oninput={(event) =>
              updateDraft({
                ...card.draft,
                secret_refs: {
                  ...card.draft.secret_refs,
                  api_key: event.currentTarget.value,
                },
              })}
            disabled={!card.editing || card.busy}
          />
        </label>
        <p class="hint">
          The name of the credential-vault entry that holds this credential, not the credential itself.
          Leave blank to use the default. Renaming it points the profile at a different entry.
        </p>
      </details>
    {/if}
  {/if}
  {:else}
    <dl class="summary">
      <div><dt>Model</dt><dd>{card.draft.default_model || "Not selected"}</dd></div>
      <div><dt>Variant</dt><dd>{card.draft.default_variant ? modelVariantLabel(card.draft.default_variant) : "Provider default"}</dd></div>
      <div><dt>Text verbosity</dt><dd>{card.draft.default_text_verbosity ? textVerbosityLabel(card.draft.default_text_verbosity) : "Provider default"}</dd></div>
      <div><dt>Endpoint</dt><dd>{card.draft.base_url}</dd></div>
    </dl>
    {#if usesOAuth}
      <div class="oauth-sign-in">
        <p>{kindSemantics.oauthDescription ?? `Connect ${oauthAccountLabel} through the verified host dialog.`}</p>
        <p class="account-status" class:connected={card.oauthStatus === "connected"} role="status">
          {oauthAccountLabel}: <strong>{oauthStatusLabel}</strong>
        </p>
        <details class="advanced-secret">
          <summary>Credential details</summary>
          <dl class="storage-metadata">
            <div><dt>Host storage entry</dt><dd><code>{oauthSecretName}</code></dd></div>
          </dl>
        </details>
        <button type="button" onclick={onSignIn} disabled={card.busy}>
          {card.oauthStatus === "connected" ? `Reconnect ${oauthAccountLabel}` : `Connect ${oauthAccountLabel}`}
        </button>
        {#if card.oauthStatus === "connected"}
          {#if confirmingDisconnect}
            <span class="confirm-inline">
              Disconnect this account?
              <button
                type="button"
                class="danger"
                onclick={() => {
                  confirmingDisconnect = false;
                  onDisconnect();
                }}
                onkeydown={cancelDisconnectOnEscape}
              >
                Disconnect
              </button>
              <button
                type="button"
                class="secondary"
                use:focusOnMount
                onclick={() => (confirmingDisconnect = false)}
                onkeydown={cancelDisconnectOnEscape}
              >
                Keep connected
              </button>
            </span>
          {:else}
            <button type="button" class="secondary" onclick={() => (confirmingDisconnect = true)} disabled={card.busy}>
              Disconnect account
            </button>
          {/if}
        {/if}
      </div>
    {/if}
    {#if isCloudProfile}
      <p class="warning">Cloud profile: chat content and tool results may leave this device.</p>
    {/if}
  {/if}

  <div class="actions">
    {#if card.editing}
    {#if !isPersisted}
      <label class="default-after-save">
        <input type="checkbox" bind:checked={makeDefault} disabled={card.busy} />
        Use as Chat default
      </label>
    {/if}
    <button type="button" onclick={() => onSave(makeDefault)} disabled={card.busy || requiresCloudAcceptance}>Save</button>
    {#if requiresCloudAcceptance}
      <button type="button" onclick={() => onAcceptCloudSave(makeDefault)} disabled={card.busy}>
        Accept data sharing and save
      </button>
    {/if}
    <button type="button" class="secondary" onclick={onCancel} disabled={card.busy}>Cancel</button>
    {/if}
    {#if !usesOAuth}
      <button type="button" class="secondary" onclick={onTest} disabled={card.busy || requiresCloudAcceptance}>
        {card.editing ? "Save & test" : "Test connection"}
      </button>
    {/if}
    {#if confirmingDelete}
      <span class="confirm-inline">
        Delete this profile?
        <button
          type="button"
          class="danger"
          onclick={() => {
            confirmingDelete = false;
            onDelete();
          }}
          onkeydown={cancelDeleteOnEscape}
        >
          Delete
        </button>
        <button
          type="button"
          class="secondary"
          use:focusOnMount
          onclick={() => (confirmingDelete = false)}
          onkeydown={cancelDeleteOnEscape}
        >
          Keep
        </button>
      </span>
    {:else}
      <button
        type="button"
        class="danger icon-button"
        onclick={() => (confirmingDelete = true)}
        disabled={card.busy || isDefault}
        aria-label={`Delete ${connectorProfileName(card.draft.id) || "profile"}`}
        title={isDefault ? "This is the default profile. Choose a new default first." : "Delete profile"}
      >
        <ActionIcon name="delete" />
      </button>
    {/if}
    {#if card.message}
      <span
        class="message {card.messageKind ?? ''}"
        role={card.messageKind === "error" ? "alert" : "status"}
      >
        {card.message}
      </span>
    {/if}
  </div>
</section>

<style>
  .panel {
    min-width: 0;
    display: grid;
    gap: 0.75rem;
    padding: 1rem;
    border: 1px solid var(--color-border);
    border-radius: 14px;
    background: var(--color-surface-muted);
  }
  .panel-header,
  .actions {
    display: flex;
    gap: 0.6rem;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
  }
  .panel-header h3,
  .panel-header p {
    margin: 0;
  }
  .panel-header p {
    color: var(--color-text-muted);
    text-transform: capitalize;
  }
  .row {
    display: grid;
    gap: 0.75rem;
    /* Intrinsic: two columns when they fit, one when they don't. */
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 14rem), 1fr));
  }
  label {
    display: grid;
    gap: 0.25rem;
  }
  input,
  select {
    width: 100%;
    min-width: 0;
    max-width: 100%;
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    padding: 0.6rem 0.7rem;
  }
  .warning {
    margin: 0;
    color: var(--color-warning-text);
    background: var(--color-warning-soft);
    border: 1px solid var(--color-warning-border);
    border-radius: 10px;
    padding: 0.75rem;
    overflow-wrap: anywhere;
  }
  .summary {
    margin: 0;
    display: grid;
    gap: 0.45rem;
  }
  .summary div {
    min-width: 0;
    display: grid;
    grid-template-columns: 5rem minmax(0, 1fr);
    gap: 0.6rem;
  }
  .summary dt {
    color: var(--color-text-muted);
  }
  .summary dd {
    min-width: 0;
    margin: 0;
    overflow-wrap: anywhere;
  }
  .advanced-secret summary {
    cursor: pointer;
    color: var(--color-text-muted);
    font-size: 0.85rem;
  }
  .oauth-sign-in {
    display: grid;
    gap: 0.65rem;
  }
  .oauth-sign-in p {
    margin: 0;
  }
  .oauth-sign-in button {
    width: fit-content;
    min-height: 2rem;
  }
  .storage-metadata {
    margin: 0.5rem 0 0;
  }
  .account-status {
    width: fit-content;
    border: 1px solid var(--color-border-strong);
    border-radius: 999px;
    padding: 0.25rem 0.65rem;
    color: var(--color-text-muted);
  }
  .account-status.connected {
    border-color: var(--color-success-border);
    color: var(--color-success-text);
    background: var(--color-success-soft);
  }
  .storage-metadata div {
    display: grid;
    gap: 0.2rem;
  }
  .storage-metadata dt {
    color: var(--color-text-muted);
    font-size: 0.85rem;
  }
  .storage-metadata dd {
    min-width: 0;
    margin: 0;
    overflow-wrap: anywhere;
  }
  .advanced-secret label {
    margin-top: 0.5rem;
  }
  .hint {
    margin: 0.35rem 0 0;
    color: var(--color-text-faint);
    font-size: 0.8rem;
  }
  .message {
    border-radius: 999px;
    padding: 0.15rem 0.6rem;
    font-size: 0.82rem;
    font-weight: 600;
  }
  .message.success {
    background: var(--color-success-soft);
    color: var(--color-success-text);
  }
  .message.error {
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }
  .model-tools {
    display: flex;
    gap: 0.75rem;
    align-items: end;
    flex-wrap: wrap;
  }
  .model-picker {
    min-width: min(28rem, 100%);
  }
  button {
    border: none;
    border-radius: 10px;
    background: var(--color-accent);
    color: var(--color-accent-contrast);
    padding: 0.6rem 0.9rem;
  }
  button.icon-button {
    width: 2.5rem;
    min-width: 2.5rem;
    min-height: 2.5rem;
    padding: 0;
    display: inline-grid;
    place-items: center;
  }
  .default-after-save {
    display: inline-flex;
    grid-column: 1 / -1;
    align-items: center;
    gap: 0.5rem;
    margin-right: auto;
  }
  .default-after-save input {
    width: auto;
  }
  .secondary {
    background: var(--color-surface-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border-strong);
  }
  .danger {
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
    border: 1px solid var(--color-danger-border);
  }
  .confirm-inline {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
    color: var(--color-warning-text);
    font-size: 0.85rem;
  }
</style>
