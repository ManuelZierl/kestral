import { beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "$lib/api";
import type { InstalledApp } from "$lib/api";
import {
  loadSurfaceHostContext,
  MODEL_PROFILE_CONTRACT_VERSION,
  MODEL_PROFILE_EXTENSION_POINT,
} from "./modelProfileEditorContext";

vi.mock("$lib/api", async (importOriginal) => ({
  ...await importOriginal<typeof import("$lib/api")>(),
  listConnectorConfigs: vi.fn(),
  discoverConnectorModelsDraft: vi.fn(),
  availableCapabilitiesFor: vi.fn(),
  getChatPromptPreview: vi.fn(),
}));

const listConnectorConfigs = vi.mocked(api.listConnectorConfigs);
const discoverConnectorModelsDraft = vi.mocked(api.discoverConnectorModelsDraft);
const availableCapabilitiesFor = vi.mocked(api.availableCapabilitiesFor);
const getChatPromptPreview = vi.mocked(api.getChatPromptPreview);

function app(contributes: boolean): InstalledApp {
  return {
    content_hash: "hash",
    installed_at: "2026-07-31T00:00:00Z",
    manifest: {
      app_id: contributes ? "com.example.model-profiles" : "com.example.weather",
      version: "1.0.0",
      display_name: contributes ? "Model Setup" : "Weather",
      description: "test",
      capabilities: [],
      surfaces: [{ name: "editor", kind: "dashboard", title: "Editor", description: "", intents: [] }],
      agents: [],
      skills: [],
      assistant_profiles: [],
      automations: [],
      connectors: [],
      config_declarations: contributes
        ? [{ name: "model-profiles", title: "Model profiles", description: "", json_schema: {}, default: {} }]
        : [],
      artifact_types: [],
      extension_points: [],
      extension_contributions: contributes
        ? [{
            target_app: "chat",
            extension_point: MODEL_PROFILE_EXTENSION_POINT,
            contract_version: MODEL_PROFILE_CONTRACT_VERSION,
            surface: "editor",
          }]
        : [],
      grant_requests: [],
      event_subscriptions: [],
    },
  };
}

beforeEach(() => {
  listConnectorConfigs.mockResolvedValue([{
    id: "llm-provider/local",
    kind: "ollama",
    base_url: "http://localhost:11434",
    default_model: "model-default",
    default_variant: "high",
    default_text_verbosity: null,
    secret_refs: {},
  }]);
  discoverConnectorModelsDraft.mockResolvedValue({
    models: [{ id: "model-a", display_name: "Model A", variants: ["low"], text_verbosity: [] }],
    message: "Discovered 1 model",
  });
  availableCapabilitiesFor.mockResolvedValue([{
    provider_app_id: "notes",
    provider_display_name: "Notes",
    capability: "read",
    description: "Read notes",
    input_schema: {},
    authorizations: [],
  }, {
    provider_app_id: "llm-provider",
    provider_display_name: "LLM Provider",
    capability: "llm.generate",
    description: "Generate text",
    input_schema: {},
    authorizations: [],
  }]);
  getChatPromptPreview.mockResolvedValue({
    system_prompt: "protocol\n\ninstructions",
    digest: "digest",
    layers: [{ id: "protocol", kind: "protocol", title: "Kestral protocol", source: "Kestral host", content: "protocol", editable: false, included: true }],
    available_skills: [],
    runtime: { host_version: "0.1.0", mode: "plain-llm", model_id: "model", connector_kind: "ollama", app_inventory: null, connection_details: null },
  });
});

describe("loadSurfaceHostContext", () => {
  it("returns no owner context to ordinary apps", async () => {
    await expect(loadSurfaceHostContext(app(false), "editor")).resolves.toEqual({});
    expect(listConnectorConfigs).not.toHaveBeenCalled();
  });

  it("returns no owner context to a surface that was not contributed", async () => {
    await expect(loadSurfaceHostContext(app(true), "other-surface")).resolves.toEqual({});
    expect(listConnectorConfigs).not.toHaveBeenCalled();
  });

  it("sanitizes provider choices, prompt layers, and Chat tools", async () => {
    const context = await loadSurfaceHostContext(app(true), "editor");
    expect(context).toMatchObject({
      kind: "model-profile-editor",
      connectors: [{
        id: "llm-provider/local",
        default_model: "model-default",
        default_variant: "high",
        discovery_error: null,
      }],
      tools: [{ reference: "notes/read", provider: "Notes", name: "read" }],
      prompt_layers: [{ id: "protocol", title: "Kestral protocol" }],
    });
    expect(JSON.stringify(context)).not.toContain("base_url");
    expect(JSON.stringify(context)).not.toContain("secret_refs");
    expect(context.connectors).toEqual(expect.arrayContaining([
      expect.objectContaining({
        models: expect.arrayContaining([
          expect.objectContaining({ id: "model-default", variants: ["high"] }),
          expect.objectContaining({ id: "model-a", variants: ["low"] }),
        ]),
      }),
    ]));
  });

  it("keeps the configured default selectable when discovery fails", async () => {
    discoverConnectorModelsDraft.mockRejectedValueOnce(new Error("provider offline"));
    const context = await loadSurfaceHostContext(app(true), "editor");
    expect(context.connectors).toEqual([
      expect.objectContaining({
        discovery_error: "Model discovery is unavailable for this provider profile.",
        models: [{ id: "model-default", display_name: null, variants: ["high"] }],
      }),
    ]);
    expect(JSON.stringify(context)).not.toContain("provider offline");
  });
});
