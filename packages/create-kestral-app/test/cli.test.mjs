import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
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

    const result = spawnSync(process.execPath, ["--test", "test/*.test.mjs"], {
      cwd: target,
      encoding: "utf8",
      shell: true,
    });
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
