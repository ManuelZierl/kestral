import { describe, expect, it } from "vitest";

import { renderMarkdown } from "./markdown";
import { partExcerpt, splitMessageParts } from "./messageParts";

describe("splitMessageParts", () => {
  it("splits blank-line separated blocks into indexed parts", () => {
    const parts = splitMessageParts("First paragraph.\n\nSecond paragraph.\n\nThird.");
    expect(parts.map((part) => part.text)).toEqual([
      "First paragraph.",
      "Second paragraph.",
      "Third.",
    ]);
    expect(parts.map((part) => part.index)).toEqual([0, 1, 2]);
  });

  it("returns no parts for empty or whitespace-only text", () => {
    expect(splitMessageParts("")).toEqual([]);
    expect(splitMessageParts("  \n\n \n")).toEqual([]);
  });

  it("keeps a fenced code block whole across its blank lines", () => {
    const text = "Intro.\n\n```js\nconst a = 1;\n\nconst b = 2;\n```\n\nOutro.";
    const parts = splitMessageParts(text);
    expect(parts).toHaveLength(3);
    expect(parts[1].text).toBe("```js\nconst a = 1;\n\nconst b = 2;\n```");
  });

  it("attaches a lone heading to the block it introduces", () => {
    const parts = splitMessageParts("# Title\n\nBody under the title.\n\nNext part.");
    expect(parts).toHaveLength(2);
    expect(parts[0].text).toBe("# Title\n\nBody under the title.");
  });

  it("splits a list into one part per item so each is markable", () => {
    const text = "Steps:\n\n1. First\n\n2. Second\n\n3. Third\n\nDone.";
    const parts = splitMessageParts(text);
    expect(parts.map((part) => part.text)).toEqual([
      "Steps:",
      "1. First",
      "2. Second",
      "3. Third",
      "Done.",
    ]);
  });

  it("splits a tight list (no blank lines) into per-item parts", () => {
    const parts = splitMessageParts("- one\n- two\n- three");
    expect(parts.map((part) => part.text)).toEqual(["- one", "- two", "- three"]);
    expect(parts.map((part) => part.kind)).toEqual(["list-item", "list-item", "list-item"]);
  });

  it("splits +/) markers the same way the renderer recognizes them", () => {
    // Splitter and renderer share one list definition (listSyntax.ts); a list
    // the splitter breaks apart must render as a real list, not stray <p>s.
    expect(splitMessageParts("+ one\n+ two").map((part) => part.text)).toEqual([
      "+ one",
      "+ two",
    ]);
    const ordered = splitMessageParts("1) first\n2) second");
    expect(ordered.map((part) => part.text)).toEqual(["1) first", "2) second"]);
    expect(renderMarkdown(ordered[0].text)).toBe("<ol><li>first</li></ol>");
  });

  it("keeps ordered numbering when a list splits across parts", () => {
    const parts = splitMessageParts("1. First\n2. Second\n3. Third");
    expect(renderMarkdown(parts[1].text)).toContain('<ol start="2">');
    expect(renderMarkdown(parts[2].text)).toContain('<ol start="3">');
  });

  it("preserves the full visible text content across the split", () => {
    const text =
      "# Title\n\nIntro with **bold**.\n\n- one\n- two\n\n- three\n\n```\ncode\n```\n\n> quote\n\nBye.";
    const normalize = (html: string) =>
      html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();
    const whole = normalize(renderMarkdown(text));
    const stitched = normalize(
      splitMessageParts(text)
        .map((part) => renderMarkdown(part.text))
        .join(" "),
    );
    expect(stitched).toBe(whole);
  });
});

describe("partExcerpt", () => {
  it("flattens to a single line and drops fence markers", () => {
    expect(partExcerpt("```js\nconst a = 1;\n```")).toBe("const a = 1;");
    expect(partExcerpt("line one\nline two")).toBe("line one line two");
  });

  it("caps at 300 characters", () => {
    expect(partExcerpt("x".repeat(500))).toHaveLength(300);
  });
});
