import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import test from "node:test";

const checker = resolve("scripts/check-doc-links.mjs");

test("requires version-neutral prefixes on Jekyll internal links", () => {
  for (const entry of readdirSync("docs", { withFileTypes: true })) {
    if (!entry.isFile() || !entry.name.endsWith(".md")) continue;

    const content = readFileSync(join("docs", entry.name), "utf8");
    const linkCount = content.match(/\{% link /g)?.length ?? 0;
    if (linkCount === 0) continue;

    const prefixedCount = content.match(/\{\{ internal_link_prefix \}\}\{% link /g)?.length ?? 0;
    assert.equal(prefixedCount, linkCount, `${entry.name} contains an unprefixed Jekyll link`);
    assert.match(content, /assign jekyll_major = jekyll\.version/);
  }
});

test("accepts project-site links and assets under the configured base path", () => {
  const root = fixtureSite();
  try {
    const result = spawnSync(process.execPath, [checker, root, "/kestral"], {
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /internal links OK/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rejects project-site assets that escape to the account root", () => {
  const root = fixtureSite();
  try {
    writeFileSync(join(root, "index.html"), '<link href="/assets/site.css">');
    const result = spawnSync(process.execPath, [checker, root, "/kestral"], {
      encoding: "utf8",
    });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /broken generated documentation links/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rejects relative links to existing files outside the generated site", () => {
  const root = fixtureSite();
  const outside = `${root}-outside.html`;
  try {
    writeFileSync(outside, "");
    writeFileSync(join(root, "index.html"), `<a href="../${basename(outside)}">Outside</a>`);
    const result = spawnSync(process.execPath, [checker, root, "/kestral"], {
      encoding: "utf8",
    });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /broken generated documentation links/);
  } finally {
    rmSync(outside, { force: true });
    rmSync(root, { recursive: true, force: true });
  }
});

function fixtureSite() {
  const root = mkdtempSync(join(tmpdir(), "kestral-doc-links-"));
  mkdirSync(join(root, "assets"));
  writeFileSync(join(root, "assets", "site.css"), "");
  writeFileSync(join(root, "app.js"), "");
  writeFileSync(join(root, "guide.html"), "");
  writeFileSync(
    join(root, "index.html"),
    [
      '<link href="/kestral/assets/site.css">',
      '<script src="/kestral/app.js"></script>',
      '<a href="/kestral/guide.html">Guide</a>',
    ].join("\n"),
  );
  return root;
}
