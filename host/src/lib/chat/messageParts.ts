// Canonical segmentation of an assistant response into reading parts.
//
// Chat owns this split: it renders the parts, hands them to message-actions
// extensions (as `parts` in the extension context), and maps text ranges back
// onto them. Keeping one segmentation here — instead of every extension
// re-deriving its own — means an extension's "part 3" is always the part the
// user sees as the third block in the conversation.
//
// A part is a natural reading unit of the Markdown response:
// - blocks separated by blank lines (outside code fences),
// - a fenced code block stays one unit no matter its blank lines,
// - a lone heading attaches to the block it introduces,
// - every list item is its own part, so a long list is many markable units
//   rather than one. Ordered items keep their real number (the renderer emits
//   `start`), so a split list still reads 1, 2, 3.
// The response text is immutable once complete, so the split is stable for a
// given message.

import { isListItem } from "./listSyntax";
import { markdownPlainText } from "./markdown";

export interface MessagePart {
  /** Position within the message; the wire identity of the part. */
  index: number;
  /** The part's raw Markdown, rendered by chat exactly like the full text. */
  text: string;
  /** Single-line plain excerpt (≤ 300 chars) for storage and recall. */
  excerpt: string;
  /** Rendered-readable text; selection ranges use offsets into this value. */
  plainText: string;
  /** Rendering rhythm for adjacent parts. */
  kind: "block" | "list-item";
}

const FENCE = /^(`{3,}|~{3,})/;
const LONE_HEADING = /^#{1,6}\s\S/;

function isListBlock(block: string): boolean {
  return isListItem(block.split("\n", 1)[0]);
}

/** Split a list block into one string per top-level item, keeping each item's
 * continuation/nested lines with it. */
function splitListItems(block: string): string[] {
  const items: string[] = [];
  let current: string[] = [];
  for (const line of block.split("\n")) {
    if (isListItem(line) && current.length > 0) {
      items.push(current.join("\n"));
      current = [];
    }
    current.push(line);
  }
  if (current.length > 0) items.push(current.join("\n"));
  return items;
}

/** Split blank-line separated blocks, keeping fenced code blocks whole. */
function rawBlocks(text: string): string[] {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const blocks: string[] = [];
  let current: string[] = [];
  let inFence = false;
  for (const line of lines) {
    const trimmed = line.trim();
    const isFence = FENCE.test(trimmed);
    if (isFence) inFence = !inFence;
    if (!inFence && !isFence && trimmed === "") {
      if (current.length > 0) {
        blocks.push(current.join("\n").trim());
        current = [];
      }
    } else {
      current.push(line);
    }
  }
  if (current.length > 0) blocks.push(current.join("\n").trim());
  return blocks.filter((block) => block !== "");
}

/** Single-line plain excerpt of a part, capped for grant-declared storage. */
export function partExcerpt(text: string): string {
  return text
    .split("\n")
    .filter((line) => !FENCE.test(line.trim()))
    .join(" ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 300);
}

export function splitMessageParts(text: string): MessagePart[] {
  const parts: { text: string; kind: MessagePart["kind"] }[] = [];
  // A lone heading introduces the block after it; hold it until we see that
  // block so the two read as one unit.
  let pendingHeading: string | null = null;
  for (const block of rawBlocks(text)) {
    if (pendingHeading !== null) {
      if (isListBlock(block)) {
        // Attach the heading to the first item, then let the rest split.
        const [first, ...rest] = splitListItems(block);
        parts.push(
          { text: `${pendingHeading}\n\n${first}`, kind: "list-item" },
          ...rest.map((text) => ({ text, kind: "list-item" as const })),
        );
      } else {
        parts.push({ text: `${pendingHeading}\n\n${block}`, kind: "block" });
      }
      pendingHeading = null;
      continue;
    }
    if (LONE_HEADING.test(block) && !block.includes("\n")) {
      pendingHeading = block;
      continue;
    }
    if (isListBlock(block)) {
      parts.push(...splitListItems(block).map((text) => ({ text, kind: "list-item" as const })));
      continue;
    }
    parts.push({ text: block, kind: "block" });
  }
  // A heading with nothing after it is still its own part.
  if (pendingHeading !== null) parts.push({ text: pendingHeading, kind: "block" });
  return parts.map(({ text, kind }, index) => ({
    index,
    text,
    excerpt: partExcerpt(text),
    plainText: markdownPlainText(text),
    kind,
  }));
}
