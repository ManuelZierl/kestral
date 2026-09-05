import { describe, expect, it } from "vitest";

import type { CapabilityDeclaration, GrantView } from "$lib/api";
import {
  effectiveGrantCondition,
  needsHostGestureAttestation,
} from "$lib/surfaces/surfaceActionAttestation";

const appId = "com.example.notes";
const capability = { provider: appId, capability: "save" };
const none = { kind: "none" } as const;

function declaration(effect: CapabilityDeclaration["effect"]): CapabilityDeclaration {
  return {
    name: "save",
    description: "Save a note",
    input_schema: { type: "object" },
    effect,
  };
}

function grant(
  condition: GrantView["condition"],
  overrides: Partial<GrantView> = {},
): GrantView {
  return {
    grant_id: `grant-${condition}`,
    holder: appId,
    holder_display_name: "Notes",
    scope: { kind: "exact-capability", provider: appId, capability: "save" },
    data_scope: none,
    condition,
    origin: "manifest-requested",
    status: "active",
    issued_at: "2026-09-05T10:00:00Z",
    expires_at: null,
    ...overrides,
  };
}

describe("surface action attestation", () => {
  it("requires a host gesture for the own read/local-write approval shortcut", () => {
    expect(
      needsHostGestureAttestation(
        appId,
        [declaration("local-write")],
        [grant("requires-approval")],
        capability,
        none,
      ),
    ).toBe(true);
  });

  it("does not add a second prompt when a less-interactive covering grant wins", () => {
    const grants = [
      grant("requires-approval", { grant_id: "grant-a" }),
      grant("silent", { grant_id: "grant-b", issued_at: "2026-09-05T11:00:00Z" }),
    ];
    expect(effectiveGrantCondition(grants, appId, capability, none)).toBe("silent");
    expect(
      needsHostGestureAttestation(appId, [declaration("read-only")], grants, capability, none),
    ).toBe(false);
  });

  it("leaves cross-app and high-risk effects to normal kernel trusted chrome", () => {
    expect(
      needsHostGestureAttestation(
        appId,
        [declaration("external-write")],
        [grant("requires-approval")],
        capability,
        none,
      ),
    ).toBe(false);
    expect(
      needsHostGestureAttestation(
        appId,
        [declaration("local-write")],
        [grant("requires-approval")],
        { provider: "com.example.other", capability: "write" },
        none,
      ),
    ).toBe(false);
  });

  it("matches resource-scope coverage and ignores inactive grants", () => {
    const requested = { kind: "resources", resource_ids: ["a"] } as const;
    const grants: GrantView[] = [
      grant("silent", {
        grant_id: "revoked",
        status: "revoked",
        data_scope: { kind: "all-resources" },
      }),
      grant("requires-approval", {
        grant_id: "approval",
        data_scope: { kind: "resources", resource_ids: ["a", "b"] },
      }),
    ];
    expect(effectiveGrantCondition(grants, appId, capability, requested)).toBe("requires-approval");
  });
});
