import { fireEvent, render, screen } from "@testing-library/svelte";
import { beforeEach, expect, it, vi } from "vitest";

const { open, save, importPortableProfile, exportPortableProfile } = vi.hoisted(() => ({
  open: vi.fn(),
  save: vi.fn(),
  importPortableProfile: vi.fn(),
  exportPortableProfile: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open, save }));
vi.mock("$lib/hostTransport", async (importOriginal) => ({
  ...(await importOriginal<typeof import("$lib/hostTransport")>()),
  isRemoteTransport: () => false,
}));
vi.mock("$lib/api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("$lib/api")>()),
  getActiveKestralProfile: vi.fn(async () => ({
    profile_id: "profile-work", display_name: "Work", slug: "work", root: "/tmp/work",
    created_at: "2026-08-09T00:00:00Z", current_runtime: true,
    selected_for_next_launch: true, source: "managed", launch_args: ["--profile", "work"],
    restart_instructions: "Restart Kestral with: --profile work",
  })),
  getPortableRecoveryStatus: vi.fn(async () => null),
  importPortableProfile,
  exportPortableProfile,
}));

import PortableTransfer from "./PortableTransfer.svelte";

beforeEach(() => {
  vi.clearAllMocks();
  importPortableProfile.mockResolvedValue({
    target: "preview", restart_required: false, restart_instructions: "",
    apps: [{ id: "com.example.app", display_name: "Example", version: "1.0.0", package_digest: "abc" }],
    secrets: [{ owner: "llm-provider", name: "api-key" }],
    file_resources: [{ resource_id: "resource-1", display_name: "Notes", kind: "directory" }],
  });
});

it("validates an archive before enabling target selection", async () => {
  open.mockResolvedValue("/tmp/workspace.kestral-portable.zip");
  render(PortableTransfer);

  await fireEvent.click(screen.getByRole("button", { name: "Choose archive" }));

  expect(await screen.findByText("Archive verified")).toBeTruthy();
  expect(importPortableProfile).toHaveBeenCalledWith("/tmp/workspace.kestral-portable.zip", { kind: "preview" });
  expect(screen.getByText("1 to re-enter")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Import as new profile" })).toBeTruthy();
});

it("requires the exact overwrite confirmation", async () => {
  open.mockResolvedValue("/tmp/workspace.kestral-portable.zip");
  render(PortableTransfer);
  await fireEvent.click(screen.getByRole("button", { name: "Choose archive" }));
  await screen.findByText("Archive verified");
  await fireEvent.click(screen.getByLabelText("Overwrite the current profile after restart"));

  const submit = screen.getByRole("button", { name: "Schedule overwrite and restart" }) as HTMLButtonElement;
  expect(submit.disabled).toBe(true);
  await fireEvent.input(screen.getByLabelText(/Type RESTORE work to confirm/), { target: { value: "RESTORE work" } });
  expect(submit.disabled).toBe(false);
});
