import { beforeEach, describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";

vi.mock("$lib/api", () => ({
  listGrants: vi.fn(),
  revokeGrant: vi.fn(async () => undefined),
}));

import { listGrants } from "$lib/api";
import { grants, grantsRevision, refreshGrants, revokeGrantAndRefresh } from "$lib/stores/grants";

const grant = {
  grant_id: "grant-1",
  holder: "chat",
  holder_display_name: "Chat",
  scope: { kind: "exact-capability" as const, provider: "notes", capability: "create_note" },
  data_scope: { kind: "none" as const },
  condition: "silent" as const,
  issued_at: "2026-07-08T00:00:00Z",
  expires_at: null,
  status: "active" as const,
  origin: "manifest-requested" as const,
};

describe("grants store", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(listGrants).mockResolvedValue([grant]);
    grants.set([]);
    grantsRevision.set(0);
  });

  it("bumps the revision only when a read changes the grants", async () => {
    await refreshGrants();
    expect(get(grants)).toHaveLength(1);
    expect(get(grantsRevision)).toBe(1);

    await refreshGrants();
    expect(get(grantsRevision)).toBe(1);
  });

  it("refreshes dependent grant-aware surfaces after revoke", async () => {
    await revokeGrantAndRefresh("grant-1");
    expect(get(grantsRevision)).toBe(1);
    expect(get(grants)[0]?.status).toBe("active");
  });

  it("coalesces a mutation refresh with its host-state invalidation refresh", async () => {
    let finishRefresh!: (value: typeof grant[]) => void;
    vi.mocked(listGrants).mockReturnValueOnce(new Promise((resolve) => { finishRefresh = resolve; }));

    const invalidationRefresh = refreshGrants();
    const mutationRefresh = refreshGrants(true);
    await Promise.resolve();
    expect(listGrants).toHaveBeenCalledTimes(1);

    finishRefresh([grant]);
    await Promise.all([invalidationRefresh, mutationRefresh]);

    expect(get(grantsRevision)).toBe(1);
    expect(get(grants)).toEqual([grant]);
  });

  it("allows a later refresh after a transport fails synchronously", async () => {
    vi.mocked(listGrants).mockImplementationOnce(() => { throw new Error("transport unavailable"); });

    await expect(refreshGrants()).rejects.toThrow("transport unavailable");
    await refreshGrants();

    expect(listGrants).toHaveBeenCalledTimes(2);
    expect(get(grants)).toEqual([grant]);
  });

  it("retries a transient busy read after a successful mutation", async () => {
    vi.mocked(listGrants)
      .mockRejectedValueOnce(new Error("kernel busy: another host operation owns the kernel"))
      .mockResolvedValueOnce([grant]);

    await revokeGrantAndRefresh("grant-1");

    expect(listGrants).toHaveBeenCalledTimes(2);
    expect(get(grants)).toEqual([grant]);
  });
});
