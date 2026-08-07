import { describe, expect, it } from "vitest";

import {
  SURFACE_BRIDGE_PROTOCOL,
  SURFACE_BRIDGE_VERSION,
  errorResponse,
  eventMessage,
  extensionEventMessage,
  initMessage,
  intentIsDeclared,
  okResponse,
  parseAppMessage,
  progressMessage,
  themeMessage,
} from "./surfaceBridgeProtocol";

const RECORD_ID = "123e4567-e89b-12d3-a456-426614174000";

function envelope(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    protocol: SURFACE_BRIDGE_PROTOCOL,
    v: SURFACE_BRIDGE_VERSION,
    type: "ready",
    instanceId: "i-1",
    ...overrides,
  };
}

describe("parseAppMessage", () => {
  it("accepts a well-formed ready message", () => {
    const result = parseAppMessage(envelope({ type: "ready" }));
    expect(result.ok).toBe(true);
    if (result.ok) expect(result.message.type).toBe("ready");
  });

  it("accepts an error message with a string message", () => {
    const result = parseAppMessage(envelope({ type: "error", message: "boom" }));
    expect(result.ok).toBe(true);
    if (result.ok && result.message.type === "error") {
      expect(result.message.message).toBe("boom");
    }
  });

  it("accepts each valid request op", () => {
    const ops = [
      { kind: "invoke", capability: { provider: "a", capability: "c" }, input: {}, data_scope: { kind: "none" }, goal: "g" },
      { kind: "get-config" },
      { kind: "update-config", config: { theme: "dark" } },
      { kind: "get-state", key: "message-1" },
      { kind: "put-state", key: "message-1", expectedRevision: 2, value: { read: true } },
      { kind: "put-state", key: "message-1", expectedRevision: 3, value: null },
      { kind: "data-v1", request: { kind: "get", collection: "items", id: RECORD_ID } },
      { kind: "data-v1", request: { kind: "list", collection: "items", query: { index: "group", equals: "one", limit: 10 } } },
      { kind: "data-v1", request: { kind: "create", collection: "items", value: { title: "One" } } },
      { kind: "data-v1", request: { kind: "replace", collection: "items", id: RECORD_ID, expectedRevision: 1, value: { title: "Two" } } },
      { kind: "data-v1", request: { kind: "delete", collection: "items", id: RECORD_ID, expectedRevision: 2 } },
      { kind: "data-v1", request: { kind: "transaction", operations: [{ kind: "create", collection: "items", value: { title: "One" } }] } },
      { kind: "data-v2", request: { kind: "read-snapshot", expectedGeneration: 0, reads: [{ kind: "record-get", collection: "items", id: RECORD_ID }, { kind: "document-list", collection: "scenes" }] } },
      { kind: "data-v2", request: { kind: "begin-batch", mutationId: "batch-1", expectedGeneration: 0, operations: [], documents: [{ kind: "create", stageId: "scene", collection: "scenes", metadata: { title: "One" }, contentLength: 5, contentSha256: `sha256-${"0".repeat(64)}` }, { kind: "update-metadata", collection: "scenes", id: RECORD_ID, expectedRevision: 1, metadata: { title: "Two" } }] } },
      { kind: "data-v2", request: { kind: "append-batch-operations", mutationId: "append-1", batchId: "batch-1", operations: [{ kind: "create", collection: "items", value: { title: "One" } }] } },
      { kind: "data-v2", request: { kind: "append-document-chunk", mutationId: "chunk-1", batchId: "batch-1", documentId: RECORD_ID, chunkIndex: 0, contentBase64: "aGVsbG8=" } },
      { kind: "list-artifacts" },
      { kind: "list-events" },
    ];
    for (const op of ops) {
      const result = parseAppMessage(envelope({ type: "request", requestId: 1, op }));
      expect(result.ok, JSON.stringify(op)).toBe(true);
    }
  });

  it("rejects malformed or extensible managed-data operations", () => {
    const invalid = [
      { kind: "data-v1", request: { kind: "get", collection: "items", id: RECORD_ID, appId: "other" } },
      { kind: "data-v1", request: { kind: "list", collection: "items", query: { index: "group" } } },
      { kind: "data-v1", request: { kind: "replace", collection: "items", id: RECORD_ID, expectedRevision: 0, value: {} } },
      { kind: "data-v1", request: { kind: "transaction", operations: [] } },
      { kind: "data-v2", request: { kind: "begin-batch", mutation_id: "batch-1", expectedGeneration: 0, operations: [], documents: [] } },
      { kind: "data-v2", request: { kind: "begin-batch", mutationId: "batch-1", expectedGeneration: 0, operations: [], documents: [
        { kind: "create", stageId: "same", collection: "scenes", metadata: {}, contentLength: 0, contentSha256: `sha256-${"0".repeat(64)}` },
        { kind: "create", stageId: "same", collection: "scenes", metadata: {}, contentLength: 0, contentSha256: `sha256-${"0".repeat(64)}` },
      ] } },
      { kind: "data-v2", request: { kind: "append-batch-operations", mutationId: "append-1", batchId: "batch-1", operations: Array.from({ length: 65 }, () => ({ kind: "create", collection: "items", value: { title: "Too many" } })) } },
    ];
    for (const op of invalid) {
      expect(parseAppMessage(envelope({ type: "request", requestId: 1, op })).ok).toBe(false);
    }
  });

  it("rejects non-objects and foreign protocols", () => {
    expect(parseAppMessage(null).ok).toBe(false);
    expect(parseAppMessage("hi").ok).toBe(false);
    expect(parseAppMessage([]).ok).toBe(false);
    expect(parseAppMessage(envelope({ protocol: "other" })).ok).toBe(false);
  });

  it("rejects an unsupported protocol version", () => {
    const result = parseAppMessage(envelope({ v: 999 }));
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toContain("unsupported protocol version");
  });

  it("rejects a missing surface instance id", () => {
    expect(parseAppMessage(envelope({ instanceId: undefined })).ok).toBe(false);
    expect(parseAppMessage(envelope({ instanceId: "" })).ok).toBe(false);
  });

  it("rejects unknown message types", () => {
    expect(parseAppMessage(envelope({ type: "exfiltrate" })).ok).toBe(false);
  });

  it("accepts an extension-state message with an object payload", () => {
    const result = parseAppMessage(
      envelope({
        type: "extension-state",
        payload: { kind: "message-text-marks", ranges: [{ part: 0, start: 0, end: 4 }] },
      }),
    );
    expect(result.ok).toBe(true);
    if (result.ok && result.message.type === "extension-state") {
      expect(result.message.payload).toEqual({
        kind: "message-text-marks",
        ranges: [{ part: 0, start: 0, end: 4 }],
      });
    }
  });

  it("rejects extension-state messages whose payload is not an object", () => {
    expect(parseAppMessage(envelope({ type: "extension-state", payload: "read" })).ok).toBe(false);
    expect(parseAppMessage(envelope({ type: "extension-state", payload: [1] })).ok).toBe(false);
    expect(parseAppMessage(envelope({ type: "extension-state" })).ok).toBe(false);
  });

  it("rejects malformed requests", () => {
    // non-numeric requestId
    expect(
      parseAppMessage(envelope({ type: "request", requestId: "1", op: { kind: "get-config" } })).ok,
    ).toBe(false);
    // unknown op kind
    expect(
      parseAppMessage(envelope({ type: "request", requestId: 1, op: { kind: "read-secret" } })).ok,
    ).toBe(false);
    // invoke with bad capability
    expect(
      parseAppMessage(
        envelope({ type: "request", requestId: 1, op: { kind: "invoke", capability: {}, input: {}, data_scope: { kind: "none" }, goal: "g" } }),
      ).ok,
    ).toBe(false);
    // invoke with non-object input
    expect(
      parseAppMessage(
        envelope({
          type: "request",
          requestId: 1,
          op: { kind: "invoke", capability: { provider: "a", capability: "c" }, input: "x", data_scope: { kind: "none" }, goal: "g" },
        }),
      ).ok,
    ).toBe(false);
    // update-config with non-object config
    expect(
      parseAppMessage(envelope({ type: "request", requestId: 1, op: { kind: "update-config", config: 5 } })).ok,
    ).toBe(false);
    expect(
      parseAppMessage(envelope({ type: "request", requestId: 1, op: { kind: "get-state", key: 5 } })).ok,
    ).toBe(false);
    // all-resources is grant-only; a frame must request exact resources.
    expect(
      parseAppMessage(envelope({
        type: "request",
        requestId: 1,
        op: {
          kind: "invoke",
          capability: { provider: "a", capability: "c" },
          input: {},
          data_scope: { kind: "all-resources" },
          goal: "g",
        },
      })).ok,
    ).toBe(false);
    expect(
      parseAppMessage(envelope({
        type: "request",
        requestId: 1,
        op: { kind: "put-state", key: "message-1", expectedRevision: -1, value: {} },
      })).ok,
    ).toBe(false);
  });

  it("rejects oversized and cyclic messages", () => {
    expect(parseAppMessage(envelope({
      type: "request",
      requestId: 1,
      op: {
        kind: "invoke",
        capability: { provider: "a", capability: "c" },
        input: { text: "x".repeat(1024 * 1024) },
        data_scope: { kind: "none" },
        goal: "g",
      },
    })).ok).toBe(false);
    const cyclic = envelope();
    cyclic.self = cyclic;
    expect(parseAppMessage(cyclic).ok).toBe(false);
  });
});

describe("intentIsDeclared", () => {
  const declared = [
    { provider: "notes", capability: "create" },
    { provider: "weather", capability: "get_forecast" },
  ];
  it("is true only for a declared capability ref", () => {
    expect(intentIsDeclared(declared, { provider: "notes", capability: "create" })).toBe(true);
    expect(intentIsDeclared(declared, { provider: "notes", capability: "delete" })).toBe(false);
    expect(intentIsDeclared(declared, { provider: "secrets", capability: "read" })).toBe(false);
  });
});

describe("message builders", () => {
  it("stamp the current protocol and version", () => {
    for (const message of [
      initMessage({ instanceId: "i", appId: "a", surface: "s", capabilities: [], configSchema: null, config: {}, theme: "dark", variables: {} }),
      okResponse(1, { ok: true }),
      errorResponse(1, "no"),
      eventMessage(),
      progressMessage(1, { content: "delta" }),
      themeMessage("light", {}),
      extensionEventMessage({
        kind: "message-text-selection",
        ranges: [{ part: 2, start: 0, end: 4, text: "read" }],
        marked: true,
      }),
    ]) {
      expect(message.protocol).toBe(SURFACE_BRIDGE_PROTOCOL);
      expect(message.v).toBe(SURFACE_BRIDGE_VERSION);
    }
  });

  it("includes the resolved theme in init and update messages", () => {
    expect(initMessage({
      instanceId: "i",
      appId: "a",
      surface: "s",
      capabilities: [],
      configSchema: null,
      config: {},
      theme: "dark",
      variables: { "--color-text": "#123456" },
    }).theme).toBe("dark");
    expect(themeMessage("light", { "--color-text": "#123456" })).toMatchObject({
      type: "theme",
      theme: "light",
      variables: { "--color-text": "#123456" },
    });
  });

  it("includes bounded host context in init messages", () => {
    expect(initMessage({
      instanceId: "i",
      appId: "a",
      surface: "s",
      capabilities: [],
      configSchema: null,
      config: {},
      theme: "light",
      variables: {},
      hostContext: { kind: "editor", choices: ["one"] },
    }).hostContext).toEqual({ kind: "editor", choices: ["one"] });
  });
});
