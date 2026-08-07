// Theme selection, custom-profile persistence, and CSS application.
// Appearance is device-local shell state, not app or host configuration.

import { writable } from "svelte/store";
import "$lib/developmentCleanStart";
import {
  themeColorTokens,
  themeCssVariables,
  themes,
  type ThemeColors,
  type ThemeColorToken,
  type ThemeId,
} from "$lib/design/colors";
import type { AppThemeColor } from "$lib/api";

export type CustomThemePreference = `custom:${string}`;
export type ThemePreference = ThemeId | "system" | CustomThemePreference;

export interface CustomThemeProfile {
  id: string;
  name: string;
  baseTheme: ThemeId;
  colors: ThemeColors;
  appColors: AppThemeColors;
}

export type AppThemeColors = Record<string, Record<string, string>>;

export interface ResolvedAppearance {
  theme: ThemeId;
  colors: ThemeColors;
  appColors: AppThemeColors;
}

interface StoredCustomThemes {
  version: 2;
  profiles: CustomThemeProfile[];
}

export const THEME_PREFERENCE_STORAGE_KEY = "host-theme-preference";
export const CUSTOM_THEMES_STORAGE_KEY = "host-custom-theme-profiles";

const PROFILE_NAME_MAX_LENGTH = 40;
const PROFILE_ID_PATTERN = /^[a-zA-Z0-9-]{1,80}$/;
const APP_ID_PATTERN = /^[a-z0-9][a-z0-9.-]{0,213}$/;
const APP_COLOR_NAME_PATTERN = /^[a-z][a-z0-9-]{0,63}$/;
const HEX_COLOR_PATTERN = /^#(?:[0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;
const THEME_EXPORT_FORMAT = "kestral-color-profile";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const actual = Object.keys(value).sort();
  return actual.length === keys.length && actual.every((key, index) => key === [...keys].sort()[index]);
}

export function isThemeColorValue(value: string): boolean {
  const trimmed = value.trim();
  if (trimmed.length > 80) return false;
  if (HEX_COLOR_PATTERN.test(trimmed)) return true;
  const rgb = trimmed.match(/^(rgb|rgba)\((.*)\)$/i);
  if (!rgb) return false;
  const parts = rgb[2].split(",").map((part) => part.trim());
  if (parts.length !== (rgb[1].toLowerCase() === "rgba" ? 4 : 3)) return false;
  const channelsValid = parts.slice(0, 3).every((part) => {
    if (!/^\d+(?:\.\d+)?%?$/.test(part)) return false;
    const amount = Number.parseFloat(part);
    return amount >= 0 && amount <= (part.endsWith("%") ? 100 : 255);
  });
  if (!channelsValid || parts.length === 3) return channelsValid;
  const alpha = parts[3];
  if (!/^(?:\d+(?:\.\d+)?|\.\d+)%?$/.test(alpha)) return false;
  const amount = Number.parseFloat(alpha);
  return amount >= 0 && amount <= (alpha.endsWith("%") ? 100 : 1);
}

export function invalidThemeColorTokens(colors: ThemeColors): ThemeColorToken[] {
  return themeColorTokens.filter((token) => !isThemeColorValue(colors[token]));
}

function parseThemeColors(value: unknown): ThemeColors | null {
  if (!isRecord(value)) return null;
  if (!hasExactKeys(value, themeColorTokens)) return null;
  for (const token of themeColorTokens) {
    if (typeof value[token] !== "string" || !isThemeColorValue(value[token])) return null;
  }
  return value as unknown as ThemeColors;
}

function parseAppThemeColors(value: unknown): AppThemeColors | null {
  if (!isRecord(value)) return null;
  const parsed: AppThemeColors = {};
  for (const [appId, colors] of Object.entries(value)) {
    if (!APP_ID_PATTERN.test(appId) || !isRecord(colors)) return null;
    const appColors: Record<string, string> = {};
    for (const [name, color] of Object.entries(colors)) {
      if (!APP_COLOR_NAME_PATTERN.test(name) || typeof color !== "string" || !isThemeColorValue(color)) return null;
      appColors[name] = color;
    }
    parsed[appId] = appColors;
  }
  return parsed;
}

function cloneAppThemeColors(value: AppThemeColors): AppThemeColors {
  return Object.fromEntries(Object.entries(value).map(([appId, colors]) => [appId, { ...colors }]));
}

function parseProfile(value: unknown): CustomThemeProfile | null {
  if (!isRecord(value) || !hasExactKeys(value, ["id", "name", "baseTheme", "colors", "appColors"])) return null;
  const { id, name, baseTheme } = value;
  const colors = parseThemeColors(value.colors);
  const appColors = parseAppThemeColors(value.appColors);
  if (
    typeof id !== "string"
    || !PROFILE_ID_PATTERN.test(id)
    || typeof name !== "string"
    || name !== name.trim()
    || name.length === 0
    || name.length > PROFILE_NAME_MAX_LENGTH
    || (baseTheme !== "light" && baseTheme !== "dark")
    || !colors
    || !appColors
  ) return null;
  return { id, name, baseTheme, colors, appColors };
}

export function parseStoredCustomThemes(value: string): CustomThemeProfile[] {
  const parsed: unknown = JSON.parse(value);
  if (
    !isRecord(parsed)
    || !hasExactKeys(parsed, ["version", "profiles"])
    || parsed.version !== 2
    || !Array.isArray(parsed.profiles)
  ) {
    throw new Error("Custom color profiles use an unsupported storage format.");
  }
  const profiles = parsed.profiles.map((profile) => parseProfile(profile));
  if (profiles.some((profile) => profile === null)) {
    throw new Error("A saved custom color profile is incomplete or contains an invalid color.");
  }
  const validProfiles = profiles as CustomThemeProfile[];
  const ids = new Set(validProfiles.map((profile) => profile.id));
  const names = new Set(validProfiles.map((profile) => profile.name.toLocaleLowerCase()));
  if (ids.size !== validProfiles.length || names.size !== validProfiles.length) {
    throw new Error("Saved custom color profiles contain duplicate names or identifiers.");
  }
  return validProfiles;
}

function isThemePreference(value: string | null): value is ThemePreference {
  return value === "light" || value === "dark" || value === "system" || value?.startsWith("custom:") === true;
}

function customProfileId(preference: ThemePreference): string | null {
  return preference.startsWith("custom:") ? preference.slice("custom:".length) : null;
}

export function customThemePreference(id: string): CustomThemePreference {
  return `custom:${id}`;
}

function loadProfiles(): { profiles: CustomThemeProfile[]; error: string | null } {
  const stored = localStorage.getItem(CUSTOM_THEMES_STORAGE_KEY);
  if (!stored) return { profiles: [], error: null };
  try {
    return { profiles: parseStoredCustomThemes(stored), error: null };
  } catch (failure) {
    return { profiles: [], error: String((failure as Error).message) };
  }
}

function loadPreference(profiles: readonly CustomThemeProfile[]): ThemePreference {
  const stored = localStorage.getItem(THEME_PREFERENCE_STORAGE_KEY);
  if (!isThemePreference(stored)) return "system";
  const profileId = customProfileId(stored);
  return profileId && !profiles.some((profile) => profile.id === profileId) ? "system" : stored;
}

function generateProfileId(): string {
  return crypto.randomUUID();
}

function validateProfileName(name: string, profiles: readonly CustomThemeProfile[], currentId?: string): string {
  const trimmed = name.trim();
  if (!trimmed) throw new Error("Enter a profile name.");
  if (trimmed.length > PROFILE_NAME_MAX_LENGTH) throw new Error(`Profile names can contain at most ${PROFILE_NAME_MAX_LENGTH} characters.`);
  if (profiles.some((profile) => profile.id !== currentId && profile.name.toLocaleLowerCase() === trimmed.toLocaleLowerCase())) {
    throw new Error("Choose a name that is not already in use.");
  }
  return trimmed;
}

const loaded = loadProfiles();
let currentProfiles = loaded.profiles;
let currentPreference = loadPreference(currentProfiles);
let profilesReady = false;

const systemDark = typeof window.matchMedia === "function"
  ? window.matchMedia("(prefers-color-scheme: dark)")
  : null;

function resolveSelection(
  preference: ThemePreference,
  profiles: readonly CustomThemeProfile[],
): { theme: ThemeId; colors: ThemeColors; appColors: AppThemeColors; profileId: string | null } {
  if (preference === "system") {
    const theme = systemDark?.matches ? "dark" : "light";
    return { theme, colors: themes[theme], appColors: {}, profileId: null };
  }
  if (preference === "light" || preference === "dark") {
    return { theme: preference, colors: themes[preference], appColors: {}, profileId: null };
  }
  const profile = profiles.find((candidate) => candidate.id === customProfileId(preference));
  return profile
    ? { theme: profile.baseTheme, colors: profile.colors, appColors: profile.appColors, profileId: profile.id }
    : { theme: "light", colors: themes.light, appColors: {}, profileId: null };
}

export function resolveTheme(preference: ThemePreference, profiles: readonly CustomThemeProfile[] = currentProfiles): ThemeId {
  return resolveSelection(preference, profiles).theme;
}

function applySelection(preference: ThemePreference, profiles: readonly CustomThemeProfile[]) {
  const selection = resolveSelection(preference, profiles);
  const root = document.documentElement;
  for (const [name, value] of Object.entries(themeCssVariables(selection.colors))) {
    root.style.setProperty(name, value);
  }
  root.dataset.theme = selection.theme;
  if (selection.profileId) root.dataset.themeProfile = selection.profileId;
  else delete root.dataset.themeProfile;
  root.style.colorScheme = selection.theme;
  resolvedTheme.set(selection.theme);
  resolvedAppearance.set({
    theme: selection.theme,
    colors: selection.colors,
    appColors: selection.appColors,
  });
}

export const customThemeStorageError = writable<string | null>(loaded.error);
export const customThemeProfiles = writable<CustomThemeProfile[]>(loaded.profiles);
export const themePreference = writable<ThemePreference>(currentPreference);
export const resolvedTheme = writable<ThemeId>(resolveTheme(currentPreference));
export const resolvedAppearance = writable<ResolvedAppearance>(resolveSelection(currentPreference, currentProfiles));

customThemeProfiles.subscribe((profiles) => {
  currentProfiles = profiles;
  if (profilesReady) {
    const stored: StoredCustomThemes = { version: 2, profiles };
    localStorage.setItem(CUSTOM_THEMES_STORAGE_KEY, JSON.stringify(stored));
    customThemeStorageError.set(null);
  }
  profilesReady = true;
  const selectedId = customProfileId(currentPreference);
  if (selectedId && !profiles.some((profile) => profile.id === selectedId)) {
    themePreference.set("system");
    return;
  }
  applySelection(currentPreference, profiles);
});

themePreference.subscribe((preference) => {
  currentPreference = preference;
  localStorage.setItem(THEME_PREFERENCE_STORAGE_KEY, preference);
  applySelection(preference, currentProfiles);
});

export function createCustomThemeProfile(
  name: string,
  baseTheme: ThemeId,
  appColors: AppThemeColors = {},
): CustomThemeProfile {
  const profile: CustomThemeProfile = {
    id: generateProfileId(),
    name: validateProfileName(name, currentProfiles),
    baseTheme,
    colors: { ...themes[baseTheme] },
    appColors: cloneAppThemeColors(appColors),
  };
  customThemeProfiles.update((profiles) => [...profiles, profile]);
  return profile;
}

export function updateCustomThemeProfile(
  id: string,
  name: string,
  colors: ThemeColors,
  appColors: AppThemeColors = {},
): void {
  const existing = currentProfiles.find((profile) => profile.id === id);
  if (!existing) throw new Error("This custom color profile no longer exists.");
  const invalidTokens = invalidThemeColorTokens(colors);
  if (invalidTokens.length > 0) throw new Error("Correct the invalid color values before saving.");
  if (!parseAppThemeColors(appColors)) throw new Error("Correct the invalid app color values before saving.");
  const trimmedName = validateProfileName(name, currentProfiles, id);
  customThemeProfiles.update((profiles) => profiles.map((profile) => (
    profile.id === id
      ? { ...profile, name: trimmedName, colors: { ...colors }, appColors: cloneAppThemeColors(appColors) }
      : profile
  )));
}

export function appCssVariableName(name: string): string {
  return `--app-color-${name}`;
}

export function defaultAppThemeColors(
  declarations: readonly AppThemeColor[],
  theme: ThemeId,
): Record<string, string> {
  return Object.fromEntries(declarations.map((declaration) => [declaration.name, declaration[theme]]));
}

export function surfaceThemeVariables(
  appId: string,
  declarations: readonly AppThemeColor[],
  appearance: ResolvedAppearance,
): Record<string, string> {
  const appOverrides = appearance.appColors[appId] ?? {};
  return {
    ...Object.fromEntries(Object.entries(themeCssVariables(appearance.colors))
      .filter(([name]) => !name.startsWith("--color-chrome-"))),
    ...Object.fromEntries(declarations.map((declaration) => [
      appCssVariableName(declaration.name),
      appOverrides[declaration.name] ?? declaration[appearance.theme],
    ])),
  };
}

export function exportCustomThemeProfile(profile: CustomThemeProfile): string {
  return JSON.stringify({
    format: THEME_EXPORT_FORMAT,
    version: 1,
    name: profile.name,
    base_theme: profile.baseTheme,
    colors: profile.colors,
    app_colors: profile.appColors,
  }, null, 2);
}

export function importCustomThemeProfile(source: string): CustomThemeProfile {
  const value: unknown = JSON.parse(source);
  if (!isRecord(value) || !hasExactKeys(value, ["format", "version", "name", "base_theme", "colors", "app_colors"])) {
    throw new Error("The selected file is not a complete Kestral color profile.");
  }
  if (value.format !== THEME_EXPORT_FORMAT || value.version !== 1) {
    throw new Error("The selected color profile uses an unsupported format.");
  }
  const colors = parseThemeColors(value.colors);
  const appColors = parseAppThemeColors(value.app_colors);
  if (
    typeof value.name !== "string"
    || (value.base_theme !== "light" && value.base_theme !== "dark")
    || !colors
    || !appColors
  ) {
    throw new Error("The selected color profile is incomplete or contains an invalid color.");
  }
  const profile: CustomThemeProfile = {
    id: generateProfileId(),
    name: validateProfileName(value.name, currentProfiles),
    baseTheme: value.base_theme,
    colors,
    appColors,
  };
  customThemeProfiles.update((profiles) => [...profiles, profile]);
  return profile;
}

export function deleteCustomThemeProfile(id: string): void {
  if (!currentProfiles.some((profile) => profile.id === id)) return;
  customThemeProfiles.update((profiles) => profiles.filter((profile) => profile.id !== id));
}

export function resetThemeState(): void {
  customThemeProfiles.set([]);
  themePreference.set("system");
  localStorage.removeItem(CUSTOM_THEMES_STORAGE_KEY);
  localStorage.removeItem(THEME_PREFERENCE_STORAGE_KEY);
}

systemDark?.addEventListener("change", () => {
  if (currentPreference === "system") applySelection(currentPreference, currentProfiles);
});
