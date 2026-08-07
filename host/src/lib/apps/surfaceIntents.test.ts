import { describe, expect, it } from "vitest";

import { capabilityForFormSurface } from "$lib/apps/surfaceIntents";
import type { InstalledApp, SurfaceDeclaration } from "$lib/api";

function app(overrides: Partial<InstalledApp> = {}): InstalledApp {
  return {
    manifest: {
      app_id: "notes",
      version: "1.0.0",
      display_name: "Notes",
      description: "notes",
      capabilities: [
        {
          name: "create_note",
          description: "Create note",
          input_schema: {},
          effect: "unspecified",
        },
      ],
      surfaces: [],
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
    },
    content_hash: "hash",
    installed_at: "2026-07-08T00:00:00Z",
    ...overrides,
  };
}

describe("surfaceIntents", () => {
  it("uses the declared form intent instead of surface naming", () => {
    const surface: SurfaceDeclaration = {
      name: "composer",
      kind: "form",
      title: "Create note",
      description: "Create note form",
      intents: [{ provider: "notes", capability: "create_note" }],
    };
    expect(capabilityForFormSurface(app(), surface)?.name).toBe("create_note");
  });

  it("does not guess when a form has no declared intent", () => {
    const surface: SurfaceDeclaration = {
      name: "create_note-form",
      kind: "form",
      title: "Compose",
      description: "Compose form",
      intents: [],
    };
    expect(capabilityForFormSurface(app(), surface)).toBeUndefined();
  });
});
