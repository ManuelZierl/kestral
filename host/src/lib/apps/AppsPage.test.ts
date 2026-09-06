import { render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AppStatusView, InstalledApp, PackageInspection } from "$lib/api";
import { apps } from "$lib/stores/apps";
import { activeAppId } from "$lib/stores/hostState";
import AppsPage from "./AppsPage.svelte";

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    listInstalledApps: vi.fn(),
    listApps: vi.fn(async () => []),
    getSurfaceUi: vi.fn(async () => null),
    inspectPackage: vi.fn(),
    inspectGitPackage: vi.fn(),
    planManagedAppTransition: vi.fn(),
    applyManagedAppTransition: vi.fn(),
    setAppEnabled: vi.fn(),
    uninstallApp: vi.fn(),
  };
});

vi.mock("$lib/stores/hostState", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/stores/hostState")>();
  return { ...actual, refreshHost: vi.fn(async () => {}) };
});

vi.mock("$lib/stores/grants", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/stores/grants")>();
  return { ...actual, refreshGrants: vi.fn(async () => {}) };
});

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

const api = await import("$lib/api");
const dialog = await import("@tauri-apps/plugin-dialog");
const listInstalledApps = vi.mocked(api.listInstalledApps);
const listApps = vi.mocked(api.listApps);
const inspectPackage = vi.mocked(api.inspectPackage);
const inspectGitPackage = vi.mocked(api.inspectGitPackage);
const planManagedAppTransition = vi.mocked(api.planManagedAppTransition);
const applyManagedAppTransition = vi.mocked(api.applyManagedAppTransition);
const setAppEnabled = vi.mocked(api.setAppEnabled);
const uninstallApp = vi.mocked(api.uninstallApp);
const openDirectory = vi.mocked(dialog.open);
const firstAppHeading = "Make Kestral useful for one real job";

function status(overrides: Partial<AppStatusView> = {}): AppStatusView {
  return {
    id: "com.example.thing",
    display_name: "Thing",
    version: "1.0.0",
    description: "A thing.",
    bundled: false,
    enabled: true,
    status: "active",
    status_detail: null,
    backend_kind: "none",
    signature: "unsigned",
    publisher: null,
    missing_permissions: 0,
    surfaces: [],
    min_host_version: "0.1.0",
    installed_at: "2026-07-10T00:00:00Z",
    revisions: [],
    extension_contributions: [],
    removable: true,
    ...overrides,
  };
}

function inspection(overrides: Partial<PackageInspection> = {}): PackageInspection {
  return {
    staged_id: "00000000-0000-4000-8000-000000000000",
    package_digest: `sha256-${"0".repeat(64)}`,
    id: "com.example.new",
    version: "1.0.0",
    display_name: "New App",
    description: "Brand new.",
    publisher: null,
    license: null,
    signature: { kind: "unsigned" },
    signature_public_key: null,
    backend_kind: "none",
    backend_detail: "No backend process",
    backend_authority_mode: null,
    data: {
      kind: "none",
      format_version: null,
      migration_protocol_version: null,
      transitions: [],
      contract_version: null,
      total_bytes: null,
      batch_operations: null,
      collections: [],
      documents: [],
      proposals: [],
    },
    min_host_version: "0.0.1",
    host_version: "0.1.0",
    host_compatible: true,
    capabilities: [],
    grant_requests: [
      { scope_label: "notes/create", data_scope_label: "All data", condition: "requires approval", reason: "make notes", duration_label: "does not expire" },
    ],
    extension_contributions: [],
    surfaces: [],
    config: [],
    secrets: [],
    artifact_types: [],
    event_subscriptions: [],
    integrity_ok: true,
    integrity_error: null,
    warnings: [],
    installable: true,
    blocking_error: null,
    ...overrides,
  };
}

function installedApp(id = "com.example.thing"): InstalledApp {
  return {
    manifest: {
      app_id: id,
      version: "1.0.0",
      display_name: "Thing manifest",
      description: "Manifest from the shared store.",
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
    installed_at: "2026-07-10T00:00:00Z",
  };
}

async function openInstaller(user: ReturnType<typeof userEvent.setup>): Promise<void> {
  await screen.findByText(firstAppHeading);
  await user.click(screen.getByRole("button", { name: "Install an app" }));
  await screen.findByLabelText("Package directory path");
}

beforeEach(() => {
  apps.set([]);
  activeAppId.set(null);
  listApps.mockResolvedValue([]);
  planManagedAppTransition.mockResolvedValue({
    transition_id: "transition-1",
    app_id: "com.example.new",
    operation: "install",
    current_revision_id: null,
    target_revision_id: "revision-target",
    target_version: "1.0.0",
    diff: {
      version_relation: "higher",
      display_name_changed: false,
      description_changed: false,
      backend_kind_changed: false,
      current_backend_authority_mode: null,
      target_backend_authority_mode: null,
      current_data: null,
      target_data: inspection().data,
      publisher_key_continuity: "new",
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
    staged_id: "staged-1",
    package_digest: `sha256-${"0".repeat(64)}`,
    revision_id: null,
  });
  applyManagedAppTransition.mockResolvedValue([]);
});

afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe("Apps manager", () => {
  it("shows an explicit loading state until both app views are ready", async () => {
    let finishStatuses!: (value: AppStatusView[]) => void;
    listInstalledApps.mockReturnValue(new Promise((resolve) => { finishStatuses = resolve; }));
    render(AppsPage);

    expect(screen.getByRole("status").textContent).toContain("Loading apps");
    expect(screen.queryByText(firstAppHeading)).toBeNull();
    expect(listApps).not.toHaveBeenCalled();

    finishStatuses([]);
    expect(await screen.findByText(firstAppHeading)).toBeTruthy();
    expect(listApps).toHaveBeenCalledOnce();
  });

  it("lists managed and bundled apps, marking bundled as read-only", async () => {
    listInstalledApps.mockResolvedValue([
      status({ id: "com.example.thing", display_name: "Thing" }),
      status({ id: "notes", display_name: "Notes", bundled: true, signature: "bundled", removable: false, backend_kind: "bundled" }),
    ]);
    render(AppsPage);

    expect(await screen.findByText("Thing")).toBeTruthy();
    const bundled = screen.getByTestId("app-notes");
    expect(bundled.textContent).toContain("Bundled");
    // Bundled apps expose no Enable/Uninstall controls.
    expect(bundled.textContent).not.toContain("Enable");
    expect(bundled.textContent).not.toContain("Uninstall");

    // A managed app exposes Disable + Uninstall.
    const managed = screen.getByTestId("app-com.example.thing");
    expect(managed.textContent).toContain("Disable");
    expect(managed.textContent).toContain("Uninstall");
  });

  it("loads once on mount instead of reacting to its own loaded state", async () => {
    listInstalledApps.mockResolvedValue([status({ display_name: "Loaded once" })]);
    render(AppsPage);

    expect(await screen.findByText("Loaded once")).toBeTruthy();
    await waitFor(() => expect(listInstalledApps).toHaveBeenCalledOnce());
    expect(screen.queryByText(/Could not load apps:/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
  });

  it("waits and retries automatically while trusted chrome owns the kernel", async () => {
    vi.useFakeTimers();
    listInstalledApps
      .mockRejectedValueOnce(new Error("kernel busy: a trusted-chrome decision is pending"))
      .mockResolvedValueOnce([status({ display_name: "Available after approval" })]);
    render(AppsPage);

    await vi.waitFor(() => {
      expect(screen.getByRole("status").textContent).toContain("Waiting for the host");
    });
    expect(screen.queryByText(/Could not load apps:/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();

    await vi.advanceTimersByTimeAsync(1000);
    await vi.waitFor(() => expect(screen.getByText("Available after approval")).toBeTruthy());
    expect(listInstalledApps).toHaveBeenCalledTimes(2);
  });

  it("uses the shared apps store to decide whether an app can open", async () => {
    const withSurface = installedApp();
    withSurface.manifest.surfaces = [
      { name: "main", kind: "panel", title: "Main", description: "", intents: [] },
    ];
    listInstalledApps.mockResolvedValue([
      status({
        surfaces: [{ name: "main", kind: "panel", title: "Main", has_custom_ui: false }],
      }),
    ]);
    listApps.mockResolvedValue([withSurface]);
    render(AppsPage);

    expect(await screen.findByRole("button", { name: "Open" })).toBeTruthy();
    expect(listApps.mock.calls.length).toBeGreaterThan(0);
  });

  it("gives an open app the whole view instead of wrapping it in host chrome", async () => {
    const withSurface = installedApp();
    withSurface.manifest.surfaces = [
      { name: "main", kind: "panel", title: "Main panel", description: "", intents: [] },
    ];
    listInstalledApps.mockResolvedValue([status()]);
    listApps.mockResolvedValue([withSurface]);
    activeAppId.set("com.example.thing");
    render(AppsPage);

    // The surface renders directly…
    expect(await screen.findByText(/surfaces are reserved here/)).toBeTruthy();
    // …without the host repeating the app's identity around it: the top bar
    // already names the app, and the sidebar's Apps tab leads to management.
    expect(screen.queryByText("Thing manifest")).toBeNull();
    expect(screen.queryByText("Manifest from the shared store.")).toBeNull();
    expect(screen.queryByText("Back to app management")).toBeNull();
    expect(screen.queryByText("All permissions granted")).toBeNull();
  });

  it("renders only standalone surfaces in an app workspace", async () => {
    const withSurfaces = installedApp();
    withSurfaces.manifest.surfaces = [
      { name: "inline", kind: "panel", title: "Inline action", description: "", intents: [] },
      { name: "dashboard", kind: "dashboard", title: "Dashboard", description: "", intents: [] },
    ];
    withSurfaces.manifest.extension_contributions = [{
      target_app: "chat",
      extension_point: "thread-actions",
      contract_version: 1,
      surface: "inline",
    }];
    listInstalledApps.mockResolvedValue([status()]);
    listApps.mockResolvedValue([withSurfaces]);
    activeAppId.set("com.example.thing");

    render(AppsPage);

    expect(await screen.findByText("dashboard surfaces are reserved here; richer renderers can slot in later.")).toBeTruthy();
    expect(screen.queryByText("panel surfaces are reserved here; richer renderers can slot in later.")).toBeNull();
  });

  it("keeps a selected app open while its management refresh waits for the kernel", async () => {
    vi.useFakeTimers();
    const withSurface = installedApp();
    withSurface.manifest.surfaces = [
      { name: "main", kind: "panel", title: "Main panel", description: "", intents: [] },
    ];
    apps.set([withSurface]);
    activeAppId.set("com.example.thing");
    listInstalledApps
      .mockRejectedValueOnce(new Error("kernel busy: a trusted-chrome decision is pending"))
      .mockResolvedValueOnce([status()]);
    listApps.mockResolvedValue([withSurface]);

    render(AppsPage);

    expect(await screen.findByText(/surfaces are reserved here/)).toBeTruthy();
    await vi.waitFor(() => {
      expect(screen.getByRole("status").textContent).toContain("Waiting for the host");
    });
    expect(screen.queryByText(/Could not load apps:/)).toBeNull();

    await vi.advanceTimersByTimeAsync(1000);
    await vi.waitFor(() => expect(listInstalledApps).toHaveBeenCalledTimes(2));
    expect(screen.getByText(/surfaces are reserved here/)).toBeTruthy();
  });

  it("shows load failures without rendering stale manifests and retries explicitly", async () => {
    apps.set([installedApp("stale-app")]);
    activeAppId.set("stale-app");
    listInstalledApps.mockResolvedValue([]);
    listApps.mockRejectedValueOnce(new Error("backend unavailable"));
    const user = userEvent.setup();
    render(AppsPage);

    expect(await screen.findByText("Error: backend unavailable")).toBeTruthy();
    expect(screen.queryByText("Back to app management")).toBeNull();

    const beforeRetry = listApps.mock.calls.length;
    await user.click(screen.getByRole("button", { name: "Refresh state" }));
    expect(await screen.findByText(firstAppHeading)).toBeTruthy();
    expect(listApps.mock.calls.length).toBeGreaterThan(beforeRetry);
  });

  it("inspects then installs a package, and installation never runs code before confirm", async () => {
    listInstalledApps.mockResolvedValue([]);
    inspectPackage.mockResolvedValue(inspection());
    planManagedAppTransition.mockResolvedValue({
      transition_id: "transition-1",
      app_id: "com.example.new",
      operation: "install",
      current_revision_id: null,
      target_revision_id: "revision-target",
      target_version: "1.0.0",
      diff: {
        version_relation: "higher",
        display_name_changed: false,
        description_changed: false,
        backend_kind_changed: false,
        current_backend_authority_mode: null,
        target_backend_authority_mode: null,
        current_data: null,
        target_data: inspection().data,
        publisher_key_continuity: "new",
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
      staged_id: "staged-1",
      package_digest: `sha256-${"0".repeat(64)}`,
      revision_id: null,
    });
    applyManagedAppTransition.mockResolvedValue([status({ id: "com.example.new", display_name: "New App" })]);
    const user = userEvent.setup();
    render(AppsPage);

    await openInstaller(user);
    await user.type(screen.getByLabelText("Package directory path"), "/pkgs/new-app");
    await user.click(screen.getByRole("button", { name: "Review app" }));

    // Inspection shown; install NOT yet called (no code executed on inspect).
    expect(await screen.findByTestId("package-inspection")).toBeTruthy();
    expect(inspectPackage).toHaveBeenCalledWith("/pkgs/new-app");
    expect(planManagedAppTransition).toHaveBeenCalledOnce();

    await user.click(screen.getByRole("button", { name: "Install app" }));
    await waitFor(() =>
      expect(applyManagedAppTransition).toHaveBeenCalledWith(expect.objectContaining({ operation: "install" })),
    );
    expect(await screen.findByText("New App")).toBeTruthy();
  });

  it("lets the user choose a package folder with the native dialog", async () => {
    listInstalledApps.mockResolvedValue([]);
    openDirectory.mockResolvedValue("C:\\apps\\example-app");
    const user = userEvent.setup();
    render(AppsPage);

    await openInstaller(user);
    await user.click(screen.getByRole("button", { name: "Browse…" }));

    expect(openDirectory).toHaveBeenCalledWith({
      directory: true,
      multiple: false,
      title: "Choose an app package",
    });
    expect((screen.getByLabelText("Package directory path") as HTMLInputElement).value)
      .toBe("C:\\apps\\example-app");
  });

  it("inspects a public Git URL before installing its staged package", async () => {
    listInstalledApps.mockResolvedValue([]);
    inspectGitPackage.mockResolvedValue(inspection());
    const user = userEvent.setup();
    render(AppsPage);
    await openInstaller(user);

    await user.click(screen.getByRole("button", { name: "Public Git URL" }));
    await user.type(screen.getByLabelText("Public Git URL"), "https://github.com/example/app.git");
    await user.click(screen.getByRole("button", { name: "Review app" }));

    expect(await screen.findByTestId("package-inspection")).toBeTruthy();
    expect(inspectGitPackage).toHaveBeenCalledWith("https://github.com/example/app.git");
    expect(planManagedAppTransition).toHaveBeenCalledOnce();
  });

  it("refuses to enable Install for an un-installable package", async () => {
    listInstalledApps.mockResolvedValue([]);
    inspectPackage.mockResolvedValue(
      inspection({ installable: false, integrity_ok: false, blocking_error: "checksum mismatch" }),
    );
    const user = userEvent.setup();
    render(AppsPage);
    await openInstaller(user);

    await user.type(screen.getByLabelText("Package directory path"), "/pkgs/bad");
    await user.click(screen.getByRole("button", { name: "Review app" }));
    await screen.findByTestId("package-inspection");

    expect(screen.queryByRole("button", { name: "Install app" })).toBeNull();
  });

  it("disables a managed app", async () => {
    listInstalledApps.mockResolvedValue([status()]);
    setAppEnabled.mockResolvedValue([status({ enabled: false, status: "disabled" })]);
    const user = userEvent.setup();
    render(AppsPage);

    await screen.findByText("Thing");
    await user.click(screen.getByRole("button", { name: "Disable" }));
    await waitFor(() => expect(setAppEnabled).toHaveBeenCalledWith("com.example.thing", false));
    await screen.findByText("Disabled");
  });

  it("uninstalls with explicit secret/data purge choices", async () => {
    listInstalledApps.mockResolvedValue([status()]);
    uninstallApp.mockResolvedValue([]);
    const user = userEvent.setup();
    render(AppsPage);

    await screen.findByText("Thing");
    await user.click(screen.getByRole("button", { name: "Uninstall" }));
    // Prompt appears with both purges defaulting OFF so a plain uninstall stays
    // reversible; deleting secrets/data is an explicit, deliberate opt-in.
    const secretsCheckbox = await screen.findByLabelText(/delete stored secrets/);
    const dataCheckbox = await screen.findByLabelText(/delete app data/);
    await user.click(secretsCheckbox); // opt into deleting secrets
    await user.click(dataCheckbox); // opt into deleting data too
    // The confirm button names the irreversible selection it will execute.
    await user.click(screen.getByRole("button", { name: "Uninstall and delete secrets + data" }));

    await waitFor(() => expect(uninstallApp).toHaveBeenCalledWith("com.example.thing", true, true));
  });
});
