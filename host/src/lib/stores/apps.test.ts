import { get } from "svelte/store";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { InstalledApp } from "$lib/api";

vi.mock("$lib/api", () => ({
  listApps: vi.fn(),
}));

import { listApps } from "$lib/api";
import { apps, appsLoaded, refreshApps } from "$lib/stores/apps";

const mockedListApps = vi.mocked(listApps);

function installedApp(id: string): InstalledApp {
  return {
    manifest: {
      app_id: id,
      version: "1.0.0",
      display_name: id,
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
      grant_requests: [],
      event_subscriptions: [],
    },
    content_hash: `hash-${id}`,
    installed_at: "2026-07-25T00:00:00Z",
  };
}

describe("apps store", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    apps.set([]);
    appsLoaded.set(false);
  });

  it("keeps an earlier successful refresh when a newer request fails", async () => {
    let finishFirst!: (value: InstalledApp[]) => void;
    mockedListApps
      .mockReturnValueOnce(new Promise((resolve) => { finishFirst = resolve; }))
      .mockRejectedValueOnce(new Error("kernel busy: a trusted-chrome decision is pending"));

    const first = refreshApps();
    const second = refreshApps();
    await expect(second).rejects.toThrow("kernel busy");
    finishFirst([installedApp("notes")]);
    await first;

    expect(get(apps).map((app) => app.manifest.app_id)).toEqual(["notes"]);
    expect(get(appsLoaded)).toBe(true);
  });

  it("does not let a late older success replace a newer successful response", async () => {
    let finishFirst!: (value: InstalledApp[]) => void;
    mockedListApps
      .mockReturnValueOnce(new Promise((resolve) => { finishFirst = resolve; }))
      .mockResolvedValueOnce([installedApp("newer")]);

    const first = refreshApps();
    await refreshApps();
    finishFirst([installedApp("older")]);
    await first;

    expect(get(apps).map((app) => app.manifest.app_id)).toEqual(["newer"]);
  });
});
