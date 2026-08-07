// Folds the flat ledger event stream into runs — the unit people reason
// about ("Chat did X, it completed") — while keeping every raw event
// available as the run's detail. Pure for unit testing.

import type { Initiator, LedgerRecord, RunTerminalState } from "$lib/api";

export interface RunGroup {
  runId: string;
  startedAt: string;
  initiator: Initiator | null;
  goal: string | null;
  /** Null while the run has no run-ended event yet (still running). */
  terminal: RunTerminalState | null;
  /** Every ledger record for this run, in sequence order. */
  events: LedgerRecord[];
}

export function groupRuns(records: LedgerRecord[]): RunGroup[] {
  const byRun = new Map<string, RunGroup>();
  const ordered = [...records].sort((left, right) => left.sequence - right.sequence);
  for (const record of ordered) {
    const runId = record.event.run_id;
    let group = byRun.get(runId);
    if (!group) {
      group = {
        runId,
        startedAt: record.recorded_at,
        initiator: null,
        goal: null,
        terminal: null,
        events: [],
      };
      byRun.set(runId, group);
    }
    group.events.push(record);
    if (record.event.kind === "run-started") {
      group.initiator = record.event.initiator;
      group.goal = record.event.goal;
    } else if (record.event.kind === "run-ended") {
      group.terminal = record.event.terminal_state;
    }
  }
  // Insertion order is oldest run first; people want the latest run on top.
  return [...byRun.values()].reverse();
}

export function runStateLabel(group: RunGroup): string {
  return group.terminal ?? "running";
}
