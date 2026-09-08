import { createHash } from "node:crypto";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function sha256(bytes) {
  return `sha256-${createHash("sha256").update(bytes).digest("hex")}`;
}

export async function buildPackage(root = projectRoot) {
  const sourceManifestPath = resolve(root, "src/app.json");
  const sourceUiPath = resolve(root, "src/ui/index.html");
  const distRoot = resolve(root, "dist");
  const distUiPath = resolve(distRoot, "ui/index.html");

  const manifest = JSON.parse(await readFile(sourceManifestPath, "utf8"));
  const ui = await readFile(sourceUiPath);
  manifest.integrity = {
    algorithm: "sha256",
    assets: { "ui/index.html": sha256(ui) },
  };

  await rm(distRoot, { recursive: true, force: true });
  await mkdir(dirname(distUiPath), { recursive: true });
  await writeFile(distUiPath, ui);
  await writeFile(resolve(distRoot, "app.json"), `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  return manifest;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await buildPackage();
}
