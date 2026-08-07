import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";

const eventHandlers = vi.hoisted(() => new Map<string, (payload: unknown) => void>());

vi.mock("$lib/api", () => ({
  bootstrapStartupApps: vi.fn(),
  ledgerRecords: vi.fn(async () => []),
}));
vi.mock("$lib/stores/chatThreads", () => ({
  ensureChatThread: vi.fn(async () => {}),
  refreshChatThreads: vi.fn(async () => {}),
}));
vi.mock("$lib/stores/apps", () => ({ refreshApps: vi.fn(async () => {}) }));
vi.mock("$lib/stores/artifacts", () => ({ refreshArtifacts: vi.fn(async () => {}) }));
vi.mock("$lib/stores/config", () => ({ refreshConfig: vi.fn(async () => {}) }));
vi.mock("$lib/stores/grants", () => ({ refreshGrants: vi.fn(async () => {}) }));
vi.mock("$lib/hostTransport", () => ({
  listenHostEvent: vi.fn(async (event: string, handler: (payload: unknown) => void) => {
    eventHandlers.set(event, handler);
    return () => eventHandlers.delete(event);
  }),
}));

import { bootstrapStartupApps, ledgerRecords } from "$lib/api";
import { refreshApps } from "$lib/stores/apps";
import { refreshArtifacts } from "$lib/stores/artifacts";
import { refreshGrants } from "$lib/stores/grants";
import {
  bootstrapFailed,
  hostInitialized,
  initializeHost,
  refreshHost,
  shellError,
  startPolling,
  stopPolling,
} from "$lib/stores/hostState";

describe("initializeHost", () => {
  beforeEach(() => {
    stopPolling();
    eventHandlers.clear();
    vi.clearAllMocks();
    vi.spyOn(console, "error").mockImplementation(() => {});
  });
  afterEach(() => stopPolling());

  it("latches the failure, offers a retry, and recovers when the retry succeeds", async () => {
    vi.mocked(bootstrapStartupApps).mockRejectedValueOnce(new Error("kernel busy at startup"));

    await initializeHost();

    expect(get(bootstrapFailed)).toBe(true);
    expect(get(hostInitialized)).toBe(false);
    expect(get(shellError)).toContain("Bootstrap failed");

    vi.mocked(bootstrapStartupApps).mockResolvedValueOnce(undefined);

    await initializeHost();

    expect(get(bootstrapFailed)).toBe(false);
    expect(get(hostInitialized)).toBe(true);
    expect(get(shellError)).toBeNull();
  });

  it("shares an in-flight host refresh instead of starting overlapping poll batches", async () => {
    let finishApps!: () => void;
    vi.mocked(refreshApps).mockReturnValueOnce(new Promise<void>((resolve) => { finishApps = resolve; }));

    const first = refreshHost();
    const second = refreshHost();

    expect(first).toBe(second);
    expect(refreshApps).toHaveBeenCalledOnce();
    finishApps();
    await first;
  });

  it("sequences kernel-backed snapshots within one refresh", async () => {
    let finishApps!: () => void;
    vi.mocked(refreshApps).mockReturnValueOnce(new Promise<void>((resolve) => { finishApps = resolve; }));

    const refresh = refreshHost({ includeRecords: false });

    expect(refreshApps).toHaveBeenCalledOnce();
    expect(refreshArtifacts).not.toHaveBeenCalled();
    expect(refreshGrants).not.toHaveBeenCalled();

    finishApps();
    await refresh;

    expect(refreshArtifacts).toHaveBeenCalledOnce();
    expect(refreshGrants).toHaveBeenCalledOnce();
    expect(ledgerRecords).not.toHaveBeenCalled();
  });

  it("adds one ledger refresh when a full refresh joins a lightweight poll", async () => {
    let finishApps!: () => void;
    vi.mocked(refreshApps).mockReturnValueOnce(new Promise<void>((resolve) => { finishApps = resolve; }));

    const poll = refreshHost({ includeRecords: false });
    const full = refreshHost();
    expect(ledgerRecords).not.toHaveBeenCalled();

    finishApps();
    await Promise.all([poll, full]);

    expect(ledgerRecords).toHaveBeenCalledOnce();
  });

  it("refreshes only state scopes named by a remote change event", async () => {
    startPolling();
    await vi.waitFor(() => expect(eventHandlers.has("host-state:changed")).toBe(true));

    eventHandlers.get("host-state:changed")?.({ scopes: ["apps", "grants"] });

    await vi.waitFor(() => expect(refreshApps).toHaveBeenCalledOnce());
    expect(refreshGrants).toHaveBeenCalledOnce();
    expect(refreshArtifacts).not.toHaveBeenCalled();
    expect(ledgerRecords).not.toHaveBeenCalled();
  });

  it("performs a full authoritative refresh after an event replay gap", async () => {
    startPolling();
    await vi.waitFor(() => expect(eventHandlers.has("host-remote:event-gap")).toBe(true));

    eventHandlers.get("host-remote:event-gap")?.({});

    await vi.waitFor(() => expect(refreshGrants).toHaveBeenCalledOnce());
    expect(refreshApps).toHaveBeenCalledOnce();
    expect(refreshArtifacts).toHaveBeenCalledOnce();
  });
});
