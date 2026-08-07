import { render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it } from "vitest";

import RunLedgerTable from "$lib/system/RunLedgerTable.svelte";
import type { LedgerRecord } from "$lib/api";
import { records, recordsLoaded, shellError } from "$lib/stores/hostState";
import { activityTarget } from "$lib/stores/navigation";

const activity: LedgerRecord[] = [
  {
    sequence: 1,
    recorded_at: "2026-07-25T12:00:00Z",
    event: {
      kind: "run-started",
      run_id: "run-target",
      initiator: { kind: "app", app_id: "chat", reason: "automation" },
      goal: "Create a note",
    },
  },
  {
    sequence: 2,
    recorded_at: "2026-07-25T12:00:01Z",
    event: {
      kind: "capability-invoked",
      run_id: "run-target",
      capability: { provider: "notes", capability: "create" },
      grant_id: "grant-target",
      input_sha256: "input",
      data_scope: { kind: "none" },
    },
  },
];

beforeEach(() => {
  records.set(activity);
  recordsLoaded.set(true);
  shellError.set(null);
  activityTarget.set(null);
});

describe("RunLedgerTable", () => {
  it("opens, focuses, and briefly highlights the invocation behind a notice", async () => {
    activityTarget.set({ request: 1, runId: "run-target", grantId: "grant-target" });
    render(RunLedgerTable);

    const event = document.getElementById("activity-event-2");
    const run = document.getElementById("activity-run-run-target");
    expect(event).toBeTruthy();
    expect(run).toBeTruthy();
    await waitFor(() => {
      expect(event?.classList.contains("highlighted-event")).toBe(true);
      expect(run?.classList.contains("highlighted")).toBe(true);
      expect(document.activeElement).toBe(event);
      expect(screen.getByText("2 events").closest("details")?.open).toBe(true);
    });
  });
});
