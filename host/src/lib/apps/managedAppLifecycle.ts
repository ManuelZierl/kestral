import type {
  InstalledApp,
  ManagedAppOperation,
  PackageInspection,
  SignatureStatus,
} from "$lib/api";

const semverPattern =
  /^(?<major>0|[1-9]\d*)\.(?<minor>0|[1-9]\d*)\.(?<patch>0|[1-9]\d*)(?:-(?<prerelease>[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

interface ParsedVersion {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[];
}

function parseVersion(value: string): ParsedVersion | null {
  const match = semverPattern.exec(value);
  if (!match?.groups) return null;
  return {
    major: Number(match.groups.major),
    minor: Number(match.groups.minor),
    patch: Number(match.groups.patch),
    prerelease: match.groups.prerelease ? match.groups.prerelease.split(".") : [],
  };
}

function comparePrerelease(left: string[], right: string[]): number {
  if (left.length === 0 && right.length === 0) return 0;
  if (left.length === 0) return 1;
  if (right.length === 0) return -1;
  const length = Math.max(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    const leftId = left[index];
    const rightId = right[index];
    if (leftId === undefined) return -1;
    if (rightId === undefined) return 1;
    const leftNumeric = /^\d+$/.test(leftId);
    const rightNumeric = /^\d+$/.test(rightId);
    if (leftNumeric && rightNumeric) {
      const delta = Number(leftId) - Number(rightId);
      if (delta !== 0) return delta;
      continue;
    }
    if (leftNumeric !== rightNumeric) return leftNumeric ? -1 : 1;
    const delta = leftId.localeCompare(rightId);
    if (delta !== 0) return delta;
  }
  return 0;
}

export function compareManagedVersions(current: string, target: string): "higher" | "lower" | "same" {
  const left = parseVersion(current);
  const right = parseVersion(target);
  if (!left || !right) return "same";
  if (left.major !== right.major) return left.major < right.major ? "higher" : "lower";
  if (left.minor !== right.minor) return left.minor < right.minor ? "higher" : "lower";
  if (left.patch !== right.patch) return left.patch < right.patch ? "higher" : "lower";
  const prerelease = comparePrerelease(left.prerelease, right.prerelease);
  if (prerelease < 0) return "higher";
  if (prerelease > 0) return "lower";
  return "same";
}

export function plannedOperationForPackage(
  inspection: PackageInspection,
  installedApp: InstalledApp | undefined,
): ManagedAppOperation {
  if (!installedApp) return "install";
  if (installedApp.content_hash === inspection.package_digest) return "reinstall";
  const relation = compareManagedVersions(installedApp.manifest.version, inspection.version);
  if (relation === "higher") return "update";
  if (relation === "lower") return "downgrade";
  return "version-conflict";
}

export function managedAppOperationLabel(operation: ManagedAppOperation): string {
  switch (operation) {
    case "install":
      return "Install app";
    case "update":
      return "Update app";
    case "reinstall":
      return "Reinstall app";
    case "version-conflict":
      return "Resolve version conflict";
    case "downgrade":
      return "Downgrade app";
    case "revert":
      return "Revert app version";
  }
}

export function signatureStatusLabel(signature: SignatureStatus): string {
  switch (signature.kind) {
    case "unsigned":
      return "Unsigned";
    case "valid-unknown-key":
      return "Valid, unknown key";
    case "trusted":
      return "Trusted";
    case "invalid":
      return "Invalid";
    case "revoked":
      return "Revoked";
  }
}

export function signatureStatusExplanation(signature: SignatureStatus): string {
  switch (signature.kind) {
    case "unsigned":
      return "No signature file was found.";
    case "valid-unknown-key":
      return "The signature verified against this package, but the signing key is not trusted yet.";
    case "trusted":
      return "The signature verified and the key is trusted for this scope.";
    case "invalid":
      return "The signature failed verification or could not be parsed.";
    case "revoked":
      return "The signature verified, but the key is revoked for this scope.";
  }
}

export function signatureTrustNote(): string {
  return "Signatures prove publisher-key continuity for this package, not that the app is safe.";
}
