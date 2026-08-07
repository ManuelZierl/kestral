import { describe, expect, it } from "vitest";
import { groupRuns, runStateLabel } from "$lib/system/runGroups";
import type { LedgerRecord } from "$lib/api";

function record(sequence: number, event: LedgerRecord["event"]): LedgerRecord {
  return { sequence, recorded_at: `2026-07-17T00:0${sequence}:00Z`, event };
}

const capability = { provider: "notes", capability: "create" };

describe("groupRuns", () => {
  it("folds events into runs, newest run first, events in sequence order", () => {
    const records = [
      record(1, { kind: "run-started", run_id: "run-a", initiator: { kind: "app", app_id: "chat", reason: "chat" }, goal: "first" }),
      record(2, { kind: "capability-invoked", run_id: "run-a", capability, grant_id: "g", input_sha256: "input", data_scope: { kind: "none" } }),
      record(3, { kind: "run-ended", run_id: "run-a", terminal_state: "completed" }),
      record(4, { kind: "run-started", run_id: "run-b", initiator: { kind: "app", app_id: "chat", reason: "chat" }, goal: "second" }),
    ];

    const runs = groupRuns(records);

    expect(runs.map((run) => run.runId)).toEqual(["run-b", "run-a"]);
    expect(runs[1].goal).toBe("first");
    expect(runs[1].terminal).toBe("completed");
    expect(runs[1].events.map((event) => event.sequence)).toEqual([1, 2, 3]);
    expect(runs[1].startedAt).toBe("2026-07-17T00:01:00Z");
  });

  it("labels a run without a terminal event as running", () => {
    const runs = groupRuns([
      record(1, { kind: "run-started", run_id: "run-a", initiator: { kind: "app", app_id: "chat", reason: "chat" }, goal: "open" }),
    ]);

    expect(runStateLabel(runs[0])).toBe("running");
  });

  it("tolerates events arriving out of sequence order", () => {
    const runs = groupRuns([
      record(3, { kind: "run-ended", run_id: "run-a", terminal_state: "failed" }),
      record(1, { kind: "run-started", run_id: "run-a", initiator: { kind: "app", app_id: "chat", reason: "chat" }, goal: "g" }),
    ]);

    expect(runs).toHaveLength(1);
    expect(runs[0].startedAt).toBe("2026-07-17T00:01:00Z");
    expect(runStateLabel(runs[0])).toBe("failed");
  });
});
