import { get } from "svelte/store";
import { beforeEach, describe, expect, it } from "vitest";

import { themes } from "$lib/design/colors";
import {
  CUSTOM_THEMES_STORAGE_KEY,
  createCustomThemeProfile,
  customThemePreference,
  customThemeProfiles,
  deleteCustomThemeProfile,
  exportCustomThemeProfile,
  importCustomThemeProfile,
  parseStoredCustomThemes,
  resetThemeState,
  themePreference,
  updateCustomThemeProfile,
  surfaceThemeVariables,
} from "$lib/stores/theme";

beforeEach(() => {
  customThemeProfiles.set([]);
  themePreference.set("system");
});

describe("custom color profiles", () => {
  it("creates a complete profile from an immutable built-in theme and persists it", () => {
    const profile = createCustomThemeProfile("Evening", "dark");

    expect(profile.colors).toEqual(themes.dark);
    expect(profile.colors).not.toBe(themes.dark);
    const stored = parseStoredCustomThemes(localStorage.getItem(CUSTOM_THEMES_STORAGE_KEY) ?? "");
    expect(stored).toEqual([profile]);
  });

  it("updates all profile state without changing its built-in base", () => {
    const profile = createCustomThemeProfile("Focus", "light");
    const colors = { ...profile.colors, accent: "#123456" };

    updateCustomThemeProfile(profile.id, "Focused", colors);

    expect(get(customThemeProfiles)).toEqual([{ ...profile, name: "Focused", colors, appColors: {} }]);
    expect(themes.light.accent).not.toBe("#123456");
  });

  it("returns to System when the selected custom profile is deleted", () => {
    const profile = createCustomThemeProfile("Temporary", "light");
    themePreference.set(customThemePreference(profile.id));

    deleteCustomThemeProfile(profile.id);

    expect(get(themePreference)).toBe("system");
  });

  it("rejects malformed, partial, or outdated-format persisted profiles", () => {
    expect(() => parseStoredCustomThemes('{"version":3,"profiles":[]}')).toThrow(/unsupported storage format/);
    expect(() => parseStoredCustomThemes(JSON.stringify({
      version: 1,
      profiles: [{ id: "partial", name: "Partial", baseTheme: "light", colors: { text: "#000000" } }],
    }))).toThrow(/unsupported storage format/);
    expect(() => parseStoredCustomThemes(JSON.stringify({
      version: 2,
      profiles: [{ id: "partial", name: "Partial", baseTheme: "light", colors: { text: "#000000" }, appColors: {} }],
    }))).toThrow(/incomplete or contains an invalid color/);
  });

  it("exports and imports a strict portable JSON profile", () => {
    const profile = createCustomThemeProfile("Portable", "dark", {
      "com.example.weather": { "storm-track": "#123456" },
    });

    const exported = exportCustomThemeProfile(profile);
    deleteCustomThemeProfile(profile.id);
    const imported = importCustomThemeProfile(exported);

    expect(imported.id).not.toBe(profile.id);
    expect(imported.name).toBe("Portable");
    expect(imported.appColors).toEqual(profile.appColors);
    expect(() => importCustomThemeProfile(exported)).toThrow(/already in use/);
  });

  it("resolves host and declared app variables with profile overrides", () => {
    const variables = surfaceThemeVariables(
      "com.example.weather",
      [{ name: "storm-track", title: "Storm track", description: "Map line", light: "#abcdef", dark: "#123456" }],
      {
        theme: "dark",
        colors: themes.dark,
        appColors: { "com.example.weather": { "storm-track": "#654321", stale: "#000000" } },
      },
    );

    expect(variables["--color-text"]).toBe(themes.dark.text);
    expect(variables["--color-chrome-bg"]).toBeUndefined();
    expect(variables["--app-color-storm-track"]).toBe("#654321");
    expect(variables["--app-color-stale"]).toBeUndefined();
  });

  it("rejects ambiguous names and invalid edited color values", () => {
    const profile = createCustomThemeProfile("Calm", "light");
    expect(() => createCustomThemeProfile(" calm ", "dark")).toThrow(/already in use/);
    expect(() => updateCustomThemeProfile(profile.id, profile.name, { ...profile.colors, text: "not-a-color" })).toThrow(/invalid color/);
  });

  it("clears persisted and live profile state during a system reset", () => {
    const profile = createCustomThemeProfile("Reset me", "dark");
    themePreference.set(customThemePreference(profile.id));

    resetThemeState();

    expect(get(customThemeProfiles)).toEqual([]);
    expect(get(themePreference)).toBe("system");
    expect(localStorage.getItem(CUSTOM_THEMES_STORAGE_KEY)).toBeNull();
    expect(document.documentElement.dataset.themeProfile).toBeUndefined();
  });
});
