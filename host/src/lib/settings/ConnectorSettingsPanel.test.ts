import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import ConnectorSettingsPanel from "$lib/settings/ConnectorSettingsPanel.svelte";
import { beginEdit, createPersistedConnectorCard, discoverySuccess } from "$lib/settings/llmProviderSettingsModel";

const handlers = {
  onBeginEdit: vi.fn(),
  onDraftChange: vi.fn(),
  onSave: vi.fn(),
  onAcceptCloudSave: vi.fn(),
  onCancel: vi.fn(),
  onDiscoverModels: vi.fn(),
  onTest: vi.fn(),
  onSignIn: vi.fn(),
  onDisconnect: vi.fn(),
  onDelete: vi.fn(),
};

describe("ConnectorSettingsPanel OAuth", () => {
  it("shows host storage metadata and sign-in, never prompt or API-key UI", () => {
    const connector = {
      id: "llm-provider/codex",
      kind: "openai-codex" as const,
      base_url: "https://chatgpt.com/backend-api",
      default_model: "gpt-5.4-mini",
      default_variant: null,
      default_text_verbosity: null,
      secret_refs: { oauth: "llm-provider/codex/oauth" },
    };
    const { container } = render(ConnectorSettingsPanel, {
      card: createPersistedConnectorCard(connector),
      isDefault: false,
      requiresCloudAcceptance: false,
      ...handlers,
    });

    expect(screen.getByRole("button", { name: "Connect ChatGPT account" })).toBeTruthy();
    expect(screen.getByText(/ChatGPT Plus or Pro account/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Test connection" })).toBeNull();
    expect(screen.getByText("llm-provider/codex/oauth")).toBeTruthy();
    expect(screen.queryByText("API key")).toBeNull();
    expect(screen.queryByRole("textbox", { name: /token|code|secret/i })).toBeNull();
    expect(container.querySelector("input[type='password']")).toBeNull();
    expect(container.textContent).not.toContain("Account secret");
  });

  it("shows discovered thinking variants and updates the profile variant", async () => {
    const connector = {
      id: "llm-provider/codex",
      kind: "openai-codex" as const,
      base_url: "https://chatgpt.com/backend-api",
      default_model: "gpt-5.6-sol",
      default_variant: null,
      default_text_verbosity: null,
      secret_refs: { oauth: "llm-provider/codex/oauth" },
    };
    const card = discoverySuccess(
      beginEdit(createPersistedConnectorCard(connector)),
      [{
        id: "gpt-5.6-sol",
        display_name: "GPT-5.6 Sol",
        variants: ["minimal", "low", "medium", "high", "xhigh", "max"],
        text_verbosity: ["low", "medium", "high"],
      }],
      "Discovered 1 model",
    );

    render(ConnectorSettingsPanel, {
      card,
      isDefault: false,
      requiresCloudAcceptance: false,
      ...handlers,
    });

    expect(screen.getByRole("option", { name: "Extra high" })).toBeTruthy();
    await fireEvent.change(screen.getByLabelText("Model variant"), { target: { value: "xhigh" } });
    expect(handlers.onDraftChange).toHaveBeenCalledWith(expect.objectContaining({
      default_model: "gpt-5.6-sol",
      default_variant: "xhigh",
    }));

    await fireEvent.change(screen.getByLabelText("Text verbosity"), { target: { value: "high" } });
    expect(handlers.onDraftChange).toHaveBeenCalledWith(expect.objectContaining({
      default_model: "gpt-5.6-sol",
      default_text_verbosity: "high",
    }));
  });

  it("clears provider-specific model controls when the provider changes", async () => {
    const connector = {
      id: "llm-provider/codex",
      kind: "openai-codex" as const,
      base_url: "https://chatgpt.com/backend-api",
      default_model: "gpt-5.4-mini",
      default_variant: "high" as const,
      default_text_verbosity: "high" as const,
      secret_refs: { oauth: "llm-provider/codex/oauth" },
    };

    render(ConnectorSettingsPanel, {
      card: beginEdit(createPersistedConnectorCard(connector)),
      isDefault: false,
      requiresCloudAcceptance: false,
      ...handlers,
    });
    await fireEvent.change(screen.getByLabelText("Kind"), { target: { value: "anthropic-oauth" } });

    expect(handlers.onDraftChange).toHaveBeenLastCalledWith(expect.objectContaining({
      kind: "anthropic-oauth",
      default_model: "",
      default_variant: null,
      default_text_verbosity: null,
    }));
  });
});
