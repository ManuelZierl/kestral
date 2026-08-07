import { describe, expect, it } from "vitest";
import { parseExpiry, secondsToExpiry } from "./grantExpiry";

describe("parseExpiry", () => {
  it("treats an empty value as never-expiring", () => {
    expect(parseExpiry("", "hours")).toEqual({ kind: "never" });
    expect(parseExpiry("   ", "days")).toEqual({ kind: "never" });
    expect(parseExpiry(undefined, "hours")).toEqual({ kind: "never" });
  });

  it("converts value + unit into seconds", () => {
    expect(parseExpiry("5", "minutes")).toEqual({ kind: "seconds", seconds: 300 });
    expect(parseExpiry("2", "hours")).toEqual({ kind: "seconds", seconds: 7200 });
    expect(parseExpiry("7", "days")).toEqual({ kind: "seconds", seconds: 604800 });
    expect(parseExpiry(2, "hours")).toEqual({ kind: "seconds", seconds: 7200 });
  });

  it("rejects non-positive, non-integer, and overflowing values", () => {
    expect(parseExpiry("0", "hours")).toEqual({ kind: "invalid" });
    expect(parseExpiry("-3", "hours")).toEqual({ kind: "invalid" });
    expect(parseExpiry("1.5", "hours")).toEqual({ kind: "invalid" });
    expect(parseExpiry("abc", "hours")).toEqual({ kind: "invalid" });
    expect(parseExpiry("100000", "days")).toEqual({ kind: "invalid" });
  });
});

describe("secondsToExpiry", () => {
  it("picks the largest whole unit", () => {
    expect(secondsToExpiry(604800)).toEqual({ value: "7", unit: "days" });
    expect(secondsToExpiry(7200)).toEqual({ value: "2", unit: "hours" });
    expect(secondsToExpiry(300)).toEqual({ value: "5", unit: "minutes" });
  });

  it("rounds an uneven span up to whole minutes", () => {
    expect(secondsToExpiry(90)).toEqual({ value: "2", unit: "minutes" });
    expect(secondsToExpiry(30)).toEqual({ value: "1", unit: "minutes" });
  });

  it("round-trips with parseExpiry", () => {
    const { value, unit } = secondsToExpiry(7200);
    expect(parseExpiry(value, unit)).toEqual({ kind: "seconds", seconds: 7200 });
  });
});
