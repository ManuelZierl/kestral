import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outputPath = resolve(repositoryRoot, process.argv[2] ?? "THIRD-PARTY-NOTICES.txt");
const licenseFilePattern = /^(licen[cs]e|copying|notice|copyright)(\..*)?$/i;

function compareText(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}

function normalize(value) {
  return value.replace(/\r\n/g, "\n").trim();
}

function repositoryUrl(repository) {
  if (typeof repository === "string") return repository;
  return repository?.url ?? "not declared";
}

function licenseFiles(packageDirectory, declaredLicenseFile) {
  const candidates = new Set();
  if (declaredLicenseFile) candidates.add(resolve(packageDirectory, declaredLicenseFile));
  for (const name of readdirSync(packageDirectory)) {
    const path = join(packageDirectory, name);
    if (licenseFilePattern.test(name) && statSync(path).isFile()) candidates.add(path);
  }
  return [...candidates]
    .filter(existsSync)
    .sort(compareText)
    .map((path) => ({ name: relative(packageDirectory, path).replaceAll("\\", "/"), text: normalize(readFileSync(path, "utf8")) }))
    .filter(({ text }) => text.length > 0);
}

function npmPackages(project) {
  const projectRoot = join(repositoryRoot, project);
  const lock = JSON.parse(readFileSync(join(projectRoot, "package-lock.json"), "utf8"));
  const packages = [];

  for (const [packagePath, lockEntry] of Object.entries(lock.packages)) {
    if (!packagePath.includes("node_modules/") || lockEntry.link) continue;
    const packageDirectory = join(projectRoot, packagePath);
    if (!existsSync(packageDirectory)) continue;

    const manifest = JSON.parse(readFileSync(join(packageDirectory, "package.json"), "utf8"));
    const license = typeof manifest.license === "string" ? manifest.license : lockEntry.license;
    if (!license) throw new Error(`Missing npm license metadata: ${manifest.name}@${manifest.version}`);
    packages.push({
      ecosystem: `npm (${project})`,
      name: manifest.name,
      version: manifest.version,
      license,
      source: repositoryUrl(manifest.repository),
      files: licenseFiles(packageDirectory, manifest.licenseFile),
    });
  }
  return packages;
}

function cargoPackages() {
  const metadata = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--locked"], {
      cwd: repositoryRoot,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    }),
  );

  return metadata.packages
    .filter(({ source }) => source !== null)
    .map((manifest) => {
      const license = manifest.license;
      if (!license && !manifest.license_file) {
        throw new Error(`Missing Cargo license metadata: ${manifest.name}@${manifest.version}`);
      }
      const packageDirectory = dirname(manifest.manifest_path);
      return {
        ecosystem: "Cargo",
        name: manifest.name,
        version: manifest.version,
        license: license ?? `SEE LICENSE FILE ${manifest.license_file}`,
        source: manifest.repository ?? manifest.source,
        files: licenseFiles(packageDirectory, manifest.license_file),
      };
    });
}

const packagesById = new Map();
for (const dependency of [...cargoPackages(), ...npmPackages("host")]) {
  const id = `${dependency.ecosystem}:${dependency.name}@${dependency.version}`;
  const existing = packagesById.get(id);
  if (!existing || dependency.files.length > existing.files.length) packagesById.set(id, dependency);
}

const dependencies = [...packagesById.values()].sort((left, right) =>
  compareText(`${left.ecosystem}:${left.name}@${left.version}`, `${right.ecosystem}:${right.name}@${right.version}`),
);
const textsByHash = new Map();
for (const dependency of dependencies) {
  for (const file of dependency.files) {
    const hash = createHash("sha256").update(file.text).digest("hex");
    const entry = textsByHash.get(hash) ?? { text: file.text, packages: [] };
    entry.packages.push(`${dependency.ecosystem}: ${dependency.name}@${dependency.version} (${file.name})`);
    textsByHash.set(hash, entry);
  }
}

const lines = [
  "KESTRAL THIRD-PARTY NOTICES",
  "===========================",
  "",
  "Generated from Cargo metadata and installed npm packages. Package license",
  "metadata is listed for every dependency; license and notice files distributed",
  "with those packages are reproduced below. This file is informational and does",
  "not replace the licenses that govern the corresponding software.",
  "",
  "DEPENDENCY INVENTORY",
  "--------------------",
];

for (const dependency of dependencies) {
  lines.push(
    "",
    `${dependency.ecosystem}: ${dependency.name}@${dependency.version}`,
    `License: ${dependency.license}`,
    `Source metadata: ${dependency.source}`,
    `Bundled license files: ${dependency.files.map(({ name }) => name).join(", ") || "none distributed in package"}`,
  );
}

lines.push("", "LICENSE AND NOTICE TEXTS", "------------------------");
for (const [hash, entry] of [...textsByHash.entries()].sort(([left], [right]) => compareText(left, right))) {
  lines.push("", `SHA-256: ${hash}`, "Used by:");
  for (const packageName of entry.packages.sort(compareText)) lines.push(`- ${packageName}`);
  lines.push("", entry.text);
}

writeFileSync(outputPath, `${lines.join("\n")}\n`, "utf8");
console.log(`Wrote ${relative(repositoryRoot, outputPath)} for ${dependencies.length} dependencies and ${textsByHash.size} unique texts.`);
