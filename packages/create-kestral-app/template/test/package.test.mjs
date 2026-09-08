import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import test from "node:test";

import { buildPackage } from "../scripts/build.mjs";

const root = resolve(import.meta.dirname, "..");

test("build keeps the starter app backend-free, narrow, and integrity-covered", async () => {
  await buildPackage(root);
  const manifest = JSON.parse(await readFile(join(root, "dist/app.json"), "utf8"));
  const html = await readFile(join(root, "dist/ui/index.html"));

  assert.equal(manifest.backend.kind, "none");
  assert.equal(manifest.data.kind, "host-managed");
  assert.equal(manifest.data.contract_version, 1);
  assert.deepEqual(manifest.manifest.surfaces[0].ui.connect_src, []);
  assert.deepEqual(manifest.manifest.surfaces[0].intents, [
    { provider: "llm-provider", capability: "llm.generate" },
  ]);
  assert.deepEqual(manifest.manifest.grant_requests.map((request) => request.scope), [
    { kind: "exact-capability", provider: "llm-provider", capability: "llm.generate" },
  ]);
  assert.equal(manifest.manifest.grant_requests[0].condition, "requires-approval");
  assert.match(manifest.manifest.grant_requests[0].reason, /titles and completion status/);
  assert.deepEqual(Object.keys(manifest.integrity.assets), ["ui/index.html"]);
  assert.equal(
    manifest.integrity.assets["ui/index.html"],
    `sha256-${createHash("sha256").update(html).digest("hex")}`,
  );
});
