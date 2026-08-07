import type { ConnectorConfigView, ModelInfo, ModelVariant, TextVerbosity } from "$lib/api";
import {
  connectorKindSemantics,
  defaultApiKeySecretName,
  defaultOAuthSecretName,
} from "$lib/settings/connectorProfiles";

/// Severity of a card's status message, so the UI can distinguish a
/// confirmation ("Saved") from a failure instead of showing both the same.
export type ConnectorMessageKind = "success" | "error" | null;
export type OAuthConnectionStatus = "checking" | "connected" | "not-connected" | "error" | null;

export interface ConnectorProfileCard {
  key: string;
  persistedConnector: ConnectorConfigView | null;
  draft: ConnectorConfigView;
  discoveredModels: ModelInfo[];
  editing: boolean;
  dirty: boolean;
  busy: boolean;
  message: string | null;
  messageKind: ConnectorMessageKind;
  oauthStatus: OAuthConnectionStatus;
}

export function defaultBaseUrlForConnectorKind(kind: ConnectorConfigView["kind"]): string {
  return connectorKindSemantics(kind).defaultBaseUrl;
}

// Monotonic per session so two "Add profile" clicks in the same millisecond
// can never mint the same id (a duplicate id silently overwrites on save).
let nextProfileSeed = Date.now();

export function blankLlmProviderProfile(seed?: number): ConnectorConfigView {
  nextProfileSeed += 1;
  return {
    id: `llm-provider/profile-${seed ?? nextProfileSeed}`,
    kind: "ollama",
    base_url: defaultBaseUrlForConnectorKind("ollama"),
    default_model: "llama3.1",
    default_variant: null,
    default_text_verbosity: null,
    secret_refs: {},
  };
}

export function createPersistedConnectorCard(connector: ConnectorConfigView): ConnectorProfileCard {
  return {
    key: connector.id,
    persistedConnector: cloneConnector(connector),
    draft: cloneConnector(connector),
    discoveredModels: [],
    editing: false,
    dirty: false,
    busy: false,
    message: null,
    messageKind: null,
    oauthStatus: connectorKindSemantics(connector.kind).oauthRequired ? "checking" : null,
  };
}

export function createDraftConnectorCard(
  connector: ConnectorConfigView,
  key: string,
): ConnectorProfileCard {
  return {
    key,
    persistedConnector: null,
    draft: cloneConnector(connector),
    discoveredModels: [],
    editing: true,
    dirty: false,
    busy: false,
    message: null,
    messageKind: null,
    oauthStatus: connectorKindSemantics(connector.kind).oauthRequired ? "not-connected" : null,
  };
}

export function syncConnectorCards(
  cards: ConnectorProfileCard[],
  persistedConnectors: ConnectorConfigView[],
): ConnectorProfileCard[] {
  const remaining = [...cards];
  const next: ConnectorProfileCard[] = [];

  for (const connector of persistedConnectors) {
    const index = remaining.findIndex((card) => matchesConnector(card, connector.id));
    if (index === -1) {
      next.push(createPersistedConnectorCard(connector));
      continue;
    }

    const [card] = remaining.splice(index, 1);
    next.push(syncConnectorCard(card, connector));
  }

  for (const card of remaining) {
    if (!card.persistedConnector) {
      next.push(card);
    }
  }

  return next;
}

export function beginEdit(card: ConnectorProfileCard): ConnectorProfileCard {
  const draft = cloneConnector(card.draft);
  if (draft.kind === "openai-codex") {
    draft.base_url = connectorKindSemantics(draft.kind).defaultBaseUrl;
  }
  return {
    ...card,
    draft,
    editing: true,
    message: null,
    messageKind: null,
  };
}

export function changeField(
  card: ConnectorProfileCard,
  draft: ConnectorConfigView,
): ConnectorProfileCard {
  const resetDiscoveredModels = discoveryInputsChanged(card.draft, draft);
  const oauthChanged = card.draft.kind !== draft.kind ||
    card.draft.secret_refs.oauth !== draft.secret_refs.oauth;
  return {
    ...card,
    draft: cloneConnector(draft),
    discoveredModels: resetDiscoveredModels ? [] : cloneModels(card.discoveredModels),
    editing: true,
    dirty: true,
    message: null,
    messageKind: null,
    oauthStatus: connectorKindSemantics(draft.kind).oauthRequired
      ? oauthChanged ? (card.persistedConnector ? "checking" : "not-connected") : card.oauthStatus
      : null,
  };
}

export function cancel(card: ConnectorProfileCard): ConnectorProfileCard | null {
  if (!card.persistedConnector) {
    return null;
  }

  return {
    ...card,
    draft: cloneConnector(card.persistedConnector),
    editing: false,
    dirty: false,
    busy: false,
    message: null,
    messageKind: null,
    oauthStatus: connectorKindSemantics(card.persistedConnector.kind).oauthRequired ? "checking" : null,
  };
}

export function saveSuccess(
  card: ConnectorProfileCard,
  connector: ConnectorConfigView,
  message: string | null,
): ConnectorProfileCard {
  return {
    ...card,
    persistedConnector: cloneConnector(connector),
    draft: cloneConnector(connector),
    editing: false,
    dirty: false,
    busy: false,
    message,
    messageKind: message ? "success" : null,
  };
}

export function saveFailure(
  card: ConnectorProfileCard,
  message: string,
): ConnectorProfileCard {
  return {
    ...card,
    busy: false,
    message,
    messageKind: "error",
  };
}

export function setBusy(card: ConnectorProfileCard): ConnectorProfileCard {
  return {
    ...card,
    busy: true,
    message: null,
    messageKind: null,
  };
}

/// Start a save: the normalized draft replaces the raw one so a later store
/// sync matches the card by its persisted (trimmed) id instead of minting a
/// duplicate card for the same connector.
export function beginSave(
  card: ConnectorProfileCard,
  normalizedDraft: ConnectorConfigView,
): ConnectorProfileCard {
  return {
    ...card,
    draft: cloneConnector(normalizedDraft),
    busy: true,
    message: null,
    messageKind: null,
  };
}

/// Commit a successful save but stay busy for a follow-up probe (connection
/// test) so the card cannot be edited or deleted mid-flight.
export function saveSuccessKeepBusy(
  card: ConnectorProfileCard,
  connector: ConnectorConfigView,
): ConnectorProfileCard {
  return {
    ...saveSuccess(card, connector, null),
    busy: true,
  };
}

export function testSuccess(card: ConnectorProfileCard, message: string): ConnectorProfileCard {
  return {
    ...card,
    busy: false,
    message,
    messageKind: "success",
  };
}

export function discoverySuccess(
  card: ConnectorProfileCard,
  models: ModelInfo[],
  message: string,
): ConnectorProfileCard {
  return {
    ...card,
    discoveredModels: cloneModels(models),
    busy: false,
    message,
    messageKind: "success",
  };
}

export function normalizeConnectorDraft(draft: ConnectorConfigView): ConnectorConfigView {
  const normalized: ConnectorConfigView = {
    ...draft,
    id: draft.id.trim(),
    base_url: draft.base_url.trim(),
    default_model: draft.default_model.trim(),
    secret_refs: { ...draft.secret_refs },
  };

  const semantics = connectorKindSemantics(normalized.kind);
  if (normalized.kind === "openai-codex") {
    normalized.base_url = semantics.defaultBaseUrl;
  }
  if (semantics.oauthRequired) {
    normalized.secret_refs.oauth =
      normalized.secret_refs.oauth?.trim() || defaultOAuthSecretName(normalized.id);
    delete normalized.secret_refs.api_key;
  } else if (normalized.kind !== "ollama") {
    normalized.secret_refs.api_key =
      normalized.secret_refs.api_key?.trim() || defaultApiKeySecretName(normalized.id);
    delete normalized.secret_refs.oauth;
  } else {
    delete normalized.secret_refs.api_key;
    delete normalized.secret_refs.oauth;
  }

  return normalized;
}

function syncConnectorCard(
  card: ConnectorProfileCard,
  connector: ConnectorConfigView,
): ConnectorProfileCard {
  const persistedConnector = cloneConnector(connector);
  if (card.editing || card.dirty || card.busy) {
    return {
      ...card,
      persistedConnector,
    };
  }

  return {
    ...card,
    persistedConnector,
    draft: cloneConnector(connector),
  };
}

function matchesConnector(card: ConnectorProfileCard, connectorId: string): boolean {
  return card.persistedConnector?.id === connectorId ||
    (!card.persistedConnector && card.draft.id === connectorId);
}

function cloneConnector(connector: ConnectorConfigView): ConnectorConfigView {
  return {
    ...connector,
    secret_refs: { ...connector.secret_refs },
  };
}

function cloneModels(models: ModelInfo[]): ModelInfo[] {
  return models.map((model) => ({
    ...model,
    variants: [...model.variants],
    text_verbosity: [...model.text_verbosity],
  }));
}

export function modelVariantLabel(variant: ModelVariant): string {
  switch (variant) {
    case "minimal": return "Minimal";
    case "low": return "Low";
    case "medium": return "Medium";
    case "high": return "High";
    case "xhigh": return "Extra high";
    case "max": return "Maximum";
  }
}

export function textVerbosityLabel(verbosity: TextVerbosity): string {
  switch (verbosity) {
    case "low": return "Low";
    case "medium": return "Medium";
    case "high": return "High";
  }
}

export function discoveryInputsChanged(
  current: ConnectorConfigView,
  next: ConnectorConfigView,
): boolean {
  const currentBaseUrl = current.base_url.trim().replace(/\/+$/, "");
  const nextBaseUrl = next.base_url.trim().replace(/\/+$/, "");
  return current.id !== next.id ||
    current.kind !== next.kind ||
    currentBaseUrl !== nextBaseUrl ||
    current.secret_refs.api_key !== next.secret_refs.api_key ||
    current.secret_refs.oauth !== next.secret_refs.oauth;
}
