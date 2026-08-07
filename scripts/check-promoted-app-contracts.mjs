import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const SHA256 = /^sha256-[0-9a-f]{64}$/;
const COMMIT = /^[0-9a-f]{40}$/;
const GITHUB_REPOSITORY = /^https:\/\/github\.com\/([^/]+)\/([^/]+)$/;
const DATE_TIME = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/;
const LIFECYCLE_CHECKS = [
  "package_inspection",
  "permission_denial",
  "activation",
  "representative_action",
  "restart",
  "update_data_preservation",
  "disable_enable",
  "keep_data_uninstall",
  "purge_data_uninstall",
];

function object(value, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value;
}

function exactKeys(value, expected, label) {
  const actual = Object.keys(object(value, label)).sort();
  const wanted = [...expected].sort();
  if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
    throw new Error(`${label} fields differ: expected ${wanted.join(", ")}; found ${actual.join(", ")}`);
  }
}

function nonEmpty(value, label) {
  if (typeof value !== "string" || value.length === 0) throw new Error(`${label} must be a non-empty string`);
}

function extensionKey(target, point) {
  return `${target}/${point}`;
}

function providerContracts(document) {
  exactKeys(document, ["format_version", "providers"], "host extension contracts");
  if (document.format_version !== 1) throw new Error("unsupported host extension contract format");
  if (!Array.isArray(document.providers)) throw new Error("host extension contracts providers must be an array");
  const contracts = new Map();
  for (const [providerIndex, provider] of document.providers.entries()) {
    const label = `provider ${providerIndex}`;
    exactKeys(provider, ["app_id", "extension_points"], label);
    nonEmpty(provider.app_id, `${label}.app_id`);
    if (!Array.isArray(provider.extension_points)) throw new Error(`${label}.extension_points must be an array`);
    for (const [pointIndex, point] of provider.extension_points.entries()) {
      const pointLabel = `${label}.extension_points[${pointIndex}]`;
      exactKeys(point, ["name", "contract_version"], pointLabel);
      nonEmpty(point.name, `${pointLabel}.name`);
      if (!Number.isSafeInteger(point.contract_version) || point.contract_version < 1) {
        throw new Error(`${pointLabel}.contract_version must be a positive integer`);
      }
      const key = extensionKey(provider.app_id, point.name);
      if (contracts.has(key)) throw new Error(`duplicate host extension contract ${key}`);
      contracts.set(key, point.contract_version);
    }
  }
  return contracts;
}

function validateContribution(contribution, label, contracts) {
  exactKeys(contribution, ["target_app", "extension_point", "contract_version"], label);
  nonEmpty(contribution.target_app, `${label}.target_app`);
  nonEmpty(contribution.extension_point, `${label}.extension_point`);
  if (!Number.isSafeInteger(contribution.contract_version) || contribution.contract_version < 1) {
    throw new Error(`${label}.contract_version must be a positive integer`);
  }
  const key = extensionKey(contribution.target_app, contribution.extension_point);
  const providerVersion = contracts.get(key);
  if (providerVersion === undefined) throw new Error(`${label} targets missing host extension contract ${key}`);
  if (providerVersion !== contribution.contract_version) {
    throw new Error(`${label} requires ${key} v${contribution.contract_version}, but the host provides v${providerVersion}`);
  }
}

export function validatePromotionDocument(document, contractDocument, hostVersion, { requireEvidence = false } = {}) {
  exactKeys(document, ["format_version", "host_version", "tested_core_commit", "apps"], "promoted apps");
  if (document.format_version !== 1) throw new Error("unsupported promoted-app record format");
  if (document.host_version !== hostVersion) {
    throw new Error(`promoted-app host version ${document.host_version} does not match product version ${hostVersion}`);
  }
  if (!Array.isArray(document.apps) || document.apps.length === 0) {
    throw new Error("promoted apps must contain at least one app");
  }
  if (document.tested_core_commit !== null && !COMMIT.test(document.tested_core_commit)) {
    throw new Error("promoted apps tested_core_commit must be null or a lowercase full Git commit");
  }

  const contracts = providerContracts(contractDocument);
  const ids = new Set();
  const repositories = new Set();
  for (const [index, app] of document.apps.entries()) {
    const label = `promoted apps[${index}]`;
    exactKeys(app, [
      "id",
      "display_name",
      "repository",
      "source_commit",
      "package_version",
      "package_digest",
      "minimum_host_version",
      "tested_host_version",
      "extension_contributions",
      "evidence_url",
      "evidence_sha256",
    ], label);
    for (const field of ["id", "display_name", "package_version"]) nonEmpty(app[field], `${label}.${field}`);
    if (!GITHUB_REPOSITORY.test(app.repository)) throw new Error(`${label}.repository must be a canonical HTTPS GitHub repository URL`);
    if (!COMMIT.test(app.source_commit)) throw new Error(`${label}.source_commit must be a lowercase full Git commit`);
    if (!SHA256.test(app.package_digest)) throw new Error(`${label}.package_digest must be a sha256 digest`);
    if (app.minimum_host_version !== hostVersion || app.tested_host_version !== hostVersion) {
      throw new Error(`${label} must declare and test exact host version ${hostVersion}`);
    }
    if (!Array.isArray(app.extension_contributions)) throw new Error(`${label}.extension_contributions must be an array`);
    const contributionKeys = new Set();
    app.extension_contributions.forEach((contribution, contributionIndex) => {
      validateContribution(contribution, `${label}.extension_contributions[${contributionIndex}]`, contracts);
      const key = extensionKey(contribution.target_app, contribution.extension_point);
      if (contributionKeys.has(key)) throw new Error(`${label} contains duplicate extension contribution ${key}`);
      contributionKeys.add(key);
    });
    if (ids.has(app.id)) throw new Error(`duplicate promoted app id ${app.id}`);
    if (repositories.has(app.repository)) throw new Error(`duplicate promoted app repository ${app.repository}`);
    ids.add(app.id);
    repositories.add(app.repository);

    const hasEvidenceUrl = app.evidence_url !== null;
    const hasEvidenceDigest = app.evidence_sha256 !== null;
    if (hasEvidenceUrl !== hasEvidenceDigest) throw new Error(`${label} must pin both evidence_url and evidence_sha256 or neither`);
    if (hasEvidenceUrl) {
      if (typeof app.evidence_url !== "string" || !app.evidence_url.startsWith(`${app.repository}/releases/download/`)) {
        throw new Error(`${label}.evidence_url must be a release asset from the declared app repository`);
      }
      if (!SHA256.test(app.evidence_sha256)) throw new Error(`${label}.evidence_sha256 must be a sha256 digest`);
    } else if (requireEvidence) {
      throw new Error(`${app.id} has no pinned lifecycle evidence`);
    }
  }
  if (requireEvidence && document.tested_core_commit === null) {
    throw new Error("promoted apps do not pin the tested core commit");
  }
  return document.apps;
}

export function validateExternalEvidence(evidence, promotedApp, hostVersion, coreCommit) {
  exactKeys(evidence, ["format_version", "app", "source", "package", "host", "run", "extension_contributions", "lifecycle"], `${promotedApp.id} evidence`);
  if (evidence.format_version !== 1) throw new Error(`${promotedApp.id} evidence has unsupported format`);
  exactKeys(evidence.app, ["id", "version"], `${promotedApp.id} evidence.app`);
  exactKeys(evidence.source, ["repository", "commit", "clean"], `${promotedApp.id} evidence.source`);
  exactKeys(evidence.package, ["digest"], `${promotedApp.id} evidence.package`);
  exactKeys(evidence.host, ["version", "commit"], `${promotedApp.id} evidence.host`);
  exactKeys(evidence.run, ["workflow_url", "tested_at", "platforms"], `${promotedApp.id} evidence.run`);
  exactKeys(evidence.lifecycle, LIFECYCLE_CHECKS, `${promotedApp.id} evidence.lifecycle`);

  const expected = {
    id: promotedApp.id,
    version: promotedApp.package_version,
    repository: promotedApp.repository,
    sourceCommit: promotedApp.source_commit,
    packageDigest: promotedApp.package_digest,
  };
  if (evidence.app.id !== expected.id || evidence.app.version !== expected.version) throw new Error(`${promotedApp.id} evidence app identity differs from the promotion record`);
  if (evidence.source.repository !== expected.repository || evidence.source.commit !== expected.sourceCommit) throw new Error(`${promotedApp.id} evidence source differs from the promotion record`);
  if (evidence.source.clean !== true) throw new Error(`${promotedApp.id} evidence was not produced from a clean source commit`);
  if (evidence.package.digest !== expected.packageDigest) throw new Error(`${promotedApp.id} evidence package digest differs from the promotion record`);
  if (evidence.host.version !== hostVersion || evidence.host.commit !== coreCommit) throw new Error(`${promotedApp.id} evidence was not produced against exact host ${hostVersion} at ${coreCommit}`);
  if (typeof evidence.run.workflow_url !== "string" || !evidence.run.workflow_url.startsWith(`${promotedApp.repository}/actions/runs/`)) throw new Error(`${promotedApp.id} evidence run must identify an Actions run in the declared app repository`);
  if (typeof evidence.run.tested_at !== "string" || !DATE_TIME.test(evidence.run.tested_at) || Number.isNaN(Date.parse(evidence.run.tested_at))) throw new Error(`${promotedApp.id} evidence run must contain an ISO timestamp`);
  if (!Array.isArray(evidence.run.platforms) || evidence.run.platforms.length === 0 || evidence.run.platforms.some((platform) => typeof platform !== "string" || platform.length === 0)) {
    throw new Error(`${promotedApp.id} evidence run must name at least one tested platform`);
  }
  if (new Set(evidence.run.platforms).size !== evidence.run.platforms.length) throw new Error(`${promotedApp.id} evidence run contains duplicate platforms`);
  if (JSON.stringify(evidence.extension_contributions) !== JSON.stringify(promotedApp.extension_contributions)) {
    throw new Error(`${promotedApp.id} evidence extension contributions differ from the promotion record`);
  }
  for (const check of LIFECYCLE_CHECKS) {
    const result = evidence.lifecycle[check];
    exactKeys(result, ["status", "observation"], `${promotedApp.id} evidence.lifecycle.${check}`);
    if (result.status !== "passed") throw new Error(`${promotedApp.id} lifecycle check '${check}' did not pass`);
    nonEmpty(result.observation, `${promotedApp.id} evidence.lifecycle.${check}.observation`);
  }
}

async function verifyRemoteCommit(app) {
  const [, owner, repository] = app.repository.match(GITHUB_REPOSITORY);
  const headers = { Accept: "application/vnd.github+json", "User-Agent": "kestral-release-contract" };
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  const response = await fetch(`https://api.github.com/repos/${owner}/${repository}/commits/${app.source_commit}`, { headers });
  if (!response.ok) throw new Error(`${app.id} source commit is not available from ${app.repository}: HTTP ${response.status}`);
  const body = await response.json();
  if (body.sha !== app.source_commit) throw new Error(`${app.id} remote commit response did not match the pinned commit`);
}

async function verifyEvidence(app, hostVersion, coreCommit) {
  const response = await fetch(app.evidence_url, { headers: { "User-Agent": "kestral-release-contract" } });
  if (!response.ok) throw new Error(`${app.id} evidence download failed: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  const digest = `sha256-${createHash("sha256").update(bytes).digest("hex")}`;
  if (digest !== app.evidence_sha256) throw new Error(`${app.id} evidence digest differs from the pinned digest`);
  let evidence;
  try {
    evidence = JSON.parse(bytes.toString("utf8"));
  } catch (error) {
    throw new Error(`${app.id} evidence is not valid JSON: ${error.message}`);
  }
  validateExternalEvidence(evidence, app, hostVersion, coreCommit);
}

function verifyReleaseCommit(testedCoreCommit, releaseCommit) {
  try {
    execFileSync("git", ["merge-base", "--is-ancestor", testedCoreCommit, releaseCommit]);
  } catch {
    throw new Error(`tested core commit ${testedCoreCommit} is not an ancestor of release commit ${releaseCommit}`);
  }
  const changed = execFileSync(
    "git",
    ["diff", "--name-only", testedCoreCommit, releaseCommit],
    { encoding: "utf8" },
  )
    .split(/\r?\n/)
    .filter(Boolean);
  validateReleaseChangedPaths(changed);
}

export function validateReleaseChangedPaths(changed) {
  // The tested core commit freezes every executable and build input. Only the
  // two self-contained release metadata files may be added after evidence.
  const allowed = new Set([
    "release/promoted-apps.json",
    "release/v0.1.0-alpha.1-evidence.md",
  ]);
  const disallowed = changed.filter((path) => !allowed.has(path));
  if (disallowed.length > 0) {
    throw new Error(`release commit changed tested core files after app evidence was collected: ${disallowed.join(", ")}`);
  }
}

async function main() {
  const args = process.argv.slice(2);
  const requireEvidence = args.includes("--require-evidence");
  const verifyRemotes = args.includes("--verify-remotes");
  const releaseCommitIndex = args.indexOf("--release-commit");
  const releaseCommit = releaseCommitIndex >= 0 ? args[releaseCommitIndex + 1] : null;
  const allowedArgs = new Set(["--require-evidence", "--verify-remotes", "--release-commit", releaseCommit]);
  const unknown = args.filter((argument) => !allowedArgs.has(argument));
  if (unknown.length > 0) throw new Error(`unknown arguments: ${unknown.join(", ")}`);
  if (requireEvidence && !COMMIT.test(releaseCommit ?? "")) throw new Error("--require-evidence requires --release-commit with a lowercase full Git commit");

  const [promotionDocument, contractDocument, packageDocument] = await Promise.all([
    readFile("release/promoted-apps.json", "utf8").then(JSON.parse),
    readFile("release/host-extension-contracts.json", "utf8").then(JSON.parse),
    readFile("host/package.json", "utf8").then(JSON.parse),
  ]);
  const apps = validatePromotionDocument(promotionDocument, contractDocument, packageDocument.version, { requireEvidence });
  if (requireEvidence) verifyReleaseCommit(promotionDocument.tested_core_commit, releaseCommit);
  if (verifyRemotes) await Promise.all(apps.map(verifyRemoteCommit));
  if (requireEvidence) await Promise.all(apps.map((app) => verifyEvidence(app, packageDocument.version, promotionDocument.tested_core_commit)));
  console.log(`Validated ${apps.length} promoted app contracts for exact host ${packageDocument.version}${requireEvidence ? " with immutable lifecycle evidence" : ""}.`);
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  await main();
}
