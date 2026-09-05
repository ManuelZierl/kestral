import { createHash } from "node:crypto";
import { cp, lstat, mkdir, mkdtemp, readdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

async function filesUnder(root, current = root) {
  const result = [];
  for (const entry of await readdir(current, { withFileTypes: true })) {
    const path = join(current, entry.name);
    if (entry.isSymbolicLink()) throw new Error(`symbolic links are not supported: ${relative(root, path)}`);
    if (entry.isDirectory()) result.push(...await filesUnder(root, path));
    else if (entry.isFile()) result.push(path);
    else throw new Error(`unsupported filesystem entry: ${relative(root, path)}`);
  }
  return result.sort();
}

function assertSourceManifest(manifest) {
  if (manifest?.format_version !== 1) throw new Error("src/app.json must use format_version 1");
  if (manifest?.backend?.kind !== "none") throw new Error("this backend-free scaffold requires backend.kind = 'none'");
  if (manifest?.integrity?.algorithm !== "sha256") throw new Error("src/app.json must use SHA-256 integrity");
  if (!Array.isArray(manifest?.manifest?.surfaces) || manifest.manifest.surfaces.length === 0) {
    throw new Error("src/app.json must declare at least one surface");
  }
  for (const surface of manifest.manifest.surfaces) {
    if (typeof surface?.ui?.entry !== "string" || !surface.ui.entry.startsWith("ui/")) {
      throw new Error("every scaffold surface must bind an entry below ui/");
    }
  }
}

async function exists(path) {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

export async function buildPackage(projectRoot, { copyUi = cp } = {}) {
  const root = resolve(projectRoot);
  const source = join(root, "src");
  const destination = join(root, "dist");
  const manifest = JSON.parse(await readFile(join(source, "app.json"), "utf8"));
  assertSourceManifest(manifest);
  const project = JSON.parse(await readFile(join(root, "package.json"), "utf8"));
  if (typeof project.version !== "string" || project.version.length === 0) {
    throw new Error("package.json must declare the app version");
  }
  manifest.version = project.version;

  const uiSource = join(source, "ui");
  const transaction = await mkdtemp(join(root, ".dist-build-"));
  const staging = join(transaction, "next");
  const backup = join(transaction, "previous");
  let movedExisting = false;
  let preserveTransaction = false;
  try {
    await mkdir(join(staging, "ui"), { recursive: true });
    await copyUi(uiSource, join(staging, "ui"), { recursive: true });

    // Hash the staged bytes, not the source paths. An editor may save while a
    // build is running; the emitted manifest must always describe the exact
    // payload that is moved into dist/.
    const stagedUi = join(staging, "ui");
    const uiFiles = await filesUnder(stagedUi);
    if (uiFiles.length === 0) throw new Error("src/ui must contain at least one file");
    const assets = {};
    for (const path of uiFiles) {
      const packagePath = `ui/${relative(stagedUi, path).split(sep).join("/")}`;
      const bytes = await readFile(path);
      assets[packagePath] = `sha256-${createHash("sha256").update(bytes).digest("hex")}`;
    }
    for (const surface of manifest.manifest.surfaces) {
      if (!Object.hasOwn(assets, surface.ui.entry)) throw new Error(`surface entry is missing: ${surface.ui.entry}`);
    }
    manifest.integrity.assets = assets;
    await writeFile(join(staging, "app.json"), `${JSON.stringify(manifest, null, 2)}\n`, "utf8");

    if (await exists(destination)) {
      await rename(destination, backup);
      movedExisting = true;
    }
    try {
      await rename(staging, destination);
    } catch (error) {
      if (movedExisting) {
        try {
          await rename(backup, destination);
        } catch (restoreError) {
          // Never delete the previous dist if another process occupied the
          // destination during the swap. Leave this invocation's private
          // transaction in place and report where recovery bytes remain.
          preserveTransaction = true;
          throw new AggregateError(
            [error, restoreError],
            `could not install the new dist or restore the previous one; previous dist remains at ${backup}`,
          );
        }
      }
      throw error;
    }
    if (movedExisting) await rm(backup, { recursive: true });
  } finally {
    if (!preserveTransaction) await rm(transaction, { recursive: true, force: true });
  }
  return destination;
}

async function main() {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const destination = await buildPackage(root);
  console.log(`Built ${destination}`);
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  await main();
}
