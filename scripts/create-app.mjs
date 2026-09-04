#!/usr/bin/env node

import { existsSync } from "node:fs";
import { cp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { basename, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const TEMPLATE_ROOT = fileURLToPath(new URL("../templates/focused-app/", import.meta.url));
const RESERVED_IDS = new Set([
  "com.ma-zierl.kestral-artifacts",
  "com.ma-zierl.host.file-broker",
  "com.ma-zierl.host.permissions",
]);
const APP_ID_PATTERN = /^(?!mcp-)[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$/;

const usage = `Create a standalone, backend-free Kestral app project.

Usage:
  node scripts/create-app.mjs <directory> --id <reverse-dns-id> --name <display-name> [--description <text>]

Example:
  node scripts/create-app.mjs ../my-focus-app --id com.example.my-focus-app --name "My Focus App"

The target directory must not already exist. The generated project has no npm
dependencies and contains a ready-to-install dist/ package.`;

function fail(message) {
  throw new Error(message);
}

export function parseArguments(argv) {
  if (argv.includes("--help") || argv.includes("-h")) return { help: true };

  const values = { directory: null, id: null, name: null, description: null };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (!argument.startsWith("-")) {
      if (values.directory !== null) fail(`unexpected positional argument '${argument}'`);
      values.directory = argument;
      continue;
    }
    const field = argument === "--id"
      ? "id"
      : argument === "--name"
        ? "name"
        : argument === "--description"
          ? "description"
          : null;
    if (field === null) fail(`unknown option '${argument}'`);
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) fail(`${argument} requires a value`);
    values[field] = value;
    index += 1;
  }

  if (values.directory === null) fail("a target directory is required");
  if (values.id === null) fail("--id is required");
  if (values.name === null) fail("--name is required");
  return values;
}

function validatedIdentity({ id, name, description }) {
  if (!APP_ID_PATTERN.test(id) || id.length > 214 || RESERVED_IDS.has(id)) {
    fail(`invalid app id '${id}': use a non-reserved lowercase reverse-DNS id such as com.example.my-app`);
  }
  const displayName = name.trim();
  if (displayName.length === 0 || displayName.length > 120 || /[\u0000-\u001f\u007f]/.test(displayName)) {
    fail("--name must contain 1-120 characters and no control characters");
  }
  const appDescription = (description ?? `A focused workspace for ${displayName}.`).trim();
  if (appDescription.length === 0 || appDescription.length > 2000 || /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(appDescription)) {
    fail("--description must contain 1-2000 characters and no unsupported control characters");
  }
  return { id, displayName, description: appDescription };
}

function htmlEscape(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function packageName(id, directory) {
  const candidate = `kestral-${id.split(".").at(-1) || basename(directory)}`
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return candidate || "kestral-focused-app";
}

async function replaceTokens(path, replacements) {
  let content = await readFile(path, "utf8");
  for (const [token, replacement] of Object.entries(replacements)) {
    content = content.replaceAll(token, replacement);
  }
  await writeFile(path, content, "utf8");
}

export async function createAppProject(options) {
  const identity = validatedIdentity(options);
  const target = resolve(options.directory);
  if (existsSync(target)) fail(`target already exists: ${target}`);

  await mkdir(target);
  try {
    // `target` was created by this invocation and is still empty. Node's `cp`
    // treats an existing destination directory as the copy root.
    await cp(TEMPLATE_ROOT, target, { recursive: true });

    const sourceManifestPath = resolve(target, "src/app.json");
    const manifest = JSON.parse(await readFile(sourceManifestPath, "utf8"));
    manifest.id = identity.id;
    manifest.display_name = identity.displayName;
    manifest.description = identity.description;
    manifest.manifest.surfaces[0].title = identity.displayName;
    manifest.manifest.surfaces[0].description = identity.description;
    await writeFile(sourceManifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");

    const projectPackagePath = resolve(target, "package.json");
    const projectPackage = JSON.parse(await readFile(projectPackagePath, "utf8"));
    projectPackage.name = packageName(identity.id, target);
    await writeFile(projectPackagePath, `${JSON.stringify(projectPackage, null, 2)}\n`, "utf8");

    await replaceTokens(resolve(target, "src/ui/index.html"), {
      "{{APP_NAME}}": htmlEscape(identity.displayName),
    });
    await replaceTokens(resolve(target, "README.md"), {
      "{{APP_NAME}}": identity.displayName,
      "{{APP_ID}}": identity.id,
    });

    const buildModule = await import(`${pathToFileURL(resolve(target, "scripts/build.mjs")).href}?initial-build`);
    await buildModule.buildPackage(target);
    return target;
  } catch (error) {
    await rm(target, { recursive: true, force: true });
    throw error;
  }
}

async function main() {
  try {
    const options = parseArguments(process.argv.slice(2));
    if (options.help) {
      console.log(usage);
      return;
    }
    const target = await createAppProject(options);
    console.log(`Created ${target}`);
    console.log("Next: run npm test there, then install its dist/ directory in Kestral.");
  } catch (error) {
    console.error(`create-app: ${error instanceof Error ? error.message : String(error)}`);
    console.error("Run with --help for usage.");
    process.exitCode = 1;
  }
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  await main();
}
