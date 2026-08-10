import { fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { beforeEach, expect, it, vi } from "vitest";
import { get } from "svelte/store";

import type { AppStatusView, ManagedAppRevisionView } from "$lib/api";
import { currentTab } from "$lib/stores/hostState";
import { appSettingsTarget } from "$lib/stores/navigation";

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    listManagedAppRevisions: vi.fn(async () => []),
    planManagedAppTransition: vi.fn(async () => ({
      transition_id: "transition-1",
      app_id: "com.example.app",
      operation: "revert",
      current_revision_id: "revision-current",
      target_revision_id: "revision-target",
      target_version: "1.0.0",
      diff: {
        version_relation: "lower",
        display_name_changed: false,
        description_changed: false,
        backend_kind_changed: false,
        current_backend_authority_mode: null,
        target_backend_authority_mode: null,
        current_data: null,
        target_data: {
          kind: "none",
          format_version: null,
          migration_protocol_version: null,
          transitions: [],
          contract_version: null,
          total_bytes: null,
          collections: [],
          documents: [],
        },
        publisher_key_continuity: "same",
        capabilities_added: [],
        capabilities_removed: [],
        surfaces_added: [],
        surfaces_removed: [],
        permissions: { unchanged: [], added: [], widened: [], removed: [] },
        consumer_permissions: { unchanged: [], added: [], widened: [], removed: [] },
        extension_warnings: [],
      },
      requires_explicit_approval: true,
      data_rollback_supported: false,
      data_rollback_caveat: null,
      data_transition: null,
      staged_id: null,
      package_digest: null,
      revision_id: "revision-target",
    })),
    applyManagedAppTransition: vi.fn(async () => []),
    disconnectMcpServer: vi.fn(async () => undefined),
    listInstalledApps: vi.fn(async () => []),
    setAppEnabled: vi.fn(async () => []),
    uninstallApp: vi.fn(async () => []),
  };
});

import * as api from "$lib/api";
import AppManagerCard from "$lib/apps/AppManagerCard.svelte";

const listManagedAppRevisions = vi.mocked(api.listManagedAppRevisions);
const planManagedAppTransition = vi.mocked(api.planManagedAppTransition);
const applyManagedAppTransition = vi.mocked(api.applyManagedAppTransition);
const disconnectMcpServer = vi.mocked(api.disconnectMcpServer);
const listInstalledApps = vi.mocked(api.listInstalledApps);

const revisions: ManagedAppRevisionView[] = [
  {
    revision_id: "revision-old",
    version: "0.9.0",
    display_name: "Example App",
    description: "Older app",
    backend_kind: "none",
    publisher: "Example Org",
    signature_verdict: "trusted",
    signature_key_id: "ed25519:key-1",
    min_host_version: "0.1.0",
    installed_at: "2026-07-09T00:00:00Z",
    payload_dir: "apps/com.example.app/revisions/revision-old",
    package_digest: "sha256-old",
  },
  {
    revision_id: "revision-current",
    version: "1.0.0",
    display_name: "Example App",
    description: "Current app",
    backend_kind: "none",
    publisher: "Example Org",
    signature_verdict: "trusted",
    signature_key_id: "ed25519:key-1",
    min_host_version: "0.1.0",
    installed_at: "2026-07-10T00:00:00Z",
    payload_dir: "apps/com.example.app/revisions/revision-current",
    package_digest: "sha256-current",
  },
];

const app: AppStatusView = {
  id: "com.example.app",
  display_name: "Example App",
  version: "1.0.0",
  description: "App description",
  bundled: false,
  enabled: true,
  status: "active",
  status_detail: null,
  backend_kind: "none",
  signature: "trusted",
  publisher: "Example Org",
  missing_permissions: 0,
  surfaces: [],
  min_host_version: "0.1.0",
  installed_at: "2026-07-10T00:00:00Z",
  revisions,
  extension_contributions: [],
  removable: true,
};

beforeEach(() => {
  vi.clearAllMocks();
  listManagedAppRevisions.mockResolvedValue(revisions);
  currentTab.set("apps");
  appSettingsTarget.set(null);
});

it("opens settings for the app", async () => {
  render(AppManagerCard, { props: { app, onChanged: vi.fn(async (): Promise<void> => undefined) } });

  await fireEvent.click(screen.getByRole("button", { name: "Settings for Example App" }));

  expect(get(currentTab)).toBe("settings");
  expect(get(appSettingsTarget)).toMatchObject({ appId: "com.example.app" });
});

it("shows why an incompatible installed contribution is dormant", () => {
  render(AppManagerCard, {
    props: {
      app: {
        ...app,
        extension_contributions: [{
          target_app: "chat",
          extension_point: "message-actions",
          contract_version: 5,
          surface: "annotation",
          compatibility: "contract-mismatch",
          target_contract_version: 6,
        }],
      },
      onChanged: vi.fn(async (): Promise<void> => undefined),
    },
  });

  expect(screen.getByRole("region", { name: "App integrations" })).toBeTruthy();
  expect(screen.getByText("Dormant: target provides contract v6")).toBeTruthy();
});

it("shows retained revisions and prepares a revert with explicit acknowledgement", async () => {
  const onChanged = vi.fn(async (): Promise<void> => undefined);
  render(AppManagerCard, { props: { app, onChanged } });

  expect(await screen.findByText("Retained revisions")).toBeTruthy();
  await fireEvent.click(screen.getAllByRole("button", { name: "Revert app version" })[0]);
  await fireEvent.click(screen.getByRole("checkbox", { name: /require a declared reverse migration/ }));

  await waitFor(() => expect(planManagedAppTransition).toHaveBeenCalledWith(expect.objectContaining({ operation: "revert", revision_id: "revision-old" })));
  await waitFor(() => expect(within(screen.getByRole("group", { name: "Revert review" })).getByRole("button", { name: "Revert app version" })).toBeTruthy());

  await fireEvent.click(within(screen.getByRole("group", { name: "Revert review" })).getByRole("button", { name: "Revert app version" }));
  await waitFor(() => expect(applyManagedAppTransition).toHaveBeenCalled());
  expect(onChanged).toHaveBeenCalled();
});

it("surfaces revision load failures without hiding the app card", async () => {
  listManagedAppRevisions.mockRejectedValueOnce(new Error("offline"));
  render(AppManagerCard, { props: { app, onChanged: vi.fn(async (): Promise<void> => undefined) } });

  expect(listManagedAppRevisions).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByRole("button", { name: "Refresh revisions" }));
  expect(await screen.findByRole("alert")).toBeTruthy();
  expect(screen.getByText(/offline/)).toBeTruthy();
});

it("does not offer managed revisions for bundled apps", async () => {
  render(AppManagerCard, {
    props: {
      app: { ...app, id: "chat", display_name: "Chat", bundled: true, removable: false },
      onChanged: vi.fn(async (): Promise<void> => undefined),
    },
  });

  expect(screen.queryByText("Retained revisions")).toBeNull();
  expect(screen.getByRole("button", { name: "Settings for Chat" })).toBeTruthy();
  expect(listManagedAppRevisions).not.toHaveBeenCalled();
});

it("disconnects an MCP tool-server app while preserving its configuration", async () => {
  const onChanged = vi.fn(async (): Promise<void> => undefined);
  render(AppManagerCard, {
    props: {
      app: {
        ...app,
        id: "mcp-team-calendar",
        display_name: "Team Calendar",
        bundled: true,
        removable: true,
      },
      onChanged,
    },
  });

  expect(screen.getByText("Tool server")).toBeTruthy();
  expect(screen.queryByText("Bundled app — managed by the host.")).toBeNull();
  expect(screen.queryByRole("button", { name: "Uninstall" })).toBeNull();

  await fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));

  await waitFor(() => expect(disconnectMcpServer).toHaveBeenCalledWith("team-calendar"));
  expect(listInstalledApps).toHaveBeenCalledOnce();
  expect(onChanged).toHaveBeenCalledWith([]);
  expect(screen.getByText(/keeps its configuration in Settings/)).toBeTruthy();
});
