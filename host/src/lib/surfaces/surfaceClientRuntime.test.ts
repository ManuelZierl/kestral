import { describe, expect, it } from "vitest";

import sdk from "../../../surface-runtime/surface-client.js?raw";
import { SURFACE_BRIDGE_PROTOCOL, SURFACE_BRIDGE_VERSION } from "./surfaceBridgeProtocol";

function fromHost(data: unknown): void {
  window.dispatchEvent(new MessageEvent("message", { data, source: window }));
}

function initialize(instanceId = "i-1"): any {
  window.eval(sdk);
  fromHost({
    protocol: SURFACE_BRIDGE_PROTOCOL,
    v: SURFACE_BRIDGE_VERSION,
    type: "init",
    instanceId,
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
  return (window as any).appHost;
}

function nextFrameRequest(): Promise<any> {
  return new Promise((resolve) => {
    const listener = (event: MessageEvent) => {
      if (event.data?.type !== "request") return;
      window.removeEventListener("message", listener);
      resolve(event.data);
    };
    window.addEventListener("message", listener);
  });
}

describe("surface client runtime", () => {
  it("pins the bridge contract and contains no Tauri path", () => {
    expect(sdk).toContain(`PROTOCOL = ${JSON.stringify(SURFACE_BRIDGE_PROTOCOL)}`);
    expect(sdk).toContain(`VERSION = ${SURFACE_BRIDGE_VERSION}`);
    expect(sdk).toContain("window.parent.postMessage");
    expect(sdk).toContain("window.appHost");
    expect(sdk).not.toContain("__TAURI__");
    expect(sdk.toLowerCase()).not.toContain("</script");
  });

  it("applies init and live theme messages", () => {
    const host = initialize();
    expect(host.theme).toBe("light");
    expect(host.hostContext).toEqual({ choices: ["one"] });
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(document.documentElement.style.getPropertyValue("--color-text")).toBe("#123456");

    fromHost({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "theme",
      theme: "dark",
      variables: { "--color-text": "#654321" },
    });
    expect(host.theme).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(document.documentElement.style.getPropertyValue("--color-text")).toBe("#654321");
    expect(document.documentElement.style.getPropertyValue("--app-color-map-line")).toBe("");
  });

  it("ignores initialization that did not come from the parent", () => {
    window.eval(sdk);
    const host = (window as any).appHost;
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
      hostContext: {},
      theme: "dark",
      variables: {},
    } }));
    expect(host.appId).toBeNull();
  });

  it("sends explicit resource scopes through the bridge", async () => {
    const host = initialize("i-scoped");
    const request = nextFrameRequest();
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

  it("keeps managed data on the closed versioned request union", async () => {
    const host = initialize("i-data");
    const request = nextFrameRequest();
    const result = host.data.v2.readSnapshot({
      expectedGeneration: 0,
      reads: [{ kind: "record-get", collection: "items", id: "item-1" }],
    });
    const message = await request;
    expect(message.op).toEqual({
      kind: "data-v2",
      request: {
        kind: "read-snapshot",
        expectedGeneration: 0,
        reads: [{ kind: "record-get", collection: "items", id: "item-1" }],
      },
    });
    fromHost({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "response",
      requestId: message.requestId,
      ok: true,
      result: { generation: 0, results: [] },
    });
    await expect(result).resolves.toEqual({ generation: 0, results: [] });
    expect(host.invokeHost).toBeUndefined();
  });

  it("relays extension events and publishes extension state", async () => {
    const host = initialize("i-extension");
    const received: unknown[] = [];
    host.onExtensionEvent((payload: unknown) => received.push(payload));
    fromHost({
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      type: "extension-event",
      payload: { kind: "selection", ranges: [] },
    });
    expect(received).toEqual([{ kind: "selection", ranges: [] }]);

    const published = new Promise<any>((resolve) => {
      const listener = (event: MessageEvent) => {
        if (event.data?.type !== "extension-state") return;
        window.removeEventListener("message", listener);
        resolve(event.data);
      };
      window.addEventListener("message", listener);
    });
    host.publishExtensionState({ kind: "marks", ranges: [] });
    expect((await published).payload).toEqual({ kind: "marks", ranges: [] });
  });
});
