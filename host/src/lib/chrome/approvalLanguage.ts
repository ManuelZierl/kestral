// Plain-language mapping for the trusted-chrome approval modal.
//
// The kernel's grant vocabulary (`silent` / `notify` / `requires-approval`,
// `provider/*`) is precise but opaque to a non-developer. On the one surface
// where comprehension is safety-critical, the user must be able to answer
// "what will happen if I approve, and can the app reuse this silently?". These
// helpers turn the enums into result sentences. Kept pure so they are unit
// testable without mounting the modal.

import type { ChromeRequest, DataScope, GrantCondition, GrantDuration, GrantScope } from "$lib/api";
import { dataScopeLabel } from "$lib/system/dataScopeLabel";

/** A result sentence describing what a grant condition means for the user. */
export function conditionSummary(condition: GrantCondition): string {
  switch (condition) {
    case "silent":
      return "Runs without asking you again";
    case "notify":
      return "Runs, and tells you each time";
    case "requires-approval":
      return "Asks your approval each time";
  }
}

/** A result sentence describing how long an approved grant would last. */
export function durationSummary(duration: GrantDuration): string {
  if (duration.kind === "non-expiring") {
    return "Lasts until you revoke it";
  }
  return `Expires ${relativeDuration(duration.seconds)} after you approve`;
}

export function dataScopeSummary(scope: DataScope): string {
  return dataScopeLabel(scope);
}

export function dataScopeIsBroad(scope: DataScope): boolean {
  return scope.kind === "all-resources";
}

/** Render a span of seconds as an approximate, human-readable interval. */
function relativeDuration(seconds: number): string {
  const units: [limit: number, size: number, name: string][] = [
    [60, 1, "second"],
    [3600, 60, "minute"],
    [86400, 3600, "hour"],
    [Infinity, 86400, "day"],
  ];
  for (const [limit, size, name] of units) {
    if (seconds < limit) {
      const value = Math.max(1, Math.round(seconds / size));
      return `in ${value} ${name}${value === 1 ? "" : "s"}`;
    }
  }
  return "later";
}

export interface ScopeSummary {
  /** Human sentence naming what is being granted. */
  text: string;
  /** The exact capability reference, for users who want the precise value. */
  code: string;
  /** True for the broad `provider/*` grant — the UI flags this distinctly. */
  wildcard: boolean;
}

/** Describe a grant scope in words, flagging the broad wildcard form. */
export function scopeSummary(scope: GrantScope): ScopeSummary {
  if (scope.kind === "all-provider-capabilities") {
    return {
      text: `Everything ${scope.provider} provides — all its actions, including ones added later`,
      code: `${scope.provider}/*`,
      wildcard: true,
    };
  }
  return {
    text: `One action: ${scope.capability}`,
    code: `${scope.provider}/${scope.capability}`,
    wildcard: false,
  };
}

/** Label for the approving button, named by the result of that request kind. */
export function approveLabel(kind: ChromeRequest["kind"]): string {
  switch (kind) {
    case "grant-issuance":
      return "Grant permission";
    case "event-subscription":
      return "Allow subscription";
    case "capability-approval":
      return "Allow once";
    case "install-approval":
      return "Grant selected";
  }
}

/** Label for the denying button. Kept constant so the safe choice is stable. */
export function denyLabel(): string {
  return "Don't allow";
}
