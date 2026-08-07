import { fireEvent, render, screen } from "@testing-library/svelte";
import { get } from "svelte/store";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { InstalledApp } from "$lib/api";
import { themes } from "$lib/design/colors";
import { apps } from "$lib/stores/apps";
import {
  createCustomThemeProfile,
  customThemeProfiles,
  customThemeStorageError,
  themePreference,
} from "$lib/stores/theme";
import ThemeSettings from "./ThemeSettings.svelte";

beforeEach(() => {
  customThemeProfiles.set([]);
  customThemeStorageError.set(null);
  themePreference.set("system");
  apps.set([]);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("ThemeSettings", () => {
  it("keeps System as the default and presents immutable built-in choices", () => {
    render(ThemeSettings);

    const selector = screen.getByLabelText("Color theme") as HTMLSelectElement;
    expect(selector.value).toBe("system");
    expect(screen.getByRole("option", { name: "System (default)" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "Light" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "Dark" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Edit Light" })).toBeNull();
  });

  it("creates, selects, edits, and deletes a complete custom profile", async () => {
    render(ThemeSettings);

    await fireEvent.click(screen.getByRole("button", { name: "Create profile" }));
    await fireEvent.input(screen.getByLabelText("Profile name"), { target: { value: "Ocean" } });
    await fireEvent.change(screen.getByLabelText("Start from"), { target: { value: "dark" } });
    await fireEvent.click(screen.getByRole("button", { name: "Create and use profile" }));

    expect(await screen.findByText("Ocean created and selected.")).toBeTruthy();
    expect(get(customThemeProfiles)).toHaveLength(1);
    expect(get(themePreference)).toMatch(/^custom:/);

    await fireEvent.click(screen.getByRole("button", { name: "Edit Ocean" }));
    const accentValue = screen.getByLabelText("Accent color value") as HTMLInputElement;
    await fireEvent.input(accentValue, { target: { value: "#123456" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    expect(await screen.findByText("Ocean saved.")).toBeTruthy();
    expect(get(customThemeProfiles)[0].colors.accent).toBe("#123456");

    await fireEvent.click(screen.getByRole("button", { name: "Delete Ocean" }));
    expect(screen.getByText("Delete Ocean?")).toBeTruthy();
    const deleteButtons = screen.getAllByRole("button", { name: "Delete" });
    await fireEvent.click(deleteButtons[0]);
    expect(await screen.findByText("Ocean deleted. System theme selected.")).toBeTruthy();
    expect(get(customThemeProfiles)).toEqual([]);
    expect(get(themePreference)).toBe("system");
  });

  it("locates invalid color values before saving", async () => {
    render(ThemeSettings);
    await fireEvent.click(screen.getByRole("button", { name: "Create profile" }));
    await fireEvent.input(screen.getByLabelText("Profile name"), { target: { value: "Broken" } });
    await fireEvent.input(screen.getByLabelText("Background gradient start color value"), { target: { value: "blue-ish" } });
    await fireEvent.click(screen.getByRole("button", { name: "Create and use profile" }));

    expect(screen.getByRole("alert").textContent).toContain("Correct the highlighted color values");
    expect(screen.getByText("Enter a HEX, rgb(), or rgba() color.")).toBeTruthy();
    expect(get(customThemeProfiles)).toEqual([]);
  });

  it("imports and selects a valid portable JSON profile", async () => {
    render(ThemeSettings);
    const source = JSON.stringify({
      format: "kestral-color-profile",
      version: 1,
      name: "Imported",
      base_theme: "dark",
      colors: themes.dark,
      app_colors: {},
    });

    await fireEvent.change(screen.getByLabelText("Import color profile JSON"), {
      target: { files: [new File([source], "imported.json", { type: "application/json" })] },
    });

    expect(await screen.findByText("Imported imported and selected.")).toBeTruthy();
    expect(get(customThemeProfiles)).toHaveLength(1);
    expect(get(themePreference)).toMatch(/^custom:/);
  });

  it("reports an invalid imported file when the profile editor is closed", async () => {
    render(ThemeSettings);

    await fireEvent.change(screen.getByLabelText("Import color profile JSON"), {
      target: { files: [new File([], "empty.json", { type: "application/json" })] },
    });

    expect((await screen.findByRole("alert")).textContent).toContain("Could not import this color profile");
    expect(get(customThemeProfiles)).toEqual([]);
  });

  it("keeps exported profile data available while the webview starts the download", async () => {
    createCustomThemeProfile("Portable", "dark");
    render(ThemeSettings);
    const createObjectURL = vi.fn((_blob: Blob) => "blob:kestral-theme");
    const revokeObjectURL = vi.fn();
    const NativeURL = URL;
    class DownloadURL extends NativeURL {
      static createObjectURL = createObjectURL;
      static revokeObjectURL = revokeObjectURL;
    }
    vi.stubGlobal("URL", DownloadURL);
    vi.useFakeTimers();
    let clickedWhileConnected = false;
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) {
      clickedWhileConnected = this.isConnected;
    });

    await fireEvent.click(screen.getByRole("button", { name: "Export Portable" }));

    const downloadedBlob = createObjectURL.mock.calls[0]?.[0];
    expect(downloadedBlob?.size).toBeGreaterThan(0);
    expect(clickedWhileConnected).toBe(true);
    expect(revokeObjectURL).not.toHaveBeenCalled();
    await vi.runAllTimersAsync();
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:kestral-theme");
  });

  it("shows declared app colors in Appearance and saves namespaced overrides", async () => {
    apps.set([{
      manifest: {
        app_id: "com.example.weather",
        version: "1.0.0",
        display_name: "Weather",
        description: "Forecasts",
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
      installed_at: "2026-07-29T00:00:00Z",
      theme_colors: [{
        name: "storm-track",
        title: "Storm track",
        description: "Forecast path on the map.",
        light: "#315ea8",
        dark: "#8db1ff",
      }],
    } satisfies InstalledApp]);
    render(ThemeSettings);

    await fireEvent.click(screen.getByRole("button", { name: "Create profile" }));
    expect(screen.getByText("Weather")).toBeTruthy();
    await fireEvent.click(screen.getByText("Weather"));
    await fireEvent.input(screen.getByLabelText("Accent color value"), {
      target: { value: "#654321" },
    });
    await fireEvent.input(screen.getByLabelText("Weather Storm track color value"), {
      target: { value: "#123456" },
    });
    await fireEvent.change(screen.getByLabelText("Start from"), { target: { value: "dark" } });
    expect((screen.getByLabelText("Accent color value") as HTMLInputElement).value).toBe("#654321");
    expect((screen.getByLabelText("Weather Storm track color value") as HTMLInputElement).value).toBe("#123456");
    await fireEvent.input(screen.getByLabelText("Profile name"), { target: { value: "Storm" } });
    await fireEvent.click(screen.getByRole("button", { name: "Create and use profile" }));

    expect(get(customThemeProfiles)[0].appColors).toEqual({
      "com.example.weather": { "storm-track": "#123456" },
    });
  });
});
