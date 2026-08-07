import { describe, expect, it } from "vitest";

import {
  EXPOSURE_BANDS,
  createReadingOpportunityTracker,
  exposureMask,
  primaryReadingRegion,
  readingZoneOverlap,
  type ReadingEnvironment,
  type ReadingOpportunityReport,
  type ReadingRegionInput,
} from "./chatReadingOpportunity";

const FOCUSED: ReadingEnvironment = { active: true, documentVisible: true, windowFocused: true };

function region(overrides: Partial<ReadingRegionInput> & { messageId: string }): ReadingRegionInput {
  return {
    requested: true,
    zoneOverlap: 100,
    exposedMask: 0xffffffff,
    focused: false,
    ...overrides,
  };
}

/// A tracker with a clock the test drives, so no assertion depends on real time.
function harness(checkpointMs = 15_000) {
  const reports: ReadingOpportunityReport[] = [];
  let clock = 0;
  let session = 0;
  const tracker = createReadingOpportunityTracker({
    onReport: (report) => reports.push(report),
    now: () => clock,
    timestamp: () => new Date(1_800_000_000_000 + clock).toISOString(),
    createSessionId: () => `session-${++session}`,
    checkpointMs,
  });
  return {
    reports,
    tracker,
    advance(ms: number) {
      clock += ms;
    },
  };
}

describe("exposure and zone geometry", () => {
  it("records only the bands actually inside the viewport", () => {
    // Element is 320px tall starting at the viewport top; the viewport shows
    // 160px of it, so exactly the first half of the bands are exposed.
    const mask = exposureMask(0, 320, 0, 160);
    expect(mask).toBe(0x0000ffff);
    expect(exposureMask(0, 320, 0, 320)).toBe(0xffffffff);
    expect(exposureMask(400, 320, 0, 320)).toBe(0);
  });

  it("keeps the mask an unsigned 32-bit integer", () => {
    const mask = exposureMask(-1000, 2000, 0, 1000);
    expect(Number.isInteger(mask)).toBe(true);
    expect(mask).toBeGreaterThanOrEqual(0);
    expect(mask).toBeLessThanOrEqual(0xffffffff);
    expect(EXPOSURE_BANDS).toBe(32);
  });

  it("measures overlap against the central reading zone only", () => {
    // Zone spans 15%-85% of a 1000px viewport, so 150..850.
    expect(readingZoneOverlap(0, 100, 0, 1000)).toBe(0);
    expect(readingZoneOverlap(0, 1000, 0, 1000)).toBe(700);
    expect(readingZoneOverlap(400, 600, 0, 1000)).toBe(200);
  });
});

describe("primary reading region", () => {
  it("ignores responses nobody asked about", () => {
    expect(primaryReadingRegion([region({ messageId: "a", requested: false })])).toBeNull();
  });

  it("prefers the greatest overlap with the reading zone", () => {
    expect(primaryReadingRegion([
      region({ messageId: "a", zoneOverlap: 40 }),
      region({ messageId: "b", zoneOverlap: 300 }),
    ])).toBe("b");
  });

  it("lets keyboard or assistive-technology focus win over mere overlap", () => {
    expect(primaryReadingRegion([
      region({ messageId: "a", zoneOverlap: 400 }),
      region({ messageId: "b", zoneOverlap: 10, focused: true }),
    ])).toBe("b");
  });
});

describe("qualified time", () => {
  it("stays idle when no response requests observation", () => {
    const { tracker, reports, advance } = harness();
    tracker.update(FOCUSED, [region({ messageId: "a", requested: false })]);
    advance(60_000);
    tracker.update(FOCUSED, [region({ messageId: "a", requested: false })]);
    tracker.flush(true);
    expect(reports).toEqual([]);
  });

  it("accumulates only while the document is visible and the window focused", () => {
    const { tracker, reports, advance } = harness();
    tracker.update(FOCUSED, [region({ messageId: "a" })]);
    advance(10_000);

    // Losing focus closes the interval; the time after it must not count.
    tracker.update({ ...FOCUSED, windowFocused: false }, [region({ messageId: "a" })]);
    advance(60_000);
    tracker.update({ ...FOCUSED, documentVisible: false }, [region({ messageId: "a" })]);
    advance(60_000);
    tracker.update(FOCUSED, [region({ messageId: "a" })]);
    advance(5_000);
    tracker.flush(true);

    expect(reports.at(-1)!.qualifiedVisibleMs).toBe(15_000);
  });

  it("counts a shared interval for one message only", () => {
    const { tracker, reports, advance } = harness();
    const visible = [
      region({ messageId: "a", zoneOverlap: 300 }),
      region({ messageId: "b", zoneOverlap: 50 }),
    ];
    tracker.update(FOCUSED, visible);
    advance(30_000);
    tracker.update(FOCUSED, visible);
    tracker.flush(true);

    const time = new Map(reports.map((report) => [report.messageId, report.qualifiedVisibleMs]));
    // 30 seconds of reading is 30 seconds, not 30 for each visible response.
    expect(time.get("a")).toBe(30_000);
    expect(time.get("b")).toBe(0);
  });

  it("gives a fast scroll broad exposure and almost no time", () => {
    const { tracker, reports, advance } = harness();
    for (const messageId of ["a", "b", "c", "d"]) {
      tracker.update(FOCUSED, [region({ messageId, exposedMask: 0xffffffff })]);
      advance(80);
    }
    tracker.flush(true);

    for (const report of reports) {
      expect(report.exposedMask).toBe(0xffffffff);
      expect(report.qualifiedVisibleMs).toBeLessThan(500);
    }
  });

  it("gives a long stationary view time but only its exposed bands", () => {
    const { tracker, reports, advance } = harness();
    // Only the top half of a long response is on screen, and the reader does
    // not scroll — so the adapter's heartbeat is the only tick.
    const partial = region({ messageId: "a", exposedMask: 0x0000ffff });
    tracker.update(FOCUSED, [partial]);
    for (let tick = 0; tick < 24; tick += 1) {
      advance(5_000);
      tracker.update(FOCUSED, [partial]);
    }
    tracker.flush(true);

    const report = reports.at(-1)!;
    // Two minutes of time, but exposure never grew past what was displayed.
    expect(report.qualifiedVisibleMs).toBe(120_000);
    expect(report.exposedMask).toBe(0x0000ffff);
  });

  it("discards an interval too long to be real", () => {
    const { tracker, reports, advance } = harness();
    tracker.update(FOCUSED, [region({ messageId: "a" })]);
    // A suspended tab can resume hours later with no visibility event and no
    // heartbeat in between; that gap is not reading time.
    advance(6 * 60 * 60 * 1000);
    tracker.flush(true);
    expect(reports.at(-1)!.qualifiedVisibleMs).toBe(0);
  });
});

describe("sessions", () => {
  it("reports cumulative values so a repeat merges instead of adding", () => {
    const { tracker, reports, advance } = harness(1_000);
    const visible = [region({ messageId: "a" })];
    tracker.update(FOCUSED, visible);
    advance(2_000);
    tracker.update(FOCUSED, visible);
    advance(2_000);
    tracker.update(FOCUSED, visible);
    tracker.flush(true);

    const forA = reports.filter((report) => report.messageId === "a");
    expect(forA.length).toBeGreaterThan(1);
    // Every report is the running total for the same session.
    expect(new Set(forA.map((report) => report.sessionId)).size).toBe(1);
    for (const [index, report] of forA.entries()) {
      if (index === 0) continue;
      expect(report.qualifiedVisibleMs).toBeGreaterThanOrEqual(forA[index - 1].qualifiedVisibleMs);
    }
    expect(forA.at(-1)!.qualifiedVisibleMs).toBe(4_000);
  });

  it("opens a new session when an old response is revisited", () => {
    const { tracker, reports, advance } = harness();
    // 10:00 the response is read briefly.
    tracker.update(FOCUSED, [region({ messageId: "a" })]);
    advance(60_000);
    tracker.update(FOCUSED, [region({ messageId: "a" })]);

    // The user scrolls away for nineteen minutes.
    tracker.update(FOCUSED, [region({ messageId: "b" })]);
    advance(19 * 60_000);
    tracker.update(FOCUSED, [region({ messageId: "b" })]);

    // 10:20 they come back to it for one more minute.
    tracker.update(FOCUSED, [region({ messageId: "a" })]);
    advance(60_000);
    tracker.flush(true);

    const forA = reports.filter((report) => report.messageId === "a");
    const sessions = new Set(forA.map((report) => report.sessionId));
    expect(sessions.size).toBe(2);
    // The nineteen minutes elsewhere belong to no session at all.
    const perSession = new Map<string, number>();
    for (const report of forA) perSession.set(report.sessionId, report.qualifiedVisibleMs);
    for (const duration of perSession.values()) {
      expect(duration).toBeLessThanOrEqual(60_000);
    }
  });

  it("marks a session final once its response leaves the viewport", () => {
    const { tracker, reports, advance } = harness();
    tracker.update(FOCUSED, [region({ messageId: "a" })]);
    advance(5_000);
    tracker.update(FOCUSED, [region({ messageId: "a", zoneOverlap: 0, exposedMask: 0 })]);

    const closed = reports.filter((report) => report.messageId === "a" && report.final);
    expect(closed.length).toBe(1);
    expect(closed[0].qualifiedVisibleMs).toBe(5_000);
  });

  it("emits only bounded integers and no geometry", () => {
    const { tracker, reports, advance } = harness();
    tracker.update(FOCUSED, [region({ messageId: "a", zoneOverlap: 123.456, exposedMask: 0xff })]);
    advance(1_234.9);
    tracker.flush(true);

    const report = reports.at(-1)!;
    expect(Object.keys(report).sort()).toEqual([
      "exposedMask",
      "final",
      "firstQualifiedAt",
      "lastQualifiedAt",
      "messageId",
      "qualifiedVisibleMs",
      "sessionId",
    ]);
    expect(Number.isInteger(report.qualifiedVisibleMs)).toBe(true);
    expect(Number.isInteger(report.exposedMask)).toBe(true);
  });

  it("drops everything on reset without reporting", () => {
    const { tracker, reports, advance } = harness();
    tracker.update(FOCUSED, [region({ messageId: "a" })]);
    advance(5_000);
    const before = reports.length;
    tracker.reset();
    tracker.flush(true);
    expect(reports.length).toBe(before);
  });
});
