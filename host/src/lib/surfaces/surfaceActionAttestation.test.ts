import { describe, expect, it } from "vitest";

import type { CapabilityDeclaration, DataScope, GrantView } from "$lib/api";
import {
  effectiveGrantCondition,
  needsHostGestureAttestation,
} from "$lib/surfaces/surfaceActionAttestation";

const appId = "com.example.notes";
const capability = { provider: appId, capability: "save" };
const none: DataScope = { kind: "none" };

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
  it("requires a host gesture for own read/local-write actions regardless of the grant snapshot", () => {
    for (const condition of ["silent", "notify", "requires-approval"] as const) {
      expect(
        needsHostGestureAttestation(
          appId,
          [declaration("local-write")],
          [grant(condition)],
          capability,
          none,
        ),
      ).toBe(true);
    }
  });

  it("does not trust an empty or stale grant snapshot to skip host attestation", () => {
    expect(
      needsHostGestureAttestation(appId, [declaration("read-only")], [], capability, none),
    ).toBe(true);
    expect(
      needsHostGestureAttestation(
        appId,
        [declaration("read-only")],
        [grant("silent", { status: "revoked" })],
        capability,
        none,
      ),
    ).toBe(true);
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

  it("mirrors resource coverage when reporting the effective grant condition", () => {
    const requested: DataScope = { kind: "resources", resource_ids: ["a"] };
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

  it("reports the broker's least-interactive covering condition", () => {
    const grants = [
      grant("requires-approval", { grant_id: "approval" }),
      grant("notify", { grant_id: "notify" }),
      grant("silent", { grant_id: "silent" }),
    ];
    expect(effectiveGrantCondition(grants, appId, capability, none)).toBe("silent");
  });
});
