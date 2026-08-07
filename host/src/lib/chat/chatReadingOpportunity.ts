// Chat-owned observation of whether reading a response was *possible*.
//
// This is deliberately an upper bound, never a claim about attention. It
// records two independent things and keeps them separate:
//
//   - qualified time: how long a response was the one plausible reading target
//     in a visible, focused Chat window;
//   - exposure: which vertical bands of that response entered the viewport.
//
// Nothing here decides that text was read. Only an explicit user mark does
// that. Scrolling fast produces broad exposure with almost no time; sitting
// still produces time for only the exposed part. That falls out of measuring
// the two separately, so no "rapid scrolling means unread" rule is needed.
//
// Raw geometry is converted to a bounded integer mask at the boundary and
// discarded. No scroll offset, viewport size, ratio, or event log is retained,
// and nothing is observed at all until an extension asks for it.

/// Vertical bands per response. 32 keeps the mask one unsigned 32-bit integer.
export const EXPOSURE_BANDS = 32;
/// Reading zone as a fraction of the log viewport, measured from its top. Time
/// is attributed to the response occupying the middle of the window, not to
/// whatever happens to be clipped at an edge.
const ZONE_TOP_FRACTION = 0.15;
const ZONE_BOTTOM_FRACTION = 0.85;
/// Bound on how often observation is reported while reading continues.
const CHECKPOINT_MS = 15_000;
/// How often the DOM adapter ticks while a response is being read. Someone
/// sitting still generates no scroll, resize, or intersection events, so without
/// this there would be no evidence that the time in between was real.
const HEARTBEAT_MS = 5_000;
/// A single interval longer than this is discarded rather than trusted. Callers
/// must tick at least this often while reading continues (see HEARTBEAT_MS); a
/// longer gap means the page was suspended or frozen without a visibility
/// change, and counting it would invent reading time.
const MAX_INTERVAL_MS = 30_000;

export interface ReadingOpportunityReport {
  messageId: string;
  sessionId: string;
  /// Cumulative for this session, so a repeated report merges rather than adds.
  qualifiedVisibleMs: number;
  exposedMask: number;
  firstQualifiedAt: string;
  lastQualifiedAt: string;
  final: boolean;
}

/// One response's current geometry, already reduced to bounded values.
export interface ReadingRegionInput {
  messageId: string;
  /// The owning extension asked for observation of this response.
  requested: boolean;
  /// Overlap with the central reading zone, in CSS pixels.
  zoneOverlap: number;
  /// Bands of this response currently inside the viewport.
  exposedMask: number;
  /// Keyboard or assistive-technology focus is inside this response.
  focused: boolean;
}

export interface ReadingEnvironment {
  /// Chat is the mounted, active destination.
  active: boolean;
  documentVisible: boolean;
  windowFocused: boolean;
}

export interface ReadingOpportunityTrackerOptions {
  onReport: (report: ReadingOpportunityReport) => void;
  /// Monotonic clock. Wall time is never differenced: it can jump.
  now?: () => number;
  timestamp?: () => string;
  createSessionId?: () => string;
  checkpointMs?: number;
}

interface SessionState {
  id: string;
  qualifiedMs: number;
  exposedMask: number;
  firstQualifiedAt: string;
  lastQualifiedAt: string;
  reportedMs: number;
  reportedMask: number;
}

/// Choose the one response a shared interval belongs to.
///
/// Several responses are partly visible at once, and counting the same 30
/// seconds for each would turn one person's reading into several messages'
/// worth of opportunity. Explicit focus wins because it is a stated target;
/// otherwise the response occupying the reading zone does.
export function primaryReadingRegion(regions: ReadingRegionInput[]): string | null {
  const candidates = regions.filter((region) => region.requested && region.zoneOverlap > 0);
  if (candidates.length === 0) return null;
  const focused = candidates.filter((region) => region.focused);
  const pool = focused.length > 0 ? focused : candidates;
  return pool.reduce((best, region) =>
    region.zoneOverlap > best.zoneOverlap ||
    (region.zoneOverlap === best.zoneOverlap && region.messageId < best.messageId)
      ? region
      : best,
  ).messageId;
}

/// Which bands of an element are inside the viewport, as a bounded bitset.
/// The source rectangle is not retained.
export function exposureMask(
  elementTop: number,
  elementHeight: number,
  viewportTop: number,
  viewportBottom: number,
): number {
  if (elementHeight <= 0 || viewportBottom <= viewportTop) return 0;
  const bandHeight = elementHeight / EXPOSURE_BANDS;
  let mask = 0;
  for (let band = 0; band < EXPOSURE_BANDS; band += 1) {
    const top = elementTop + band * bandHeight;
    if (top + bandHeight > viewportTop && top < viewportBottom) mask |= 1 << band;
  }
  return mask >>> 0;
}

/// Overlap between an element and the central reading zone, in CSS pixels.
export function readingZoneOverlap(
  elementTop: number,
  elementBottom: number,
  viewportTop: number,
  viewportHeight: number,
): number {
  const height = viewportHeight * (ZONE_BOTTOM_FRACTION - ZONE_TOP_FRACTION);
  if (height <= 0) return 0;
  const zoneTop = viewportTop + viewportHeight * ZONE_TOP_FRACTION;
  const zoneBottom = viewportTop + viewportHeight * ZONE_BOTTOM_FRACTION;
  return Math.max(0, Math.min(elementBottom, zoneBottom) - Math.max(elementTop, zoneTop));
}

export interface ReadingOpportunityTracker {
  /// Fold the current environment and geometry in. Safe to call on every
  /// scroll: it only accumulates, and reporting stays on its own cadence.
  ///
  /// Callers must call this at least every MAX_INTERVAL_MS while reading
  /// continues. A longer gap is not credited, because nothing distinguishes a
  /// quiet reader from a frozen tab except the tick itself.
  update: (environment: ReadingEnvironment, regions: ReadingRegionInput[]) => void;
  /// Report unreported progress now, closing every session when `final`.
  flush: (final: boolean) => void;
  /// Discard state for responses that are gone (thread switch, disable).
  reset: () => void;
}

export function createReadingOpportunityTracker(
  options: ReadingOpportunityTrackerOptions,
): ReadingOpportunityTracker {
  const now = options.now ?? (() => performance.now());
  const timestamp = options.timestamp ?? (() => new Date().toISOString());
  const createSessionId = options.createSessionId ?? (() => crypto.randomUUID());
  const checkpointMs = options.checkpointMs ?? CHECKPOINT_MS;

  const sessions = new Map<string, SessionState>();
  let primary: string | null = null;
  let intervalOpenedAt: number | null = null;

  function session(messageId: string): SessionState {
    const existing = sessions.get(messageId);
    if (existing) return existing;
    const stamp = timestamp();
    const created: SessionState = {
      id: createSessionId(),
      qualifiedMs: 0,
      exposedMask: 0,
      firstQualifiedAt: stamp,
      lastQualifiedAt: stamp,
      reportedMs: 0,
      reportedMask: 0,
    };
    sessions.set(messageId, created);
    return created;
  }

  function report(messageId: string, state: SessionState, final: boolean): void {
    state.reportedMs = state.qualifiedMs;
    state.reportedMask = state.exposedMask;
    options.onReport({
      messageId,
      sessionId: state.id,
      qualifiedVisibleMs: Math.round(state.qualifiedMs),
      exposedMask: state.exposedMask,
      firstQualifiedAt: state.firstQualifiedAt,
      lastQualifiedAt: state.lastQualifiedAt,
      final,
    });
  }

  function closeInterval(): void {
    if (primary === null || intervalOpenedAt === null) {
      intervalOpenedAt = null;
      return;
    }
    const elapsed = now() - intervalOpenedAt;
    intervalOpenedAt = null;
    if (elapsed <= 0 || elapsed > MAX_INTERVAL_MS) return;
    const state = session(primary);
    state.qualifiedMs += elapsed;
    state.lastQualifiedAt = timestamp();
  }

  return {
    update(environment, regions) {
      // Exposure needs the tab to be showing; time additionally needs focus and
      // a single primary target. A visible but unfocused window can still have
      // put text in front of someone, so the two gates differ on purpose.
      const visible = environment.active && environment.documentVisible;
      const next = visible && environment.windowFocused
        ? primaryReadingRegion(regions)
        : null;

      if (next !== primary) {
        closeInterval();
        if (primary !== null) {
          const previous = sessions.get(primary);
          if (previous && previous.qualifiedMs > previous.reportedMs) report(primary, previous, false);
        }
        primary = next;
        intervalOpenedAt = next === null ? null : now();
      } else if (primary !== null) {
        const elapsed = now() - (intervalOpenedAt ?? now());
        if (elapsed >= checkpointMs || elapsed > MAX_INTERVAL_MS) {
          closeInterval();
          intervalOpenedAt = now();
        }
      }

      if (visible) {
        for (const region of regions) {
          if (!region.requested || region.exposedMask === 0) continue;
          const state = session(region.messageId);
          state.exposedMask = (state.exposedMask | region.exposedMask) >>> 0;
        }
      }

      // A response that left the viewport ends its session. Coming back later
      // opens a new one, so the time in between is never counted.
      const present = new Set(
        regions
          .filter((region) => region.requested && (region.zoneOverlap > 0 || region.exposedMask !== 0))
          .map((region) => region.messageId),
      );
      for (const [messageId, state] of [...sessions]) {
        if (present.has(messageId) || messageId === primary) continue;
        report(messageId, state, true);
        sessions.delete(messageId);
      }

      for (const [messageId, state] of sessions) {
        const checkpointDue = state.qualifiedMs - state.reportedMs >= checkpointMs;
        // A scroll-through never dwells anywhere, so without this first report
        // its exposure would only surface when the session closes.
        const firstExposure = state.reportedMs === 0 && state.reportedMask === 0 &&
          state.exposedMask !== 0;
        if (checkpointDue || firstExposure) report(messageId, state, false);
      }
    },

    flush(final) {
      closeInterval();
      for (const [messageId, state] of [...sessions]) {
        const changed = state.qualifiedMs > state.reportedMs || state.exposedMask !== state.reportedMask;
        if (changed || final) report(messageId, state, final);
        if (final) sessions.delete(messageId);
      }
      if (final) primary = null;
      else if (primary !== null) intervalOpenedAt = now();
    },

    reset() {
      closeInterval();
      sessions.clear();
      primary = null;
      intervalOpenedAt = null;
    },
  };
}

export interface ChatReadingObserverOptions {
  /// The scrolling Chat log. Observation is idle until this exists.
  root: HTMLElement;
  tracker: ReadingOpportunityTracker;
  /// Responses whose extension asked for observation.
  isRequested: (messageId: string) => boolean;
  isActive: () => boolean;
}

export interface ChatReadingObserver {
  register: (messageId: string, element: HTMLElement) => void;
  unregister: (messageId: string) => void;
  /// Re-evaluate after a request, activity, or layout change.
  refresh: () => void;
  destroy: () => void;
}

/// Wire one observer, one scroll listener, and the document/window gates to a
/// tracker. Exactly one of these exists per Chat log — never one per message or
/// per extension frame.
export function attachChatReadingObserver(
  options: ChatReadingObserverOptions,
): ChatReadingObserver {
  const elements = new Map<string, HTMLElement>();
  const observed = new WeakMap<Element, string>();
  const intersecting = new Set<string>();
  let frame: number | null = null;
  let destroyed = false;

  const observer = typeof IntersectionObserver === "undefined"
    ? null
    : new IntersectionObserver(
        (entries) => {
          for (const entry of entries) {
            const messageId = observed.get(entry.target);
            if (messageId === undefined) continue;
            if (entry.isIntersecting) intersecting.add(messageId);
            else intersecting.delete(messageId);
          }
          schedule();
        },
        { root: options.root, threshold: [0, 0.01, 0.5, 1] },
      );

  function measure(): void {
    if (destroyed) return;
    const viewport = options.root.getBoundingClientRect();
    const active = options.isActive();
    const regions: ReadingRegionInput[] = [];
    for (const [messageId, element] of elements) {
      const requested = options.isRequested(messageId);
      if (!requested) continue;
      if (!intersecting.has(messageId)) {
        regions.push({ messageId, requested, zoneOverlap: 0, exposedMask: 0, focused: false });
        continue;
      }
      const rect = element.getBoundingClientRect();
      regions.push({
        messageId,
        requested,
        zoneOverlap: readingZoneOverlap(rect.top, rect.bottom, viewport.top, viewport.height),
        exposedMask: exposureMask(rect.top, rect.height, viewport.top, viewport.bottom),
        focused: element.contains(document.activeElement),
      });
    }
    options.tracker.update(
      {
        active,
        documentVisible: document.visibilityState !== "hidden",
        windowFocused: document.hasFocus(),
      },
      regions,
    );
  }

  function schedule(): void {
    if (destroyed || frame !== null) return;
    frame = requestAnimationFrame(() => {
      frame = null;
      measure();
    });
  }

  function handleGateChange(): void {
    // Measure immediately: a checkpoint owed at blur must not wait for a frame
    // the browser may never run while the window is in the background.
    measure();
    options.tracker.flush(false);
  }

  // Reading generates no events. This heartbeat is the only thing that tells
  // the tracker a quiet minute was spent in front of the text rather than in a
  // suspended tab.
  const heartbeat = setInterval(measure, HEARTBEAT_MS);

  options.root.addEventListener("scroll", schedule, { passive: true });
  document.addEventListener("visibilitychange", handleGateChange);
  window.addEventListener("focus", schedule);
  window.addEventListener("blur", handleGateChange);
  window.addEventListener("resize", schedule, { passive: true });

  return {
    register(messageId, element) {
      elements.set(messageId, element);
      observed.set(element, messageId);
      observer?.observe(element);
      schedule();
    },
    unregister(messageId) {
      const element = elements.get(messageId);
      if (element) {
        observer?.unobserve(element);
        observed.delete(element);
      }
      elements.delete(messageId);
      intersecting.delete(messageId);
      schedule();
    },
    refresh: schedule,
    destroy() {
      destroyed = true;
      clearInterval(heartbeat);
      if (frame !== null) cancelAnimationFrame(frame);
      observer?.disconnect();
      options.root.removeEventListener("scroll", schedule);
      document.removeEventListener("visibilitychange", handleGateChange);
      window.removeEventListener("focus", schedule);
      window.removeEventListener("blur", handleGateChange);
      window.removeEventListener("resize", schedule);
      options.tracker.flush(true);
    },
  };
}
