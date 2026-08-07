// A deliberately ordinary MCP server, written in plain JavaScript with zero
// dependencies. This is the whole point of the degraded-mode bridge:
// an app author writes only this file — the host derives the manifest, the
// form surfaces, the result-card artifacts, and the requires-approval grants.
//
// Transport: newline-delimited JSON-RPC 2.0 over stdio (the MCP stdio
// transport). Implements: initialize, ping, tools/list, tools/call.

import { createInterface } from "node:readline";

const SERVER_INFO = { name: "demo-weather", version: "0.1.0" };
const PROTOCOL_VERSION = "2025-06-18";

const TOOLS = [
  {
    name: "get_forecast",
    description: "Get the weather forecast for a city",
    inputSchema: {
      type: "object",
      properties: { city: { type: "string", minLength: 1 } },
      required: ["city"],
      additionalProperties: false,
    },
  },
];

// Deterministic fake forecast: same city, same weather — keeps the demo
// reproducible and the third-party-parity story honest.
function forecast(city) {
  const conditions = ["sunny", "cloudy", "rainy", "windy", "snowy"];
  let hash = 0;
  for (const char of city) hash = (hash * 31 + char.codePointAt(0)) >>> 0;
  return {
    city,
    forecast: conditions[hash % conditions.length],
    high_celsius: 15 + (hash % 20),
  };
}

function send(message) {
  process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n");
}

function handle(request) {
  const { id, method, params } = request;
  if (id === undefined) return; // notification (e.g. notifications/initialized)

  switch (method) {
    case "initialize":
      send({
        id,
        result: {
          protocolVersion: params?.protocolVersion ?? PROTOCOL_VERSION,
          capabilities: { tools: {} },
          serverInfo: SERVER_INFO,
        },
      });
      return;
    case "ping":
      send({ id, result: {} });
      return;
    case "tools/list":
      send({ id, result: { tools: TOOLS } });
      return;
    case "tools/call": {
      if (params?.name !== "get_forecast") {
        send({
          id,
          error: { code: -32602, message: `unknown tool '${params?.name}'` },
        });
        return;
      }
      const data = forecast(params.arguments.city);
      send({
        id,
        result: {
          content: [{ type: "text", text: JSON.stringify(data) }],
          structuredContent: data,
        },
      });
      return;
    }
    default:
      send({
        id,
        error: { code: -32601, message: `method '${method}' not found` },
      });
  }
}

const lines = createInterface({ input: process.stdin });
lines.on("line", (line) => {
  if (line.trim() === "") return;
  let request;
  try {
    request = JSON.parse(line);
  } catch {
    return; // a malformed line has no id to answer to
  }
  handle(request);
});
lines.on("close", () => process.exit(0));
