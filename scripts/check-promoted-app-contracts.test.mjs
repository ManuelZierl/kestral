import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  validateExternalEvidence,
  validatePromotionDocument,
  validateReleaseChangedPaths,
} from "./check-promoted-app-contracts.mjs";

const { version: hostVersion } = JSON.parse(await readFile("host/package.json", "utf8"));
const coreCommit = "a".repeat(40);
const contracts = JSON.parse(await readFile("release/host-extension-contracts.json", "utf8"));
const promotion = JSON.parse(await readFile("release/promoted-apps.json", "utf8"));
const expectedAppIds = [
  "com.ma-zierl.kestral-chat-export",
  "com.ma-zierl.kestral-excalidraw",
  "com.ma-zierl.kestral-model-profiles",
  "com.ma-zierl.kestral-pi",
  "dev.kestral.goal-chat",
];

function clone(value) {
  return structuredClone(value);
}

test("current promoted packages match the exact host and provider contracts", () => {
  const apps = validatePromotionDocument(promotion, contracts, hostVersion);
  assert.deepEqual(apps.map((app) => app.id).sort(), expectedAppIds);
});

test("contract drift fails closed", () => {
  const changed = clone(promotion);
  const contributor = changed.apps.find((app) => app.extension_contributions.length > 0);
  assert.ok(contributor);
  contributor.extension_contributions[0].contract_version += 1;
  assert.throws(
    () => validatePromotionDocument(changed, contracts, hostVersion),
    /host provides v1/,
  );
});

test("a different tested host release fails closed", () => {
  const changed = clone(promotion);
  changed.apps[0].tested_host_version = "0.1.0-alpha.2";
  assert.throws(
    () => validatePromotionDocument(changed, contracts, hostVersion),
    /exact host version 0\.1\.0-alpha\.1/,
  );
});

test("release mode refuses missing lifecycle evidence", () => {
  const pendingPromotion = clone(promotion);
  pendingPromotion.tested_core_commit = null;
  for (const app of pendingPromotion.apps) {
    app.evidence_url = null;
    app.evidence_sha256 = null;
  }
  assert.throws(
    () => validatePromotionDocument(pendingPromotion, contracts, hostVersion, { requireEvidence: true }),
    /has no pinned lifecycle evidence/,
  );
});

test("the post-evidence release commit can change only release metadata", () => {
  const evidencePath = `release/v${hostVersion}-evidence.md`;
  assert.doesNotThrow(() => validateReleaseChangedPaths(["release/promoted-apps.json"], hostVersion));
  assert.doesNotThrow(() => validateReleaseChangedPaths(["release/promoted-apps.json", evidencePath], hostVersion));
  assert.throws(
    () => validateReleaseChangedPaths([evidencePath, "host/src-tauri/src/package.rs"], hostVersion),
    /changed tested core files/,
  );
  assert.throws(
    () => validateReleaseChangedPaths([evidencePath], "0.1.0-alpha.2"),
    /changed tested core files/,
  );
});

test("external evidence binds every lifecycle result to exact source, package, and host", () => {
  const app = promotion.apps[0];
  const lifecycle = Object.fromEntries([
    "package_inspection",
    "permission_denial",
    "activation",
    "representative_action",
    "restart",
    "update_data_preservation",
    "disable_enable",
    "keep_data_uninstall",
    "purge_data_uninstall",
  ].map((name) => [name, { status: "passed", observation: `${name} observed in the retained run log` }]));
  const evidence = {
    format_version: 1,
    app: { id: app.id, version: app.package_version },
    source: { repository: app.repository, commit: app.source_commit, clean: true },
    package: { digest: app.package_digest },
    host: { version: hostVersion, commit: coreCommit },
    run: {
      workflow_url: `${app.repository}/actions/runs/1`,
      tested_at: "2026-08-03T12:00:00Z",
      platforms: ["ubuntu-22.04"],
    },
    extension_contributions: app.extension_contributions,
    lifecycle,
  };
  assert.doesNotThrow(() => validateExternalEvidence(evidence, app, hostVersion, coreCommit));
  evidence.lifecycle.restart.status = "pending";
  assert.throws(
    () => validateExternalEvidence(evidence, app, hostVersion, coreCommit),
    /restart.*did not pass/,
  );
});
