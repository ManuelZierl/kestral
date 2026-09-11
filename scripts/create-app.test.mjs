import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cp, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";

import { createAppProject, parseArguments } from "./create-app.mjs";

async function scratch(t) {
  const root = await mkdtemp(join(tmpdir(), "kestral-create-app-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

test("parses the documented command shape", () => {
  assert.deepEqual(
    parseArguments(["my-app", "--id", "com.example.my-app", "--name", "My App"]),
    { directory: "my-app", id: "com.example.my-app", name: "My App", description: null },
  );
});

test("creates a dependency-free, installable-shaped focused app project", async (t) => {
  const root = await scratch(t);
  const target = join(root, "my-app");
  await createAppProject({
    directory: target,
    id: "com.example.my-app",
    name: "My & App",
    description: "A workspace shaped around my own review ritual.",
  });

  const project = JSON.parse(await readFile(join(target, "package.json"), "utf8"));
  assert.deepEqual(project.dependencies, undefined);
  assert.equal(project.scripts.build, "node scripts/build.mjs");
  assert.equal(project.scripts.test, "node --test");

  const manifest = JSON.parse(await readFile(join(target, "dist/app.json"), "utf8"));
  assert.equal(manifest.id, "com.example.my-app");
  assert.equal(manifest.display_name, "My & App");
  assert.equal(manifest.backend.kind, "none");
  assert.equal(manifest.data.kind, "host-managed");
  assert.equal(manifest.manifest.surfaces[0].ui.entry, "ui/index.html");
  assert.deepEqual(manifest.manifest.surfaces[0].ui.connect_src, []);
  assert.deepEqual(manifest.manifest.surfaces[0].intents, [
    { provider: "llm-provider", capability: "llm.generate" },
  ]);
  assert.equal(manifest.manifest.grant_requests[0].condition, "requires-approval");
  assert.equal(
    manifest.manifest.grant_requests[0].reason,
    "Send only the visible item titles and completion status when you explicitly ask for a suggested next step.",
  );

  const html = await readFile(join(target, "dist/ui/index.html"), "utf8");
  assert.match(html, /<title>My &amp; App<\/title>/);
  assert.match(html, /window\.appHost\.data\.v1/);
  assert.match(html, /window\.appHost\.invoke/);
  assert.doesNotMatch(html, /\{\{APP_NAME\}\}/);
  assert.equal(
    manifest.integrity.assets["ui/index.html"],
    `sha256-${createHash("sha256").update(html).digest("hex")}`,
  );

  const generatedTests = spawnSync(process.execPath, ["--test"], {
    cwd: target,
    encoding: "utf8",
  });
  assert.equal(generatedTests.status, 0, generatedTests.stderr || generatedTests.stdout);
});

test("refuses unsafe identities and existing targets without changing them", async (t) => {
  const root = await scratch(t);
  const invalidTarget = join(root, "invalid");
  await assert.rejects(
    createAppProject({ directory: invalidTarget, id: "My App", name: "My App" }),
    /invalid app id/,
  );

  const target = join(root, "existing");
  await createAppProject({ directory: target, id: "com.example.first", name: "First" });
  const before = await readFile(join(target, "dist/app.json"), "utf8");
  await assert.rejects(
    createAppProject({ directory: target, id: "com.example.second", name: "Second" }),
    /target already exists/,
  );
  assert.equal(await readFile(join(target, "dist/app.json"), "utf8"), before);
});

test("a build never touches a pre-existing backup path", async (t) => {
  const root = await scratch(t);
  const target = join(root, "my-app");
  await createAppProject({ directory: target, id: "com.example.safe-build", name: "Safe Build" });

  const unrelatedBackup = join(target, ".dist-backup");
  await mkdir(unrelatedBackup);
  await writeFile(join(unrelatedBackup, "user-data.txt"), "keep this\n", "utf8");

  const { buildPackage } = await import(pathToFileURL(join(target, "scripts/build.mjs")).href);
  await buildPackage(target);

  assert.equal(await readFile(join(unrelatedBackup, "user-data.txt"), "utf8"), "keep this\n");
  assert.deepEqual(
    (await readdir(target)).filter((entry) => entry.startsWith(".dist-build-")),
    [],
  );
});

test("integrity hashes describe the staged snapshot when source changes during a build", async (t) => {
  const root = await scratch(t);
  const target = join(root, "my-app");
  await createAppProject({ directory: target, id: "com.example.snapshot", name: "Snapshot" });

  const sourceHtml = join(target, "src/ui/index.html");
  const before = "<!doctype html><title>staged snapshot</title>\n";
  const after = "<!doctype html><title>later source edit</title>\n";
  await writeFile(sourceHtml, before, "utf8");

  const { buildPackage } = await import(pathToFileURL(join(target, "scripts/build.mjs")).href);
  await buildPackage(target, {
    copyUi: async (source, destination, options) => {
      await cp(source, destination, options);
      await writeFile(sourceHtml, after, "utf8");
    },
  });

  const distHtml = await readFile(join(target, "dist/ui/index.html"), "utf8");
  const manifest = JSON.parse(await readFile(join(target, "dist/app.json"), "utf8"));
  assert.equal(distHtml, before);
  assert.equal(await readFile(sourceHtml, "utf8"), after);
  assert.equal(
    manifest.integrity.assets["ui/index.html"],
    `sha256-${createHash("sha256").update(distHtml).digest("hex")}`,
  );
});
