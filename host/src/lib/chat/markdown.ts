// Minimal, escape-first Markdown renderer for assistant chat messages.
//
// Assistant replies are Markdown. Rendered as plain text they show literal
// `**bold**`, `# heading`, `- lists`, and ``` code fences ```, which is the
// single biggest readability gap in Chat. This renders a safe subset to HTML
// for use with Svelte `{@html}`.
//
// SECURITY: the entire input is HTML-escaped BEFORE any markup is inserted, so
// no span of user text can ever become an HTML tag or attribute — only the
// tags this module itself emits appear in the output. Links are restricted to
// an http/https/mailto scheme allowlist; anything else renders as inert text.
// The unit tests exercise the standard injection vectors.

import { matchListItem } from "./listSyntax";

type TableAlignment = "left" | "center" | "right" | null;

const ESCAPE: Record<string, string> = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
};

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (char) => ESCAPE[char]);
}

// A control-character sentinel (U+0001) that cannot occur in normal text; any
// stray occurrences are stripped from the input up front. It shields extracted
// code spans from later inline formatting. Built with fromCharCode so the
// source has no invisible characters.
const SENTINEL = String.fromCharCode(1);
const RESTORE = new RegExp(`${SENTINEL}(\\d+)${SENTINEL}`, "g");

// URLs reach this already HTML-escaped; we only inspect the scheme.
const SAFE_SCHEME = /^(https?:|mailto:)/i;

/** Render inline spans within a single already-escaped line of text. */
function renderInline(escaped: string): string {
  // Pull code spans out first so their contents are never re-formatted.
  const codes: string[] = [];
  let text = escaped.replace(/`([^`]+)`/g, (_match, code: string) => {
    codes.push(`<code>${code}</code>`);
    return `${SENTINEL}${codes.length - 1}${SENTINEL}`;
  });

  text = text.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (match, label: string, url: string) => {
    // The url is HTML-escaped; unescape only for the scheme check.
    const scheme = url.replace(/&amp;/g, "&");
    if (!SAFE_SCHEME.test(scheme)) return match; // leave as literal, inert text
    return `<a href="${url}" target="_blank" rel="noreferrer noopener">${label}</a>`;
  });

  text = text.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  text = text.replace(/(^|[^*])\*([^*\s][^*]*)\*(?!\*)/g, "$1<em>$2</em>");
  text = text.replace(/(^|[^\w])_([^_\s][^_]*)_(?![\w])/g, "$1<em>$2</em>");

  return text.replace(RESTORE, (_match, position: string) => codes[Number(position)]);
}

/** Split one pipe table row without treating escaped or inline-code pipes as separators. */
function splitTableRow(line: string): string[] | null {
  const cells: string[] = [];
  let cell = "";
  let inCode = false;
  let hasSeparator = false;

  for (let index = 0; index < line.length; index += 1) {
    const character = line[index];
    if (!inCode && character === "\\" && line[index + 1] === "|") {
      cell += "|";
      index += 1;
      continue;
    }
    if (character === "`") {
      inCode = !inCode;
      cell += character;
      continue;
    }
    if (character === "|" && !inCode) {
      cells.push(cell.trim());
      cell = "";
      hasSeparator = true;
      continue;
    }
    cell += character;
  }

  if (!hasSeparator) return null;
  cells.push(cell.trim());
  if (cells[0] === "") cells.shift();
  if (cells.at(-1) === "") cells.pop();
  return cells.length > 0 ? cells : null;
}

function tableHeader(
  headerLine: string,
  delimiterLine: string,
): { cells: string[]; alignments: TableAlignment[] } | null {
  const cells = splitTableRow(headerLine);
  const delimiters = splitTableRow(delimiterLine);
  if (!cells || !delimiters || cells.length !== delimiters.length) return null;

  const alignments: TableAlignment[] = [];
  for (const delimiter of delimiters) {
    const match = delimiter.match(/^(:)?-{3,}(:)?$/);
    if (!match) return null;
    alignments.push(
      match[1] && match[2] ? "center" : match[2] ? "right" : match[1] ? "left" : null,
    );
  }
  return { cells, alignments };
}

function tableCell(tag: "th" | "td", value: string, alignment: TableAlignment): string {
  const className = alignment ? ` class="align-${alignment}"` : "";
  return `<${tag}${className}>${renderInline(value)}</${tag}>`;
}

/** Render a safe Markdown subset to an HTML string. */
export function renderMarkdown(source: string): string {
  const input = source.split(SENTINEL).join("").replace(/\r\n?/g, "\n");
  const lines = escapeHtml(input).split("\n");
  const out: string[] = [];
  let paragraph: string[] = [];
  let listType: "ul" | "ol" | null = null;
  let index = 0;

  const flushParagraph = () => {
    if (paragraph.length > 0) {
      out.push(`<p>${renderInline(paragraph.join(" "))}</p>`);
      paragraph = [];
    }
  };
  const closeList = () => {
    if (listType) {
      out.push(`</${listType}>`);
      listType = null;
    }
  };

  while (index < lines.length) {
    const line = lines[index];

    if (/^```/.test(line)) {
      flushParagraph();
      closeList();
      const body: string[] = [];
      index += 1;
      while (index < lines.length && !/^```/.test(lines[index])) {
        body.push(lines[index]);
        index += 1;
      }
      index += 1; // consume the closing fence (if present)
      out.push(`<pre><code>${body.join("\n")}</code></pre>`);
      continue;
    }

    if (line.trim() === "") {
      flushParagraph();
      closeList();
      index += 1;
      continue;
    }

    const header = index + 1 < lines.length ? tableHeader(line, lines[index + 1]) : null;
    if (header) {
      flushParagraph();
      closeList();
      const rows: string[][] = [];
      index += 2;
      while (index < lines.length) {
        const row = splitTableRow(lines[index]);
        if (!row) break;
        rows.push(row);
        index += 1;
      }
      const headings = header.cells
        .map((cell, column) => tableCell("th", cell, header.alignments[column]))
        .join("");
      const body = rows
        .map((row) => `<tr>${header.alignments
          .map((alignment, column) => tableCell("td", row[column] ?? "", alignment))
          .join("")}</tr>`)
        .join("");
      out.push(
        `<div class="markdown-table"><table><thead><tr>${headings}</tr></thead>` +
          `${body ? `<tbody>${body}</tbody>` : ""}</table></div>`,
      );
      continue;
    }

    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    if (heading) {
      flushParagraph();
      closeList();
      const level = heading[1].length;
      out.push(`<h${level}>${renderInline(heading[2])}</h${level}>`);
      index += 1;
      continue;
    }

    if (/^(-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
      flushParagraph();
      closeList();
      out.push("<hr>");
      index += 1;
      continue;
    }

    // '>' was escaped to '&gt;' before block parsing.
    const quote = line.match(/^&gt;\s?(.*)$/);
    if (quote) {
      flushParagraph();
      closeList();
      out.push(`<blockquote>${renderInline(quote[1])}</blockquote>`);
      index += 1;
      continue;
    }

    const item = matchListItem(line);
    if (item) {
      flushParagraph();
      const wanted = item.ordered ? "ol" : "ul";
      if (listType !== wanted) {
        closeList();
        // Honor the first item's number so a list that begins mid-sequence
        // (e.g. a single item split out of a longer list) still numbers
        // correctly. Omit `start` at 1 so ordinary lists render unchanged.
        out.push(
          item.ordered && item.start !== null && item.start !== 1
            ? `<ol start="${item.start}">`
            : `<${wanted}>`,
        );
        listType = wanted;
      }
      out.push(`<li>${renderInline(item.content)}</li>`);
      index += 1;
      continue;
    }

    paragraph.push(line);
    index += 1;
  }

  flushParagraph();
  closeList();
  // Structural newlines between generated HTML blocks become selectable text
  // nodes in the browser. Keep canonical readable offsets limited to content
  // the user can actually select.
  return out.join("");
}

/**
 * Canonical readable text for selection offsets. This strips only the tags
 * emitted by `renderMarkdown`; decoding once matches the browser text nodes
 * produced from that safe HTML.
 */
export function markdownPlainText(source: string): string {
  return renderMarkdown(source)
    .replace(/<[^>]+>/g, "")
    .replaceAll("&quot;", '"')
    .replaceAll("&#39;", "'")
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&amp;", "&");
}
