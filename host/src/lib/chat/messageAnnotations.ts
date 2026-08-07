// Versioned text-mark contract for Chat's `message-actions` extension point.
// Payloads originate in sandboxed app frames and are validated here before
// they affect host-owned rendering and interactions.

import type { JsonObject } from "$lib/api";
import { renderMarkdown } from "$lib/chat/markdown";
import type { MessagePart } from "$lib/chat/messageParts";

export const TEXT_ANNOTATION_CONTRACT = 6;
export const TEXT_MARKS_KIND = "message-text-marks";
export const TEXT_SELECTION_KIND = "message-text-selection";
export const TEXT_COMMENT_KIND = "message-text-comment";
export const READING_OPPORTUNITY_KIND = "message-reading-opportunity";
const MAX_TEXT_COMMENTS = 100;
const MAX_TEXT_GROUPS = 500;
export const MAX_TEXT_COMMENT_CHARACTERS = 500;

export interface TextMarkRange {
  part: number;
  start: number;
  end: number;
}

export interface TextSelectionRange extends TextMarkRange {
  text: string;
}

export interface TextMarkLabels {
  mark: string;
  unmark: string;
}

export interface TextMarkGroup {
  id: string;
  ranges: TextMarkRange[];
}

export interface TextMarkComment {
  id: string;
  ranges: TextMarkRange[];
  text: string;
}

export interface TextCommentLabels {
  add: string;
  edit: string;
}

export interface TextCommentOperation {
  id: string;
  status: "pending" | "completed" | "failed";
  error: string | null;
}

/// How much of a response a passive estimate says was exposed. A closed set of
/// words, never a ratio: the estimate is too coarse to justify a number that
/// reads as measurement.
export const TEXT_EXPOSURE_LEVELS = ["none", "some", "about-half", "most", "all"] as const;
export type TextExposureLevel = (typeof TEXT_EXPOSURE_LEVELS)[number];

/// An app-owned upper bound on how much of a response could have been read.
/// The host renders it; it never becomes a mark and never contradicts one.
export interface ReadingOpportunitySummary {
  possibleWordsUpperBound: number;
  totalWords: number;
  textExposure: TextExposureLevel;
}

export interface MessageTextMarks {
  groups: TextMarkGroup[];
  ranges: TextMarkRange[];
  labels: TextMarkLabels;
  comments: TextMarkComment[] | null;
  commentLabels: TextCommentLabels | null;
  commentOperation: TextCommentOperation | null;
  /// The app's own revision for this state. Lets the host tell a material
  /// change from a republish of the same snapshot.
  stateRevision: number;
  /// The app asks Chat to observe reading opportunity for this response.
  observeReadingOpportunity: boolean;
  readingOpportunity: ReadingOpportunitySummary | null;
}

const DEFAULT_LABELS: TextMarkLabels = {
  mark: "Mark selected text",
  unmark: "Unmark selected text",
};
const COMMENT_ID = /^[a-zA-Z0-9][a-zA-Z0-9_-]{0,99}$/;

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function characterCount(value: string): number {
  return Array.from(value).length;
}

function isValidXmlText(value: string): boolean {
  for (const character of value) {
    const codePoint = character.codePointAt(0)!;
    if (
      codePoint !== 0x9 && codePoint !== 0xa && codePoint !== 0xd &&
      (
        codePoint < 0x20 ||
        (codePoint >= 0xd800 && codePoint <= 0xdfff) ||
        codePoint === 0xfffe ||
        codePoint === 0xffff
      )
    ) {
      return false;
    }
  }
  return true;
}

export function normalizeTextRanges(ranges: TextMarkRange[]): TextMarkRange[] {
  const sorted = ranges
    .map((range) => ({ ...range }))
    .sort((left, right) => left.part - right.part || left.start - right.start || left.end - right.end);
  const normalized: TextMarkRange[] = [];
  for (const range of sorted) {
    const previous = normalized.at(-1);
    if (previous && previous.part === range.part && range.start < previous.end) {
      previous.end = Math.max(previous.end, range.end);
    } else {
      normalized.push(range);
    }
  }
  return normalized;
}

export function parseTextMarks(payload: JsonObject, parts: MessagePart[]): MessageTextMarks | null {
  if (payload.kind !== TEXT_MARKS_KIND || payload.contract !== TEXT_ANNOTATION_CONTRACT) return null;
  if (!Array.isArray(payload.groups) || payload.groups.length > MAX_TEXT_GROUPS) return null;

  function parseRanges(value: unknown): TextMarkRange[] | null {
    if (!Array.isArray(value) || value.length === 0) return null;
    const ranges: TextMarkRange[] = [];
    for (const candidate of value) {
      if (!isObject(candidate)) return null;
      const { part, start, end } = candidate;
      if (
        typeof part !== "number" || !Number.isInteger(part) || part < 0 || part >= parts.length ||
        typeof start !== "number" || !Number.isInteger(start) || start < 0 ||
        typeof end !== "number" || !Number.isInteger(end) || end <= start ||
        end > parts[part].plainText.length
      ) return null;
      ranges.push({ part, start, end });
    }
    return normalizeTextRanges(ranges);
  }

  function rangesAreContinuous(ranges: TextMarkRange[]): boolean {
    return ranges.every((range, index) => {
      const previous = ranges[index - 1];
      if (!previous) return true;
      if (range.part === previous.part) return range.start === previous.end;
      if (range.part <= previous.part) return false;
      if (previous.end !== parts[previous.part].plainText.length || range.start !== 0) return false;
      return parts
        .slice(previous.part + 1, range.part)
        .every((part) => part.plainText.length === 0);
    });
  }

  const groups: TextMarkGroup[] = [];
  const groupIds = new Set<string>();
  for (const candidate of payload.groups) {
    if (!isObject(candidate)) return null;
    const id = candidate.id;
    const ranges = parseRanges(candidate.ranges);
    if (
      typeof id !== "string" || !COMMENT_ID.test(id) || groupIds.has(id) ||
      !ranges || !rangesAreContinuous(ranges)
    ) return null;
    groupIds.add(id);
    groups.push({ id, ranges });
  }
  groups.sort((left, right) =>
    left.ranges[0].part - right.ranges[0].part ||
    left.ranges[0].start - right.ranges[0].start ||
    left.ranges[0].end - right.ranges[0].end
  );
  const memberRanges = groups.flatMap((group) => group.ranges);
  const normalizedRanges = normalizeTextRanges(memberRanges);
  // Overlap belongs to one logical group. Reject ambiguous app state instead
  // of flattening away which user action owned each span.
  if (normalizedRanges.length !== memberRanges.length) return null;

  const labels = { ...DEFAULT_LABELS };
  if (isObject(payload.labels)) {
    if (typeof payload.labels.mark === "string" && payload.labels.mark.trim() !== "") {
      labels.mark = payload.labels.mark.slice(0, 120);
    }
    if (typeof payload.labels.unmark === "string" && payload.labels.unmark.trim() !== "") {
      labels.unmark = payload.labels.unmark.slice(0, 120);
    }
  }

  let comments: TextMarkComment[] | null = null;
  let commentLabels: TextCommentLabels | null = null;
  if (payload.comments !== undefined) {
    if (!Array.isArray(payload.comments) || payload.comments.length > MAX_TEXT_COMMENTS) return null;
    comments = [];
    const commentIds = new Set<string>();
    for (const candidate of payload.comments) {
      if (!isObject(candidate)) return null;
      const { id, text } = candidate;
      const ranges = parseRanges(candidate.ranges);
      if (
        typeof id !== "string" || !COMMENT_ID.test(id) ||
        commentIds.has(id) || !ranges || !rangesAreContinuous(ranges) ||
        typeof text !== "string" || text.trim() === "" ||
        characterCount(text) > MAX_TEXT_COMMENT_CHARACTERS || !isValidXmlText(text) ||
        !rangesContainSelection(normalizedRanges, ranges)
      ) {
        return null;
      }
      commentIds.add(id);
      comments.push({ id, ranges, text });
    }
    const commentRanges = comments.flatMap((comment) => comment.ranges);
    if (normalizeTextRanges(commentRanges).length !== commentRanges.length) return null;

    commentLabels = { add: "Add comment", edit: "Edit comment" };
    if (isObject(payload.comment_labels)) {
      if (typeof payload.comment_labels.add === "string" && payload.comment_labels.add.trim() !== "") {
        commentLabels.add = payload.comment_labels.add.slice(0, 120);
      }
      if (typeof payload.comment_labels.edit === "string" && payload.comment_labels.edit.trim() !== "") {
        commentLabels.edit = payload.comment_labels.edit.slice(0, 120);
      }
    }
  }

  let commentOperation: TextCommentOperation | null = null;
  if (payload.comment_operation !== undefined) {
    if (!isObject(payload.comment_operation)) return null;
    const { id, status, error } = payload.comment_operation;
    if (
      typeof id !== "string" || !COMMENT_ID.test(id) ||
      (status !== "pending" && status !== "completed" && status !== "failed") ||
      (error !== undefined && error !== null && typeof error !== "string")
    ) return null;
    const boundedError = typeof error === "string" ? error.slice(0, 300) : null;
    if (status === "failed" && !boundedError) return null;
    commentOperation = { id, status, error: boundedError };
  }

  const stateRevision = payload.state_revision;
  if (
    typeof stateRevision !== "number" || !Number.isInteger(stateRevision) ||
    stateRevision < 0 || stateRevision > Number.MAX_SAFE_INTEGER
  ) {
    return null;
  }

  const observeReadingOpportunity = payload.observe_reading_opportunity;
  if (
    (observeReadingOpportunity !== undefined && typeof observeReadingOpportunity !== "boolean")
  ) {
    return null;
  }

  let readingOpportunity: ReadingOpportunitySummary | null = null;
  if (payload.reading_opportunity !== undefined) {
    if (!isObject(payload.reading_opportunity)) return null;
    const { possible_words_upper_bound: possible, total_words: total, text_exposure: exposure } =
      payload.reading_opportunity;
    if (
      typeof possible !== "number" || !Number.isInteger(possible) || possible < 0 ||
      typeof total !== "number" || !Number.isInteger(total) || total < 0 ||
      // An upper bound above the response's own word count is not a bound.
      possible > total ||
      typeof exposure !== "string" ||
      !(TEXT_EXPOSURE_LEVELS as readonly string[]).includes(exposure)
    ) {
      return null;
    }
    readingOpportunity = {
      possibleWordsUpperBound: possible,
      totalWords: total,
      textExposure: exposure as TextExposureLevel,
    };
  }

  return {
    groups,
    ranges: normalizedRanges,
    labels,
    comments,
    commentLabels,
    commentOperation,
    stateRevision,
    observeReadingOpportunity: observeReadingOpportunity === true,
    readingOpportunity,
  };
}

export function textSelectionEvent(ranges: TextSelectionRange[], marked: boolean): JsonObject {
  return {
    kind: TEXT_SELECTION_KIND,
    contract: TEXT_ANNOTATION_CONTRACT,
    ranges: ranges.map((range) => ({ ...range })),
    marked,
  };
}

export function textCommentEvent(
  operationId: string,
  action: "upsert" | "delete",
  commentId: string,
  ranges: TextSelectionRange[],
  text?: string,
): JsonObject {
  return {
    kind: TEXT_COMMENT_KIND,
    contract: TEXT_ANNOTATION_CONTRACT,
    operation_id: operationId,
    action,
    comment_id: commentId,
    ranges: ranges.map((range) => ({ ...range })),
    ...(text === undefined ? {} : { text }),
  };
}

export function rangesContainSelection(marks: TextMarkRange[], selection: TextMarkRange[]): boolean {
  return selection.every((selected) =>
    marks.some(
      (mark) =>
        mark.part === selected.part && mark.start <= selected.start && mark.end >= selected.end,
    ),
  );
}

interface RenderedTextMarkRange extends TextMarkRange {
  interactive?: boolean;
  sourcePart?: number;
  commentId?: string;
  commentRange?: TextMarkRange;
}

export function renderMarkdownWithMarks(source: string, ranges: RenderedTextMarkRange[]): string {
  const html = renderMarkdown(source);
  if (ranges.length === 0 || typeof DOMParser === "undefined") return html;

  const document = new DOMParser().parseFromString(`<body>${html}</body>`, "text/html");
  const walker = document.createTreeWalker(document.body, 4);
  const nodes: { node: Text; start: number; end: number }[] = [];
  const seenInteractiveKeys = new Set<string>();
  let offset = 0;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node as Text;
    const start = offset;
    offset += text.data.length;
    nodes.push({ node: text, start, end: offset });
  }

  for (const { node, start, end } of nodes) {
    const boundaries = new Set([0, node.data.length]);
    let overlaps = false;
    for (const range of ranges) {
      if (range.end <= start || range.start >= end) continue;
      overlaps = true;
      boundaries.add(Math.max(0, range.start - start));
      boundaries.add(Math.min(node.data.length, range.end - start));
    }
    if (!overlaps) continue;
    const ordered = [...boundaries].sort((left, right) => left - right);
    const fragment = document.createDocumentFragment();
    for (let index = 0; index < ordered.length - 1; index += 1) {
      const localStart = ordered[index];
      const localEnd = ordered[index + 1];
      const text = node.data.slice(localStart, localEnd);
      const selected = ranges.some(
        (range) => range.start < start + localEnd && range.end > start + localStart,
      );
      if (selected) {
        const mark = document.createElement("mark");
        mark.dataset.chatTextMark = "";
        mark.dataset.chatTextStart = String(start + localStart);
        mark.dataset.chatTextEnd = String(start + localEnd);
        const interactiveRange = ranges.find((range) =>
          range.interactive && range.start < start + localEnd && range.end > start + localStart,
        );
        if (interactiveRange?.commentId) {
          mark.dataset.chatTextComment = interactiveRange.commentId;
          mark.dataset.chatTextCommentStart = String(interactiveRange.commentRange?.start ?? interactiveRange.start);
          mark.dataset.chatTextCommentEnd = String(interactiveRange.commentRange?.end ?? interactiveRange.end);
        }
        if (!node.parentElement?.closest("a, button") && interactiveRange) {
          mark.dataset.chatTextActions = "";
          mark.dataset.chatTextPart = String(interactiveRange.sourcePart ?? interactiveRange.part);
          const logicalStart = interactiveRange.commentRange?.start ?? interactiveRange.start;
          const logicalEnd = interactiveRange.commentRange?.end ?? interactiveRange.end;
          const key = `${mark.dataset.chatTextComment ?? ""}:${mark.dataset.chatTextPart}:${logicalStart}:${logicalEnd}`;
          const isFirstFragment = !seenInteractiveKeys.has(key);
          seenInteractiveKeys.add(key);
          mark.tabIndex = isFirstFragment ? 0 : -1;
          if (isFirstFragment) {
            mark.setAttribute("role", "button");
            mark.setAttribute(
              "aria-label",
              interactiveRange.commentId
                ? "Commented text. Activate to edit comment."
                : "Marked text. Activate to add a comment.",
            );
          }
          mark.dataset.chatTextLogicalStart = String(logicalStart);
          mark.dataset.chatTextLogicalEnd = String(logicalEnd);
        }
        mark.textContent = text;
        fragment.append(mark);
      } else {
        fragment.append(document.createTextNode(text));
      }
    }
    node.replaceWith(fragment);
  }
  return document.body.innerHTML;
}

/// Cumulative, bounded observation aggregates for one response and session.
/// Cumulative rather than incremental so a resend merges instead of double
/// counting, and aggregate so no scroll position or viewport size ever leaves
/// the host.
export function readingOpportunityEvent(report: {
  sessionId: string;
  qualifiedVisibleMs: number;
  exposedMask: number;
  firstQualifiedAt: string;
  lastQualifiedAt: string;
  final: boolean;
}): JsonObject {
  return {
    kind: READING_OPPORTUNITY_KIND,
    contract: TEXT_ANNOTATION_CONTRACT,
    session_id: report.sessionId,
    qualified_visible_ms: Math.max(0, Math.round(report.qualifiedVisibleMs)),
    exposed_mask: report.exposedMask,
    first_qualified_at: report.firstQualifiedAt,
    last_qualified_at: report.lastQualifiedAt,
    final: report.final,
  };
}
