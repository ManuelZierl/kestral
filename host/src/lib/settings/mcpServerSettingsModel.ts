import type { McpServerConfigView, McpServerStatusView } from "$lib/api";

/// Form state for adding or editing one MCP server. Kept flat and stringly
/// for binding; `draftToServer` validates and produces the wire shape.
export interface McpServerDraft {
  id: string;
  displayName: string;
  transportKind: "stdio" | "streamable-http";
  command: string;
  /// One argument per line in the form; spaces remain part of an argument.
  args: string;
  url: string;
  httpAuthKind: "none" | "bearer" | "custom-header";
  httpHeaderName: string;
  httpValuePrefix: string;
}

export function emptyMcpServerDraft(): McpServerDraft {
  return {
    id: "",
    displayName: "",
    transportKind: "stdio",
    command: "",
    args: "",
    url: "",
    httpAuthKind: "none",
    httpHeaderName: "",
    httpValuePrefix: "",
  };
}

export function draftFromServer(server: McpServerStatusView): McpServerDraft {
  return {
    id: server.id,
    displayName: server.display_name,
    transportKind: server.transport.kind,
    command: server.transport.kind === "stdio" ? server.transport.command : "",
    args: server.transport.kind === "stdio" ? server.transport.args.join("\n") : "",
    url: server.transport.kind === "streamable-http" ? server.transport.url : "",
    httpAuthKind: server.transport.kind !== "streamable-http" || server.transport.authentication.kind === "none"
      ? "none"
      : server.transport.authentication.header_name.toLowerCase() === "authorization" &&
          server.transport.authentication.value_prefix === "Bearer "
        ? "bearer"
        : "custom-header",
    httpHeaderName: server.transport.kind === "streamable-http" && server.transport.authentication.kind === "static-header"
      ? server.transport.authentication.header_name
      : "",
    httpValuePrefix: server.transport.kind === "streamable-http" && server.transport.authentication.kind === "static-header"
      ? server.transport.authentication.value_prefix
      : "",
  };
}

export type DraftResult =
  | { ok: true; server: McpServerConfigView }
  | { ok: false; error: string };

export function draftToServer(draft: McpServerDraft): DraftResult {
  const id = draft.id.trim();
  if (id === "") {
    return { ok: false, error: "Give the server a short id (e.g. weather)." };
  }
  if (/\s/.test(id)) {
    return { ok: false, error: "The server id cannot contain spaces." };
  }
  const displayName = draft.displayName.trim() || id;
  if (draft.transportKind === "stdio") {
    const command = draft.command.trim();
    if (command === "") {
      return { ok: false, error: "A command is required (for example, node)." };
    }
    return {
      ok: true,
      server: {
        id,
        display_name: displayName,
        transport: {
          kind: "stdio",
          command,
          args: draft.args === "" ? [] : draft.args.split(/\r?\n/),
        },
      },
    };
  }
  const url = draft.url.trim();
  if (!url.startsWith("http://") && !url.startsWith("https://")) {
    return { ok: false, error: "The endpoint must be an http(s) URL." };
  }
  let authentication: Extract<McpServerConfigView["transport"], { kind: "streamable-http" }>["authentication"];
  if (draft.httpAuthKind === "none") {
    authentication = { kind: "none" };
  } else if (draft.httpAuthKind === "bearer") {
    authentication = { kind: "static-header", header_name: "Authorization", value_prefix: "Bearer " };
  } else {
    const headerName = draft.httpHeaderName.trim();
    if (headerName === "") {
      return { ok: false, error: "Enter the HTTP authentication header name." };
    }
    if (/[^!#$%&'*+.^_`|~0-9A-Za-z-]/.test(headerName)) {
      return { ok: false, error: "The HTTP authentication header name is invalid." };
    }
    if (/\r|\n/.test(draft.httpValuePrefix)) {
      return { ok: false, error: "The HTTP authentication prefix cannot contain a line break." };
    }
    authentication = {
      kind: "static-header",
      header_name: headerName,
      value_prefix: draft.httpValuePrefix,
    };
  }
  return {
    ok: true,
    server: { id, display_name: displayName, transport: { kind: "streamable-http", url, authentication } },
  };
}

export function transportSummary(server: McpServerStatusView): string {
  if (server.transport.kind === "stdio") {
    return [server.transport.command, ...server.transport.args].join(" ");
  }
  const auth = server.transport.authentication.kind === "none"
    ? "No authentication"
    : `${server.transport.authentication.header_name} authentication`;
  return `${server.transport.url} · ${auth}`;
}
