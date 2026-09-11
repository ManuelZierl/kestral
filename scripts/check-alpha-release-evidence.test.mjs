import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  parseArguments,
  validateEvidenceReport,
} from "./check-alpha-release-evidence.mjs";

const { version: hostVersion } = JSON.parse(await readFile("host/package.json", "utf8"));
const coreCommit = "a".repeat(40);
const releaseCommit = "b".repeat(40);
const promotion = JSON.parse(await readFile("release/promoted-apps.json", "utf8"));
const contractsReport = await readFile(`release/v${hostVersion}-evidence.md`, "utf8");

function completeReport() {
  return contractsReport
    .replaceAll("PENDING", "recorded")
    .replaceAll("- [ ] ", "- [x] ")
    .replace(/^- Tested core commit: `[^`]+`$/m, `- Tested core commit: \`${coreCommit}\``)
    .replace(/^- Candidate source tree: `[^`]+`$/m, "- Candidate source tree: `clean`")
    .replace(/^- Executable\/build source freeze: `[^`]+`$/m, "- Executable/build source freeze: `frozen`")
    .replace(/^- Decision \(`APPROVE` or `HOLD`\): `[^`]+`$/m, "- Decision (`APPROVE` or `HOLD`): `APPROVE`")
    .replace(/^- Remaining blockers or accepted limitations: .*$/m, "- Remaining blockers or accepted limitations: recorded");
}

test("the current report has the exact required structure", () => {
  const { testedCoreCommit, decision } = validateEvidenceReport(contractsReport, promotion, hostVersion);
  assert.match(testedCoreCommit, /^(?:PENDING|[0-9a-f]{40})$/);
  assert.ok(["PENDING", "HOLD", "APPROVE"].includes(decision));
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

test("complete mode refuses pending markers", () => {
  const pendingReport = completeReport().replace(
    "- Remaining blockers or accepted limitations: recorded",
    "- Remaining blockers or accepted limitations: PENDING",
  );
  assert.throws(
    () => validateEvidenceReport(pendingReport, promotion, hostVersion, { requireComplete: true, releaseCommit }),
    /PENDING markers/,
  );
});

test("complete mode refuses unchecked boxes", () => {
  const uncheckedReport = completeReport().replace("- [x] Core Rust", "- [ ] Core Rust");
  assert.throws(
    () => validateEvidenceReport(uncheckedReport, promotion, hostVersion, { requireComplete: true, releaseCommit }),
    /unchecked boxes/,
  );
});

test("complete mode refuses a hold decision", () => {
  const completePromotion = structuredClone(promotion);
  completePromotion.tested_core_commit = coreCommit;
  const heldReport = completeReport().replace(
    "- Decision (`APPROVE` or `HOLD`): `APPROVE`",
    "- Decision (`APPROVE` or `HOLD`): `HOLD`",
  );
  assert.throws(
    () => validateEvidenceReport(heldReport, completePromotion, hostVersion, { requireComplete: true, releaseCommit }),
    /requires final decision APPROVE/,
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
