import { render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AppStatusView, InstalledApp, ManagedAppTransitionPlan, PackageInspection } from "$lib/api";
import InstallPanel from "$lib/apps/InstallPanel.svelte";
import { apps } from "$lib/stores/apps";

vi.mock("$lib/hostTransport", () => ({ isRemoteTransport: vi.fn(() => false) }));

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    inspectPackage: vi.fn(),
    inspectGitPackage: vi.fn(),
    listManagedAppRevisions: vi.fn(),
    planManagedAppTransition: vi.fn(),
    applyManagedAppTransition: vi.fn(),
    trustPublisherKey: vi.fn(),
  };
});

const api = await import("$lib/api");
const inspectPackage = vi.mocked(api.inspectPackage);
const trustPublisherKey = vi.mocked(api.trustPublisherKey);
const planManagedAppTransition = vi.mocked(api.planManagedAppTransition);
const applyManagedAppTransition = vi.mocked(api.applyManagedAppTransition);

function installedApp(version = "1.0.0", contentHash = "sha256-current"): InstalledApp {
  return {
    manifest: {
      app_id: "com.example.app",
      version,
      display_name: "Example App",
      description: "Installed app",
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
    content_hash: contentHash,
    installed_at: "2026-07-10T00:00:00Z",
  };
}

function inspection(overrides: Partial<PackageInspection> = {}): PackageInspection {
  return {
    staged_id: "staged-1",
    package_digest: "sha256-package",
    id: "com.example.app",
    version: "1.0.0",
    display_name: "Example App",
    description: "Package inspection",
    publisher: { name: "Example Org", homepage: null, key_id: "ed25519:key-1" },
    license: null,
    signature: { kind: "unsigned" },
    signature_public_key: null,
    backend_kind: "none",
    backend_detail: "No backend",
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
    min_host_version: "0.1.0",
    host_version: "0.1.0",
    host_compatible: true,
    capabilities: [],
    grant_requests: [
      {
        scope_label: "com.example.app/open",
        data_scope_label: "all data",
        condition: "requires-approval",
        reason: "Needs approval",
        duration_label: "non-expiring",
      },
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

function plan(operation: ManagedAppTransitionPlan["operation"]): ManagedAppTransitionPlan {
  return {
    transition_id: "transition-1",
    app_id: "com.example.app",
    operation,
    current_revision_id: "revision-current",
    target_revision_id: "revision-target",
    target_version: operation === "downgrade" ? "0.9.0" : operation === "update" ? "1.1.0" : "1.0.0",
    diff: {
      version_relation: operation === "downgrade" ? "lower" : operation === "update" ? "higher" : "same",
      display_name_changed: false,
      description_changed: false,
      backend_kind_changed: false,
      current_backend_authority_mode: null,
      target_backend_authority_mode: null,
      current_data: null,
      target_data: inspection().data,
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
    staged_id: "staged-1",
    package_digest: "sha256-package",
    revision_id: null,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  apps.set([]);
  inspectPackage.mockReset();
  planManagedAppTransition.mockReset();
  applyManagedAppTransition.mockReset();
  trustPublisherKey.mockReset();
});

describe("InstallPanel", () => {
  it("trusts a valid unknown key for the exact app id and re-inspects instead of installing immediately", async () => {
    inspectPackage
      .mockResolvedValueOnce(
        inspection({
          signature: { kind: "valid-unknown-key", key_id: "ed25519:key-1" },
          signature_public_key: "BASE64KEY",
        }),
      )
      .mockResolvedValueOnce(
        inspection({
          signature: { kind: "trusted", key_id: "ed25519:key-1", scope: { kind: "app-id", app_id: "com.example.app" } },
          signature_public_key: "BASE64KEY",
        }),
      );
    planManagedAppTransition.mockResolvedValue(plan("install"));
    const user = userEvent.setup();
    render(InstallPanel, { props: { onInstalled: vi.fn(async () => undefined) } });

    await user.type(screen.getByLabelText("Package directory path"), "C:/pkgs/example");
    await user.click(screen.getByRole("button", { name: "Review app" }));
    await screen.findByTestId("package-inspection");

    await user.click(screen.getByRole("button", { name: "Trust key for com.example.app" }));

    expect(trustPublisherKey).toHaveBeenCalledWith({
      key_id: "ed25519:key-1",
      public_key: "BASE64KEY",
      scope: { kind: "app-id", app_id: "com.example.app" },
    });
    expect(inspectPackage).toHaveBeenCalledTimes(2);
  });

  it("puts trust, runtime isolation, and permissions first without a duplicate install diff", async () => {
    inspectPackage.mockResolvedValue(
      inspection({
        backend_kind: "executable",
        backend_detail: "Sandboxed executable",
        backend_authority_mode: "sandboxed",
      }),
    );
    planManagedAppTransition.mockResolvedValue(plan("install"));
    const user = userEvent.setup();
    render(InstallPanel, { props: { onInstalled: vi.fn(async () => undefined) } });

    await user.type(screen.getByLabelText("Package directory path"), "C:/pkgs/example");
    await user.click(screen.getByRole("button", { name: "Review app" }));

    expect(await screen.findByText("Publisher trust")).toBeTruthy();
    expect(screen.getByText("Sandboxed backend")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Permissions this app requests (1)" })).toBeTruthy();
    expect(screen.getAllByText("com.example.app/open")).toHaveLength(1);
    expect(screen.queryByRole("region", { name: "Managed app review" })).toBeNull();
    expect(screen.getByRole("button", { name: "Install app" })).toBeTruthy();
  });

  it("discloses host-managed access, retention, and the stored record schema", async () => {
    inspectPackage.mockResolvedValue(
      inspection({
        data: {
          kind: "host-managed",
          format_version: null,
          migration_protocol_version: null,
          transitions: [],
          contract_version: 2,
          total_bytes: 1_048_576,
          batch_operations: 2048,
          collections: [{
            name: "items",
            schema: {
              type: "object",
              additionalProperties: false,
              required: ["title"],
              properties: { title: { type: "string" } },
            },
            operations: ["get", "create"],
            records: 100,
            record_bytes: 4096,
            query_results: 10,
            indexes: [],
            unique_indexes: [],
          }],
          documents: [{
            name: "scenes",
            metadata_schema: {
              type: "object",
              additionalProperties: false,
              required: ["title"],
              properties: { title: { type: "string" } },
            },
            operations: ["get", "list", "create", "replace", "delete"],
            documents: 100,
            metadata_bytes: 4096,
            content_bytes: 8_388_608,
          }],
          proposals: [{
            capability: "propose_item",
            artifact_type: "item-proposal",
            title: "Propose item change",
            description: "Create a reviewable artifact.",
            target_kind: "record",
            collection: "items",
            max_payload_bytes: 4096,
            payload_schema: { type: "object", additionalProperties: false },
          }],
        },
      }),
    );
    planManagedAppTransition.mockResolvedValue(plan("install"));
    const user = userEvent.setup();
    render(InstallPanel, { props: { onInstalled: vi.fn(async () => undefined) } });

    await user.type(screen.getByLabelText("Package directory path"), "C:/pkgs/managed");
    await user.click(screen.getByRole("button", { name: "Review app" }));

    await screen.findByText("Data storage and retention");
    await user.click(screen.getByText("Data storage and retention"));
    expect(screen.getByRole("heading", { name: "How this app stores data" })).toBeTruthy();
    expect(screen.getByText(/access from Chat or other apps still requires a permission/)).toBeTruthy();
    expect(screen.getByText(/kept when the app is disabled or removed/)).toBeTruthy();
    expect(screen.getByText(/Reviewable proposals/)).toBeTruthy();
    expect(screen.getByText(/does not change managed data/)).toBeTruthy();
    expect(screen.getByText(/record target: items/)).toBeTruthy();
    expect(screen.getByText(/up to 4096 payload bytes/)).toBeTruthy();
    await user.click(screen.getByText("Technical package details"));
    expect(screen.getByText(/Managed document collections/)).toBeTruthy();
    await user.click(screen.getByText("Stored record fields"));
    expect(screen.getAllByText(/"title"/).length).toBeGreaterThanOrEqual(1);
  });

  it("reviews and commits an update without skipping the permission review", async () => {
    apps.set([installedApp("1.0.0", "sha256-current")]);
    inspectPackage.mockResolvedValue(
      inspection({
        version: "1.1.0",
        package_digest: "sha256-new",
        signature_public_key: "BASE64KEY",
      }),
    );
    planManagedAppTransition.mockResolvedValue({
      ...plan("update"),
      diff: {
        ...plan("update").diff,
        permissions: {
          unchanged: [],
          added: [{
            scope_label: "com.example.app/open",
            data_scope_label: "all data",
            condition: "requires-approval",
            reason: "Needs approval",
            duration_label: "non-expiring",
          }],
          widened: [],
          removed: [],
        },
      },
      data_rollback_supported: true,
      data_transition: {
        source_format_version: 1,
        target_format_version: 2,
        destructive: true,
        reverse_migration: false,
      },
    });
    applyManagedAppTransition.mockResolvedValue([{
      id: "com.example.app",
      display_name: "Example App",
      version: "1.1.0",
      description: "Updated app",
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
      installed_at: "2026-07-18T00:00:00Z",
      revisions: [],
      extension_contributions: [],
      removable: true,
    }]);
    const onInstalled = vi.fn(async () => undefined);
    const user = userEvent.setup();
    render(InstallPanel, { props: { onInstalled } });

    await user.type(screen.getByLabelText("Package directory path"), "C:/pkgs/example");
    await user.click(screen.getByRole("button", { name: "Review app" }));
    await screen.findByRole("button", { name: "Update app" });
    expect(screen.getByText(/Format 1 → 2/)).toBeTruthy();
    expect(screen.getByText(/marks the migration as destructive/)).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Permission diff" })).toBeTruthy();
    expect(screen.getAllByText("com.example.app/open")).toHaveLength(1);

    await user.click(screen.getByRole("button", { name: "Update app" }));

    expect(applyManagedAppTransition).toHaveBeenCalledWith(expect.objectContaining({ operation: "update" }));
    expect(onInstalled).toHaveBeenCalled();
  });

  it("keeps native authority and extension breakage warnings visible in review", async () => {
    apps.set([installedApp("1.0.0", "sha256-current")]);
    inspectPackage.mockResolvedValue(
      inspection({
        version: "1.1.0",
        package_digest: "sha256-new",
        backend_kind: "agent-worker",
        backend_detail: "Agent worker protocol v1 (unsandboxed)",
        backend_authority_mode: "unsandboxed",
      }),
    );
    planManagedAppTransition.mockResolvedValue({
      ...plan("update"),
      diff: {
        ...plan("update").diff,
        current_backend_authority_mode: "unsandboxed",
        target_backend_authority_mode: "unsandboxed",
        extension_warnings: [{
          contributor_app_id: "com.example.annotator",
          extension_point: "message-actions",
          surface: "annotation",
          contribution_contract_version: 6,
          current_target_contract_version: 6,
          target_contract_version: 7,
        }],
      },
    });
    const user = userEvent.setup();
    render(InstallPanel, { props: { onInstalled: vi.fn(async () => undefined) } });

    await user.type(screen.getByLabelText("Package directory path"), "C:/pkgs/example");
    await user.click(screen.getByRole("button", { name: "Review app" }));

    expect(await screen.findAllByRole("region", { name: "Unsandboxed native backend warning" })).toHaveLength(1);
    expect(screen.getByRole("region", { name: "Extension compatibility warning" })).toBeTruthy();
    expect(screen.getByText(/will leave 1 installed contribution dormant/)).toBeTruthy();
    expect((screen.getByRole("button", { name: "Update app" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("requires a downgrade acknowledgement before preparing the downgrade review", async () => {
    apps.set([installedApp("2.0.0", "sha256-current")]);
    inspectPackage.mockResolvedValue(
      inspection({
        version: "1.0.0",
        package_digest: "sha256-new",
        signature_public_key: "BASE64KEY",
      }),
    );
    planManagedAppTransition.mockResolvedValue(plan("downgrade"));
    const user = userEvent.setup();
    render(InstallPanel, { props: { onInstalled: vi.fn(async () => undefined) } });

    await user.type(screen.getByLabelText("Package directory path"), "C:/pkgs/example");
    await user.click(screen.getByRole("button", { name: "Review app" }));
    await screen.findByText("Check the downgrade acknowledgement to review the diff.");

    expect(screen.getByRole("button", { name: "Downgrade app" }).hasAttribute("disabled")).toBe(true);
    await user.click(screen.getByRole("checkbox", { name: /I understand this is a downgrade/ }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Downgrade app" })).toBeTruthy());
  });
});
