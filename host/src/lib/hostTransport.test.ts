import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("$lib/remotePasskeys", () => ({
  createPasskey: vi.fn().mockResolvedValue({ id: "registered" }),
  getPasskey: vi.fn().mockResolvedValue({ id: "authenticated" }),
}));

import {
  clearRemoteConnection,
  invokeHost,
  invokeHostWithProgress,
  isRemoteConnectionReady,
  listenHostEvent,
  needsRemoteConnection,
  openExternalUrl,
  pairRemoteConnection,
  remoteConnectionAuthenticated,
  remoteUrlDefault,
  restoreRemoteConnection,
  signInRemoteConnection,
  signOutRemoteConnection,
} from "$lib/hostTransport";
import { get } from "svelte/store";

class FakeEventSource {
  static instances: FakeEventSource[] = [];
  readonly listeners = new Map<string, Array<(event: Event) => void>>();
  closed = false;

  constructor(
    readonly url: string,
    readonly options?: EventSourceInit,
  ) {
    FakeEventSource.instances.push(this);
    queueMicrotask(() => this.emit("open", new Event("open")));
  }

  addEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (!listener) return;
    const callback = typeof listener === "function" ? listener : (event: Event) => listener.handleEvent(event);
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), callback]);
  }

  emit(type: string, event: Event) {
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }

  emitBatch(batch: unknown) {
    this.emit("remote-events", new MessageEvent("remote-events", { data: JSON.stringify(batch) }));
  }

  close() {
    this.closed = true;
  }
}

describe("remote owner session", () => {
  beforeEach(() => {
    sessionStorage.clear();
    clearRemoteConnection();
    FakeEventSource.instances = [];
    vi.stubGlobal("EventSource", FakeEventSource);
  });
  afterEach(() => vi.unstubAllGlobals());

  it("clears the host URL", () => {
    sessionStorage.setItem("host.remote.url", "https://kestral.example");

    clearRemoteConnection();

    expect(sessionStorage.getItem("host.remote.url")).toBeNull();
    expect(needsRemoteConnection()).toBe(true);
  });

  it("defaults browser host mode to the page origin", () => {
    expect(remoteUrlDefault()).toBe(window.location.origin);
  });

  it("opens remote OAuth URLs with the browser instead of a Tauri plugin", async () => {
    sessionStorage.setItem("host.remote.url", "https://kestral.example");
    const open = vi.fn().mockReturnValue({});
    vi.stubGlobal("open", open);

    await openExternalUrl("https://login.example/oauth");

    expect(open).toHaveBeenCalledWith(
      "https://login.example/oauth",
      "_blank",
      "noopener,noreferrer",
    );
  });

  it("rejects non-HTTP sign-in URLs", async () => {
    sessionStorage.setItem("host.remote.url", "https://kestral.example");
    const open = vi.fn();
    vi.stubGlobal("open", open);

    for (const url of ["javascript:alert(1)", "data:text/html,test", "file:///tmp/token"]) {
      await expect(openExternalUrl(url)).rejects.toThrow("HTTP or HTTPS");
    }
    expect(open).not.toHaveBeenCalled();
  });

  it("restores an HttpOnly-cookie session without a browser-readable credential", async () => {
    sessionStorage.setItem("host.remote.url", "https://kestral.example");
    const fetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ paired: true, authenticated: true }), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetch);

    await expect(restoreRemoteConnection()).resolves.toBe(true);

    expect(isRemoteConnectionReady()).toBe(true);
    expect(fetch).toHaveBeenCalledWith("https://kestral.example/api/auth/status", {
      credentials: "include",
      headers: {},
    });
  });

  it("signs in with a passkey and sends cookie-authenticated commands", async () => {
    const fetch = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ ceremony_id: "login-1", options: {} }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ authenticated: true }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify([]), { status: 200 }));
    vi.stubGlobal("fetch", fetch);

    await signInRemoteConnection("https://kestral.example/");
    await invokeHost("list_apps");

    expect(isRemoteConnectionReady()).toBe(true);
    expect(fetch).toHaveBeenNthCalledWith(1, "https://kestral.example/api/auth/login/start", {
      method: "POST",
      body: "{}",
      credentials: "include",
      headers: { "Content-Type": "application/json" },
    });
    expect(fetch).toHaveBeenNthCalledWith(3, "https://kestral.example/api/invoke/list_apps", {
      method: "POST",
      body: "{}",
      credentials: "include",
      headers: { "Content-Type": "application/json" },
    });
  });

  it("publishes authentication loss after a command returns 401", async () => {
    const fetch = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ ceremony_id: "login-1", options: {} }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ authenticated: true }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ error: "expired" }), { status: 401 }));
    vi.stubGlobal("fetch", fetch);
    await signInRemoteConnection("https://kestral.example");
    expect(get(remoteConnectionAuthenticated)).toBe(true);

    await expect(invokeHost("list_apps")).rejects.toThrow("expired");

    expect(get(remoteConnectionAuthenticated)).toBe(false);
  });

  it("opens one credentialed event stream instead of polling continuously", async () => {
    const fetch = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ ceremony_id: "login-1", options: {} }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ authenticated: true }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ instance_id: "instance-1", requests: [] }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({
        instance_id: "instance-1",
        oldest_sequence: 0,
        next_sequence: 0,
        events: [],
      }), { status: 200 }));
    vi.stubGlobal("fetch", fetch);
    await signInRemoteConnection("https://kestral.example");

    const unlistenFirst = await listenHostEvent("trusted-chrome:request", vi.fn());
    const unlistenSecond = await listenHostEvent("trusted-chrome:notice", vi.fn());
    await vi.waitFor(() => expect(fetch).toHaveBeenCalledTimes(4));

    expect(FakeEventSource.instances).toHaveLength(1);
    expect(FakeEventSource.instances[0].url).toBe("https://kestral.example/api/events/stream?after=0");
    expect(FakeEventSource.instances[0].options).toEqual({ withCredentials: true });

    unlistenFirst();
    expect(FakeEventSource.instances[0].closed).toBe(false);
    unlistenSecond();
    expect(FakeEventSource.instances[0].closed).toBe(true);
  });

  it("does not redeliver an event seen by both recovery and SSE", async () => {
    const batch = {
      instance_id: "instance-1",
      oldest_sequence: 0,
      next_sequence: 1,
      events: [{
        sequence: 0,
        event: "trusted-chrome:notice",
        payload: { sequence: 7, message: "Saved" },
      }],
    };
    const fetch = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ ceremony_id: "login-1", options: {} }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ authenticated: true }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ instance_id: "instance-1", requests: [] }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify(batch), { status: 200 }));
    vi.stubGlobal("fetch", fetch);
    await signInRemoteConnection("https://kestral.example");
    const notice = vi.fn();
    const unlisten = await listenHostEvent("trusted-chrome:notice", notice);
    await vi.waitFor(() => expect(notice).toHaveBeenCalledOnce());

    FakeEventSource.instances[0].emitBatch(batch);
    await Promise.resolve();

    expect(notice).toHaveBeenCalledOnce();
    unlisten();
  });

  it("drains request-correlated progress when a remote command fails", async () => {
    let eventDelivered = false;
    let commandFinished = false;
    let resolveInitialPoll!: () => void;
    const initialPoll = new Promise<void>((resolve) => {
      resolveInitialPoll = resolve;
    });
    const fetch = vi.fn(async (input: string | URL | Request) => {
      const url = String(input);
      if (url.endsWith("/api/auth/login/start")) {
        return new Response(JSON.stringify({ ceremony_id: "login-1", options: {} }), { status: 200 });
      }
      if (url.endsWith("/api/auth/login/finish")) {
        return new Response(JSON.stringify({ authenticated: true }), { status: 200 });
      }
      if (url.endsWith("/api/approvals")) {
        return new Response(JSON.stringify({ instance_id: "instance-1", requests: [] }), { status: 200 });
      }
      if (url.includes("/api/events?after=0") && !commandFinished) {
        resolveInitialPoll();
        return new Response(JSON.stringify({
          instance_id: "instance-1",
          oldest_sequence: 0,
          next_sequence: 0,
          events: [],
        }), { status: 200 });
      }
      if (url.includes("/api/events?after=0") && !eventDelivered) {
        eventDelivered = true;
        return new Response(JSON.stringify({
          instance_id: "instance-1",
          oldest_sequence: 0,
          next_sequence: 1,
          events: [{
            sequence: 0,
            event: "host-progress:submit_action_with_progress:00000000-0000-4000-8000-000000000001",
            payload: { kind: "status", message: "Working" },
          }],
        }), { status: 200 });
      }
      if (url.includes("/api/events")) {
        return new Response(JSON.stringify({
          instance_id: "instance-1",
          oldest_sequence: 1,
          next_sequence: 1,
          events: [],
        }), { status: 200 });
      }
      if (url.endsWith("/api/invoke/submit_action_with_progress")) {
        await initialPoll;
        commandFinished = true;
        return new Response(JSON.stringify({ error: "action failed" }), { status: 400 });
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal("fetch", fetch);
    vi.spyOn(crypto, "randomUUID").mockReturnValue("00000000-0000-4000-8000-000000000001");
    await signInRemoteConnection("https://kestral.example");
    const onProgress = vi.fn();

    await expect(invokeHostWithProgress(
      "submit_action_with_progress",
      { binding: {}, intent: {} },
      onProgress,
    )).rejects.toThrow("action failed");

    expect(onProgress).toHaveBeenCalledWith({ kind: "status", message: "Working" });
    expect(fetch).toHaveBeenCalledWith(
      "https://kestral.example/api/invoke/submit_action_with_progress",
      expect.objectContaining({
        body: JSON.stringify({
          binding: {},
          intent: {},
          requestId: "00000000-0000-4000-8000-000000000001",
        }),
      }),
    );
  });

  it("replays a reused approval id after the backend restarts at the same event cursor", async () => {
    let instanceId = "instance-old";
    const request = () => ({
      kind: "grant-issuance",
      request_id: 0,
      prompt: { app_id: instanceId },
    });
    const fetch = vi.fn(async (input: string | URL | Request) => {
      const url = String(input);
      if (url.endsWith("/api/auth/login/start")) {
        return new Response(JSON.stringify({ ceremony_id: "login-1", options: {} }), { status: 200 });
      }
      if (url.endsWith("/api/auth/login/finish")) {
        return new Response(JSON.stringify({ authenticated: true }), { status: 200 });
      }
      if (url.endsWith("/api/approvals")) {
        return new Response(JSON.stringify({ instance_id: instanceId, requests: [request()] }), { status: 200 });
      }
      if (url.includes("/api/events")) {
        return new Response(JSON.stringify({
          instance_id: instanceId,
          oldest_sequence: 0,
          next_sequence: 1,
          events: url.includes("after=0")
            ? [{ sequence: 0, event: "trusted-chrome:request", payload: request() }]
            : [],
        }), { status: 200 });
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal("fetch", fetch);
    await signInRemoteConnection("https://kestral.example");
    const approvals: Array<{ prompt: { app_id: string } }> = [];
    const unlisten = await listenHostEvent<typeof approvals[number]>(
      "trusted-chrome:request",
      (approval) => approvals.push(approval),
    );

    try {
      await vi.waitFor(() => expect(approvals.map((item) => item.prompt.app_id)).toEqual(["instance-old"]));
      instanceId = "instance-new";
      FakeEventSource.instances[0].emitBatch({
        instance_id: "instance-new",
        oldest_sequence: 0,
        next_sequence: 1,
        events: [],
      });
      await vi.waitFor(
        () => expect(approvals.map((item) => item.prompt.app_id)).toEqual(["instance-old", "instance-new"]),
        { timeout: 1000 },
      );
    } finally {
      unlisten();
    }
  });

  it("discards an in-flight approval poll after the remote connection is cleared", async () => {
    let finishApprovals!: (response: Response) => void;
    const approvalsResponse = new Promise<Response>((resolve) => { finishApprovals = resolve; });
    const fetch = vi.fn(async (input: string | URL | Request) => {
      const url = String(input);
      if (url.endsWith("/api/auth/login/start")) {
        return new Response(JSON.stringify({ ceremony_id: "login-1", options: {} }), { status: 200 });
      }
      if (url.endsWith("/api/auth/login/finish")) {
        return new Response(JSON.stringify({ authenticated: true }), { status: 200 });
      }
      if (url.endsWith("/api/approvals")) return approvalsResponse;
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal("fetch", fetch);
    await signInRemoteConnection("https://kestral.example");
    const approval = vi.fn();
    const unlisten = await listenHostEvent("trusted-chrome:request", approval);

    try {
      await vi.waitFor(() => expect(fetch).toHaveBeenCalledWith(
        "https://kestral.example/api/approvals",
        expect.anything(),
      ));
      clearRemoteConnection();
      finishApprovals(new Response(JSON.stringify({
        instance_id: "instance-old",
        requests: [{ kind: "grant-issuance", request_id: 0, prompt: {} }],
      }), { status: 200 }));
      await Promise.resolve();
      await Promise.resolve();

      expect(approval).not.toHaveBeenCalled();
    } finally {
      unlisten();
    }
  });

  it("discards approvals when recovery endpoints straddle a backend restart", async () => {
    const fetch = vi.fn(async (input: string | URL | Request) => {
      const url = String(input);
      if (url.endsWith("/api/auth/login/start")) {
        return new Response(JSON.stringify({ ceremony_id: "login-1", options: {} }), { status: 200 });
      }
      if (url.endsWith("/api/auth/login/finish")) {
        return new Response(JSON.stringify({ authenticated: true }), { status: 200 });
      }
      if (url.endsWith("/api/approvals")) {
        return new Response(JSON.stringify({
          instance_id: "instance-old",
          requests: [{ kind: "grant-issuance", request_id: 0, prompt: {} }],
        }), { status: 200 });
      }
      if (url.includes("/api/events")) {
        return new Response(JSON.stringify({
          instance_id: "instance-new",
          oldest_sequence: 0,
          next_sequence: 0,
          events: [],
        }), { status: 200 });
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal("fetch", fetch);
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    await signInRemoteConnection("https://kestral.example");
    const approval = vi.fn();
    const unlisten = await listenHostEvent("trusted-chrome:request", approval);

    try {
      await vi.waitFor(() => expect(fetch).toHaveBeenCalledWith(
        expect.stringContaining("/api/events"),
        expect.anything(),
      ));
      await Promise.resolve();
      expect(approval).not.toHaveBeenCalled();
    } finally {
      unlisten();
      warn.mockRestore();
    }
  });

  it("clears the local connection even when server logout fails", async () => {
    sessionStorage.setItem("host.remote.url", "https://kestral.example");
    const fetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: "backend unavailable" }), { status: 503 }),
    );
    vi.stubGlobal("fetch", fetch);

    await expect(signOutRemoteConnection()).rejects.toThrow("backend unavailable");

    expect(sessionStorage.getItem("host.remote.url")).toBeNull();
    expect(isRemoteConnectionReady()).toBe(false);
  });

  it("consumes a one-time host code only for passkey registration", async () => {
    const fetch = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ ceremony_id: "register-1", options: {} }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ authenticated: true }), { status: 200 }));
    vi.stubGlobal("fetch", fetch);

    await pairRemoteConnection("https://kestral.example", "one-time-code");

    expect(fetch).toHaveBeenNthCalledWith(1, "https://kestral.example/api/auth/register/start", {
      method: "POST",
      body: JSON.stringify({ pairing_code: "one-time-code" }),
      credentials: "include",
      headers: { "Content-Type": "application/json" },
    });
  });

  it("revokes the server session when signing out", async () => {
    sessionStorage.setItem("host.remote.url", "https://kestral.example");
    const fetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ authenticated: false }), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetch);

    await signOutRemoteConnection();

    expect(fetch).toHaveBeenCalledWith("https://kestral.example/api/auth/logout", {
      method: "POST",
      body: "{}",
      credentials: "include",
      headers: { "Content-Type": "application/json" },
    });
    expect(sessionStorage.getItem("host.remote.url")).toBeNull();
  });
});
