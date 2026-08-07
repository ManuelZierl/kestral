import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, expect, it, vi } from "vitest";

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    listTrustedFileResources: vi.fn(async () => []),
    registerFileResource: vi.fn(async () => undefined),
    removeFileResource: vi.fn(async () => undefined),
    grantFileResourceAccess: vi.fn(async () => undefined),
  };
});

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import FileResourcesSettings from "./FileResourcesSettings.svelte";
import * as api from "$lib/api";
import type { TrustedFileResourceView } from "$lib/api";
import { apps } from "$lib/stores/apps";
import { fileResources, fileResourcesLoaded, refreshFileResources } from "$lib/stores/fileResources";

beforeEach(() => {
  sessionStorage.clear();
  fileResources.set([]);
  fileResourcesLoaded.set(true);
  vi.clearAllMocks();
  vi.mocked(api.listTrustedFileResources).mockReset().mockResolvedValue([]);
  vi.mocked(api.registerFileResource).mockReset().mockResolvedValue({
    resource_id: "registered-resource",
    display_name: "Registered resource",
    canonical_path: "/registered-resource",
    kind: "directory",
    created_at: "2026-08-03T00:00:00Z",
    status: "active",
  } satisfies TrustedFileResourceView);
  vi.mocked(api.removeFileResource).mockReset().mockResolvedValue(undefined);
  vi.mocked(api.grantFileResourceAccess).mockReset().mockResolvedValue(undefined);
});

it("registers a server-side path in browser host mode", async () => {
  sessionStorage.setItem("host.remote.url", "http://localhost:1420");
  render(FileResourcesSettings);

  const input = screen.getByLabelText("File or folder path on the Kestral host");
  await fireEvent.input(input, { target: { value: "/srv/kestral/documents" } });
  await fireEvent.click(screen.getByRole("button", { name: "Register host path" }));

  await waitFor(() => expect(api.registerFileResource).toHaveBeenCalledWith(
    "/srv/kestral/documents",
  ));
});

it("renders the file resources settings shell", async () => {
  render(FileResourcesSettings);
  await refreshFileResources();

  expect(screen.getByText(/Register a file or folder here\./)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Add file" })).toBeTruthy();
  expect(screen.getByText("No file resources registered yet.")).toBeTruthy();
});

it("shows and retries an initial file-resource load failure", async () => {
  fileResourcesLoaded.set(false);
  vi.mocked(api.listTrustedFileResources)
    .mockRejectedValueOnce(new Error("registry unavailable"))
    .mockResolvedValueOnce([]);
  render(FileResourcesSettings);

  expect((await screen.findByRole("alert")).textContent).toContain("registry unavailable");
  expect(screen.queryByText("Loading file resources…")).toBeNull();

  await fireEvent.click(screen.getByRole("button", { name: "Retry file resources" }));
  expect(await screen.findByText("No file resources registered yet.")).toBeTruthy();
});

it("sends backend operation names when granting file access", async () => {
  apps.set([{
    manifest: { app_id: "chat", display_name: "Chat" },
  } as never]);
  vi.mocked(api.listTrustedFileResources).mockResolvedValueOnce([{
    resource_id: "resource-1",
    display_name: "Documents",
    canonical_path: "C:\\Users\\person\\Documents",
    kind: "directory",
    created_at: "2026-07-20T12:00:00Z",
    status: "active",
  }]);
  render(FileResourcesSettings);

  await fireEvent.click(await screen.findByRole("button", { name: "List" }));

  await waitFor(() => expect(api.grantFileResourceAccess).toHaveBeenCalledWith(
    "chat",
    "resource-1",
    ["list"],
  ));
});
