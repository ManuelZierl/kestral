import { render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("$lib/remotePasskeys", () => ({
  createPasskey: vi.fn().mockResolvedValue({ id: "registered" }),
  getPasskey: vi.fn().mockResolvedValue({ id: "authenticated" }),
}));
vi.mock("$lib/shell/HostShell.svelte", async () => ({
  default: (await import("./test/RouteHostShell.svelte")).default,
}));
vi.mock("$lib/shell/RemoteConnection.svelte", async () => ({
  default: (await import("./test/RouteRemoteConnection.svelte")).default,
}));

import {
  clearRemoteConnection,
  signInRemoteConnection,
  signOutRemoteConnection,
} from "$lib/hostTransport";
import Page from "./+page.svelte";

describe("host route connection state", () => {
  beforeEach(() => {
    sessionStorage.clear();
    clearRemoteConnection();
  });

  afterEach(() => {
    clearRemoteConnection();
    vi.unstubAllGlobals();
  });

  it("returns to the connection screen after a remote owner signs out", async () => {
    const fetch = vi.fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ ceremony_id: "login-1", options: {} }), { status: 200 }),
      )
      .mockResolvedValueOnce(new Response(JSON.stringify({ authenticated: true }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ authenticated: false }), { status: 200 }));
    vi.stubGlobal("fetch", fetch);
    render(Page);

    expect(await screen.findByText("Remote connection")).toBeTruthy();

    await signInRemoteConnection("https://kestral.example");
    expect(await screen.findByText("Host shell")).toBeTruthy();

    await signOutRemoteConnection();
    await waitFor(() => expect(screen.queryByText("Host shell")).toBeNull());
    expect(screen.getByText("Remote connection")).toBeTruthy();
  });

  it("renders the host shell directly in the Tauri runtime", async () => {
    vi.stubGlobal("__TAURI_INTERNALS__", {});

    render(Page);

    expect(await screen.findByText("Host shell")).toBeTruthy();
    expect(screen.queryByText("Remote connection")).toBeNull();
  });
});
