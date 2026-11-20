import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("$lib/hostTransport", () => ({
  pairRemoteConnection: vi.fn(),
  remoteUrlDefault: vi.fn(() => "http://localhost:1420"),
  signInRemoteConnection: vi.fn(),
}));
vi.mock("$lib/remotePasskeys", () => ({
  passkeysAvailable: vi.fn(() => true),
}));

import { signInRemoteConnection } from "$lib/hostTransport";
import RemoteConnection from "./RemoteConnection.svelte";

describe("browser Host URL connection screen", () => {
  beforeEach(() => vi.clearAllMocks());

  it("shows the same-origin default and explains when it needs an API proxy", async () => {
    render(RemoteConnection, { onConnected: vi.fn() });

    const input = screen.getByLabelText("Host URL") as HTMLInputElement;
    expect(input.value).toBe("http://localhost:1420");
    expect(input.getAttribute("aria-describedby")).toBe("host-url-help");
    expect(screen.getByText(/The page address works only if it proxies/)).toBeTruthy();
    expect(screen.getByText(/http:\/\/localhost:4310/)).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Sign in with passkey" }));
    await waitFor(() => expect(signInRemoteConnection).toHaveBeenCalledWith("http://localhost:1420"));
  });

  it("uses a separately served backend address entered by the user", async () => {
    render(RemoteConnection, { onConnected: vi.fn() });

    await fireEvent.input(screen.getByLabelText("Host URL"), {
      target: { value: "http://localhost:4310" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Sign in with passkey" }));

    await waitFor(() => expect(signInRemoteConnection).toHaveBeenCalledWith("http://localhost:4310"));
  });

  it("gives actionable guidance when the backend cannot be reached", async () => {
    vi.mocked(signInRemoteConnection).mockRejectedValueOnce(new TypeError("Failed to fetch"));
    render(RemoteConnection, { onConnected: vi.fn() });

    await fireEvent.click(screen.getByRole("button", { name: "Sign in with passkey" }));

    const error = await screen.findByRole("alert");
    expect(error.textContent).toContain("Could not connect to the Kestral backend");
    expect(error.textContent).toContain("HOST_REMOTE_ORIGIN");
  });
});
