import { readFile } from "node:fs/promises";

const expected = process.argv[2];
const jsonFiles = [
  "host/package.json",
  "host/package-lock.json",
  "host/src-tauri/tauri.conf.json",
];
const cargoFiles = [
  "crates/kernel/Cargo.toml",
  "crates/mcp-adapter/Cargo.toml",
  "host/src-tauri/Cargo.toml",
];

const versions = new Map();
for (const path of jsonFiles) {
  versions.set(path, JSON.parse(await readFile(path, "utf8")).version);
}
for (const path of cargoFiles) {
  const source = await readFile(path, "utf8");
  const version = source.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (!version) throw new Error(`No package version found in ${path}`);
  versions.set(path, version);
}
const cargoLock = await readFile("Cargo.lock", "utf8");
for (const packageName of ["app-host-kernel", "mcp-adapter", "host"]) {
  const escapedName = packageName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const version = cargoLock.match(
    new RegExp(`\\[\\[package\\]\\]\\r?\\nname = "${escapedName}"\\r?\\nversion = "([^"]+)"`),
  )?.[1];
  if (!version) throw new Error(`No ${packageName} package version found in Cargo.lock`);
  versions.set(`Cargo.lock (${packageName})`, version);
}

const unique = new Set(versions.values());
if (unique.size !== 1) {
  throw new Error(`Product versions differ: ${JSON.stringify(Object.fromEntries(versions))}`);
}
const [version] = unique;
if (!/^0\.1\.0(?:-(?:alpha|beta)\.\d+)?$/.test(version)) {
  throw new Error(`Unsupported release version ${version}`);
}
if (expected && version !== expected) {
  throw new Error(`Tag version ${expected} does not match product version ${version}`);
}
console.log(`Release version ${version} is consistent across ${versions.size} manifests.`);
