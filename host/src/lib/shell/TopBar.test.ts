import { fireEvent, render, screen } from "@testing-library/svelte";
import { beforeEach, describe, expect, it } from "vitest";
import { get } from "svelte/store";

import type { InstalledApp } from "$lib/api";
import { apps } from "$lib/stores/apps";
import { activeAppId, currentTab } from "$lib/stores/hostState";
import { appSettingsTarget, permissionTarget } from "$lib/stores/navigation";
import TopBar from "./TopBar.svelte";

function installedApp(appId: string, displayName: string): InstalledApp {
  return {
    manifest: {
      app_id: appId,
      version: "1.0.0",
      display_name: displayName,
      description: `${displayName} description`,
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
    content_hash: "hash",
    installed_at: "2026-07-25T00:00:00Z",
  };
}

describe("TopBar", () => {
  beforeEach(() => {
    apps.set([]);
    activeAppId.set(null);
    currentTab.set("chat");
    appSettingsTarget.set(null);
    permissionTarget.set(null);
  });

  it("opens permissions for a selected standalone app", async () => {
    apps.set([installedApp("com.example.notes", "Notes")]);
    activeAppId.set("com.example.notes");
    render(TopBar, { tab: "apps" });

    await fireEvent.click(screen.getByRole("button", { name: "Permissions for Notes" }));

    expect(get(currentTab)).toBe("settings");
    expect(get(permissionTarget)).toMatchObject({ kind: "app", appId: "com.example.notes" });
  });

  it("opens settings for a selected standalone app", async () => {
    apps.set([installedApp("com.example.notes", "Notes")]);
    activeAppId.set("com.example.notes");
    render(TopBar, { tab: "apps" });

    await fireEvent.click(screen.getByRole("button", { name: "Settings for Notes" }));

    expect(get(currentTab)).toBe("settings");
    expect(get(appSettingsTarget)).toMatchObject({ appId: "com.example.notes" });
  });

  it("shows the same app actions for the internal Chat app", () => {
    apps.set([installedApp("chat", "Chat")]);

    render(TopBar, { tab: "chat" });

    expect(
      screen.getByText(
        "Talk to your apps. Actions routed through Kestral are checked and recorded.",
      ),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Settings for Chat" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Permissions for Chat" })).toBeTruthy();
  });

  it("does not show app settings on host screens or for a stale app selection", () => {
    activeAppId.set("missing-app");
    const view = render(TopBar, { tab: "apps" });

    expect(screen.queryByRole("button", { name: /Settings for/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /Permissions for/ })).toBeNull();

    view.rerender({ tab: "system" });
    expect(screen.queryByRole("button", { name: /Settings for/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /Permissions for/ })).toBeNull();
  });
});
