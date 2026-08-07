import type { Initiator, LedgerEvent, LedgerRecord } from "$lib/api";
import { capabilityLabel } from "$lib/system/capabilityLabel";
import { dataScopeLabel } from "$lib/system/dataScopeLabel";

function initiatorLabel(initiator: Initiator): string {
  switch (initiator.kind) {
    case "surface-action":
      return `${initiator.app_id} surface ${initiator.surface}`;
    case "app":
      return `${initiator.app_id} (${initiator.reason})`;
    case "run":
      return `${initiator.app_id}, follow-up of an earlier run`;
  }
}

export function ledgerEventSummary(record: LedgerRecord): string {
  const event: LedgerEvent = record.event;
  switch (event.kind) {
    case "run-started":
      return `Started by ${initiatorLabel(event.initiator)} — ${event.goal}`;
    case "capability-invoked":
      return `Invoked ${capabilityLabel(event.capability)}`;
    case "capability-completed":
      return `Completed ${capabilityLabel(event.capability)}`;
    case "capability-failed":
      return `${capabilityLabel(event.capability)} failed: ${event.error}`;
    case "invocation-refused":
      return `${capabilityLabel(event.capability)} refused: ${event.reason}`;
    case "invocation-cancelled":
      return `${capabilityLabel(event.capability)} cancelled`;
    case "approval-requested":
      return `Approval requested for ${capabilityLabel(event.capability)}`;
    case "approval-granted":
      return `Approval granted for ${capabilityLabel(event.capability)}`;
    case "approval-denied":
      return `Approval denied for ${capabilityLabel(event.capability)}`;
    case "artifact-produced":
      return `Produced ${event.artifact_type}`;
    case "run-ended":
      return `Run ${event.terminal_state}`;
  }
}

// The identifying grant/artifact id behind an event. The one-line summaries
// above stay readable by leaving ids out of the sentence; this exposes them for
// a hover/title so the inspector keeps the "every action is attributable"
// grant → run → artifact correlation. Null when the event has no such id.
export function ledgerEventDetail(record: LedgerRecord): string | null {
  const event: LedgerEvent = record.event;
  switch (event.kind) {
    case "capability-invoked":
    case "capability-completed":
    case "capability-failed":
    case "approval-requested":
    case "approval-granted":
    case "approval-denied":
      return `Grant ${event.grant_id} · Requested scope: ${dataScopeLabel(event.data_scope)}`;
    case "invocation-refused":
    case "invocation-cancelled":
      return `Requested scope: ${dataScopeLabel(event.data_scope)}`;
    case "artifact-produced":
      return `Artifact ${event.artifact_id}`;
    default:
      return null;
  }
}
