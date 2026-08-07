import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const COMMIT = /^[0-9a-f]{40}$/;
const SHA256 = /^sha256-[0-9a-f]{64}$/;
const TITLE_PREFIX = "# Kestral ";
const REQUIRED_SECTIONS = [
  "Tested Root",
  "Automated Gates",
  "Dependency Audits",
  "Promoted Apps",
  "Clean-Machine Manual Matrix",
  "Recovery",
  "Performance Baseline And Ceilings",
  "Final Decision",
];
const CHECKLISTS = {
  "Automated Gates": [
    "Core Rust, MCP, host, frontend, provider-worker, and split-mode tests",
    "Rust formatting and strict workspace lints",
    "Release version, promoted-app contract, and external-app isolation checks",
    "Documentation build and internal-link check",
    "Reproducible tracked-output checks after generation and packaging",
    "Release matrix assembly and artifact checksum checks",
  ],
  "Dependency Audits": [
    "Rust dependency audit (`cargo audit`)",
    "Frontend dependency audit (`npm audit --audit-level=high`)",
    "Third-party notices generation and tracked-output verification",
  ],
  "Clean-Machine Manual Matrix": [
    "Standard-account Windows installer install, first launch, restart, and uninstall",
    "Windows portable archive and no-elevation behavior",
    "Linux AppImage, `.deb`, backend archive, and removal behavior",
    "Split browser client over HTTPS, VPN, or encrypted tunnel",
    "Chat without Kestral Pi, then optional Kestral Pi install and one agent action",
    "Clean profile startup, provider setup with disposable credentials, and app navigation",
  ],
  Recovery: [
    "Forced termination during an invocation followed by restart",
    "Interrupted Run and pending-operation recovery",
    "Profile, host-owned, and app-owned data preservation after restart/update",
    "Corrupt-state refusal is visible and does not widen authority or destroy the original",
  ],
};

function fail(message) {
  throw new Error(`release evidence: ${message}`);
}

function sectionBody(report, heading, nextHeading) {
  const start = report.indexOf(`## ${heading}\n`);
  const end = nextHeading ? report.indexOf(`## ${nextHeading}\n`, start) : report.length;
  if (start < 0 || end < 0) fail(`missing section '${heading}'`);
  return report.slice(start + heading.length + 4, end);
}

function exactSectionStructure(report, version) {
  if (!report.startsWith(`${TITLE_PREFIX}v${version} Release Evidence\n`)) {
    fail(`title must be '# Kestral v${version} Release Evidence'`);
  }
  if (!report.endsWith("\n")) fail("must end with a newline");
  if (report.includes("\r")) fail("must use LF line endings");
  if (/^#{3,}\s/m.test(report)) fail("nested headings are not allowed");
  const topLevelHeadings = [...report.matchAll(/^# (.+)$/gm)].map((match) => match[1]);
  if (JSON.stringify(topLevelHeadings) !== JSON.stringify([`Kestral v${version} Release Evidence`])) {
    fail("top-level headings differ from the required title");
  }
  const headings = [...report.matchAll(/^## (.+)$/gm)].map((match) => match[1]);
  if (JSON.stringify(headings) !== JSON.stringify(REQUIRED_SECTIONS)) {
    fail(`sections differ: expected ${REQUIRED_SECTIONS.join(", ")}; found ${headings.join(", ")}`);
  }
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function fieldValue(body, label) {
  const matches = [...body.matchAll(new RegExp(`^- ${escapeRegExp(label)}: (.+)$`, "gm"))];
  if (matches.length !== 1) fail(`section field '${label}' must occur exactly once`);
  const value = matches[0][1].trim();
  if (value.length === 0) fail(`section field '${label}' must not be empty`);
  return value.replace(/^`|`$/g, "");
}

function validateChecklist(body, labels, section) {
  const expected = new Set(labels);
  for (const line of body.split("\n").filter((line) => /^- \[[ xX]\] /.test(line))) {
    const label = line.match(/^- \[[ xX]\] (.+?): /)?.[1];
    if (!label || !expected.has(label)) fail(`${section} contains an unexpected checklist item`);
  }
  for (const label of labels) {
    const matches = [...body.matchAll(new RegExp(`^- \\[([ xX])\\] ${escapeRegExp(label)}: (.+)$`, "gm"))];
    if (matches.length !== 1) fail(`${section} check '${label}' must occur exactly once`);
    if (matches[0][2].trim().length === 0) fail(`${section} check '${label}' must have a result`);
  }
}

function tableCells(line, section) {
  if (!/^\|.*\|$/.test(line)) fail(`${section} contains a malformed table row`);
  return line.slice(1, -1).split("|").map((cell) => cell.trim());
}

function validatePromotedApps(body, promotion) {
  const lines = body.split("\n").filter((line) => line.startsWith("|"));
  const header = ["App ID", "Source commit", "Package digest", "Evidence asset", "Lifecycle result"];
  if (lines.length !== promotion.apps.length + 2) fail("promoted app table has the wrong number of rows");
  if (JSON.stringify(tableCells(lines[0], "Promoted Apps")) !== JSON.stringify(header)) {
    fail("promoted app table header differs");
  }
  if (JSON.stringify(tableCells(lines[1], "Promoted Apps")) !== JSON.stringify(["---", "---", "---", "---", "---"])) {
    fail("promoted app table separator differs");
  }
  const seen = new Set();
  promotion.apps.forEach((app, index) => {
    const cells = tableCells(lines[index + 2], "Promoted Apps");
    if (cells.length !== header.length) fail(`promoted app row ${index} has the wrong number of fields`);
    const id = cells[0].replace(/^`|`$/g, "");
    const source = cells[1].replace(/^`|`$/g, "");
    const digest = cells[2].replace(/^`|`$/g, "");
    if (id !== app.id || source !== app.source_commit || digest !== app.package_digest) {
      fail(`promoted app row ${index} does not match ${app.id}`);
    }
    if (seen.has(id)) fail(`promoted app ${id} is duplicated`);
    seen.add(id);
    if (cells[3].length === 0 || cells[4].length === 0) fail(`promoted app ${id} has an empty result`);
    if (!SHA256.test(app.package_digest)) fail(`promoted app ${id} has an invalid package digest`);
  });
}

export function validateEvidenceReport(report, promotion, hostVersion, { requireComplete = false, releaseCommit = null } = {}) {
  if (typeof report !== "string") fail("document must be text");
  exactSectionStructure(report, hostVersion);

  const sections = Object.fromEntries(REQUIRED_SECTIONS.map((heading, index) => [
    heading,
    sectionBody(report, heading, REQUIRED_SECTIONS[index + 1]),
  ]));
  if (fieldValue(sections["Tested Root"], "Release version") !== hostVersion) {
    fail("tested root release version does not match the host version");
  }
  const testedCoreCommit = fieldValue(sections["Tested Root"], "Tested core commit");
  if (testedCoreCommit !== "PENDING" && !COMMIT.test(testedCoreCommit)) {
    fail("tested core commit must be PENDING or a lowercase full Git commit");
  }
  for (const field of ["Candidate source tree", "Executable/build source freeze"]) {
    const value = fieldValue(sections["Tested Root"], field);
    if (value !== "PENDING" && value.trim().length === 0) fail(`tested root field '${field}' is invalid`);
  }

  for (const [section, labels] of Object.entries(CHECKLISTS)) validateChecklist(sections[section], labels, section);
  validatePromotedApps(sections["Promoted Apps"], promotion);
  for (const field of [
    "Startup to first useful result baseline / ceiling",
    "Idle memory baseline / ceiling",
    "Installed footprint baseline / ceiling",
    "Worker cost per representative invocation baseline / ceiling",
  ]) fieldValue(sections["Performance Baseline And Ceilings"], field);

  const decision = fieldValue(sections["Final Decision"], "Decision (`APPROVE` or `HOLD`)");
  fieldValue(sections["Final Decision"], "Release owner and date");
  fieldValue(sections["Final Decision"], "Remaining blockers or accepted limitations");
  if (releaseCommit !== null && !COMMIT.test(releaseCommit)) fail("release commit must be a lowercase full Git commit");
  if (requireComplete) {
    if (report.includes("PENDING")) fail("complete mode does not allow PENDING markers");
    if (/^- \[ \] /m.test(report)) fail("complete mode does not allow unchecked boxes");
    if (!COMMIT.test(testedCoreCommit)) fail("complete mode requires a full tested core commit");
    if (promotion.tested_core_commit !== testedCoreCommit) {
      fail("report tested core commit must equal promoted-apps tested_core_commit");
    }
    if (!/^(?:clean|verified|passed)$/i.test(fieldValue(sections["Tested Root"], "Candidate source tree"))) {
      fail("complete mode requires a clean candidate source tree");
    }
    if (!/^(?:frozen|verified|passed)$/i.test(fieldValue(sections["Tested Root"], "Executable/build source freeze"))) {
      fail("complete mode requires an explicit executable/build source freeze");
    }
    if (decision !== "APPROVE") fail("complete mode requires final decision APPROVE");
    if (!releaseCommit) fail("complete mode requires --release-commit with a full 40-hex commit");
  }
  return { testedCoreCommit, decision };
}

export function parseArguments(args) {
  let requireComplete = false;
  let releaseCommit = null;
  const unknown = [];
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === "--require-complete") {
      requireComplete = true;
    } else if (argument === "--release-commit") {
      releaseCommit = args[index + 1] ?? null;
      index += 1;
    } else {
      unknown.push(argument);
    }
  }
  if (unknown.length > 0) fail(`unknown arguments: ${unknown.join(", ")}`);
  if (releaseCommit === null && args.includes("--release-commit")) fail("--release-commit requires a value");
  if (releaseCommit !== null && !COMMIT.test(releaseCommit)) fail("release commit must be a lowercase full Git commit");
  if (requireComplete && releaseCommit === null) fail("--require-complete requires --release-commit with a full 40-hex commit");
  return { requireComplete, releaseCommit };
}

async function main() {
  const { requireComplete, releaseCommit } = parseArguments(process.argv.slice(2));
  const [report, promotion, packageDocument] = await Promise.all([
    readFile("release/v0.1.0-alpha.1-evidence.md", "utf8"),
    readFile("release/promoted-apps.json", "utf8").then(JSON.parse),
    readFile("host/package.json", "utf8").then(JSON.parse),
  ]);
  const result = validateEvidenceReport(report, promotion, packageDocument.version, { requireComplete, releaseCommit });
  console.log(`Validated release evidence structure for ${packageDocument.version}${requireComplete ? ` at tested core ${result.testedCoreCommit}` : " (pending allowed)"}.`);
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) await main();
