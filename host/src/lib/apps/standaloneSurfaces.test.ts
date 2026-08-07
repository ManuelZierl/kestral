import { describe, expect, it } from "vitest";

import type { AppManifest, SurfaceDeclaration } from "$lib/api";
import { standaloneSurfaces } from "$lib/apps/standaloneSurfaces";

function surface(name: string, kind: SurfaceDeclaration["kind"]): SurfaceDeclaration {
  return { name, kind, title: name, description: "", intents: [] };
}

function manifest(surfaces: SurfaceDeclaration[]): AppManifest {
  return {
    app_id: "com.example.app",
    version: "1.0.0",
    display_name: "Example",
    description: "Example app",
    capabilities: [],
    surfaces,
    agents: [],
    skills: [],
    assistant_profiles: [],
    automations: [],
    connectors: [],
    config_declarations: [],
    artifact_types: [],
    extension_points: [],
    extension_contributions: [],
    grant_requests: [],
    event_subscriptions: [],
  };
}

describe("standaloneSurfaces", () => {
  it("keeps a contributed dashboard available as a standalone app", () => {
    const app = manifest([surface("profiles", "dashboard")]);
    app.extension_contributions = [{
      target_app: "chat",
      extension_point: "model-profile-editor",
      contract_version: 1,
      surface: "profiles",
    }];

    expect(standaloneSurfaces(app).map((item) => item.name)).toEqual(["profiles"]);
  });

  it("hides contributed inline surfaces but keeps ordinary surfaces", () => {
    const app = manifest([
      surface("inline-action", "panel"),
      surface("workspace", "panel"),
    ]);
    app.extension_contributions = [{
      target_app: "chat",
      extension_point: "thread-actions",
      contract_version: 1,
      surface: "inline-action",
    }];

    expect(standaloneSurfaces(app).map((item) => item.name)).toEqual(["workspace"]);
  });
});
