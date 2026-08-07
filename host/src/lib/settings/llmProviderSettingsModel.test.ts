import { describe, expect, it } from "vitest";

import type { ConnectorConfigView } from "$lib/api";
import {
  beginEdit,
  beginSave,
  blankLlmProviderProfile,
  cancel,
  changeField,
  createDraftConnectorCard,
  createPersistedConnectorCard,
  discoverySuccess,
  discoveryInputsChanged,
  defaultBaseUrlForConnectorKind,
  normalizeConnectorDraft,
  saveFailure,
  saveSuccess,
  saveSuccessKeepBusy,
  syncConnectorCards,
} from "$lib/settings/llmProviderSettingsModel";

function connector(overrides: Partial<ConnectorConfigView> = {}): ConnectorConfigView {
  return {
    id: "llm-provider/local-ollama",
    kind: "ollama",
    base_url: "http://localhost:11434",
    default_model: "llama3.1",
    default_variant: null,
    default_text_verbosity: null,
    secret_refs: {},
    ...overrides,
  };
}

describe("llmProviderSettingsModel", () => {
  it("keeps an edited persisted draft through connector refreshes", () => {
    const current = changeField(
      beginEdit(createPersistedConnectorCard(connector())),
      connector({ default_model: "qwen3" }),
    );

    const next = syncConnectorCards([current], [connector({ default_model: "llama3.2" })]);

    expect(next).toHaveLength(1);
    expect(next[0].editing).toBe(true);
    expect(next[0].dirty).toBe(true);
    expect(next[0].draft.default_model).toBe("qwen3");
    expect(next[0].persistedConnector?.default_model).toBe("llama3.2");
  });

  it("promotes a newly saved draft card into the persisted list without dropping it", () => {
    const draftCard = createDraftConnectorCard(
      connector({ id: "llm-provider/work-openai", kind: "open-ai-compatible" }),
      "draft:1",
    );

    const next = syncConnectorCards([draftCard], [
      connector({
        id: "llm-provider/work-openai",
        kind: "open-ai-compatible",
        base_url: "https://example.test/v1",
        default_model: "gpt-4.1-mini",
        secret_refs: { api_key: "work-key" },
      }),
    ]);

    expect(next).toHaveLength(1);
    expect(next[0].key).toBe("draft:1");
    expect(next[0].persistedConnector?.id).toBe("llm-provider/work-openai");
    expect(next[0].draft.id).toBe("llm-provider/work-openai");
  });

  it("save failure keeps the current draft and editing state", () => {
    const current = {
      ...changeField(
        beginEdit(createPersistedConnectorCard(connector())),
        connector({ default_model: "qwen3" }),
      ),
      busy: true,
    };

    const next = saveFailure(current, "save failed");

    expect(next.busy).toBe(false);
    expect(next.editing).toBe(true);
    expect(next.draft.default_model).toBe("qwen3");
    expect(next.message).toBe("save failed");
  });

  it("cancel restores the persisted connector snapshot", () => {
    const current = changeField(
      beginEdit(createPersistedConnectorCard(connector())),
      connector({ default_model: "qwen3" }),
    );

    const next = cancel(current);

    expect(next?.editing).toBe(false);
    expect(next?.dirty).toBe(false);
    expect(next?.draft.default_model).toBe("llama3.1");
  });

  it("normalizes openai-compatible drafts without discarding secret refs", () => {
    const next = normalizeConnectorDraft(
      connector({
        id: " llm-provider/work-openai ",
        kind: "open-ai-compatible",
        base_url: " https://example.test/v1 ",
        default_model: " gpt-4.1 ",
        secret_refs: { api_key: "  " },
      }),
    );

    expect(next.id).toBe("llm-provider/work-openai");
    expect(next.base_url).toBe("https://example.test/v1");
    expect(next.default_model).toBe("gpt-4.1");
    expect(next.secret_refs.api_key).toBe("llm-provider/work-openai/api_key");
  });

  it("save success exits edit mode and keeps the persisted connector visible", () => {
    const current = changeField(
      createDraftConnectorCard(blankLlmProviderProfile(1), "draft:1"),
      connector({ id: "llm-provider/work-openai", kind: "open-ai-compatible" }),
    );

    const next = saveSuccess(
      current,
      connector({
        id: "llm-provider/work-openai",
        kind: "open-ai-compatible",
        base_url: "https://example.test/v1",
        default_model: "gpt-4.1-mini",
        secret_refs: { api_key: "work-key" },
      }),
      "Saved",
    );

    expect(next.editing).toBe(false);
    expect(next.dirty).toBe(false);
    expect(next.message).toBe("Saved");
    expect(next.persistedConnector?.id).toBe("llm-provider/work-openai");
  });

  it("discovered model selection updates the draft default model without clearing the list", () => {
    const discovered = discoverySuccess(
      createPersistedConnectorCard(connector()),
      [{ id: "llama3.2", display_name: null, variants: [], text_verbosity: [] }],
      "Discovered 1 model",
    );

    const next = changeField(discovered, connector({ default_model: "llama3.2" }));

    expect(next.draft.default_model).toBe("llama3.2");
    expect(next.discoveredModels).toEqual([{ id: "llama3.2", display_name: null, variants: [], text_verbosity: [] }]);
  });

  it("discovery failure keeps the current draft intact", () => {
    const current = {
      ...changeField(
        createDraftConnectorCard(blankLlmProviderProfile(1), "draft:1"),
        connector({
          id: "llm-provider/work-openai",
          kind: "open-ai-compatible",
          base_url: "https://example.test/v1",
          default_model: "gpt-4.1-mini",
          secret_refs: { api_key: "work-key" },
        }),
      ),
      busy: true,
    };

    const next = saveFailure(current, "discovery failed");

    expect(next.busy).toBe(false);
    expect(next.editing).toBe(true);
    expect(next.draft.default_model).toBe("gpt-4.1-mini");
    expect(next.message).toBe("discovery failed");
  });

  it("uses provider-appropriate default base URLs", () => {
    expect(defaultBaseUrlForConnectorKind("ollama")).toBe("http://localhost:11434");
    expect(defaultBaseUrlForConnectorKind("open-ai-compatible")).toBe("https://api.openai.com/v1");
    expect(defaultBaseUrlForConnectorKind("openai")).toBe("https://api.openai.com/v1");
    expect(defaultBaseUrlForConnectorKind("anthropic")).toBe("https://api.anthropic.com");
    expect(defaultBaseUrlForConnectorKind("anthropic-oauth")).toBe("https://api.anthropic.com");
    expect(defaultBaseUrlForConnectorKind("openai-codex")).toBe("https://chatgpt.com/backend-api");
    expect(defaultBaseUrlForConnectorKind("github-copilot")).toBe("https://api.githubcopilot.com");
    expect(defaultBaseUrlForConnectorKind("openrouter")).toBe("https://openrouter.ai/api/v1");
    expect(defaultBaseUrlForConnectorKind("google")).toBe("https://generativelanguage.googleapis.com");
    expect(defaultBaseUrlForConnectorKind("mistral")).toBe("https://api.mistral.ai/v1");
    expect(defaultBaseUrlForConnectorKind("amazon-bedrock")).toBe("https://bedrock-runtime.us-east-1.amazonaws.com");
  });

  it("normalizes the first supported credential shape for every non-Ollama kind", () => {
    for (const kind of ["open-ai-compatible", "openai", "anthropic", "openrouter", "google", "mistral", "amazon-bedrock"] as const) {
      expect(normalizeConnectorDraft(connector({ id: `llm-provider/${kind}`, kind })).secret_refs.api_key)
        .toBe(`llm-provider/${kind}/api_key`);
    }
  });

  it("drafts only an OAuth secret reference for OAuth providers", () => {
    for (const kind of ["anthropic-oauth", "openai-codex", "github-copilot"] as const) {
      const normalized = normalizeConnectorDraft(connector({
        id: `llm-provider/${kind}`,
        kind,
        secret_refs: { api_key: "must-not-survive" },
      }));
      expect(normalized.secret_refs).toEqual({ oauth: `llm-provider/${kind}/oauth` });
    }
  });

  it("normalizes Codex profiles to the ChatGPT subscription endpoint", () => {
    const normalized = normalizeConnectorDraft(connector({
      kind: "openai-codex",
      base_url: "https://api.openai.com/v1",
    }));

    expect(normalized.base_url).toBe("https://chatgpt.com/backend-api");
  });

  it("treats trailing-slash base URL edits as the same discovery endpoint", () => {
    expect(
      discoveryInputsChanged(
        connector({ base_url: "http://localhost:11434" }),
        connector({ base_url: "http://localhost:11434/" }),
      ),
    ).toBe(false);
  });

  it("mints unique profile ids for rapid consecutive adds", () => {
    expect(blankLlmProviderProfile().id).not.toBe(blankLlmProviderProfile().id);
  });

  it("beginSave adopts the normalized draft so a store refresh matches by trimmed id", () => {
    const untrimmed = changeField(
      createDraftConnectorCard(blankLlmProviderProfile(1), "draft:1"),
      connector({ id: " llm-provider/work-openai ", kind: "open-ai-compatible" }),
    );
    const normalized = normalizeConnectorDraft(untrimmed.draft);

    const saving = beginSave(untrimmed, normalized);
    expect(saving.busy).toBe(true);
    expect(saving.draft.id).toBe("llm-provider/work-openai");

    // The refresh that arrives mid-save must not mint a second card for the
    // same connector.
    const synced = syncConnectorCards([saving], [normalized]);
    expect(synced).toHaveLength(1);
    expect(synced[0].key).toBe("draft:1");
  });

  it("save-and-test stays busy after the save commits, until the probe resolves", () => {
    const editing = changeField(
      beginEdit(createPersistedConnectorCard(connector())),
      connector({ default_model: "qwen3" }),
    );

    const saved = saveSuccessKeepBusy(beginSave(editing, editing.draft), editing.draft);

    expect(saved.busy).toBe(true);
    expect(saved.editing).toBe(false);
    expect(saved.dirty).toBe(false);
    expect(saved.persistedConnector?.default_model).toBe("qwen3");
  });
});
