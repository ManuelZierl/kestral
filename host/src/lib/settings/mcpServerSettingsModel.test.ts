import { describe, expect, it } from "vitest";

import type { McpServerStatusView } from "$lib/api";
import {
  draftFromServer,
  draftToServer,
  emptyMcpServerDraft,
  transportSummary,
} from "./mcpServerSettingsModel";

describe("mcpServerSettingsModel", () => {
  it("builds a stdio server from one argument per line", () => {
    const result = draftToServer({
      ...emptyMcpServerDraft(),
      id: " weather ",
      displayName: "Weather",
      command: "node",
      args: "server.mjs\n--verbose",
    });
    expect(result).toEqual({
      ok: true,
      server: {
        id: "weather",
        display_name: "Weather",
        transport: { kind: "stdio", command: "node", args: ["server.mjs", "--verbose"] },
      },
    });
  });

  it("round-trips arguments containing spaces without shell parsing", () => {
    const server: McpServerStatusView = {
      id: "spaced",
      display_name: "Spaced",
      transport: {
        kind: "stdio",
        command: "C:\\Program Files\\nodejs\\node.exe",
        args: ["C:\\My Servers\\server.mjs", "--label=My Server", ""],
      },
      connected: false,
    };

    expect(draftToServer(draftFromServer(server))).toEqual({
      ok: true,
      server: {
        id: server.id,
        display_name: server.display_name,
        transport: server.transport,
      },
    });
  });

  it("builds a streamable-http server and falls back to the id as name", () => {
    const result = draftToServer({
      ...emptyMcpServerDraft(),
      id: "remote",
      transportKind: "streamable-http",
      url: "https://mcp.example/mcp",
    });
    expect(result).toEqual({
      ok: true,
      server: {
        id: "remote",
        display_name: "remote",
        transport: { kind: "streamable-http", url: "https://mcp.example/mcp", authentication: { kind: "none" } },
      },
    });
  });

  it("rejects invalid drafts with a usable message", () => {
    expect(draftToServer(emptyMcpServerDraft()).ok).toBe(false);
    expect(draftToServer({ ...emptyMcpServerDraft(), id: "has space" }).ok).toBe(false);
    expect(draftToServer({ ...emptyMcpServerDraft(), id: "x" }).ok).toBe(false); // no command
    expect(
      draftToServer({
        ...emptyMcpServerDraft(),
        id: "x",
        transportKind: "streamable-http",
        url: "ftp://nope",
      }).ok,
    ).toBe(false);
  });

  it("round-trips a server through a draft", () => {
    const server: McpServerStatusView = {
      id: "weather",
      display_name: "Weather",
      transport: { kind: "stdio", command: "node", args: ["server.mjs"] },
      connected: false,
    };
    const draft = draftFromServer(server);
    const rebuilt = draftToServer(draft);
    expect(rebuilt).toEqual({
      ok: true,
      server: {
        id: "weather",
        display_name: "Weather",
        transport: { kind: "stdio", command: "node", args: ["server.mjs"] },
      },
    });
  });

  it("summarizes transports for the list row", () => {
    expect(
      transportSummary({
        id: "w",
        display_name: "W",
        transport: { kind: "stdio", command: "node", args: ["server.mjs"] },
        connected: false,
      }),
    ).toBe("node server.mjs");
    expect(
      transportSummary({
        id: "r",
        display_name: "R",
        transport: { kind: "streamable-http", url: "https://mcp.example/mcp", authentication: { kind: "none" } },
        connected: true,
      }),
    ).toBe("https://mcp.example/mcp · No authentication");
  });

  it("builds generic static-header and Bearer authentication", () => {
    expect(draftToServer({
      ...emptyMcpServerDraft(),
      id: "x",
      transportKind: "streamable-http",
      url: "https://api.x.com/mcp",
      httpAuthKind: "bearer",
    })).toEqual({
      ok: true,
      server: {
        id: "x",
        display_name: "x",
        transport: {
          kind: "streamable-http",
          url: "https://api.x.com/mcp",
          authentication: { kind: "static-header", header_name: "Authorization", value_prefix: "Bearer " },
        },
      },
    });
  });
});
