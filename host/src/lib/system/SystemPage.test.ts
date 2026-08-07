import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import SystemPage from "./SystemPage.svelte";

const { getActiveKestralProfile, getConfigStorageInfo, requestSystemReset } = vi.hoisted(() => ({
  getActiveKestralProfile: vi.fn(),
  getConfigStorageInfo: vi.fn(),
  requestSystemReset: vi.fn(),
}));

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    getActiveKestralProfile,
    getConfigStorageInfo,
    requestSystemReset,
    listTrustedNotices: vi.fn(async () => []),
  };
});

vi.mock("$lib/hostTransport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/hostTransport")>();
  return { ...actual, isRemoteTransport: vi.fn(() => false) };
});

beforeEach(() => {
  vi.clearAllMocks();
  getConfigStorageInfo.mockResolvedValue({
    config_path: "C:/data/host-config.json",
    secrets_path: "C:/data/host-secrets.json",
    chat_store_path: "C:/data/chat.json",
    file_resource_registry_path: "C:/data/file-resources.json",
    profile_registry_path: "C:/data/profiles.json",
  });
  getActiveKestralProfile.mockResolvedValue({
    profile_id: "profile-default",
    display_name: "Default Kestral profile",
    slug: "default",
    root: "C:/data",
    created_at: "2026-07-26T00:00:00Z",
    current_runtime: true,
    selected_for_next_launch: true,
    source: "managed",
    launch_args: ["--profile", "default"],
    restart_instructions: "Restart Kestral with: --profile default",
  });
  requestSystemReset.mockResolvedValue({ restart_required: false });
});

describe("SystemPage", () => {
  it("renders storage paths returned by the backend", async () => {
    render(SystemPage);

    expect(await screen.findByText("C:/data/host-config.json")).toBeTruthy();
    expect(screen.getByText("C:/data/chat.json")).toBeTruthy();
    expect(getConfigStorageInfo).toHaveBeenCalledTimes(1);
  });

  it("renders the backend error instead of leaving storage in a loading state", async () => {
    getConfigStorageInfo.mockRejectedValueOnce(new Error("storage unavailable"));
    render(SystemPage);

    expect(await screen.findByText("Error: storage unavailable")).toBeTruthy();
    expect(screen.queryByText("Loading storage paths...")).toBeNull();
  });

  it("requires the exact profile phrase before requesting a reset", async () => {
    const user = userEvent.setup();
    localStorage.setItem("host-theme-preference", "dark");
    localStorage.setItem("host-custom-theme-profiles", '{"version":1,"profiles":[]}');
    localStorage.setItem("host-sidebar-layout", '{"version":1,"collapsed":false,"order":[],"hidden":[]}');
    localStorage.setItem("kernel.active-chat-thread", "thread-1");
    render(SystemPage);

    await user.click(await screen.findByRole("button", { name: "Review system reset" }));
    const resetButton = screen.getByRole("button", { name: "Reset and restart Kestral" }) as HTMLButtonElement;
    expect(resetButton.disabled).toBe(true);

    await user.type(screen.getByLabelText(/Type RESET default/), "RESET default");
    expect(resetButton.disabled).toBe(false);
    await user.click(resetButton);

    expect(requestSystemReset).toHaveBeenCalledWith("RESET default");
    expect(await screen.findByText("Reset scheduled. Kestral is restarting to finish it.")).toBeTruthy();
    expect(localStorage.getItem("host-theme-preference")).toBeNull();
    expect(localStorage.getItem("host-custom-theme-profiles")).toBeNull();
    expect(localStorage.getItem("host-sidebar-layout")).toBeNull();
    expect(localStorage.getItem("kernel.active-chat-thread")).toBeNull();
  });
});
