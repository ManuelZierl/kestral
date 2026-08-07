import { fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    listGrants: vi.fn(async () => []),
    issueEditorGrant: vi.fn(async () => undefined),
    replaceGrant: vi.fn(async () => undefined),
    requestManifestGrant: vi.fn(async () => undefined),
    revokeGrant: vi.fn(async () => undefined),
  };
});

import GrantPolicyEditor from "./GrantPolicyEditor.svelte";
import * as api from "$lib/api";
import type { GrantView, InstalledApp } from "$lib/api";
import { apps, appsLoaded } from "$lib/stores/apps";
import { grants, grantsLoaded } from "$lib/stores/grants";
import { permissionTarget } from "$lib/stores/navigation";

const notes: InstalledApp = {
  manifest: {
    app_id: "notes",
    version: "1",
    display_name: "Notes",
    description: "",
    capabilities: [{ name: "create", description: "", input_schema: {}, effect: "local-write" }],
    surfaces: [], agents: [], skills: [], assistant_profiles: [], automations: [], connectors: [], config_declarations: [], artifact_types: [], extension_points: [], extension_contributions: [], event_subscriptions: [],
    grant_requests: [{ scope: { kind: "exact-capability" as const, provider: "notes", capability: "create" }, data_scope: { kind: "none" as const }, condition: "silent" as const, reason: "Create notes", duration: { kind: "non-expiring" as const } }],
  },
  content_hash: "hash",
  installed_at: "2026-07-10T00:00:00Z",
};

const mcpCalendar: InstalledApp = {
  manifest: {
    app_id: "mcp-calendar",
    version: "1",
    display_name: "Calendar MCP",
    description: "",
    capabilities: [{ name: "create_event", description: "", input_schema: {}, effect: "external-write" }],
    surfaces: [], agents: [], skills: [], assistant_profiles: [], automations: [], connectors: [], config_declarations: [], artifact_types: [], extension_points: [], extension_contributions: [], event_subscriptions: [],
    grant_requests: [],
  },
  content_hash: "mcp-hash",
  installed_at: "2026-07-10T00:00:00Z",
};

const artifactBrowser: InstalledApp = {
  manifest: {
    app_id: "com.ma-zierl.kestral-artifacts",
    version: "1",
    display_name: "Artifacts",
    description: "",
    capabilities: [
      { name: "artifacts.query", description: "", input_schema: {}, effect: "read-only" },
      { name: "artifacts.read", description: "", input_schema: {}, effect: "read-only" },
    ],
    surfaces: [], agents: [], skills: [], assistant_profiles: [], automations: [], connectors: [], config_declarations: [], artifact_types: [], extension_points: [], extension_contributions: [], event_subscriptions: [],
    grant_requests: [],
  },
  content_hash: "artifact-hash",
  installed_at: "2026-07-10T00:00:00Z",
};

function fact(overrides: Partial<GrantView>): GrantView {
  return {
    grant_id: "grant-1", holder: "notes", holder_display_name: "Notes",
    scope: { kind: "exact-capability", provider: "notes", capability: "create" },
    data_scope: { kind: "none" },
    condition: "silent", issued_at: "2026-07-10T00:00:00Z", expires_at: null,
    status: "active", origin: "manifest-requested",
    ...overrides,
  };
}

describe("GrantPolicyEditor", () => {
  beforeEach(() => {
    apps.set([notes]);
    appsLoaded.set(true);
    grants.set([]);
    grantsLoaded.set(true);
    permissionTarget.set(null);
    vi.clearAllMocks();
  });

  it("shows one row per scope even when revoked and re-granted facts coexist", () => {
    grants.set([
      fact({ grant_id: "grant-old", status: "revoked", issued_at: "2026-07-09T00:00:00Z" }),
      fact({ grant_id: "grant-new", issued_at: "2026-07-10T00:00:00Z" }),
    ]);
    render(GrantPolicyEditor);

    const group = within(screen.getByRole("group", { name: "Permissions held by Notes" }));
    expect(group.getAllByText("Notes: create")).toHaveLength(1);
    expect(group.getByText("Runs silently")).toBeTruthy();
    expect(group.getByText("1 earlier audit record")).toBeTruthy();
    expect(group.getByText("1 active")).toBeTruthy();
  });

  it("scrolls to, focuses, and briefly highlights a targeted permission", async () => {
    grants.set([fact({ grant_id: "grant-target", condition: "notify" })]);
    permissionTarget.set({ request: 1, kind: "grant", grantId: "grant-target" });
    render(GrantPolicyEditor);

    const row = screen.getByRole("button", { name: "Edit Notes: create for Notes" }).closest("li");
    expect(row).toBeTruthy();
    await waitFor(() => {
      expect(row?.classList.contains("highlighted")).toBe(true);
      expect(document.activeElement).toBe(row);
    });
  });

  it("scrolls to, focuses, and briefly highlights a targeted app group", async () => {
    grants.set([fact({ grant_id: "grant-target" })]);
    permissionTarget.set({ request: 2, kind: "app", appId: "notes" });
    render(GrantPolicyEditor);

    const group = screen.getByRole("group", { name: "Permissions held by Notes" });
    await waitFor(() => {
      expect(group.classList.contains("highlighted")).toBe(true);
      expect(document.activeElement).toBe(group);
    });
  });

  it("edits an active permission in place through the replacement command", async () => {
    grants.set([fact({})]);
    render(GrantPolicyEditor);

    await fireEvent.click(screen.getByRole("button", { name: "Edit Notes: create for Notes" }));
    const editor = screen.getByRole("form", { name: "Edit Notes: create" });
    await fireEvent.change(within(editor).getByLabelText("Approval"), { target: { value: "notify" } });
    await fireEvent.click(within(editor).getByRole("button", { name: "Apply change" }));

    expect(api.replaceGrant).toHaveBeenCalledWith("grant-1", expect.objectContaining({
      holder: "notes",
      scope: { kind: "exact-capability", provider: "notes", capability: "create" },
      condition: "notify",
      duration: { kind: "non-expiring" },
    }));
  });

  it("revokes the active grant from its row after an inline confirm", async () => {
    grants.set([fact({})]);
    render(GrantPolicyEditor);

    await fireEvent.click(screen.getByRole("button", { name: "Revoke Notes: create for Notes" }));
    expect(api.revokeGrant).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByRole("button", { name: "Revoke" }));

    expect(api.revokeGrant).toHaveBeenCalledWith("grant-1");
  });

  it("includes file resource scope in permission action names", () => {
    grants.set([fact({ data_scope: { kind: "resources", resource_ids: ["workspace"] } })]);
    render(GrantPolicyEditor);

    expect(screen.getByRole("button", {
      name: "Edit Notes: create for Resources: workspace for Notes",
    })).toBeTruthy();
  });

  it("keeps the grant when the inline revoke confirm is dismissed", async () => {
    grants.set([fact({})]);
    render(GrantPolicyEditor);

    await fireEvent.click(screen.getByRole("button", { name: "Revoke Notes: create for Notes" }));
    await fireEvent.click(screen.getByRole("button", { name: "Keep" }));

    expect(api.revokeGrant).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Revoke Notes: create for Notes" })).toBeTruthy();
  });

  it("re-grants a revoked manifest-declared permission in one click", async () => {
    grants.set([fact({ status: "revoked" })]);
    render(GrantPolicyEditor);

    await fireEvent.click(screen.getByRole("button", { name: "Grant Notes: create for Notes again" }));

    expect(api.requestManifestGrant).toHaveBeenCalledWith("notes", notes.manifest.grant_requests[0]);
  });

  it("offers never-granted manifest requests with a one-click grant", async () => {
    render(GrantPolicyEditor);

    expect(screen.getByText("Not granted")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Grant Notes: create for Notes" }));

    expect(api.requestManifestGrant).toHaveBeenCalledWith("notes", notes.manifest.grant_requests[0]);
  });

  it("issues a custom exact-capability grant with optional reason and expiry", async () => {
    render(GrantPolicyEditor);

    await fireEvent.click(screen.getByRole("button", { name: "Issue grant" }));

    expect(api.issueEditorGrant).toHaveBeenCalledWith(expect.objectContaining({
      holder: "notes",
      scope: { kind: "exact-capability", provider: "notes", capability: "create" },
      allow_all_provider_scope: false,
      duration: { kind: "non-expiring" },
      reason: "",
    }));
  });

  it("replaces the exact capability control with provider-wide scope when advanced is selected", async () => {
    render(GrantPolicyEditor);

    await fireEvent.click(screen.getByRole("checkbox", { name: /Advanced: all provider capabilities/ }));

    expect(screen.queryByLabelText("Capability")).toBeNull();
    expect(screen.getByText("All current and future capabilities")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Issue grant" }));
    expect(api.issueEditorGrant).toHaveBeenCalledWith(expect.objectContaining({
      scope: { kind: "all-provider-capabilities", provider: "notes" },
      allow_all_provider_scope: true,
    }));
  });

  it("requires an explicit warning acknowledgement before relaxing MCP approval", async () => {
    apps.set([notes, mcpCalendar]);
    grants.set([fact({
      scope: {
        kind: "exact-capability",
        provider: "mcp-calendar",
        capability: "create_event",
      },
      condition: "requires-approval",
    })]);
    render(GrantPolicyEditor);

    await fireEvent.click(screen.getByRole("button", {
      name: "Edit Calendar MCP: create_event for Notes",
    }));
    const editor = screen.getByRole("form", { name: "Edit Calendar MCP: create_event" });
    await fireEvent.change(within(editor).getByLabelText("Approval"), {
      target: { value: "silent" },
    });

    const apply = within(editor).getByRole("button", { name: "Apply change" });
    expect((apply as HTMLButtonElement).disabled).toBe(true);
    const warning = within(editor).getByRole("checkbox", { name: /Future Chat and LLM-driven calls/ });
    await fireEvent.click(warning);
    expect((apply as HTMLButtonElement).disabled).toBe(false);
    await fireEvent.click(apply);

    expect(api.replaceGrant).toHaveBeenCalledWith("grant-1", expect.objectContaining({
      condition: "silent",
      acknowledge_less_interactive_mcp: true,
    }));
  });

  it("requires an explicit artifact selection for a custom Artifacts permission", async () => {
    apps.set([notes, artifactBrowser]);
    render(GrantPolicyEditor);

    await fireEvent.change(screen.getByLabelText("App that provides the action"), {
      target: { value: artifactBrowser.manifest.app_id },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Issue grant" }));
    expect(screen.getByRole("alert").textContent).toContain("Choose access to all artifacts");
    expect(api.issueEditorGrant).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByRole("checkbox", { name: /All current and future artifacts/ }));
    await fireEvent.click(screen.getByRole("button", { name: "Issue grant" }));
    expect(api.issueEditorGrant).toHaveBeenCalledWith(expect.objectContaining({
      scope: {
        kind: "exact-capability",
        provider: artifactBrowser.manifest.app_id,
        capability: "artifacts.query",
      },
      data_scope: { kind: "all-resources" },
    }));
  });

  it("repairs an existing Artifacts permission with no artifact access", async () => {
    apps.set([notes, artifactBrowser]);
    grants.set([fact({
      scope: {
        kind: "all-provider-capabilities",
        provider: artifactBrowser.manifest.app_id,
      },
      data_scope: { kind: "none" },
      condition: "requires-approval",
    })]);
    render(GrantPolicyEditor);

    expect(screen.getByText("No artifact access")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", {
      name: "Edit All Artifacts capabilities for Notes",
    }));
    await fireEvent.click(screen.getByRole("checkbox", { name: /All current and future artifacts/ }));
    await fireEvent.click(screen.getByRole("button", { name: "Apply change" }));

    expect(api.replaceGrant).toHaveBeenCalledWith("grant-1", expect.objectContaining({
      data_scope: { kind: "all-resources" },
    }));
  });
});
