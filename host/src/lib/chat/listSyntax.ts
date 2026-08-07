// Single source of truth for how assistant-Markdown list items are detected
// and parsed.
//
// Two places need to agree on "what is a list item": messageParts.ts splits a
// list block into one markable part per item, and markdown.ts renders those
// same lines as <li>. When the two definitions drift, a line that the splitter
// treats as a list item renders as a stray paragraph instead of a list — so
// both import from here rather than carrying their own regex.
//
// The recognized shapes (CommonMark subset): up to 3 leading spaces, then a
// bullet (`-`, `*`, `+`) or an ordered marker (1–9 digits followed by `.` or
// `)`), then at least one space and some non-space content.

export interface ListMarker {
  /** true for ordered items (`1.` / `1)`), false for bullets (`-`, `*`, `+`). */
  ordered: boolean;
  /** For ordered items, the literal start number (e.g. 7 for `7)`); null for bullets. */
  start: number | null;
  /** The item's content after the marker. */
  content: string;
}

const LIST_ITEM = /^(?:\s{0,3})(?:([-*+])|(\d{1,9})[.)])\s+(\S.*)$/;

/** Parse a single line as a list item, or return null if it is not one. */
export function matchListItem(line: string): ListMarker | null {
  const match = LIST_ITEM.exec(line);
  if (!match) return null;
  const [, bullet, digits, content] = match;
  if (bullet !== undefined) return { ordered: false, start: null, content };
  return { ordered: true, start: Number(digits), content };
}

/** Whether a line begins a list item under the shared syntax. */
export function isListItem(line: string): boolean {
  return LIST_ITEM.test(line);
}
