import { describe, expect, it } from "vitest";

import {
  capabilityAccessBadge,
  capabilityAccessState,
  missingCapabilityWarning,
} from "$lib/apps/capabilityAccess";
import type { CapabilityUseView, DataScope } from "$lib/api";

describe("capabilityAccess", () => {
  it("disables forms when no grant exists", () => {
    expect(capabilityAccessState([], "notes", "create_note")).toEqual({
      available: false,
      grantCondition: null,
    });
    expect(missingCapabilityWarning("notes", "create_note")).toBe(
      "Notes: create note isn't allowed right now. Enable it in Settings → Permissions.",
    );
  });

  it("disables actions when the intent capability is no longer granted", () => {
    const unrelated: CapabilityUseView[] = [
      {
        provider_app_id: "notes",
        provider_display_name: "Notes",
        capability: "list",
        description: "List notes",
        input_schema: {},
        authorizations: [{ data_scope: { kind: "none" }, condition: "silent" }],
      },
    ];

    expect(capabilityAccessState(unrelated, "notes", "create_note")).toEqual({
      available: false,
      grantCondition: null,
    });
  });

  it("requires an authorization that covers the invocation data scope", () => {
    const capability: CapabilityUseView = {
      provider_app_id: "notes",
      provider_display_name: "Notes",
      capability: "export",
      description: "Export notes",
      input_schema: {},
      authorizations: [{ data_scope: { kind: "none" }, condition: "silent" }],
    };
    const requested: DataScope = { kind: "resources", resource_ids: ["app-data:notes:notes"] };

    expect(capabilityAccessState([capability], "notes", "export", requested)).toEqual({
      available: false,
      grantCondition: null,
    });
    capability.authorizations.push({ data_scope: { kind: "all-resources" }, condition: "notify" });
    expect(capabilityAccessState([capability], "notes", "export", requested)).toEqual({
      available: true,
      grantCondition: "notify",
    });
  });

  it("maps grant conditions to visible badges", () => {
    expect(capabilityAccessBadge("requires-approval")).toBe(
      "Approval on delegated/high-impact use",
    );
    expect(capabilityAccessBadge("notify")).toBe("Notifies on delegated use");
    expect(capabilityAccessBadge("silent")).toBe("Allowed");
  });
});
