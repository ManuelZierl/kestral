import { describe, expect, it } from "vitest";
import { ledgerEventDetail, ledgerEventSummary } from "$lib/system/LedgerEventSummary";
import type { LedgerRecord } from "$lib/api";

function record(event: LedgerRecord["event"]): LedgerRecord {
  return { sequence: 1, recorded_at: new Date().toISOString(), event };
}

describe("ledgerEventSummary", () => {
  it("covers every event variant", () => {
    const cases: LedgerRecord[] = [
      record({ kind: "run-started", run_id: "run-1", initiator: { kind: "app", app_id: "chat", reason: "chat" }, goal: "hello" }),
      record({ kind: "capability-invoked", run_id: "run-1", capability: { provider: "notes", capability: "create_note" }, grant_id: "grant-1", input_sha256: "input", data_scope: { kind: "none" } }),
      record({ kind: "capability-completed", run_id: "run-1", capability: { provider: "notes", capability: "create_note" }, grant_id: "grant-1", result_sha256: "result", data_scope: { kind: "none" } }),
      record({ kind: "capability-failed", run_id: "run-1", capability: { provider: "notes", capability: "create_note" }, grant_id: "grant-1", error: "boom", data_scope: { kind: "none" } }),
      record({ kind: "invocation-refused", run_id: "run-1", capability: { provider: "notes", capability: "create_note" }, reason: "no-grant", data_scope: { kind: "none" } }),
      record({ kind: "approval-requested", run_id: "run-1", capability: { provider: "notes", capability: "create_note" }, grant_id: "grant-1", data_scope: { kind: "none" } }),
      record({ kind: "approval-granted", run_id: "run-1", capability: { provider: "notes", capability: "create_note" }, grant_id: "grant-1", data_scope: { kind: "none" } }),
      record({ kind: "approval-denied", run_id: "run-1", capability: { provider: "notes", capability: "create_note" }, grant_id: "grant-1", data_scope: { kind: "none" } }),
      record({ kind: "artifact-produced", run_id: "run-1", artifact_id: "artifact-1", artifact_type: "note" }),
      record({ kind: "run-ended", run_id: "run-1", terminal_state: "completed" }),
    ];

    expect(cases.map(ledgerEventSummary)).toHaveLength(10);
  });
});

describe("ledgerEventDetail", () => {
  it("exposes the grant id behind capability and approval events", () => {
    expect(
      ledgerEventDetail(
        record({
          kind: "capability-invoked",
          run_id: "run-1",
          capability: { provider: "notes", capability: "create_note" },
          grant_id: "grant-42",
          input_sha256: "input",
          data_scope: { kind: "resources", resource_ids: ["artifact-1"] },
        }),
      ),
    ).toBe("Grant grant-42 · Requested scope: Resources: artifact-1");
  });

  it("exposes the artifact id behind artifact-produced", () => {
    expect(
      ledgerEventDetail(
        record({ kind: "artifact-produced", run_id: "run-1", artifact_id: "artifact-7", artifact_type: "note" }),
      ),
    ).toBe("Artifact artifact-7");
  });

  it("has no detail for events that carry no grant or artifact id", () => {
    expect(
      ledgerEventDetail(record({ kind: "run-ended", run_id: "run-1", terminal_state: "completed" })),
    ).toBeNull();
  });
});
