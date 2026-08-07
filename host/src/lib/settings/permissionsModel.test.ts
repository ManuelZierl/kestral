import { describe, expect, it } from "vitest";
import { dataScopeCovers } from "$lib/apps/appMetadata";
import { conditionLabel, groupPermissions, scopeKey } from "$lib/settings/permissionsModel";
import type { DataScope, GrantRequest, GrantView, InstalledApp } from "$lib/api";

function grant(overrides: Partial<GrantView>): GrantView {
  return {
    grant_id: "grant-1",
    holder: "notes",
    holder_display_name: "Notes",
    scope: { kind: "exact-capability", provider: "notes", capability: "create" },
    data_scope: { kind: "none" },
    condition: "silent",
    issued_at: "2026-07-10T00:00:00Z",
    expires_at: null,
    status: "active",
    origin: "manifest-requested",
    ...overrides,
  };
}

function app(appId: string, displayName: string, grantRequests: GrantRequest[]): InstalledApp {
  return {
    manifest: {
      app_id: appId,
      version: "1",
      display_name: displayName,
      description: "",
      capabilities: [],
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
      event_subscriptions: [],
      grant_requests: grantRequests,
    },
    content_hash: "hash",
    installed_at: "2026-07-10T00:00:00Z",
  } as InstalledApp;
}

const createRequest: GrantRequest = {
  scope: { kind: "exact-capability", provider: "notes", capability: "create" },
  data_scope: { kind: "none" },
  condition: "silent",
  reason: "Create notes",
  duration: { kind: "non-expiring" },
};

describe("dataScopeCovers", () => {
  it("lets a broad grant cover exact resources without reversing that authority", () => {
    const exact: DataScope = { kind: "resources", resource_ids: ["thread-1"] };
    expect(dataScopeCovers(exact, { kind: "all-resources" })).toBe(true);
    expect(dataScopeCovers({ kind: "all-resources" }, exact)).toBe(false);
    expect(dataScopeCovers({ kind: "none" }, { kind: "all-resources" })).toBe(false);
  });
});

describe("groupPermissions", () => {
  it("collapses a revoked-then-regranted scope into one entry with history", () => {
    const revoked = grant({ grant_id: "grant-old", status: "revoked", issued_at: "2026-07-09T00:00:00Z" });
    const active = grant({ grant_id: "grant-new", issued_at: "2026-07-10T00:00:00Z" });

    const groups = groupPermissions([revoked, active], [app("notes", "Notes", [])]);

    expect(groups).toHaveLength(1);
    expect(groups[0].entries).toHaveLength(1);
    expect(groups[0].entries[0].current.grant_id).toBe("grant-new");
    expect(groups[0].entries[0].history.map((fact) => fact.grant_id)).toEqual(["grant-old"]);
  });

  it("prefers the active fact even when a newer fact is revoked", () => {
    const active = grant({ grant_id: "grant-active", issued_at: "2026-07-09T00:00:00Z" });
    const newerRevoked = grant({ grant_id: "grant-revoked", status: "revoked", issued_at: "2026-07-10T00:00:00Z" });

    const groups = groupPermissions([active, newerRevoked], [app("notes", "Notes", [])]);

    expect(groups[0].entries[0].current.grant_id).toBe("grant-active");
  });

  it("keeps scopes distinct and sorts entries by scope", () => {
    const create = grant({ grant_id: "g1" });
    const broad = grant({
      grant_id: "g2",
      scope: { kind: "all-provider-capabilities", provider: "llm-provider" },
    });

    const groups = groupPermissions(
      [create, broad],
      [app("notes", "Notes", []), app("llm-provider", "LLM Provider", [])],
    );

    expect(groups[0].entries.map((entry) => scopeKey(entry.scope, entry.dataScope))).toEqual([
      "llm-provider/* :: Not tied to a registered resource",
      "notes/create :: Not tied to a registered resource",
    ]);
  });

  it("attaches the manifest request to a matching entry and lists unseen requests as never granted", () => {
    const revoked = grant({ status: "revoked" });
    const otherRequest: GrantRequest = {
      ...createRequest,
      scope: { kind: "exact-capability", provider: "notes", capability: "delete" },
    };

    const groups = groupPermissions([revoked], [app("notes", "Notes", [createRequest, otherRequest])]);

    expect(groups[0].entries[0].declared).toEqual(createRequest);
    expect(groups[0].neverGranted).toEqual([otherRequest]);
  });

  it("keeps obsolete inactive manifest grants only in the audit view", () => {
    const obsolete = grant({
      status: "revoked",
      scope: { kind: "exact-capability", provider: "notes", capability: "removed-capability" },
    });

    expect(groupPermissions([obsolete], [app("notes", "Notes", [])])).toEqual([]);
  });

  it("keeps inactive user-added grants manageable", () => {
    const custom = grant({ status: "revoked", origin: "user-added" });

    const groups = groupPermissions([custom], [app("notes", "Notes", [])]);

    expect(groups[0].entries[0].current.grant_id).toBe("grant-1");
  });

  it("does not list exact requests as missing when an active provider-wide grant covers them", () => {
    const broad = grant({
      holder: "chat",
      holder_display_name: "Chat",
      scope: { kind: "all-provider-capabilities", provider: "notes" },
    });

    const groups = groupPermissions(
      [broad],
      [app("chat", "Chat", [createRequest]), app("notes", "Notes", [])],
    );

    expect(groups[0].entries).toHaveLength(1);
    expect(groups[0].neverGranted).toEqual([]);
  });

  it("tracks Chat's own request and an external agent's consumer grant independently", () => {
    const llmRequest: GrantRequest = {
      ...createRequest,
      scope: { kind: "exact-capability", provider: "llm-provider", capability: "llm.generate" },
    };
    const agentRequest: GrantRequest = {
      ...createRequest,
      scope: { kind: "exact-capability", provider: "com.example.agent-engine", capability: "agent.run" },
    };
    const llmGrant = grant({
      holder: "chat",
      holder_display_name: "Chat",
      scope: llmRequest.scope,
    });
    const agentGrant = grant({
      grant_id: "agent-grant",
      holder: "chat",
      holder_display_name: "Chat",
      scope: agentRequest.scope,
    });

    const groups = groupPermissions(
      [llmGrant, agentGrant],
      [
        app("chat", "Chat", [llmRequest]),
        app("llm-provider", "LLM Provider", []),
        app("com.example.agent-engine", "Example Agent", []),
      ],
    );

    expect(groups[0].entries.map((entry) => entry.current.scope)).toEqual([
      agentRequest.scope,
      llmRequest.scope,
    ]);
    expect(groups[0].neverGranted).toEqual([]);
  });

  it("groups by holder app sorted by display name and skips apps with nothing to show", () => {
    const chatGrant = grant({ holder: "chat", holder_display_name: "Chat" });
    const notesGrant = grant({ grant_id: "g2" });

    const groups = groupPermissions(
      [notesGrant, chatGrant],
      [app("chat", "Chat", []), app("notes", "Notes", []), app("zzz-empty", "Empty", [])],
    );

    expect(groups.map((group) => group.appId)).toEqual(["chat", "notes"]);
  });

  it("hides grants whose holder or provider is no longer installed", () => {
    const holderGone = grant({ grant_id: "g1", holder: "removed", holder_display_name: "Removed" });
    const providerGone = grant({
      grant_id: "g2",
      scope: { kind: "exact-capability", provider: "removed", capability: "create" },
      status: "revoked",
    });
    const requestForGoneProvider: GrantRequest = {
      ...createRequest,
      scope: { kind: "exact-capability", provider: "removed", capability: "create" },
    };

    const groups = groupPermissions(
      [holderGone, providerGone],
      [app("notes", "Notes", [requestForGoneProvider])],
    );

    expect(groups).toEqual([]);
  });
});

describe("conditionLabel", () => {
  it("humanizes every condition", () => {
    expect(conditionLabel("silent")).toBe("Runs silently");
    expect(conditionLabel("notify")).toBe("Notifies you");
    expect(conditionLabel("requires-approval")).toBe("Asks for approval");
  });
});
