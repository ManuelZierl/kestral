import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { get } from "svelte/store";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ConnectorConfigView, HostConfig } from "$lib/api";
import LlmProviderSettings from "$lib/settings/LlmProviderSettings.svelte";
import { connectorConfigs, hostConfig } from "$lib/stores/config";
import {
  oauthSessionResults,
  recordOAuthSessionResult,
  startedOAuthSessions,
} from "$lib/stores/chromeState";

const {
  clearSecret,
  getHostConfig,
  hasSecret,
  listConnectorConfigs,
  startLlmOAuth,
  updateHostConfig,
  upsertConnectorConfig,
} = vi.hoisted(() => ({
  clearSecret: vi.fn(),
  getHostConfig: vi.fn(),
  hasSecret: vi.fn(),
  listConnectorConfigs: vi.fn(),
  startLlmOAuth: vi.fn(),
  updateHostConfig: vi.fn(),
  upsertConnectorConfig: vi.fn(),
}));

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    clearSecret,
    getHostConfig,
    hasSecret,
    listConnectorConfigs,
    startLlmOAuth,
    updateHostConfig,
    upsertConnectorConfig,
  };
});

function config(defaultProfile: string | null = "local-ollama"): HostConfig {
  return {
    version: 1,
    host: {
      default_llm_provider: "llm-provider",
      default_llm_profile: defaultProfile,
      cloud_llm_egress_accepted_profiles: [],
      app_data_backup_retention: 1,
    },
    apps: {},
    connectors: {},
    mcp_servers: {},
    mcp_exports: {},
    mcp_export_transitions: {},
    mcp_gateway: {
      enabled: false,
      bind_address: "127.0.0.1:8137",
      allowed_origins: [],
      oauth_enabled: false,
    },
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  startLlmOAuth.mockResolvedValue("oauth-session-1");
  clearSecret.mockResolvedValue(undefined);
  hasSecret.mockResolvedValue(false);
  const current = config();
  updateHostConfig.mockResolvedValue(current);
  getHostConfig.mockResolvedValue(current);
  listConnectorConfigs.mockResolvedValue([]);
  upsertConnectorConfig.mockImplementation(async (connector) => connector);
  hostConfig.set(current);
  connectorConfigs.set([{
    id: "llm-provider/codex",
    kind: "openai-codex",
     base_url: "https://chatgpt.com/backend-api",
     default_model: "gpt-5.4-mini",
     default_variant: null,
     default_text_verbosity: null,
     secret_refs: { oauth: "llm-provider/codex/oauth" },
  }]);
  startedOAuthSessions.set([]);
  oauthSessionResults.set([]);
});

describe("LlmProviderSettings OAuth", () => {
  it("starts only a saved profile and hands interaction to trusted chrome", async () => {
    render(LlmProviderSettings);
    expect(await screen.findByText("Not connected")).toBeTruthy();
    await fireEvent.click(await screen.findByRole("button", { name: "Connect ChatGPT account" }));

    expect(startLlmOAuth).toHaveBeenCalledWith("llm-provider/codex");
    expect(get(startedOAuthSessions)).toContain("oauth-session-1");
    expect(await screen.findByText("Complete sign-in in the verified host dialog.")).toBeTruthy();

    recordOAuthSessionResult({ sessionId: "oauth-session-1", status: "completed", message: null });
    expect(await screen.findByText("ChatGPT account connected.")).toBeTruthy();
    expect(screen.getByText("Connected")).toBeTruthy();
  });

  it("creates a ChatGPT subscription draft with a usable endpoint and model", async () => {
    connectorConfigs.set([]);
    render(LlmProviderSettings);

    await fireEvent.click(screen.getByRole("button", { name: "Add ChatGPT account" }));

    expect((screen.getByLabelText("Kind") as HTMLSelectElement).value).toBe("openai-codex");
    expect((screen.getByLabelText("Base URL") as HTMLInputElement).value).toBe("https://chatgpt.com/backend-api");
    expect((screen.getByLabelText("Default model") as HTMLInputElement).value).toBe("gpt-5.4-mini");
  });

  it("can make a new profile the Chat default when saving", async () => {
    connectorConfigs.set([]);
    render(LlmProviderSettings);

    await fireEvent.click(screen.getByRole("button", { name: "Add another provider" }));
    const profileId = (screen.getByLabelText("Profile id") as HTMLInputElement).value;
    await fireEvent.click(screen.getByRole("checkbox", { name: "Use as Chat default" }));
    await fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(upsertConnectorConfig).toHaveBeenCalled();
      expect(updateHostConfig).toHaveBeenCalledWith({
        host: { default_llm_profile: profileId.replace("llm-provider/", "") },
      });
    });
  });

  it("disconnects a connected ChatGPT account through protected host storage", async () => {
    hasSecret.mockResolvedValue(true);
    render(LlmProviderSettings);
    expect(await screen.findByText("Connected")).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Disconnect account" }));
    await fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));

    expect(clearSecret).toHaveBeenCalledWith("llm-provider", "llm-provider/codex/oauth");
    expect(await screen.findByText("ChatGPT account disconnected.")).toBeTruthy();
  });
});

describe("LlmProviderSettings default profile", () => {
  const providers: ConnectorConfigView[] = [
    {
      id: "llm-provider/local-ollama",
      kind: "ollama",
      base_url: "http://localhost:11434",
      default_model: "llama3.1",
      default_variant: null,
      default_text_verbosity: null,
      secret_refs: {},
    },
    {
      id: "llm-provider/work-openai",
      kind: "open-ai-compatible",
      base_url: "https://example.test/v1",
      default_model: "gpt-4.1",
      default_variant: null,
      default_text_verbosity: null,
      secret_refs: { api_key: "work-key" },
    },
  ];

  it("shows an explicit unconfigured state for a fresh profile", () => {
    hostConfig.set(config(null));
    connectorConfigs.set([]);

    render(LlmProviderSettings);

    expect(screen.getByText("Add and save a provider profile to choose a default.")).toBeTruthy();
    expect(screen.getByText("No provider profiles configured.")).toBeTruthy();
  });

  it("allows the Chat default to be cleared without deleting saved profiles", async () => {
    connectorConfigs.set(providers);
    render(LlmProviderSettings);

    await fireEvent.change(screen.getByLabelText("Provider profile"), {
      target: { value: "" },
    });

    expect(updateHostConfig).toHaveBeenCalledWith({
      host: { default_llm_profile: null },
    });
  });

  it("shows each saved profile with its model and confirms cloud egress before changing it", async () => {
    connectorConfigs.set(providers);
    render(LlmProviderSettings);

    expect(screen.getByRole("option", { name: "local-ollama - llama3.1" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "work-openai - gpt-4.1" })).toBeTruthy();

    await fireEvent.change(screen.getByLabelText("Provider profile"), {
      target: { value: "llm-provider/work-openai" },
    });
    expect(screen.getByRole("alert").textContent).toContain(
      "work-openai: chat content and tool results may leave this device.",
    );
    await fireEvent.click(screen.getByRole("button", { name: "Accept and make default" }));

    expect(updateHostConfig).toHaveBeenCalledWith({
      host: {
        default_llm_profile: "work-openai",
        cloud_llm_egress_accepted_profiles: ["llm-provider/work-openai"],
      },
    });
  });

  it("keeps accepted cloud egress visible for the active default", () => {
    const current = config();
    current.host.default_llm_profile = "work-openai";
    current.host.cloud_llm_egress_accepted_profiles = ["llm-provider/work-openai"];
    hostConfig.set(current);
    connectorConfigs.set(providers);

    render(LlmProviderSettings);

    expect(screen.getByText("Cloud profile active")).toBeTruthy();
  });
});
