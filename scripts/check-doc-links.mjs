import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, extname, isAbsolute, join, relative, resolve } from "node:path";

const root = resolve(process.argv[2] ?? "docs/_site");

if (!existsSync(root) || !statSync(root).isDirectory()) {
  throw new Error(`generated documentation directory not found: ${root}`);
}

const pages = collectHtml(root);
const broken = [];

for (const page of pages) {
  const html = readFileSync(page, "utf8");
  for (const match of html.matchAll(/href="([^"]+)"/g)) {
    const href = match[1];
    const target = href.split("#", 1)[0].split("?", 1)[0];
    if (!target || /^(?:https?:|mailto:|javascript:|data:)/.test(target)) {
      continue;
    }

    const decoded = decodeURI(target);
    const path = isAbsolute(decoded)
      ? join(root, decoded.replace(/^[/\\]+/, ""))
      : resolve(dirname(page), decoded);
    const candidates = [path, `${path}.html`, join(path, "index.html")];
    if (!candidates.some(existsSync)) {
      broken.push(`${relative(root, page)} -> ${href}`);
    }
  }
}

if (broken.length > 0) {
  throw new Error(`broken generated documentation links:\n${broken.join("\n")}`);
}

console.log(`Checked ${pages.length} generated pages: internal links OK`);

function collectHtml(directory) {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...collectHtml(path));
    } else if (extname(entry.name) === ".html") {
      files.push(path);
    }
  }
  return files;
}
