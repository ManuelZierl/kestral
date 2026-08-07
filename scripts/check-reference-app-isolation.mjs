import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";

const externalRoot = "reference-apps/";
const self = "scripts/check-reference-app-isolation.mjs";
const textExtensions = /\.(?:js|json|md|mjs|ps1|rs|sh|svelte|toml|ts|ya?ml)$/;
const tracked = execFileSync("git", ["ls-files", "-z"], { encoding: "utf8" })
  .split("\0")
  .filter(Boolean);

const violations = tracked.filter((path) => {
  if (
    path === self ||
    path.startsWith(externalRoot) ||
    !textExtensions.test(path) ||
    !existsSync(path)
  ) {
    return false;
  }
  const source = readFileSync(path, "utf8");
  return source.includes(externalRoot);
});

if (violations.length > 0) {
  throw new Error(
    `Core files must not depend on external app source paths or runtime identities:\n${violations.map((path) => `- ${path}`).join("\n")}`,
  );
}

console.log("Core source, tests, and release tooling are isolated from external app repository paths.");
