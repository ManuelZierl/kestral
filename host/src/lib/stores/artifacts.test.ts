import { get } from "svelte/store";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Artifact } from "$lib/api";

vi.mock("$lib/api", () => ({
  listArtifacts: vi.fn(),
}));

import { listArtifacts } from "$lib/api";
import {
  artifacts,
  artifactsLoaded,
  refreshArtifacts,
  synchronizeArtifactReferences,
} from "$lib/stores/artifacts";

const mockedListArtifacts = vi.mocked(listArtifacts);

function artifact(id: string): Artifact {
  return {
    artifact_id: id,
    artifact_type: "note",
    title: id,
    content: {},
    provenance: {
      run_id: "run-1",
      capability: { provider: "notes", capability: "create" },
      grant_id: "grant-1",
      produced_by: "notes",
      recorded_at: "2026-07-25T00:00:00Z",
    },
  };
}

describe("artifacts store", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    artifacts.set([]);
    artifactsLoaded.set(false);
  });

  it("invalidates an older snapshot when a thread introduces a new reference", async () => {
    let finishOld!: (value: Artifact[]) => void;
    let finishCurrent!: (value: Artifact[]) => void;
    mockedListArtifacts
      .mockReturnValueOnce(new Promise((resolve) => { finishOld = resolve; }))
      .mockReturnValueOnce(new Promise((resolve) => { finishCurrent = resolve; }));

    const oldRefresh = refreshArtifacts();
    const synchronize = synchronizeArtifactReferences(["artifact-new"]);
    finishOld([]);
    await oldRefresh;

    expect(get(artifactsLoaded)).toBe(false);

    finishCurrent([artifact("artifact-new")]);
    await synchronize;
    expect(get(artifacts).map((item) => item.artifact_id)).toEqual(["artifact-new"]);
    expect(get(artifactsLoaded)).toBe(true);
  });

  it("leaves references unsettled when their authoritative refresh fails", async () => {
    artifactsLoaded.set(true);
    mockedListArtifacts.mockRejectedValueOnce(new Error("kernel busy"));

    await expect(synchronizeArtifactReferences(["artifact-new"])).rejects.toThrow("kernel busy");

    expect(get(artifactsLoaded)).toBe(false);
  });
});
