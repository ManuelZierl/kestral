import { describe, expect, it } from "vitest";
import { markdownPlainText, renderMarkdown } from "./markdown";

describe("renderMarkdown formatting", () => {
  it("renders bold and italic", () => {
    expect(renderMarkdown("**bold** and *italic*")).toBe(
      "<p><strong>bold</strong> and <em>italic</em></p>",
    );
  });

  it("renders underscore italics but leaves intra-word underscores alone", () => {
    expect(renderMarkdown("_emph_")).toBe("<p><em>emph</em></p>");
    expect(renderMarkdown("a_b_c")).toBe("<p>a_b_c</p>");
  });

  it("renders headings at their level", () => {
    expect(renderMarkdown("# Title")).toBe("<h1>Title</h1>");
    expect(renderMarkdown("### Small")).toBe("<h3>Small</h3>");
  });

  it("renders unordered and ordered lists", () => {
    expect(renderMarkdown("- one\n- two")).toBe("<ul><li>one</li><li>two</li></ul>");
    expect(renderMarkdown("1. one\n2. two")).toBe("<ol><li>one</li><li>two</li></ol>");
    // A list beginning mid-sequence (e.g. one item split out of a longer list)
    // keeps its real numbering via `start`.
    expect(renderMarkdown("3. three\n4. four")).toBe(
      '<ol start="3"><li>three</li><li>four</li></ol>',
    );
  });

  it("renders the list markers the part-splitter also recognizes", () => {
    // These share one definition with messageParts.ts (listSyntax.ts); the
    // renderer must not drop markers the splitter treats as list items, or a
    // split list renders as stray paragraphs.
    expect(renderMarkdown("+ one\n+ two")).toBe("<ul><li>one</li><li>two</li></ul>");
    expect(renderMarkdown("1) one\n2) two")).toBe("<ol><li>one</li><li>two</li></ol>");
    expect(renderMarkdown("  - indented")).toBe("<ul><li>indented</li></ul>");
  });

  it("preserves spaces and words in normal prose", () => {
    expect(renderMarkdown("go to step 0 then step 1")).toBe(
      "<p>go to step 0 then step 1</p>",
    );
  });

  it("renders inline code without formatting its contents", () => {
    expect(renderMarkdown("use `**not bold**` here")).toBe(
      "<p>use <code>**not bold**</code> here</p>",
    );
  });

  it("renders fenced code blocks verbatim", () => {
    expect(renderMarkdown("```\nlet x = 1;\n```")).toBe(
      "<pre><code>let x = 1;</code></pre>",
    );
  });

  it("separates paragraphs on blank lines", () => {
    expect(renderMarkdown("one\n\ntwo")).toBe("<p>one</p><p>two</p>");
  });

  it("renders safe links with target and rel", () => {
    expect(renderMarkdown("[docs](https://example.com)")).toBe(
      '<p><a href="https://example.com" target="_blank" rel="noreferrer noopener">docs</a></p>',
    );
  });

  it("renders pipe tables with semantic structure and alignment", () => {
    expect(renderMarkdown(
      "| Name | Score | Note |\n| :--- | ---: | :---: |\n| Ada | **10** | `a|b` |\n| Lin | 9 | clear |",
    )).toBe(
      '<div class="markdown-table"><table><thead><tr>' +
        '<th class="align-left">Name</th><th class="align-right">Score</th>' +
        '<th class="align-center">Note</th></tr></thead><tbody>' +
        '<tr><td class="align-left">Ada</td><td class="align-right"><strong>10</strong></td>' +
        '<td class="align-center"><code>a|b</code></td></tr>' +
        '<tr><td class="align-left">Lin</td><td class="align-right">9</td>' +
        '<td class="align-center">clear</td></tr></tbody></table></div>',
    );
  });

  it("supports escaped pipes and normalizes missing or extra body cells", () => {
    expect(renderMarkdown(
      "Label | Value\n--- | ---\nA \\| B | yes | ignored\nOnly one |",
    )).toBe(
      '<div class="markdown-table"><table><thead><tr><th>Label</th><th>Value</th></tr></thead>' +
        '<tbody><tr><td>A | B</td><td>yes</td></tr>' +
        '<tr><td>Only one</td><td></td></tr></tbody></table></div>',
    );
  });

  it("preserves escaped pipes literally inside table code spans", () => {
    expect(renderMarkdown(
      "Expression | Result\n--- | ---\n`a\\|b` | value",
    )).toContain("<td><code>a\\|b</code></td>");
  });

  it("requires a delimiter row and keeps table text offsets readable", () => {
    expect(renderMarkdown("Name | Value\nnot | a table")).toBe(
      "<p>Name | Value not | a table</p>",
    );
    expect(markdownPlainText("Name | Value\n--- | ---\nAda | 10")).toBe("NameValueAda10");
  });
});

describe("renderMarkdown safety", () => {
  it("escapes raw HTML so it cannot execute", () => {
    const html = renderMarkdown("<script>alert(1)</script>");
    expect(html).not.toContain("<script>");
    expect(html).toContain("&lt;script&gt;");
  });

  it("neutralizes javascript: links, leaving them inert text", () => {
    const html = renderMarkdown("[click](javascript:alert(1))");
    // No anchor is emitted and the scheme never reaches an href — it survives
    // only as escaped, non-clickable text, which cannot execute.
    expect(html).not.toContain("<a ");
    expect(html).not.toContain('href="javascript');
    expect(html).toContain("[click](javascript:alert(1))");
  });

  it("cannot break out of an href attribute", () => {
    const html = renderMarkdown('[x](https://e.com" onmouseover="alert(1))');
    expect(html).not.toContain('onmouseover="alert(1)"');
  });

  it("escapes HTML inside inline code and code blocks", () => {
    expect(renderMarkdown("`<img onerror=x>`")).toContain("&lt;img onerror=x&gt;");
    expect(renderMarkdown("```\n<b>hi</b>\n```")).toContain("&lt;b&gt;hi&lt;/b&gt;");
  });

  it("escapes HTML and unsafe links inside table cells", () => {
    const html = renderMarkdown(
      "Name | Value\n--- | ---\n<script>x</script> | [click](javascript:alert(1))",
    );
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("<a ");
    expect(html).toContain("&lt;script&gt;x&lt;/script&gt;");
  });
});
