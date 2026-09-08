import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, mkdir, copyFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { buildPackage } from "../scripts/build.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

async function json(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

test("tracked dist is reproducible from source on Node 22+", async () => {
  assert.ok(Number(process.versions.node.split(".")[0]) >= 22);
  const temp = await mkdtemp(resolve(tmpdir(), "kestral-daily-review-"));
  try {
    await mkdir(resolve(temp, "src/ui"), { recursive: true });
    await copyFile(resolve(root, "src/app.json"), resolve(temp, "src/app.json"));
    await copyFile(resolve(root, "src/ui/index.html"), resolve(temp, "src/ui/index.html"));
    await buildPackage(temp);

    assert.deepEqual(await json(resolve(temp, "dist/app.json")), await json(resolve(root, "dist/app.json")));
    assert.deepEqual(
      await readFile(resolve(temp, "dist/ui/index.html")),
      await readFile(resolve(root, "dist/ui/index.html")),
    );
  } finally {
    await rm(temp, { recursive: true, force: true });
  }
});

test("authoring path has no checkout-local package dependency", async () => {
  const project = await json(resolve(root, "package.json"));
  assert.deepEqual(project.dependencies ?? {}, {});
  assert.deepEqual(project.devDependencies ?? {}, {});

  const build = await readFile(resolve(root, "scripts/build.mjs"), "utf8");
  const manifest = await readFile(resolve(root, "src/app.json"), "utf8");
  const ui = await readFile(resolve(root, "src/ui/index.html"), "utf8");
  for (const source of [build, manifest, ui]) {
    assert.doesNotMatch(source, /(?:\.\.\/)+(?:host|crates|scripts|templates)\//);
  }
});

test("workflow stays backend-free and model assistance is optional and review-before-apply", async () => {
  const manifest = await json(resolve(root, "dist/app.json"));
  const ui = await readFile(resolve(root, "dist/ui/index.html"), "utf8");

  assert.deepEqual(manifest.backend, { kind: "none" });
  assert.equal(manifest.data.kind, "host-managed");
  assert.equal(manifest.data.contract_version, 2);
  assert.equal(manifest.manifest.grant_requests.length, 1);
  assert.deepEqual(manifest.manifest.grant_requests[0].scope, {
    kind: "exact-capability",
    provider: "llm-provider",
    capability: "llm.generate",
  });
  assert.equal(manifest.manifest.grant_requests[0].condition, "requires-approval");

  assert.match(ui, /- \[ \] Add a task/);
  assert.match(ui, /e\.key==="Tab"&&selected/);
  assert.match(ui, /root\.value\.done\?archiveEl:tasksEl/);
  assert.match(ui, /Review the proposal before applying it\./);
  assert.match(ui, /AI review unavailable; nothing changed/);
  assert.match(ui, /data\.v2\.readSnapshot/);
  assert.match(ui, /expectedGeneration:generation/);
  assert.match(ui, /getFullYear\(\)/);
  assert.doesNotMatch(ui, /toISOString\(\)\.slice\(0,10\)/);
});

test("Chat composition is grant-mediated and proposals cannot overwrite stale tasks", async () => {
  const manifest = await json(resolve(root, "dist/app.json"));
  const ui = await readFile(resolve(root, "dist/ui/index.html"), "utf8");

  assert.deepEqual(manifest.data.exports, [
    { capability: "list_tasks", operation: "list", collection: "tasks" },
  ]);
  assert.equal(manifest.data.proposals[0].capability, "propose_task_title");
  assert.equal(manifest.data.proposals[0].target.kind, "record");
  assert.equal(manifest.data.proposals[0].target.collection, "tasks");

  const readGrant = manifest.consumer_grant_requests.find(
    (grant) => grant.holder === "chat" && grant.request.scope.capability === "list_tasks",
  );
  assert.ok(readGrant);
  assert.deepEqual(readGrant.request.data_scope, {
    kind: "resources",
    resource_ids: ["app-data:com.ma-zierl.daily-review:tasks"],
  });

  const proposalGrant = manifest.consumer_grant_requests.find(
    (grant) => grant.holder === "chat" && grant.request.scope.capability === "propose_task_title",
  );
  assert.ok(proposalGrant);
  assert.equal(proposalGrant.request.condition, "requires-approval");
  assert.deepEqual(proposalGrant.request.data_scope, { kind: "all-resources" });

  assert.match(ui, /window\.appHost\.listArtifacts\(\)/);
  assert.match(ui, /artifact_type==="task-title-proposal"/);
  assert.match(ui, /task\.revision!==c\.targetRevision\|\|generation!==c\.targetGeneration/);
  assert.match(ui, /apply\.disabled=stale/);
  assert.match(ui, /expectedGeneration:c\.targetGeneration/);
  assert.match(ui, /expectedRevision:c\.targetRevision/);
  assert.match(ui, /Proposal became stale; nothing was overwritten/);
});
