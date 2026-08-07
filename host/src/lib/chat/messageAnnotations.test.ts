import { describe, expect, it } from "vitest";

import type { JsonObject, JsonValue } from "$lib/api";
import { splitMessageParts } from "./messageParts";
import {
  TEXT_ANNOTATION_CONTRACT,
  TEXT_COMMENT_KIND,
  TEXT_MARKS_KIND,
  TEXT_SELECTION_KIND,
  parseTextMarks,
  rangesContainSelection,
  renderMarkdownWithMarks,
  textCommentEvent,
  textSelectionEvent,
} from "./messageAnnotations";

const parts = splitMessageParts("Alpha beta.\n\nGamma.");

function payload(overrides: Record<string, unknown> = {}): JsonObject {
  const ranges = (overrides.ranges ?? [{ part: 0, start: 0, end: 5 }]) as JsonValue[];
  const { ranges: _ranges, ...rest } = overrides;
  return {
    kind: TEXT_MARKS_KIND,
    contract: TEXT_ANNOTATION_CONTRACT,
    groups: ranges.map((range, index) => ({ id: `group-${index + 1}`, ranges: [range] })),
    labels: { mark: "Mark as read", unmark: "Mark as unread" },
    state_revision: 7,
    ...(rest as JsonObject),
  };
}

describe("parseTextMarks", () => {
  it("accepts, sorts, and merges valid ranges", () => {
    const marks = parseTextMarks(payload({
      groups: [
        { id: "group-1", ranges: [
          { part: 0, start: 3, end: 8 },
          { part: 0, start: 0, end: 5 },
        ] },
        { id: "group-2", ranges: [{ part: 1, start: 0, end: 3 }] },
      ],
    }), parts);
    expect(marks?.ranges).toEqual([
      { part: 0, start: 0, end: 8 },
      { part: 1, start: 0, end: 3 },
    ]);
    expect(marks?.labels).toEqual({ mark: "Mark as read", unmark: "Mark as unread" });
  });

  it("rejects malformed ranges and foreign contracts", () => {
    expect(parseTextMarks(payload({ contract: 1 }), parts)).toBeNull();
    expect(parseTextMarks(payload({ ranges: [{ part: 2, start: 0, end: 1 }] }), parts)).toBeNull();
    expect(parseTextMarks(payload({ ranges: [{ part: 0, start: 2, end: 2 }] }), parts)).toBeNull();
    expect(parseTextMarks(payload({ ranges: [{ part: 0, start: 0, end: 100 }] }), parts)).toBeNull();
  });

  it("requires a state revision and defaults every request off", () => {
    const marks = parseTextMarks(payload(), parts)!;
    expect(marks.stateRevision).toBe(7);
    expect(marks.observeReadingOpportunity).toBe(false);
    expect(marks.readingOpportunity).toBeNull();

    for (const stateRevision of [undefined, -1, 1.5, "7"]) {
      expect(parseTextMarks(payload({ state_revision: stateRevision }), parts)).toBeNull();
    }
    expect(parseTextMarks(payload({ observe_reading_opportunity: 1 }), parts)).toBeNull();
  });

  it("accepts only bounded integer opportunity aggregates", () => {
    const valid = parseTextMarks(payload({
      reading_opportunity: {
        possible_words_upper_bound: 120,
        total_words: 184,
        text_exposure: "most",
      },
    }), parts)!;
    expect(valid.readingOpportunity).toEqual({
      possibleWordsUpperBound: 120,
      totalWords: 184,
      textExposure: "most",
    });

    const broken: Record<string, unknown>[] = [
      { possible_words_upper_bound: 200, total_words: 184, text_exposure: "most" },
      { possible_words_upper_bound: 12.5, total_words: 184, text_exposure: "most" },
      { possible_words_upper_bound: -1, total_words: 184, text_exposure: "most" },
      { possible_words_upper_bound: 120, total_words: 184, text_exposure: "quite-a-lot" },
      { possible_words_upper_bound: 120, total_words: 184, text_exposure: 0.75 },
      { possible_words_upper_bound: 120, total_words: 184 },
    ];
    for (const reading_opportunity of broken) {
      expect(parseTextMarks(payload({ reading_opportunity }), parts)).toBeNull();
    }
  });

  it("accepts bounded non-overlapping comments only inside marked text", () => {
    const valid = parseTextMarks(payload({
      comments: [{ id: "comment-1", ranges: [{ part: 0, start: 0, end: 5 }], text: "Key point" }],
      comment_labels: { add: "Add note", edit: "Edit note" },
    }), parts);
    expect(valid?.comments).toEqual([
      { id: "comment-1", ranges: [{ part: 0, start: 0, end: 5 }], text: "Key point" },
    ]);
    expect(valid?.commentLabels).toEqual({ add: "Add note", edit: "Edit note" });
    expect(parseTextMarks(payload({
      comments: [{ id: "comment-1", ranges: [{ part: 0, start: 6, end: 10 }], text: "Unread" }],
    }), parts)).toBeNull();
    expect(parseTextMarks(payload({
      comments: [{ id: "comment-1", ranges: [{ part: 0, start: 0, end: 5 }], text: "\u0000" }],
    }), parts)).toBeNull();
  });
});

describe("text selection and rendering", () => {
  it("builds a selection event and detects a fully marked selection", () => {
    const selection = [{ part: 0, start: 1, end: 4, text: "lph" }];
    expect(textSelectionEvent(selection, true)).toEqual({
      kind: TEXT_SELECTION_KIND,
      contract: TEXT_ANNOTATION_CONTRACT,
      ranges: selection,
      marked: true,
    });
    expect(rangesContainSelection([{ part: 0, start: 0, end: 5 }], selection)).toBe(true);
    expect(rangesContainSelection([{ part: 0, start: 0, end: 2 }], selection)).toBe(false);
    expect(textCommentEvent("operation-1", "upsert", "comment-1", selection, "Remember")).toEqual({
      kind: TEXT_COMMENT_KIND,
      contract: TEXT_ANNOTATION_CONTRACT,
      operation_id: "operation-1",
      action: "upsert",
      comment_id: "comment-1",
      ranges: selection,
      text: "Remember",
    });
  });

  it("highlights only the selected readable text", () => {
    const html = renderMarkdownWithMarks("Alpha **beta**.", [{ part: 0, start: 6, end: 10 }]);
    expect(html).toContain("<strong><mark data-chat-text-mark=\"\" data-chat-text-start=\"6\" data-chat-text-end=\"10\">beta</mark></strong>");
    expect(html).not.toContain("<mark data-chat-text-mark=\"\">Alpha");
  });

  it("keeps one logical comment target across inline markdown text nodes", () => {
    const selected = "Alpha beta gamma";
    const html = renderMarkdownWithMarks(
      "Alpha `beta` gamma",
      [{
        part: 0,
        start: 0,
        end: selected.length,
        interactive: true,
        commentId: "comment-1",
        commentRange: { part: 0, start: 0, end: selected.length },
      }],
    );

    expect(html).toContain('data-chat-text-comment="comment-1"');
    expect(html).toContain(`data-chat-text-logical-start="0"`);
    expect(html).toContain(`data-chat-text-logical-end="${selected.length}"`);
    expect((html.match(/tabindex="0"/g) ?? []).length).toBe(1);
  });

  it("highlights canonical text inside table cells", () => {
    const source = "Name | Score\n--- | ---\nAda | 10";
    const html = renderMarkdownWithMarks(source, [{ part: 0, start: 12, end: 14 }]);

    expect(html).toContain(
      '<td><mark data-chat-text-mark="" data-chat-text-start="12" data-chat-text-end="14">10</mark></td>',
    );
  });
});
