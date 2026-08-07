import { describe, expect, it, vi } from "vitest";

import type {
  Artifact,
  AppEventView,
  JsonObject,
  SurfaceActionOutcome,
  SurfaceBinding,
} from "$lib/api";
import {
  SURFACE_BRIDGE_PROTOCOL,
  SURFACE_BRIDGE_VERSION,
  type HostToAppMessage,
} from "./surfaceBridgeProtocol";
import {
  createSurfaceBridge,
  trustedSourceGuard,
  type SurfaceBridgeActions,
} from "./hostSurfaceBridge";

const BINDING: SurfaceBinding = { app_id: "weather", surface: "panel", instance_id: "i-1" };
const FRAME = { name: "frame-window" } as unknown as Window;
const RECORD_ID = "123e4567-e89b-12d3-a456-426614174000";

const DECLARED = [{ provider: "weather", capability: "get_forecast" }];

function outcome(): SurfaceActionOutcome {
  return { run_id: "run-1", result: { kind: "completed", result: { ok: true }, artifacts: [] } };
}

function stubActions(overrides: Partial<SurfaceBridgeActions> = {}): SurfaceBridgeActions {
  return {
    invoke: vi.fn(async () => outcome()),
    cancelRun: vi.fn(async () => undefined),
    getConfig: vi.fn(async () => ({ theme: "dark" }) as JsonObject),
    updateConfig: vi.fn(async (config: JsonObject) => config),
    getState: vi.fn(async () => ({ revision: 0, value: null })),
    putState: vi.fn(async (_key, expectedRevision, value) => ({
      revision: expectedRevision + 1,
      value,
    })),
    managedData: vi.fn(async () => ({ id: RECORD_ID })),
    listArtifacts: vi.fn(async () => [] as Artifact[]),
    listEvents: vi.fn(async () => [] as AppEventView[]),
    ...overrides,
  };
}

interface Harness {
  post: ReturnType<typeof vi.fn>;
  rejected: ReturnType<typeof vi.fn>;
  ready: ReturnType<typeof vi.fn>;
  appError: ReturnType<typeof vi.fn>;
  resize: ReturnType<typeof vi.fn>;
  extensionState: ReturnType<typeof vi.fn>;
  actions: SurfaceBridgeActions;
  send(data: unknown, from?: { source?: unknown; origin?: string }): void;
}

function harness(actions = stubActions()): Harness {
  const post = vi.fn<(m: HostToAppMessage) => void>();
  const rejected = vi.fn<(reason: string) => void>();
  const ready = vi.fn();
  const appError = vi.fn<(m: string) => void>();
  const resize = vi.fn<(height: number) => void>();
  const extensionState = vi.fn<(payload: JsonObject) => void>();
  const bridge = createSurfaceBridge({
    binding: BINDING,
    declaredIntents: DECLARED,
    actions,
    post,
    isTrustedSource: (event) => event.source === FRAME && event.origin === "null",
    onReady: ready,
    onAppError: appError,
    onResize: resize,
    onExtensionState: extensionState,
    onRejected: rejected,
  });
  return {
    post,
    rejected,
    ready,
    appError,
    resize,
    extensionState,
    actions,
    send(data, from = {}) {
      const event = {
        data,
        source: from.source ?? FRAME,
        origin: from.origin ?? "null",
      } as unknown as MessageEvent;
      bridge.handleMessage(event);
    },
  };
}

function request(op: unknown, requestId = 1, instanceId = "i-1") {
  return {
    protocol: SURFACE_BRIDGE_PROTOCOL,
    v: SURFACE_BRIDGE_VERSION,
    type: "request",
    instanceId,
    requestId,
    op,
  };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("surface bridge lifecycle", () => {
  it("reports ready and app-side errors", () => {
    const h = harness();
    h.send({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "ready",
      instanceId: "i-1",
    });
    expect(h.ready).toHaveBeenCalledOnce();
    h.send({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "error",
      instanceId: "i-1",
      message: "render failed",
    });
    expect(h.appError).toHaveBeenCalledWith("render failed");
  });

  it("reports a content-height resize", () => {
    const h = harness();
    h.send({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "resize",
      instanceId: "i-1",
      height: 42,
    });
    expect(h.resize).toHaveBeenCalledWith(42);
  });

  it("routes published extension state to the slot owner", () => {
    const h = harness();
    h.send({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "extension-state",
      instanceId: "i-1",
      payload: { kind: "message-text-marks", ranges: [{ part: 0, start: 0, end: 4 }] },
    });
    expect(h.extensionState).toHaveBeenCalledWith({
      kind: "message-text-marks",
      ranges: [{ part: 0, start: 0, end: 4 }],
    });
  });

  it("drops extension state with a non-object payload or foreign instance", () => {
    const h = harness();
    h.send({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "extension-state",
      instanceId: "i-1",
      payload: "read",
    });
    h.send({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "extension-state",
      instanceId: "someone-else",
      payload: {},
    });
    expect(h.extensionState).not.toHaveBeenCalled();
    expect(h.rejected).toHaveBeenCalledTimes(2);
  });

  it("drops a resize carrying an invalid height", () => {
    const h = harness();
    h.send({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "resize",
      instanceId: "i-1",
      height: -5,
    });
    expect(h.resize).not.toHaveBeenCalled();
    expect(h.rejected).toHaveBeenCalled();
  });
});

describe("surface bridge ops", () => {
  it("invokes a declared capability and posts the outcome", async () => {
    const h = harness();
    h.send(
      request({
        kind: "invoke",
        capability: DECLARED[0],
        input: { city: "Berlin" },
        data_scope: { kind: "none" },
        goal: "forecast",
      }),
    );
    await flush();
    expect(h.actions.invoke).toHaveBeenCalledWith({
      capability: DECLARED[0],
      input: { city: "Berlin" },
      data_scope: { kind: "none" },
      goal: "forecast",
    }, expect.any(Function));
    const [message] = h.post.mock.calls.at(-1)!;
    expect(message).toMatchObject({ type: "response", requestId: 1, ok: true });
  });

  it("routes read ops to their actions", async () => {
    const h = harness();
    h.send(request({ kind: "get-config" }, 2));
    h.send(request({ kind: "list-artifacts" }, 3));
    h.send(request({ kind: "list-events" }, 4));
    h.send(request({ kind: "update-config", config: { theme: "light" } }, 5));
    h.send(request({ kind: "get-state", key: "message-1" }, 6));
    h.send(request({
      kind: "put-state",
      key: "message-1",
      expectedRevision: 0,
      value: { read: true },
    }, 7));
    h.send(request({
      kind: "data-v1",
      request: { kind: "get", collection: "items", id: RECORD_ID },
    }, 8));
    h.send(request({
      kind: "data-v2",
      request: {
        kind: "read-snapshot",
        expectedGeneration: 0,
        reads: [{ kind: "record-get", collection: "items", id: RECORD_ID }],
      },
    }, 9));
    await flush();
    expect(h.actions.getConfig).toHaveBeenCalledOnce();
    expect(h.actions.listArtifacts).toHaveBeenCalledOnce();
    expect(h.actions.listEvents).toHaveBeenCalledOnce();
    expect(h.actions.updateConfig).toHaveBeenCalledWith({ theme: "light" });
    expect(h.actions.getState).toHaveBeenCalledWith("message-1");
    expect(h.actions.putState).toHaveBeenCalledWith("message-1", 0, { read: true });
    expect(h.actions.managedData).toHaveBeenCalledWith({
      kind: "get",
      collection: "items",
      id: RECORD_ID,
    });
    expect(h.actions.managedData).toHaveBeenCalledWith({
      contractVersion: 2,
      request: {
        kind: "read-snapshot",
        expectedGeneration: 0,
        reads: [{ kind: "record-get", collection: "items", id: RECORD_ID }],
      },
    });
    const responses = h.post.mock.calls.map(([m]) => m);
    expect(responses.every((m) => m.type === "response" && m.ok)).toBe(true);
  });
});

describe("surface bridge — permission (declared intent)", () => {
  it("refuses to invoke an undeclared capability and never calls the action", async () => {
    const h = harness();
    h.send(
      request({
        kind: "invoke",
        capability: { provider: "secrets", capability: "read" },
        input: {},
        data_scope: { kind: "none" },
        goal: "x",
      }),
    );
    await flush();
    expect(h.actions.invoke).not.toHaveBeenCalled();
    const [message] = h.post.mock.calls.at(-1)!;
    expect(message).toMatchObject({ type: "response", requestId: 1, ok: false });
    if (message.type === "response") expect(message.error).toContain("not declared");
  });
});

describe("surface bridge — spoofing", () => {
  it("drops messages from a foreign window source", async () => {
    const h = harness();
    h.send(request({ kind: "get-config" }), { source: { other: true } });
    await flush();
    expect(h.actions.getConfig).not.toHaveBeenCalled();
    expect(h.post).not.toHaveBeenCalled();
    expect(h.rejected).toHaveBeenCalledWith("untrusted message source");
  });

  it("drops messages from a foreign origin", async () => {
    const h = harness();
    h.send(request({ kind: "get-config" }), { origin: "https://evil.example" });
    await flush();
    expect(h.actions.getConfig).not.toHaveBeenCalled();
    expect(h.rejected).toHaveBeenCalledWith("untrusted message source");
  });

  it("drops messages that spoof a different surface instance id", async () => {
    const h = harness();
    h.send(request({ kind: "get-config" }, 1, "i-OTHER"));
    await flush();
    expect(h.actions.getConfig).not.toHaveBeenCalled();
    expect(h.rejected).toHaveBeenCalledWith("surface instance id mismatch");
  });
});

describe("surface bridge — malformed messages", () => {
  it("drops garbage and rejects malformed requests without leaving the app waiting", async () => {
    const h = harness();
    expect(() => {
      h.send(undefined);
      h.send("not-json");
      h.send({ protocol: "something-else" });
      h.send({
        protocol: SURFACE_BRIDGE_PROTOCOL,
        v: SURFACE_BRIDGE_VERSION + 1,
        type: "ready",
        instanceId: "i-1",
      });
      h.send(request({ kind: "delete-everything" }));
    }).not.toThrow();
    await flush();
    expect(h.post).toHaveBeenCalledTimes(1);
    expect(h.post.mock.calls[0][0]).toMatchObject({
      type: "response",
      requestId: 1,
      ok: false,
    });
    expect(h.rejected).toHaveBeenCalled();
  });
});

describe("surface bridge — host stability", () => {
  it("turns an action failure into an error response, never a throw", async () => {
    const actions = stubActions({
      invoke: vi.fn(async () => {
        throw new Error("kernel busy");
      }),
    });
    const h = harness(actions);
    expect(() =>
      h.send(
        request({
          kind: "invoke",
          capability: DECLARED[0],
          input: {},
          data_scope: { kind: "none" },
          goal: "g",
        }),
      ),
    ).not.toThrow();
    await flush();
    const [message] = h.post.mock.calls.at(-1)!;
    expect(message).toMatchObject({ type: "response", ok: false });
    if (message.type === "response") expect(message.error).toBe("kernel busy");
  });

  it("bounds concurrent requests from a hostile frame", async () => {
    const actions = stubActions({
      getConfig: vi.fn(() => new Promise<JsonObject>(() => {})),
    });
    const h = harness(actions);

    for (let requestId = 1; requestId <= 17; requestId += 1) {
      h.send(request({ kind: "get-config" }, requestId));
    }
    await flush();

    expect(actions.getConfig).toHaveBeenCalledTimes(16);
    expect(h.post.mock.calls.at(-1)?.[0]).toMatchObject({
      type: "response",
      requestId: 17,
      ok: false,
      error: "too many surface requests are pending",
    });
  });
});

describe("trustedSourceGuard", () => {
  const guard = trustedSourceGuard(() => FRAME);

  it("accepts the frame window with an opaque origin", () => {
    expect(guard({ source: FRAME, origin: "null" } as unknown as MessageEvent)).toBe(true);
  });

  it("rejects a different window or any non-opaque origin", () => {
    expect(guard({ source: {}, origin: "null" } as unknown as MessageEvent)).toBe(false);
    expect(guard({ source: FRAME, origin: "" } as unknown as MessageEvent)).toBe(false);
    expect(guard({ source: FRAME, origin: "http://localhost:1420" } as unknown as MessageEvent)).toBe(false);
    expect(guard({ source: FRAME, origin: "https://evil.example" } as unknown as MessageEvent)).toBe(false);
  });

  it("rejects when the frame is gone", () => {
    const goneGuard = trustedSourceGuard(() => null);
    expect(goneGuard({ source: FRAME, origin: "null" } as unknown as MessageEvent)).toBe(false);
  });
});
