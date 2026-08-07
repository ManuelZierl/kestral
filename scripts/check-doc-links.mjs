import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, extname, isAbsolute, join, relative, resolve, sep } from "node:path";

const root = resolve(process.argv[2] ?? "docs/_site");
const basePath = normalizeBasePath(process.argv[3] ?? "");

if (!existsSync(root) || !statSync(root).isDirectory()) {
  throw new Error(`generated documentation directory not found: ${root}`);
}

const pages = collectHtml(root);
const broken = [];

for (const page of pages) {
  const html = readFileSync(page, "utf8");
  for (const match of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
    const reference = match[1];
    const target = reference.split("#", 1)[0].split("?", 1)[0];
    if (!target || /^(?:https?:|mailto:|javascript:|data:)/.test(target)) {
      continue;
    }

    const decoded = decodeURI(target);
    const sitePath = isAbsolute(decoded) ? stripBasePath(decoded, basePath) : decoded;
    if (sitePath === null) {
      broken.push(`${relative(root, page)} -> ${reference}`);
      continue;
    }
    const path = isAbsolute(decoded)
      ? join(root, sitePath.replace(/^[/\\]+/, ""))
      : resolve(dirname(page), sitePath);
    if (!isWithinRoot(path)) {
      broken.push(`${relative(root, page)} -> ${reference}`);
      continue;
    }
    const candidates = [path, `${path}.html`, join(path, "index.html")];
    if (!candidates.some(existsSync)) {
      broken.push(`${relative(root, page)} -> ${reference}`);
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

function normalizeBasePath(value) {
  if (value === "") return "";
  if (!value.startsWith("/") || value.endsWith("/")) {
    throw new Error(`site base path must start with '/' and omit the trailing slash: ${value}`);
  }
  return value;
}

function stripBasePath(path, basePath) {
  if (basePath === "") return path;
  if (path === basePath) return "/";
  if (path.startsWith(`${basePath}/`)) return path.slice(basePath.length);
  return null;
}

function isWithinRoot(path) {
  const localPath = relative(root, path);
  return localPath === "" || (localPath !== ".." && !localPath.startsWith(`..${sep}`));
}
