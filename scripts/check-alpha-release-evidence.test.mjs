import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  parseArguments,
  validateEvidenceReport,
} from "./check-alpha-release-evidence.mjs";

const hostVersion = "0.1.0-alpha.1";
const coreCommit = "a".repeat(40);
const releaseCommit = "b".repeat(40);
const promotion = JSON.parse(await readFile("release/promoted-apps.json", "utf8"));
const contractsReport = await readFile("release/v0.1.0-alpha.1-evidence.md", "utf8");

function completeReport() {
  return contractsReport
    .replaceAll("PENDING", "recorded")
    .replaceAll("- [ ] ", "- [x] ")
    .replace("- Tested core commit: `recorded`", `- Tested core commit: \`${coreCommit}\``)
    .replace("- Candidate source tree: `recorded`", "- Candidate source tree: `clean`")
    .replace("- Executable/build source freeze: `recorded`", "- Executable/build source freeze: `frozen`")
    .replace("- Decision (`APPROVE` or `HOLD`): `recorded`", "- Decision (`APPROVE` or `HOLD`): `APPROVE`");
}

test("the pending report has the exact required structure", () => {
  assert.deepEqual(
    validateEvidenceReport(contractsReport, promotion, hostVersion),
    { testedCoreCommit: "PENDING", decision: "PENDING" },
  );
});

test("structure drift fails closed", () => {
  assert.throws(
    () => validateEvidenceReport(contractsReport.replace("## Recovery", "## Recovery Notes"), promotion, hostVersion),
    /sections differ/,
  );
  assert.throws(
    () => validateEvidenceReport(contractsReport.replace("| App ID |", "| Package |"), promotion, hostVersion),
    /promoted app table header differs/,
  );
  assert.throws(
    () => validateEvidenceReport(`${contractsReport}\n# Extra\n`, promotion, hostVersion),
    /top-level headings differ/,
  );
});

test("complete mode refuses pending markers and unchecked boxes", () => {
  assert.throws(
    () => validateEvidenceReport(contractsReport, promotion, hostVersion, { requireComplete: true, releaseCommit }),
    /PENDING markers/,
  );
});

test("complete mode binds the report to the promoted tested core and approval", () => {
  const completePromotion = structuredClone(promotion);
  completePromotion.tested_core_commit = coreCommit;
  assert.deepEqual(
    validateEvidenceReport(completeReport(), completePromotion, hostVersion, { requireComplete: true, releaseCommit }),
    { testedCoreCommit: coreCommit, decision: "APPROVE" },
  );
  completePromotion.tested_core_commit = "c".repeat(40);
  assert.throws(
    () => validateEvidenceReport(completeReport(), completePromotion, hostVersion, { requireComplete: true, releaseCommit }),
    /must equal promoted-apps tested_core_commit/,
  );
});

test("complete mode requires a full release commit argument", () => {
  assert.throws(() => parseArguments(["--require-complete"]), /requires --release-commit/);
  assert.throws(() => parseArguments(["--release-commit", "abc"]), /full Git commit/);
  assert.deepEqual(parseArguments(["--require-complete", "--release-commit", releaseCommit]), {
    requireComplete: true,
    releaseCommit,
  });
});
