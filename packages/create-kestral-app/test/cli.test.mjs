import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import test from "node:test";

import { createAppProject, parseArguments } from "../cli.mjs";

test("parses the public creator command", () => {
  assert.deepEqual(parseArguments(["demo", "--id", "com.example.demo", "--name", "Demo"]), {
    directory: "demo",
    id: "com.example.demo",
    name: "Demo",
    description: null,
  });
});

test("creates an independently buildable project", async () => {
  const root = await mkdtemp(resolve(tmpdir(), "create-kestral-app-"));
  const target = resolve(root, "demo");
  try {
    await createAppProject({
      directory: target,
      id: "com.example.demo",
      name: "Demo",
      description: "A creator-tool qualification app.",
    });
    const manifest = JSON.parse(await readFile(resolve(target, "dist/app.json"), "utf8"));
    assert.equal(manifest.id, "com.example.demo");
    assert.equal(manifest.display_name, "Demo");

    const result = spawnSync(process.execPath, ["--test"], {
      cwd: target,
      encoding: "utf8",
    });
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("packed creator installs and runs through its public npm binary outside the checkout", async (t) => {
  const root = await mkdtemp(resolve(tmpdir(), "packed creator "));
  t.after(() => rm(root, { recursive: true, force: true }));
  const npm = process.env.npm_execpath;
  assert.ok(npm, "run creator qualification with npm test so npm's CLI path is available");
  const packageRoot = fileURLToPath(new URL("../", import.meta.url));
  const run = (script, args, cwd) => {
    const result = spawnSync(process.execPath, [script, ...args], {
      cwd,
      encoding: "utf8",
    });
    assert.equal(result.status, 0, `${result.error ?? ""}\n${result.stdout}\n${result.stderr}`);
    return result.stdout;
  };
  const packed = JSON.parse(run(npm, ["pack", "--ignore-scripts", "--json", "--pack-destination", root], packageRoot));
  const consumer = resolve(root, "consumer");
  await mkdir(consumer);
  run(npm, ["install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", "--package-lock=false", resolve(root, packed[0].filename)], consumer);
  run(npm, ["exec", "--offline", "--", "create-kestral-app", "my app", "--id", "com.example.packed", "--name", "Packed App"], consumer);

  const project = resolve(consumer, "my app");
  const manifest = JSON.parse(await readFile(resolve(project, "dist/app.json"), "utf8"));
  assert.equal(manifest.id, "com.example.packed");
  assert.equal(manifest.display_name, "Packed App");
  const before = await readFile(resolve(project, "dist/app.json"), "utf8");
  run(resolve(project, "scripts/build.mjs"), [], project);
  assert.equal(await readFile(resolve(project, "dist/app.json"), "utf8"), before);
  run("--test", [], project);
});
