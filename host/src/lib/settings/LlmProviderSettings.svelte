<script lang="ts">
  import { startLlmOAuth, type ConnectorConfigView } from "$lib/api";
  import ConnectorSettingsPanel from "$lib/settings/ConnectorSettingsPanel.svelte";
  import { validateConnectorConfig } from "$lib/settings/configValidation";
  import {
    CHATGPT_CODEX_DEFAULT_MODEL,
    connectorIsCloud,
    connectorKindSemantics,
    connectorProfileName,
    connectorUsesOAuth,
    defaultApiKeySecretName,
  } from "$lib/settings/connectorProfiles";
  import {
    acknowledgeCloudLlmProfile,
    selectedCloudLlmPolicy,
  } from "$lib/settings/llmProfilePolicy";
  import {
    beginEdit,
    beginSave,
    blankLlmProviderProfile,
    cancel,
    changeField,
    createDraftConnectorCard,
    discoverySuccess,
    modelVariantLabel,
    normalizeConnectorDraft,
    saveFailure,
    saveSuccess,
    saveSuccessKeepBusy,
    setBusy,
    syncConnectorCards,
    testSuccess,
    type ConnectorProfileCard,
  } from "$lib/settings/llmProviderSettingsModel";
  import {
    connectorConfigs,
    checkSecret,
    hostConfig,
    removeConnector,
    removeSecret,
    runConnectorTest,
    runDraftModelDiscovery,
    saveConnector,
    saveHostPatch,
  } from "$lib/stores/config";
  import { onDestroy } from "svelte";
  import {
    forgetOAuthSessionResult,
    oauthSessionResults,
    registerStartedOAuthSession,
    type OAuthSessionResult,
  } from "$lib/stores/chromeState";
  import LoadingIndicator from "$lib/shell/LoadingIndicator.svelte";

  let cards = $state<ConnectorProfileCard[]>([]);
  let nextDraftKey = 0;
  let pendingDefaultLlmConnectorId = $state<string | null>(null);
  let policyMessage = $state<string | null>(null);
  const oauthSessionTargets = new Map<string, {
    cardKey: string;
    accountLabel: string;
    previousStatus: ConnectorProfileCard["oauthStatus"];
  }>();
  let latestOAuthResults: OAuthSessionResult[] = [];

  const defaultConnectorId = $derived(
    $hostConfig?.host.default_llm_profile
      ? `${$hostConfig.host.default_llm_provider}/${$hostConfig.host.default_llm_profile}`
      : null,
  );
  const providerConfigs = $derived(
    $connectorConfigs.filter((connector) => connector.id.startsWith("llm-provider/")),
  );
  const selectedDefaultLlmConnectorId = $derived(
    pendingDefaultLlmConnectorId ?? defaultConnectorId ?? "",
  );
  const activeCloudPolicy = $derived(
    selectedCloudLlmPolicy($hostConfig, $connectorConfigs, defaultConnectorId),
  );
  const pendingCloudPolicy = $derived(
    selectedCloudLlmPolicy($hostConfig, $connectorConfigs, pendingDefaultLlmConnectorId),
  );

  const unsubscribe = connectorConfigs.subscribe((allConnectors) => {
    const persistedConnectors = allConnectors.filter((connector) =>
      connector.id.startsWith("llm-provider/"),
    );
    cards = syncConnectorCards(cards, persistedConnectors);
    for (const card of cards) {
      if (card.persistedConnector && card.oauthStatus === "checking") {
        void refreshOAuthStatus(card.key);
      }
    }
  });

  const unsubscribeOAuthResults = oauthSessionResults.subscribe((results) => {
    latestOAuthResults = results;
    applyOAuthSessionResults(results);
  });

  onDestroy(() => {
    unsubscribe();
    unsubscribeOAuthResults();
  });

  function updateCard(key: string, updater: (card: ConnectorProfileCard) => ConnectorProfileCard | null) {
    cards = cards.flatMap((card) => {
      if (card.key !== key) {
        return [card];
      }
      const next = updater(card);
      return next ? [next] : [];
    });
  }

  function findCard(key: string): ConnectorProfileCard | undefined {
    return cards.find((card) => card.key === key);
  }

  function addProfile() {
    nextDraftKey += 1;
    cards = [...cards, createDraftConnectorCard(blankLlmProviderProfile(), `draft:${nextDraftKey}`)];
  }

  function addChatGptProfile() {
    nextDraftKey += 1;
    const draft = blankLlmProviderProfile();
    cards = [...cards, createDraftConnectorCard({
      ...draft,
      kind: "openai-codex",
      base_url: connectorKindSemantics("openai-codex").defaultBaseUrl,
      default_model: CHATGPT_CODEX_DEFAULT_MODEL,
      default_variant: null,
      default_text_verbosity: null,
    }, `draft:${nextDraftKey}`)];
  }

  async function updateDefaultLlmProfile(connectorId: string) {
    policyMessage = null;
    if (connectorId === "") {
      pendingDefaultLlmConnectorId = null;
      try {
        await saveHostPatch({ host: { default_llm_profile: null } });
      } catch (error) {
        policyMessage = String(error);
      }
      return;
    }
    const policy = selectedCloudLlmPolicy($hostConfig, $connectorConfigs, connectorId);
    if (policy && !policy.acknowledged) {
      pendingDefaultLlmConnectorId = connectorId;
      return;
    }
    pendingDefaultLlmConnectorId = null;
    try {
      await saveHostPatch({ host: { default_llm_profile: connectorProfileName(connectorId) } });
    } catch (error) {
      policyMessage = String(error);
    }
  }

  async function acceptCloudDefault(connectorId: string) {
    if (!$hostConfig) return;
    const policy = selectedCloudLlmPolicy($hostConfig, $connectorConfigs, connectorId);
    if (!policy) {
      pendingDefaultLlmConnectorId = null;
      return;
    }
    policyMessage = null;
    try {
      await saveHostPatch({
        host: {
          default_llm_profile: connectorProfileName(connectorId),
          cloud_llm_egress_accepted_profiles: acknowledgeCloudLlmProfile(
            $hostConfig.host.cloud_llm_egress_accepted_profiles,
            policy.connectorId,
          ),
        },
      });
      pendingDefaultLlmConnectorId = null;
    } catch (error) {
      policyMessage = String(error);
    }
  }

  function cancelPendingCloudDefault() {
    pendingDefaultLlmConnectorId = null;
    policyMessage = null;
  }

  async function refreshOAuthStatus(key: string) {
    const current = findCard(key);
    const connector = current?.persistedConnector;
    if (!connector || !connectorUsesOAuth(connector.kind)) return;
    const secretName = connector.secret_refs.oauth;
    if (!secretName) return;
    try {
      const connected = await checkSecret("llm-provider", secretName);
      updateCard(key, (card) => {
        if (card.persistedConnector?.id !== connector.id || card.persistedConnector.secret_refs.oauth !== secretName) {
          return card;
        }
        return { ...card, oauthStatus: connected ? "connected" : "not-connected" };
      });
    } catch {
      updateCard(key, (card) => card.persistedConnector?.id === connector.id
        ? { ...card, oauthStatus: "error" }
        : card);
    }
  }

  function applyOAuthSessionResults(results: OAuthSessionResult[]) {
    for (const result of results) {
      const target = oauthSessionTargets.get(result.sessionId);
      if (!target) continue;
      oauthSessionTargets.delete(result.sessionId);
      if (result.status === "completed") {
        updateCard(target.cardKey, (card) => testSuccess(
          { ...card, oauthStatus: "connected" },
          `${target.accountLabel} connected.`,
        ));
      } else {
        updateCard(target.cardKey, (card) => saveFailure(
          { ...card, oauthStatus: target.previousStatus },
          result.message ? `Sign-in failed: ${result.message}` : "Sign-in failed.",
        ));
      }
      forgetOAuthSessionResult(result.sessionId);
    }
  }

  function editCard(key: string) {
    updateCard(key, beginEdit);
  }

  function changeDraft(key: string, draft: ConnectorConfigView) {
    updateCard(key, (card) => changeField(card, draft));
  }

  function cancelCard(key: string) {
    updateCard(key, cancel);
    void refreshOAuthStatus(key);
  }

  function requiresCloudAcceptance(card: ConnectorProfileCard): boolean {
    return card.persistedConnector?.id === defaultConnectorId
      && connectorIsCloud(card.draft)
      && !($hostConfig?.host.cloud_llm_egress_accepted_profiles.includes(card.draft.id) ?? false);
  }

  async function saveCard(key: string, acknowledgeDataEgress = false, makeDefault = false) {
    const current = findCard(key);
    if (!current) {
      return;
    }

    let normalized: ConnectorConfigView;
    try {
      normalized = normalizeConnectorDraft(current.draft);
      validateConnectorConfig(normalized);
    } catch (error) {
      updateCard(key, (card) => saveFailure(card, String(error)));
      return;
    }

    updateCard(key, (card) => beginSave(card, normalized));
    try {
      const saved = await saveConnector(normalized, acknowledgeDataEgress);
      updateCard(key, (card) => saveSuccess(card, saved, "Saved"));
      if (makeDefault) await updateDefaultLlmProfile(saved.id);
    } catch (error) {
      updateCard(key, (card) => saveFailure(card, String(error)));
    }
  }

  async function testCard(key: string) {
    const current = findCard(key);
    if (!current) {
      return;
    }

    let connectorId = current.persistedConnector?.id ?? current.draft.id;
    let normalized: ConnectorConfigView | null = null;

    if (current.editing) {
      try {
        normalized = normalizeConnectorDraft(current.draft);
        validateConnectorConfig(normalized);
        connectorId = normalized.id;
      } catch (error) {
        updateCard(key, (card) => saveFailure(card, String(error)));
        return;
      }
    }

    updateCard(key, (card) => (normalized ? beginSave(card, normalized) : setBusy(card)));
    try {
      if (normalized) {
        const saved = await saveConnector(normalized);
        // Stay busy through the connection test so the card cannot be
        // edited or deleted while the probe is in flight.
        updateCard(key, (card) => saveSuccessKeepBusy(card, saved));
      }
      const result = await runConnectorTest(connectorId);
      updateCard(key, (card) => testSuccess(card, result.message));
    } catch (error) {
      updateCard(key, (card) => saveFailure(card, String(error)));
    }
  }

  // Discovery needs no save or full validation. OAuth providers use pi-ai's
  // bundled catalog; other providers may probe the draft endpoint directly.
  async function discoverModelsForCard(key: string) {
    const current = findCard(key);
    if (!current) {
      return;
    }

    const draft = current.draft;
    if (draft.base_url.trim() === "") {
      updateCard(key, (card) => saveFailure(card, "Enter a base URL first."));
      return;
    }
    const apiKeySecretName =
      draft.kind !== "ollama" && !connectorUsesOAuth(draft.kind)
        ? draft.secret_refs.api_key?.trim() || defaultApiKeySecretName(draft.id.trim())
        : null;

    updateCard(key, setBusy);
    try {
      const result = await runDraftModelDiscovery(
        draft.kind,
        draft.base_url.trim(),
        draft.default_model.trim() || null,
        apiKeySecretName,
      );
      updateCard(key, (card) => discoverySuccess(card, result.models, result.message));
    } catch (error) {
      updateCard(key, (card) => saveFailure(card, String(error)));
    }
  }

  async function deleteCard(key: string) {
    const current = findCard(key);
    if (!current) {
      return;
    }

    if (!current.persistedConnector) {
      updateCard(key, () => null);
      return;
    }

    if (current.persistedConnector.id === defaultConnectorId) {
      updateCard(key, (card) => saveFailure(card, "Cannot delete the default LLM profile. Clear it or choose a new default first."));
      return;
    }

    updateCard(key, setBusy);
    try {
      await removeConnector(current.persistedConnector.id);
      updateCard(key, () => null);
    } catch (error) {
      updateCard(key, (card) => saveFailure(card, String(error)));
    }
  }

  async function signInCard(key: string) {
    const current = findCard(key);
    if (!current?.persistedConnector || current.editing) {
      return;
    }
    updateCard(key, setBusy);
    try {
      const sessionId = await startLlmOAuth(current.persistedConnector.id);
      const semantics = connectorKindSemantics(current.persistedConnector.kind);
      oauthSessionTargets.set(sessionId, {
        cardKey: key,
        accountLabel: semantics.oauthAccountLabel ?? "Model account",
        previousStatus: current.oauthStatus,
      });
      registerStartedOAuthSession(sessionId);
      updateCard(key, (card) => ({
        ...card,
        message: "Complete sign-in in the verified host dialog.",
        messageKind: null,
      }));
      applyOAuthSessionResults(latestOAuthResults);
    } catch (error) {
      updateCard(key, (card) => saveFailure(card, String(error)));
    }
  }

  async function disconnectCard(key: string) {
    const current = findCard(key);
    const connector = current?.persistedConnector;
    const secretName = connector?.secret_refs.oauth;
    if (!connector || !secretName || current.editing) return;
    updateCard(key, setBusy);
    try {
      await removeSecret("llm-provider", secretName);
      const accountLabel = connectorKindSemantics(connector.kind).oauthAccountLabel ?? "Model account";
      updateCard(key, (card) => testSuccess(
        { ...card, oauthStatus: "not-connected" },
        `${accountLabel} disconnected.`,
      ));
    } catch (error) {
      updateCard(key, (card) => saveFailure(card, String(error)));
    }
  }
</script>

<section class="stack">
  {#if !$hostConfig}
    <LoadingIndicator fill label="Loading providers…" />
  {:else}
    <section class="default-choice" aria-labelledby="default-chat-model-heading">
      <div class="section-heading">
        <h3 id="default-chat-model-heading">Default for Chat</h3>
      </div>
      {#if providerConfigs.length > 0}
        <label>
          Provider profile
          <select
            value={selectedDefaultLlmConnectorId}
            onchange={(event) => updateDefaultLlmProfile((event.currentTarget as HTMLSelectElement).value)}
          >
            <option value="">No provider selected</option>
            {#each providerConfigs as connector (connector.id)}
              <option value={connector.id}>
                {connectorProfileName(connector.id)} - {connector.default_model}{connector.default_variant ? ` (${modelVariantLabel(connector.default_variant)})` : ""}
              </option>
            {/each}
          </select>
        </label>
        {#if pendingCloudPolicy && !pendingCloudPolicy.acknowledged}
          <div class="warning" role="alert">
            <strong>Cloud profile</strong>
            <p>{pendingCloudPolicy.profileId}: chat content and tool results may leave this device.</p>
            <div class="warning-actions">
              <button type="button" class="secondary-button" onclick={cancelPendingCloudDefault}>
                Keep current default
              </button>
              <button type="button" onclick={() => acceptCloudDefault(pendingCloudPolicy.connectorId)}>
                Accept and make default
              </button>
            </div>
          </div>
        {:else if activeCloudPolicy}
          <div class="warning">
            <strong>Cloud profile active</strong>
            <p>{activeCloudPolicy.profileId}: chat content and tool results may leave this device.</p>
            {#if !activeCloudPolicy.acknowledged}
              <div class="warning-actions">
                <button type="button" onclick={() => acceptCloudDefault(activeCloudPolicy.connectorId)}>
                  Accept
                </button>
              </div>
            {/if}
          </div>
        {/if}
        {#if policyMessage}
          <p class="error" role="alert">{policyMessage}</p>
        {/if}
      {:else}
        <p class="empty-copy">Add and save a provider profile to choose a default.</p>
      {/if}
    </section>
  {/if}
  <div class="section-heading provider-heading">
    <h3>Provider profiles</h3>
  </div>
  {#each cards as card (card.key)}
    <ConnectorSettingsPanel
      {card}
      isDefault={card.persistedConnector?.id === defaultConnectorId}
      requiresCloudAcceptance={requiresCloudAcceptance(card)}
      onBeginEdit={() => editCard(card.key)}
      onDraftChange={(draft) => changeDraft(card.key, draft)}
      onSave={(makeDefault) => saveCard(card.key, false, makeDefault)}
      onAcceptCloudSave={(makeDefault) => saveCard(card.key, true, makeDefault)}
      onCancel={() => cancelCard(card.key)}
      onDiscoverModels={() => discoverModelsForCard(card.key)}
      onTest={() => testCard(card.key)}
      onSignIn={() => signInCard(card.key)}
      onDisconnect={() => disconnectCard(card.key)}
      onDelete={() => deleteCard(card.key)}
    />
  {:else}
    <p class="empty-copy">No provider profiles configured.</p>
  {/each}
  <div class="add-actions">
    <button class="add chatgpt" onclick={addChatGptProfile}>Add ChatGPT account</button>
    <button class="add" onclick={addProfile}>Add another provider</button>
  </div>
</section>

<style>
  .stack {
    display: grid;
    gap: 0.85rem;
  }
  .default-choice {
    min-width: 0;
    display: grid;
    gap: 0.75rem;
    padding-bottom: 1rem;
    border-bottom: 1px solid var(--color-border-subtle);
  }
  .section-heading {
    display: grid;
    gap: 0.2rem;
  }
  .section-heading h3,
  .empty-copy {
    margin: 0;
  }
  .section-heading h3 {
    font-size: 1.02rem;
  }
  .empty-copy {
    color: var(--color-text-muted);
    font-size: 0.85rem;
    overflow-wrap: anywhere;
  }
  .provider-heading {
    margin-top: 0.15rem;
  }
  label {
    min-width: 0;
    display: grid;
    gap: 0.25rem;
  }
  select {
    width: 100%;
    min-width: 0;
    max-width: 100%;
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    padding: 0.6rem 0.7rem;
  }
  .warning {
    min-width: 0;
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
  .warning-actions {
    display: flex;
    gap: 0.6rem;
    flex-wrap: wrap;
  }
  .secondary-button {
    background: var(--color-surface-raised);
    color: var(--color-text);
  }
  .error {
    margin: 0;
    color: var(--color-danger-text);
  }
  .add {
    width: fit-content;
    border: 1px dashed var(--color-border-strong);
    background: transparent;
    border-radius: 10px;
    padding: 0.6rem 0.9rem;
  }
  .add-actions {
    display: flex;
    gap: 0.65rem;
    flex-wrap: wrap;
  }
  .add.chatgpt {
    border-style: solid;
    border-color: var(--color-accent);
    color: var(--color-accent);
  }
  @media (max-width: 30em) {
    .warning button,
    .add-actions button {
      width: 100%;
      max-width: 100%;
    }
  }
</style>
