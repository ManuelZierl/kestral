import { describe, expect, it } from "vitest";

import type { AppStatusView } from "$lib/api";
import { hasUsableFocusedApp } from "./appReadiness";

function app(overrides: Partial<AppStatusView> = {}): AppStatusView {
  return {
    id: "com.example.focused",
    display_name: "Focused",
    version: "1.0.0",
    description: "Focused workflow",
    bundled: false,
    enabled: true,
    status: "active",
    status_detail: null,
    backend_kind: "none",
    signature: "unsigned",
    publisher: null,
    missing_permissions: 0,
    surfaces: [{ name: "workspace", kind: "dashboard", title: "Workspace", has_custom_ui: true }],
    min_host_version: "0.1.0-alpha.1",
    installed_at: "2026-09-08T00:00:00Z",
    revisions: [],
    extension_contributions: [],
    removable: true,
    ...overrides,
  };
}

describe("focused app readiness", () => {
  it("does not mistake bundled startup apps for a completed app-first journey", () => {
    const bundled = app({
      id: "chat",
      bundled: true,
      signature: "bundled",
      removable: false,
    });
    expect(hasUsableFocusedApp([bundled])).toBe(false);
  });

  it("requires an enabled active independently installed custom screen", () => {
    expect(hasUsableFocusedApp([app()])).toBe(true);
    expect(hasUsableFocusedApp([app({ enabled: false, status: "disabled" })])).toBe(false);
    expect(hasUsableFocusedApp([app({ status: "needs-permissions" })])).toBe(false);
    expect(hasUsableFocusedApp([app({ surfaces: [] })])).toBe(false);
    expect(hasUsableFocusedApp([app({ surfaces: [{ name: "form", kind: "form", title: "Form", has_custom_ui: false }] })])).toBe(false);
  });
});
