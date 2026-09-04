import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("$lib/remotePasskeys", () => ({
  createPasskey: vi.fn().mockResolvedValue({ id: "registered" }),
  getPasskey: vi.fn().mockResolvedValue({ id: "authenticated" }),
}));

import { clearRemoteConnection, listenHostEvent, signInRemoteConnection } from "$lib/hostTransport";

class RecoveryEventSource {
  static current: RecoveryEventSource;
  private handlers = new Map<string, EventListenerOrEventListenerObject[]>();

  constructor() {
    RecoveryEventSource.current = this;
  }

  addEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (listener) this.handlers.set(type, [...(this.handlers.get(type) ?? []), listener]);
  }

  open() {
    const event = new Event("open");
    for (const handler of this.handlers.get("open") ?? []) {
      if (typeof handler === "function") handler(event);
      else handler.handleEvent(event);
    }
  }

  close() {}
}

describe("authoritative approval recovery", () => {
  beforeEach(() => {
    clearRemoteConnection();
    vi.stubGlobal("EventSource", RecoveryEventSource);
    vi.spyOn(console, "error").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterEach(() => {
    clearRemoteConnection();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it.each(["approvals", "events", "restart"] as const)(
    "recovers after %s fails without needing another SSE event",
    async (failure) => {
      let approvalPolls = 0;
      let eventPolls = 0;
      const request = { kind: "grant-issuance", request_id: 77, prompt: { app_id: "notes" } };
      const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
      vi.stubGlobal("fetch", vi.fn(async (input: string | URL | Request) => {
        const url = String(input);
        if (url.endsWith("/api/auth/login/start")) return json({ ceremony_id: "login", options: {} });
        if (url.endsWith("/api/auth/login/finish")) return json({ authenticated: true });
        if (url.endsWith("/api/approvals")) {
          approvalPolls += 1;
          if (failure === "approvals" && approvalPolls === 1) return json({ error: "unavailable" }, 503);
          return json({
            instance_id: failure === "restart" && approvalPolls === 1 ? "old" : "current",
            requests: [request],
          });
        }
        if (url.includes("/api/events?")) {
          eventPolls += 1;
          if (failure === "events" && eventPolls === 1) return json({ error: "unavailable" }, 503);
          return json({ instance_id: "current", oldest_sequence: 0, next_sequence: 0, events: [] });
        }
        throw new Error(`unexpected fetch: ${url}`);
      }));
      await signInRemoteConnection("https://kestral.example");
      const approval = vi.fn();
      const unlisten = await listenHostEvent("trusted-chrome:request", approval);
      try {
        // No wake-up event is sent: reconnect recovery must retry by itself.
        RecoveryEventSource.current.open();
        await vi.waitFor(() => expect(approval).toHaveBeenCalledOnce(), { timeout: 2_000 });
        expect(approval).toHaveBeenCalledWith(request);
        expect(approvalPolls).toBeGreaterThanOrEqual(2);
        expect(eventPolls).toBeGreaterThanOrEqual(2);
      } finally {
        unlisten();
      }
    },
  );

  it("cancels a scheduled recovery when the connection is cleared", async () => {
    vi.useFakeTimers();
    const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
    const fetch = vi.fn(async (input: string | URL | Request) => {
      const url = String(input);
      if (url.endsWith("/api/auth/login/start")) return json({ ceremony_id: "login", options: {} });
      if (url.endsWith("/api/auth/login/finish")) return json({ authenticated: true });
      return json({ error: "unavailable" }, 503);
    });
    vi.stubGlobal("fetch", fetch);
    let unlisten: (() => void) | undefined;
    try {
      await signInRemoteConnection("https://kestral.example");
      unlisten = await listenHostEvent("trusted-chrome:request", vi.fn());
      RecoveryEventSource.current.open();
      await vi.advanceTimersByTimeAsync(0);
      expect(vi.getTimerCount()).toBe(1);
      clearRemoteConnection();
      expect(vi.getTimerCount()).toBe(0);
      const requests = fetch.mock.calls.length;
      await vi.advanceTimersByTimeAsync(4_000);
      expect(fetch).toHaveBeenCalledTimes(requests);
    } finally {
      unlisten?.();
      vi.useRealTimers();
    }
  });
});
