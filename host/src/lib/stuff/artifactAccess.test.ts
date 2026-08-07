import { describe, expect, it } from "vitest";

import type { GrantView } from "$lib/api";
import {
  ARTIFACTS_APP_ID,
  CHAT_APP_ID,
  chatCanAccessAllArtifacts,
  chatCanAccessArtifact,
} from "$lib/stuff/artifactAccess";

function grant(
  capability: string,
  dataScope: GrantView["data_scope"],
  overrides: Partial<GrantView> = {},
): GrantView {
  return {
    grant_id: `grant-${capability}`,
    holder: CHAT_APP_ID,
    holder_display_name: "Chat",
    scope: { kind: "exact-capability", provider: ARTIFACTS_APP_ID, capability },
    data_scope: dataScope,
    condition: "requires-approval",
    issued_at: "2026-08-02T00:00:00Z",
    expires_at: null,
    status: "active",
    origin: "user-added",
    ...overrides,
  };
}

describe("artifact Chat access", () => {
  it("requires both query and read access to the artifact", () => {
    const query = grant("artifacts.query", { kind: "resources", resource_ids: ["artifact-1"] });
    const read = grant("artifacts.read", { kind: "resources", resource_ids: ["artifact-1"] });

    expect(chatCanAccessArtifact([query], "artifact-1")).toBe(false);
    expect(chatCanAccessArtifact([query, read], "artifact-1")).toBe(true);
    expect(chatCanAccessArtifact([query, read], "artifact-2")).toBe(false);
  });

  it("recognizes all-resource and provider-wide grants", () => {
    const broad = grant("unused", { kind: "all-resources" }, {
      scope: { kind: "all-provider-capabilities", provider: ARTIFACTS_APP_ID },
    });

    expect(chatCanAccessArtifact([broad], "artifact-1")).toBe(true);
    expect(chatCanAccessAllArtifacts([broad])).toBe(true);
  });

  it("does not mistake an unscoped capability grant for artifact access", () => {
    const query = grant("artifacts.query", { kind: "none" });
    const read = grant("artifacts.read", { kind: "none" });

    expect(chatCanAccessArtifact([query, read], "artifact-1")).toBe(false);
    expect(chatCanAccessAllArtifacts([query, read])).toBe(false);
  });
});
