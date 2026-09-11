import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, mkdir, copyFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { buildPackage } from "../scripts/build.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

async function json(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

async function sourceCopy(temp) {
  await mkdir(resolve(temp, "src/ui"), { recursive: true });
  await copyFile(resolve(root, "src/app.json"), resolve(temp, "src/app.json"));
  await copyFile(resolve(root, "src/ui/index.html"), resolve(temp, "src/ui/index.html"));
}

test("tracked dist is reproducible from source on Node 22+", async () => {
  assert.ok(Number(process.versions.node.split(".")[0]) >= 22);
  const temp = await mkdtemp(resolve(tmpdir(), "kestral-daily-review-"));
  try {
    await sourceCopy(temp);
    await buildPackage(temp);
    assert.deepEqual(await json(resolve(temp, "dist/app.json")), await json(resolve(root, "dist/app.json")));
    assert.deepEqual(await readFile(resolve(temp, "dist/ui/index.html")), await readFile(resolve(root, "dist/ui/index.html")));
  } finally {
    await rm(temp, { recursive: true, force: true });
  }
});

test("package text and digests are stable across CRLF and trailing-newline edits", async () => {
  const temp = await mkdtemp(resolve(tmpdir(), "kestral-daily-review-newlines-"));
  try {
    await sourceCopy(temp);
    const source = await readFile(resolve(temp, "src/ui/index.html"), "utf8");
    for (const suffix of ["", "\n", "\n\n"]) {
      await writeFile(resolve(temp, "src/ui/index.html"), (source.trimEnd() + suffix).replace(/\n/g, "\r\n"));
      await buildPackage(temp);
      assert.deepEqual(await json(resolve(temp, "dist/app.json")), await json(resolve(root, "dist/app.json")));
      assert.deepEqual(await readFile(resolve(temp, "dist/ui/index.html")), await readFile(resolve(root, "dist/ui/index.html")));
    }
  } finally {
    await rm(temp, { recursive: true, force: true });
  }
});

test("authoring path has no checkout-local package dependency", async () => {
  const project = await json(resolve(root, "package.json"));
  assert.deepEqual(project.dependencies ?? {}, {});
  assert.deepEqual(project.devDependencies ?? {}, {});
  for (const path of ["scripts/build.mjs", "src/app.json", "src/ui/index.html"]) {
    assert.doesNotMatch(await readFile(resolve(root, path), "utf8"), /(?:\.\.\/)+(?:host|crates|scripts|templates)\//);
  }
});

test("workflow stays backend-free and model assistance is optional and review-before-apply", async () => {
  const manifest = await json(resolve(root, "dist/app.json"));
  assert.deepEqual(manifest.backend, { kind: "none" });
  assert.equal(manifest.data.kind, "host-managed");
  assert.equal(manifest.data.contract_version, 2);
  assert.equal(manifest.manifest.grant_requests.length, 1);
  assert.deepEqual(manifest.manifest.grant_requests[0].scope, {
    kind: "exact-capability", provider: "llm-provider", capability: "llm.generate",
  });
  assert.equal(manifest.manifest.grant_requests[0].condition, "requires-approval");
  // Actual denial, staging, and acceptance behavior is exercised in ui.test.mjs.
});

test("Chat composition is grant-mediated and proposals bind the target task", async () => {
  const manifest = await json(resolve(root, "dist/app.json"));
  assert.deepEqual(manifest.data.exports, [{ capability: "list_tasks", operation: "list", collection: "tasks" }]);
  assert.equal(manifest.data.proposals[0].capability, "propose_task_title");
  assert.equal(manifest.data.proposals[0].target.kind, "record");
  assert.equal(manifest.data.proposals[0].target.collection, "tasks");
  const readGrant = manifest.consumer_grant_requests.find(
    grant => grant.holder === "chat" && grant.request.scope.capability === "list_tasks",
  );
  assert.ok(readGrant);
  assert.deepEqual(readGrant.request.data_scope, {
    kind: "resources", resource_ids: ["app-data:com.ma-zierl.daily-review:tasks"],
  });
  const proposalGrant = manifest.consumer_grant_requests.find(
    grant => grant.holder === "chat" && grant.request.scope.capability === "propose_task_title",
  );
  assert.ok(proposalGrant);
  assert.equal(proposalGrant.request.condition, "requires-approval");
  assert.deepEqual(proposalGrant.request.data_scope, { kind: "all-resources" });
  // Stale display and racing writes are checked through real UI handlers, not
  // source-string assertions that can pass while the workflow still loses data.
});
