import { describe, expect, it } from "vitest";

import type { SurfaceUiBundle } from "$lib/api";
import { buildClientSdk, buildSurfaceSrcdoc } from "./surfaceClientSdk";
import { SURFACE_BRIDGE_PROTOCOL, SURFACE_BRIDGE_VERSION } from "./surfaceBridgeProtocol";

const RECORD_ID = "123e4567-e89b-12d3-a456-426614174000";

// Delivers a host-to-frame message the way a real frame receives it: in jsdom
// window.parent === window, so `source: window` is exactly what the SDK's
// parent-source guard accepts inside a real sandboxed iframe.
function fromHost(data: unknown): void {
  window.dispatchEvent(new MessageEvent("message", { data, source: window }));
}

function bundle(overrides: Partial<SurfaceUiBundle> = {}): SurfaceUiBundle {
  return {
    protocol_version: SURFACE_BRIDGE_VERSION,
    html: "<!doctype html><html><head><title>App</title></head><body><p id=\"marker\">hi</p></body></html>",
    csp: "default-src 'none'; connect-src 'none'; base-uri 'none'",
    ...overrides,
  };
}

describe("buildClientSdk", () => {
  it("pins the protocol and version so frame and host cannot drift", () => {
    const sdk = buildClientSdk();
    expect(sdk).toContain(JSON.stringify(SURFACE_BRIDGE_PROTOCOL));
    expect(sdk).toContain(`VERSION=${SURFACE_BRIDGE_VERSION}`);
    // The SDK talks to the host only via the parent — never Tauri or kernel.
    expect(sdk).toContain("window.parent.postMessage");
    expect(sdk).not.toContain("__TAURI__");
    expect(sdk).toContain("window.appHost");
  });

  it("applies init and live theme messages to the sandbox document", () => {
    window.eval(buildClientSdk());
    const host = (window as any).appHost;
    fromHost({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "init",
      instanceId: "i-1",
      appId: "weather",
      surface: "panel",
      capabilities: [],
      configSchema: null,
      config: {},
      extensionContext: {},
      hostContext: { choices: ["one"] },
      theme: "light",
      variables: { "--color-text": "#123456", "--app-color-map-line": "#abcdef" },
    });
    expect(host.theme).toBe("light");
    expect(host.hostContext).toEqual({ choices: ["one"] });
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(document.documentElement.style.colorScheme).toBe("light");
    expect(document.documentElement.style.getPropertyValue("--color-text")).toBe("#123456");
    expect(document.documentElement.style.getPropertyValue("--app-color-map-line")).toBe("#abcdef");

    fromHost({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "theme",
      theme: "dark",
      variables: { "--color-text": "#654321" },
    });
    expect(host.theme).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(document.documentElement.style.colorScheme).toBe("dark");
    expect(host.variables).toEqual({ "--color-text": "#654321" });
    expect(document.documentElement.style.getPropertyValue("--color-text")).toBe("#654321");
    expect(document.documentElement.style.getPropertyValue("--app-color-map-line")).toBe("");
  });

  it("ignores messages that do not come from the parent window", () => {
    window.eval(buildClientSdk());
    const host = (window as any).appHost;
    // A forged message (sibling frame / self-dispatch) has a source other
    // than window.parent — or none, like this synthetic event.
    window.dispatchEvent(new MessageEvent("message", { data: {
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "init",
      instanceId: "forged",
      appId: "forged",
      surface: "panel",
      capabilities: [],
      configSchema: null,
      config: {},
      extensionContext: {},
      theme: "dark",
      variables: {},
    } }));
    expect(host.appId).toBeNull();
    expect(host.theme).toBeNull();
  });

  it("sends an explicit resource scope through the declared-intent bridge", async () => {
    window.eval(buildClientSdk());
    const host = (window as any).appHost;
    fromHost({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "init",
      instanceId: "i-scoped",
      appId: "com.example.export",
      surface: "thread-export",
      capabilities: [{ provider: "chat", capability: "chat.read_thread" }],
      configSchema: null,
      config: {},
      extensionContext: {},
      hostContext: {},
      theme: "light",
      variables: {},
    });
    const request = new Promise<any>((resolve) => {
      const listener = (event: MessageEvent) => {
        if (event.data?.type === "request") {
          window.removeEventListener("message", listener);
          resolve(event.data);
        }
      };
      window.addEventListener("message", listener);
    });

    void host.invokeScoped(
      { provider: "chat", capability: "chat.read_thread" },
      { resource_id: "chat-thread-1" },
      { kind: "resources", resource_ids: ["chat-thread-1"] },
      "Export this conversation",
    );

    expect((await request).op).toEqual({
      kind: "invoke",
      capability: { provider: "chat", capability: "chat.read_thread" },
      input: { resource_id: "chat-thread-1" },
      data_scope: { kind: "resources", resource_ids: ["chat-thread-1"] },
      goal: "Export this conversation",
    });
  });

  it("exposes managed data through versioned request unions without host handles", async () => {
    window.eval(buildClientSdk());
    const host = (window as any).appHost;
    fromHost({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "init",
      instanceId: "i-data",
      appId: "com.example.data",
      surface: "panel",
      capabilities: [],
      configSchema: null,
      config: {},
      extensionContext: {},
      hostContext: {},
      theme: "light",
      variables: {},
    });
    const request = new Promise<any>((resolve) => {
      const listener = (event: MessageEvent) => {
        if (event.data?.type === "request") {
          window.removeEventListener("message", listener);
          resolve(event.data);
        }
      };
      window.addEventListener("message", listener);
    });

    void host.data.v1.replace("items", RECORD_ID, 3, { title: "Updated" });

    expect((await request).op).toEqual({
      kind: "data-v1",
      request: {
        kind: "replace",
        collection: "items",
        id: RECORD_ID,
        expectedRevision: 3,
        value: { title: "Updated" },
      },
    });
    expect(host.data.v2).toBeDefined();
    const v2Request = new Promise<any>((resolve) => {
      const listener = (event: MessageEvent) => {
        if (event.data?.type === "request") {
          window.removeEventListener("message", listener);
          resolve(event.data);
        }
      };
      window.addEventListener("message", listener);
    });
    const v2Result = host.data.v2.readSnapshot({
      expectedGeneration: 0,
      reads: [{ kind: "record-get", collection: "items", id: "00000000-0000-4000-8000-000000000001" }],
    });
    const v2Message = await v2Request;
    expect(v2Message.op).toEqual({
      kind: "data-v2",
      request: {
        kind: "read-snapshot",
        expectedGeneration: 0,
        reads: [{ kind: "record-get", collection: "items", id: "00000000-0000-4000-8000-000000000001" }],
      },
    });
    fromHost({ protocol: SURFACE_BRIDGE_PROTOCOL, v: SURFACE_BRIDGE_VERSION, type: "response", requestId: v2Message.requestId, ok: true, result: { generation: 0, results: [{ kind: "record-get", record: null }] } });
    await expect(v2Result).resolves.toEqual({ generation: 0, results: [{ kind: "record-get", record: null }] });

    const nextRequest = () => new Promise<any>((resolve) => {
      const listener = (event: MessageEvent) => {
        if (event.data?.type === "request") {
          window.removeEventListener("message", listener);
          resolve(event.data);
        }
      };
      window.addEventListener("message", listener);
    });
    const beginResult = host.data.v2.beginBatch({
      expectedGeneration: 0,
      mutationId: "batch-1",
      operations: [],
      documents: [{
        kind: "create",
        stageId: "scene",
        collection: "scenes",
        metadata: { title: "Board" },
        contentLength: 5,
        contentSha256: `sha256-${"0".repeat(64)}`,
      }, {
        kind: "update-metadata",
        collection: "scenes",
        id: RECORD_ID,
        expectedRevision: 1,
        metadata: { title: "Updated" },
      }],
    });
    const beginMessage = await nextRequest();
    expect(beginMessage.op).toEqual({
      kind: "data-v2",
      request: {
        kind: "begin-batch",
        expectedGeneration: 0,
        mutationId: "batch-1",
        operations: [],
        documents: [{
          kind: "create",
          stageId: "scene",
          collection: "scenes",
          metadata: { title: "Board" },
          contentLength: 5,
          contentSha256: `sha256-${"0".repeat(64)}`,
        }, {
          kind: "update-metadata",
          collection: "scenes",
          id: RECORD_ID,
          expectedRevision: 1,
          metadata: { title: "Updated" },
        }],
      },
    });
    fromHost({ protocol: SURFACE_BRIDGE_PROTOCOL, v: SURFACE_BRIDGE_VERSION, type: "response", requestId: beginMessage.requestId, ok: true, result: { batchId: "batch-1", generation: 0, documents: [{ stageId: "scene", documentId: RECORD_ID }] } });
    await expect(beginResult).resolves.toEqual({ batchId: "batch-1", generation: 0, documents: [{ stageId: "scene", documentId: RECORD_ID }] });

    const appendOperationsResult = host.data.v2.appendBatchOperations({
      mutationId: "append-1",
      batchId: "batch-1",
      operations: [{ kind: "create", collection: "items", value: { title: "One" } }],
    });
    const appendOperationsMessage = await nextRequest();
    expect(appendOperationsMessage.op).toEqual({ kind: "data-v2", request: { kind: "append-batch-operations", mutationId: "append-1", batchId: "batch-1", operations: [{ kind: "create", collection: "items", value: { title: "One" } }] } });
    fromHost({ protocol: SURFACE_BRIDGE_PROTOCOL, v: SURFACE_BRIDGE_VERSION, type: "response", requestId: appendOperationsMessage.requestId, ok: true, result: { batchId: "batch-1", appended: 1 } });
    await appendOperationsResult;

    const appendResult = host.data.v2.appendDocumentChunk({ mutationId: "chunk-1", batchId: "batch-1", documentId: RECORD_ID, chunkIndex: 0, contentBase64: "aGVsbG8=" });
    const appendMessage = await nextRequest();
    expect(appendMessage.op).toEqual({ kind: "data-v2", request: { kind: "append-document-chunk", mutationId: "chunk-1", batchId: "batch-1", documentId: RECORD_ID, chunkIndex: 0, contentBase64: "aGVsbG8=" } });
    fromHost({ protocol: SURFACE_BRIDGE_PROTOCOL, v: SURFACE_BRIDGE_VERSION, type: "response", requestId: appendMessage.requestId, ok: true, result: { batchId: "batch-1", documentId: RECORD_ID, chunkIndex: 0 } });
    await appendResult;

    const commitResult = host.data.v2.commitBatch({ mutationId: "commit-1", batchId: "batch-1" });
    const commitMessage = await nextRequest();
    expect(commitMessage.op).toEqual({ kind: "data-v2", request: { kind: "commit-batch", mutationId: "commit-1", batchId: "batch-1" } });
    fromHost({ protocol: SURFACE_BRIDGE_PROTOCOL, v: SURFACE_BRIDGE_VERSION, type: "response", requestId: commitMessage.requestId, ok: true, result: { generation: 1, records: [], documents: [] } });
    await commitResult;
    expect(host.invokeHost).toBeUndefined();
  });
  it("relays extension events to the app and publishes extension state to the host", async () => {
    window.eval(buildClientSdk());
    const host = (window as any).appHost;

    const received: unknown[] = [];
    host.onExtensionEvent((payload: unknown) => received.push(payload));

    fromHost({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "init",
      instanceId: "i-ext",
      appId: "org.example.annotator",
      surface: "message-reading-mark",
      capabilities: [],
      configSchema: null,
      config: {},
      extensionContext: { part_count: 3 },
      theme: "light",
      variables: {},
    });

    fromHost({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "extension-event",
      payload: {
        kind: "message-text-selection",
        ranges: [{ part: 1, start: 0, end: 4, text: "read" }],
        marked: true,
      },
    });
    expect(received).toEqual([{
      kind: "message-text-selection",
      ranges: [{ part: 1, start: 0, end: 4, text: "read" }],
      marked: true,
    }]);

    // In jsdom window.parent === window, so the SDK's postMessage lands back
    // on this window; capture the published state from there.
    const published = new Promise<any>((resolve) => {
      const listener = (event: MessageEvent) => {
        if (event.data?.type === "extension-state") {
          window.removeEventListener("message", listener);
          resolve(event.data);
        }
      };
      window.addEventListener("message", listener);
    });
    host.publishExtensionState({
      kind: "message-text-marks",
      ranges: [{ part: 1, start: 0, end: 4 }],
    });
    const state = await published;
    expect(state.instanceId).toBe("i-ext");
    expect(state.v).toBe(SURFACE_BRIDGE_VERSION);
    expect(state.payload).toEqual({
      kind: "message-text-marks",
      ranges: [{ part: 1, start: 0, end: 4 }],
    });
  });
});

describe("buildSurfaceSrcdoc", () => {
  it("injects the bundle CSP as the first thing in <head>", () => {
    const doc = buildSurfaceSrcdoc(bundle());
    expect(doc).toContain('http-equiv="Content-Security-Policy"');
    // Single quotes stay literal inside the double-quoted attribute.
    expect(doc).toContain("default-src 'none'");
    // CSP meta precedes the app's own <title> content in the head.
    const cspAt = doc.indexOf("Content-Security-Policy");
    const titleAt = doc.indexOf("<title>");
    expect(cspAt).toBeGreaterThan(-1);
    expect(cspAt).toBeLessThan(titleAt);
  });

  it("injects the bridge SDK and preserves the app's own markup", () => {
    const doc = buildSurfaceSrcdoc(bundle());
    expect(doc).toContain("window.appHost");
    expect(doc).toContain('id="marker"');
  });

  it("omits frame-ancestors because browsers ignore it in a meta CSP", () => {
    const doc = buildSurfaceSrcdoc(bundle({ csp: "default-src 'none'; frame-ancestors 'none'; connect-src 'none'" }));
    expect(doc).not.toContain("frame-ancestors");
    expect(doc).toContain("connect-src 'none'");
  });

  it("wraps a bare fragment in a document scaffold it controls", () => {
    const doc = buildSurfaceSrcdoc(bundle({ html: "<p>fragment</p>" }));
    expect(doc.startsWith("<!doctype html>")).toBe(true);
    expect(doc).toContain("Content-Security-Policy");
    expect(doc).toContain("window.appHost");
    expect(doc).toContain("<p>fragment</p>");
  });

  it("escapes the CSP so a hostile bundle cannot break out of the attribute", () => {
    const doc = buildSurfaceSrcdoc(bundle({ csp: 'x"><script>alert(1)</script>' }));
    expect(doc).not.toContain('"><script>alert(1)');
    expect(doc).toContain("&quot;&gt;&lt;script&gt;");
  });
});
