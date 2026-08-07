import { describe, expect, it } from "vitest";
import {
  approveLabel,
  conditionSummary,
  dataScopeIsBroad,
  dataScopeSummary,
  denyLabel,
  durationSummary,
  scopeSummary,
} from "./approvalLanguage";

describe("dataScopeSummary", () => {
  it("flags persistent access to every current and future resource", () => {
    const scope = { kind: "all-resources" } as const;
    expect(dataScopeSummary(scope)).toBe("All current and future resources");
    expect(dataScopeIsBroad(scope)).toBe(true);
    expect(dataScopeIsBroad({ kind: "resources", resource_ids: ["thread-1"] })).toBe(false);
  });
});

describe("conditionSummary", () => {
  it("names silent reuse as the highest-trust outcome", () => {
    expect(conditionSummary("silent")).toBe("Runs without asking you again");
  });
  it("distinguishes notify from silent", () => {
    expect(conditionSummary("notify")).toBe("Runs, and tells you each time");
  });
  it("names per-use approval", () => {
    expect(conditionSummary("requires-approval")).toBe("Asks your approval each time");
  });
});

describe("scopeSummary", () => {
  it("describes an exact capability without the wildcard flag", () => {
    const summary = scopeSummary({
      kind: "exact-capability",
      provider: "notes",
      capability: "create",
    });
    expect(summary.wildcard).toBe(false);
    expect(summary.code).toBe("notes/create");
    expect(summary.text).toContain("create");
  });
  it("flags the broad provider wildcard and explains future actions", () => {
    const summary = scopeSummary({
      kind: "all-provider-capabilities",
      provider: "notes",
    });
    expect(summary.wildcard).toBe(true);
    expect(summary.code).toBe("notes/*");
    expect(summary.text).toContain("Everything notes");
    expect(summary.text).toContain("added later");
  });
});

describe("durationSummary", () => {
  it("names a non-expiring grant as lasting until revoked", () => {
    expect(durationSummary({ kind: "non-expiring" })).toBe("Lasts until you revoke it");
  });
  it("renders a day-scale expiry in days", () => {
    expect(durationSummary({ kind: "expires-after", seconds: 86400 })).toBe(
      "Expires in 1 day after you approve",
    );
  });
  it("renders an hour-scale expiry in hours", () => {
    expect(durationSummary({ kind: "expires-after", seconds: 7200 })).toBe(
      "Expires in 2 hours after you approve",
    );
  });
  it("renders a minute-scale expiry in minutes", () => {
    expect(durationSummary({ kind: "expires-after", seconds: 300 })).toBe(
      "Expires in 5 minutes after you approve",
    );
  });
});

describe("button labels", () => {
  it("names the approve action by result per request kind", () => {
    expect(approveLabel("grant-issuance")).toBe("Grant permission");
    expect(approveLabel("event-subscription")).toBe("Allow subscription");
    expect(approveLabel("capability-approval")).toBe("Allow once");
    // The batched install checklist grants only the boxes left checked, so its
    // label must say "selected", not a blanket "grant everything".
    expect(approveLabel("install-approval")).toBe("Grant selected");
  });
  it("keeps the deny label stable", () => {
    expect(denyLabel()).toBe("Don't allow");
  });
});
