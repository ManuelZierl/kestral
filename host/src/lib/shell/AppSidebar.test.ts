import { fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AppIcon, InstalledApp } from "$lib/api";
import { apps } from "$lib/stores/apps";
import { artifacts } from "$lib/stores/artifacts";
import { chatThreads } from "$lib/stores/chatThreads";
import { activeAppId, shellError } from "$lib/stores/hostState";
import { resetSidebarLayout, SIDEBAR_LAYOUT_STORAGE_KEY } from "$lib/stores/sidebarLayout";
import AppSidebar from "./AppSidebar.svelte";

function installedApp(displayName: string, icon?: AppIcon): InstalledApp {
  const id = `com.example.${displayName.toLowerCase()}`;
  return {
    manifest: {
      app_id: id,
      version: "1.0.0",
      display_name: displayName,
      description: `${displayName} fixture`,
      capabilities: [],
      surfaces: [{ name: "main", kind: "panel", title: displayName, description: "", intents: [] }],
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
    content_hash: `hash-${displayName}`,
    installed_at: "2026-07-27T00:00:00Z",
    icon,
  };
}

describe("AppSidebar", () => {
  beforeEach(() => {
    apps.set([]);
    artifacts.set([]);
    chatThreads.set([]);
    activeAppId.set(null);
    shellError.set(null);
    resetSidebarLayout();
  });

  it("keeps only the product name and host status around the navigation", () => {
    render(AppSidebar, { current: "chat", onSelect: vi.fn() });

    expect(screen.getByRole("heading", { name: "Kestral" })).toBeTruthy();
    expect(screen.queryByText("AI App Host")).toBeNull();
    expect(screen.queryByText("Trusted boundary")).toBeNull();
    expect(screen.getByText("0 chats")).toBeTruthy();
    expect(screen.getByText("0 apps")).toBeTruthy();
    expect(screen.getByText("0 artifacts")).toBeTruthy();
    expect(screen.getByText("Host connected")).toBeTruthy();
  });

  it("lets the user collapse and expand the navigation without hiding its destinations", async () => {
    const onSelect = vi.fn();
    render(AppSidebar, { current: "chat", onSelect });

    const sidebar = screen.getByRole("complementary", { name: "App navigation" });
    const collapse = screen.getByRole("button", { name: "Collapse navigation" });
    expect(sidebar.classList.contains("collapsed")).toBe(false);

    await fireEvent.click(collapse);

    const expand = screen.getByRole("button", { name: "Expand navigation" });
    expect(sidebar.classList.contains("collapsed")).toBe(true);
    expect(screen.getByRole("button", { name: "Chat" }).getAttribute("title")).toBe("Chat");

    await fireEvent.click(expand);
    expect(screen.getByRole("button", { name: "Collapse navigation" })).toBeTruthy();
    expect(sidebar.classList.contains("collapsed")).toBe(false);
    expect(localStorage.getItem(SIDEBAR_LAYOUT_STORAGE_KEY)).toContain('"collapsed":false');
  });

  it("hides, shows, and reorders destinations from an always-available editor", async () => {
    render(AppSidebar, { current: "chat", onSelect: vi.fn() });
    const navigation = screen.getByRole("navigation", { name: "Primary" });

    await fireEvent.click(screen.getByRole("button", { name: "Customize navigation" }));
    const dialog = screen.getByRole("dialog", { name: "Customize navigation" });
    expect(document.activeElement).toBe(within(dialog).getByRole("button", { name: "Close navigation customization" }));

    await fireEvent.click(within(dialog).getByRole("checkbox", { name: "Show Artifacts" }));
    expect(within(navigation).queryByRole("button", { name: "Artifacts" })).toBeNull();

    await fireEvent.click(within(dialog).getByRole("button", { name: "Move Chat down" }));
    await fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));

    const destinationNames = within(navigation).getAllByRole("button").map((button) => button.textContent?.trim());
    expect(destinationNames.slice(0, 2)).toEqual(["Apps", "Chat"]);
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("button", { name: "Customize navigation" })));
    expect(localStorage.getItem(SIDEBAR_LAYOUT_STORAGE_KEY)).toContain('"host:stuff"');

    await fireEvent.click(screen.getByRole("button", { name: "Customize navigation" }));
    await fireEvent.click(screen.getByRole("checkbox", { name: "Show Artifacts" }));
    expect(within(navigation).getByRole("button", { name: "Artifacts" })).toBeTruthy();
  });

  it("restores the default visibility and order", async () => {
    render(AppSidebar, { current: "chat", onSelect: vi.fn() });
    const navigation = screen.getByRole("navigation", { name: "Primary" });

    await fireEvent.click(screen.getByRole("button", { name: "Customize navigation" }));
    await fireEvent.click(screen.getByRole("checkbox", { name: "Show Apps" }));
    await fireEvent.click(screen.getByRole("button", { name: "Move Chat down" }));
    await fireEvent.click(screen.getByRole("button", { name: "Reset to default" }));

    expect(within(navigation).getAllByRole("button").map((button) => button.textContent?.trim())).toEqual([
      "Chat",
      "Apps",
      "Artifacts",
      "Settings",
      "System",
    ]);
    expect(localStorage.getItem(SIDEBAR_LAYOUT_STORAGE_KEY)).toBeNull();
  });

  it("renders multicolor assets, current-color assets, catalog icons, and letter fallbacks", () => {
    apps.set([
      installedApp("Custom", {
        kind: "asset",
        media_type: "image/svg+xml",
        data_base64: "PHN2Zy8+",
      }),
      installedApp("Notes", {
        kind: "asset",
        media_type: "image/svg+xml",
        data_base64: btoa('<svg xmlns="http://www.w3.org/2000/svg"><path stroke="currentColor"/></svg>'),
      }),
      installedApp("Library", { kind: "kestral", name: "book-open" }),
      installedApp("Default"),
    ]);

    render(AppSidebar, { current: "chat", onSelect: vi.fn() });

    const custom = screen.getByRole("button", { name: "Custom" });
    expect(custom.querySelector("img")?.getAttribute("src")).toBe(
      "data:image/svg+xml;base64,PHN2Zy8+",
    );
    const notes = screen.getByRole("button", { name: "Notes" });
    expect(notes.querySelector("img")).toBeNull();
    expect(notes.querySelector(".monochrome-icon")?.getAttribute("style")).toContain(
      "data:image/svg+xml;base64,",
    );
    expect(
      screen.getByRole("button", { name: "Library" }).querySelector('[data-icon-name="book-open"]'),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Default" }).textContent).toContain("D");
  });

  it("does not create a standalone destination for an extension-only app", () => {
    const extension = installedApp("Chat Export");
    extension.manifest.extension_contributions = [{
      target_app: "chat",
      extension_point: "thread-actions",
      contract_version: 1,
      surface: "main",
    }];
    apps.set([extension]);

    render(AppSidebar, { current: "chat", onSelect: vi.fn() });

    expect(screen.queryByRole("button", { name: "Chat Export" })).toBeNull();
  });

  it("keeps a contributed dashboard available as a standalone destination", () => {
    const profiles = installedApp("Model Profiles");
    profiles.manifest.surfaces = [{
      name: "model-profiles",
      kind: "dashboard",
      title: "Model Profiles",
      description: "",
      intents: [],
    }];
    profiles.manifest.extension_contributions = [{
      target_app: "chat",
      extension_point: "model-profile-editor",
      contract_version: 1,
      surface: "model-profiles",
    }];
    apps.set([profiles]);

    render(AppSidebar, { current: "chat", onSelect: vi.fn() });

    expect(screen.getByRole("button", { name: "Model Profiles" })).toBeTruthy();
  });
});
