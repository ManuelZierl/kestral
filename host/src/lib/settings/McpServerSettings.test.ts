import { fireEvent, render, screen } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  listMcpServers: vi.fn(),
  connectMcpServer: vi.fn(),
  refreshHost: vi.fn(async () => undefined),
}));

vi.mock("$lib/api", () => ({
  clearMcpHttpAuthSecret: vi.fn(async () => undefined),
  connectMcpServer: mocks.connectMcpServer,
  deleteMcpServer: vi.fn(async () => undefined),
  disconnectMcpServer: vi.fn(async () => undefined),
  hasMcpHttpAuthSecret: vi.fn(async () => false),
  listMcpServers: mocks.listMcpServers,
  putMcpHttpAuthSecret: vi.fn(async () => undefined),
  upsertMcpServer: vi.fn(async (server) => server),
}));

vi.mock("$lib/stores/hostState", () => ({ refreshHost: mocks.refreshHost }));

import McpServerSettings from "$lib/settings/McpServerSettings.svelte";

const server = {
  id: "x",
  display_name: "X tools",
  transport: {
    kind: "streamable-http" as const,
    url: "https://api.x.com/mcp",
    authentication: { kind: "none" as const },
  },
  connected: false,
};

describe("McpServerSettings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listMcpServers.mockResolvedValue([server]);
  });

  it("keeps a failed connection visible through the final status refresh and retries the connection", async () => {
    mocks.connectMcpServer
      .mockRejectedValueOnce(new Error("MCP endpoint answered HTTP 401 Unauthorized"))
      .mockResolvedValueOnce(undefined);
    render(McpServerSettings);

    await fireEvent.click(await screen.findByRole("button", { name: "Connect" }));

    expect((await screen.findByRole("alert")).textContent).toContain("Authentication failed");
    expect(mocks.listMcpServers).toHaveBeenCalledTimes(2);
    const retry = screen.getByRole("button", { name: "Retry connection" });
    await fireEvent.click(retry);
    await vi.waitFor(() => expect(mocks.connectMcpServer).toHaveBeenCalledTimes(2));
    expect(screen.queryByText(/Authentication failed/)).toBeNull();
  });
});
