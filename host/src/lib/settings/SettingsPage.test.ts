import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";

import type { ConnectorConfigView, HostConfig, InstalledApp } from "$lib/api";
import { apps } from "$lib/stores/apps";
import { connectorConfigs, hostConfig } from "$lib/stores/config";
import { activeAppId, currentTab } from "$lib/stores/hostState";
import { appSettingsTarget, permissionTarget } from "$lib/stores/navigation";
import SettingsPage from "./SettingsPage.svelte";

const { updateHostConfig } = vi.hoisted(() => ({ updateHostConfig: vi.fn() }));

vi.mock("$lib/stores/theme", async () => {
  const { writable } = await import("svelte/store");
  return {
    themePreference: writable("system"),
    customThemeProfiles: writable([]),
    customThemeStorageError: writable(null),
    customThemePreference: (id: string) => `custom:${id}`,
    createCustomThemeProfile: vi.fn(),
    updateCustomThemeProfile: vi.fn(),
    deleteCustomThemeProfile: vi.fn(),
    defaultAppThemeColors: vi.fn(() => ({})),
    exportCustomThemeProfile: vi.fn(() => "{}"),
    importCustomThemeProfile: vi.fn(),
    invalidThemeColorTokens: vi.fn(() => []),
    isThemeColorValue: vi.fn(() => true),
    appCssVariableName: (name: string) => `--app-color-${name}`,
  };
});

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    updateHostConfig,
    listPublisherTrust: vi.fn(async () => []),
    trustPublisherKey: vi.fn(async () => []),
    revokePublisherKey: vi.fn(async () => []),
    listTrustedFileResources: vi.fn(async () => []),
  };
});

function config(): HostConfig {
  return {
    version: 1,
    host: {
      default_llm_provider: "llm-provider",
      default_llm_profile: "local-ollama",
      cloud_llm_egress_accepted_profiles: [],
      app_data_backup_retention: 1,
    },
    apps: {},
    connectors: {},
    mcp_servers: {},
    mcp_exports: {},
    mcp_export_transitions: {},
    mcp_gateway: {
      enabled: false,
      bind_address: "127.0.0.1:8137",
      allowed_origins: [],
      oauth_enabled: false,
    },
  };
}

const connectors: ConnectorConfigView[] = [
  {
    id: "llm-provider/local-ollama",
    kind: "ollama",
    base_url: "http://localhost:11434",
    default_model: "llama3.1",
    default_variant: null,
    default_text_verbosity: null,
    secret_refs: {},
  },
  {
    id: "llm-provider/work-openai",
    kind: "open-ai-compatible",
    base_url: "https://example.test/v1",
    default_model: "gpt-4.1",
    default_variant: null,
    default_text_verbosity: null,
    secret_refs: { api_key: "work-key" },
  },
];

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

beforeEach(() => {
  vi.clearAllMocks();
  const current = config();
  updateHostConfig.mockResolvedValue(current);
  hostConfig.set(current);
  connectorConfigs.set(connectors);
  apps.set([]);
  activeAppId.set(null);
  currentTab.set("settings");
  permissionTarget.set(null);
  appSettingsTarget.set(null);
});

describe("SettingsPage", () => {
  it("saves the global app-data backup retention with a minimum of one", async () => {
    render(SettingsPage);
    await fireEvent.click(screen.getByRole("button", { name: "Kestral profiles" }));

    const retention = screen.getByRole("spinbutton", { name: "Backups per app" });
    await fireEvent.input(retention, { target: { value: "3" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save retention" }));

    await waitFor(() => {
      expect(updateHostConfig).toHaveBeenCalledWith({
        host: { app_data_backup_retention: 3 },
      });
    });
  });

  it("groups settings by task and keeps model choice with model providers", async () => {
    render(SettingsPage);

    expect(screen.getByRole("group", { name: "Personal" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Chat" })).toBeTruthy();
    expect(screen.getByRole("group", { name: "Connections" })).toBeTruthy();
    expect(screen.getByRole("group", { name: "Apps & access" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Appearance" })).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "Host protocol" })).toBeNull();

    await fireEvent.click(screen.getByRole("button", { name: "Chat" }));
    expect(screen.getByText("Set the default prompt behavior, app guidance, and runtime privacy controls for Chat.")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Model providers" }));
    expect(screen.getByRole("heading", { name: "Model providers" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Default for Chat" })).toBeTruthy();
    expect(screen.getByLabelText("Provider profile")).toBeTruthy();
  });

  it("shows the package trust section alongside the existing preserved settings groups", async () => {
    render(SettingsPage);

    expect(screen.getByRole("button", { name: "Package trust" })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Package trust" }));
    expect(screen.getByRole("heading", { name: "Package trust" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "File resources" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Kestral profiles" })).toBeTruthy();
  });

  it("opens the permissions section for a permission deep link", () => {
    permissionTarget.set({ request: 1, kind: "grant", grantId: "grant-target" });

    render(SettingsPage);

    expect(screen.getByRole("heading", { name: "Permissions" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Permissions" }).getAttribute("aria-current")).toBe("page");
  });

  it("opens and focuses the requested app settings", async () => {
    apps.set([installedApp("com.example.notes", "Notes")]);
    appSettingsTarget.set({ request: 2, appId: "com.example.notes", displayName: "Notes" });

    render(SettingsPage);

    expect(screen.getByRole("button", { name: "App settings" }).getAttribute("aria-current")).toBe("page");
    const panel = screen.getByRole("heading", { name: "Notes" }).closest(".app-settings-panel");
    await waitFor(() => expect(document.activeElement).toBe(panel));
    expect(screen.getByText("This app has no configurable settings.")).toBeTruthy();
  });

  it("directs structured app settings to the app dashboard", async () => {
    const profiles = installedApp("com.example.model-profiles", "Model Profiles");
    profiles.manifest.surfaces = [{
      name: "model-profiles",
      kind: "dashboard",
      title: "Model Profiles",
      description: "Create profiles",
      intents: [],
    }];
    profiles.manifest.extension_contributions = [{
      target_app: "chat",
      extension_point: "model-profile-editor",
      contract_version: 1,
      surface: "model-profiles",
    }];
    profiles.manifest.config_declarations = [{
      name: "model-profiles",
      title: "Model profiles",
      description: "Reusable model setups",
      json_schema: {
        type: "object",
        properties: { profiles: { type: "array", items: { type: "object" } } },
        required: ["profiles"],
      },
      default: { profiles: [] },
    }];
    apps.set([profiles]);
    appSettingsTarget.set({ request: 6, appId: profiles.manifest.app_id, displayName: "Model Profiles" });

    render(SettingsPage);

    expect(screen.queryByRole("textbox", { name: "profiles" })).toBeNull();
    expect(screen.getByText("Use the Model Profiles app screen to create and edit these structured settings.")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Open Model Profiles" }));
    expect(get(activeAppId)).toBe(profiles.manifest.app_id);
    expect(get(currentTab)).toBe("apps");
  });

  it("routes built-in settings to their task editors instead of generic empty forms", async () => {
    apps.set([
      installedApp("chat", "Chat"),
      installedApp("llm-provider", "LLM Provider"),
      installedApp("com.ma-zierl.host.file-broker", "File Broker"),
      installedApp("com.example.notes", "Notes"),
    ]);
    appSettingsTarget.set({ request: 3, appId: "chat", displayName: "Chat" });

    render(SettingsPage);

    expect(screen.getByRole("button", { name: "Chat" }).getAttribute("aria-current")).toBe("page");
    expect(screen.getByText("Set the default prompt behavior, app guidance, and runtime privacy controls for Chat.")).toBeTruthy();
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("heading", { name: "Assistant behavior" }).closest(".app-settings-panel")));

    appSettingsTarget.set({ request: 4, appId: "llm-provider", displayName: "LLM Provider" });
    await waitFor(() => expect(screen.getByRole("button", { name: "Model providers" }).getAttribute("aria-current")).toBe("page"));
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("heading", { name: "Provider profiles" }).closest(".app-settings-panel")));

    appSettingsTarget.set({ request: 5, appId: "com.ma-zierl.host.file-broker", displayName: "File Broker" });
    await waitFor(() => expect(screen.getByRole("button", { name: "File resources" }).getAttribute("aria-current")).toBe("page"));
    await waitFor(() => expect(document.activeElement).toBe(screen.getByText(/Register a file or folder here/).closest(".app-settings-panel")));

    await fireEvent.click(screen.getByRole("button", { name: "App settings" }));
    expect(screen.getByRole("heading", { name: "Notes" })).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "Chat" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "LLM Provider" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "File Broker" })).toBeNull();
    expect(screen.getByText("Settings declared by installed apps appear here. Built-in settings stay in their task sections.")).toBeTruthy();
  });

  it("explains when settings for an inactive app are unavailable", async () => {
    appSettingsTarget.set({ request: 6, appId: "com.example.disabled", displayName: "Disabled App" });

    render(SettingsPage);

    expect(screen.getByRole("button", { name: "App settings" }).getAttribute("aria-current")).toBe("page");
    expect(screen.getByText("Settings for Disabled App are unavailable.")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Open Apps" }));
    expect(get(currentTab)).toBe("apps");
  });
});
