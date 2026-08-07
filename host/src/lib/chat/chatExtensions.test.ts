import { describe, expect, it } from "vitest";

import type { InstalledApp } from "$lib/api";
import { resolveChatExtensions } from "./chatExtensions";

function app(overrides: Partial<InstalledApp["manifest"]>): InstalledApp {
  return {
    content_hash: "hash",
    installed_at: "2026-07-11T00:00:00Z",
    manifest: {
      app_id: "app",
      version: "1.0.0",
      display_name: "App",
      description: "test",
      capabilities: [], surfaces: [], agents: [], skills: [], assistant_profiles: [], automations: [], connectors: [],
      config_declarations: [], artifact_types: [], extension_points: [], extension_contributions: [],
      grant_requests: [], event_subscriptions: [],
      ...overrides,
    },
  };
}

describe("resolveChatExtensions", () => {
  it("returns only exact, version-compatible contributions with declared surfaces", () => {
    const chat = app({
      app_id: "chat",
      extension_points: [{ name: "message-actions", contract_version: 6, context_schema: {} }],
    });
    const annotator = app({
      app_id: "annotator",
      display_name: "Text Annotator",
      surfaces: [{ name: "annotate", kind: "card", title: "Annotate", description: "", intents: [] }],
      extension_contributions: [{
        target_app: "chat", extension_point: "message-actions", contract_version: 6, surface: "annotate",
      }],
    });
    const incompatible = app({
      app_id: "old-extension",
      extension_contributions: [{
        target_app: "chat", extension_point: "message-actions", contract_version: 7, surface: "missing",
      }],
    });

    expect(resolveChatExtensions([chat, incompatible, annotator], "message-actions")).toEqual([
      { app: annotator, surface: annotator.manifest.surfaces[0] },
    ]);
  });
});
