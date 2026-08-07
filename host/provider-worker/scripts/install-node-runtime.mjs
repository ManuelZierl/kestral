import { createHash, randomUUID } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import { chmod, copyFile, mkdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { pipeline } from "node:stream/promises";
import { spawn } from "node:child_process";

const NODE_VERSION = "22.19.0";
const METADATA_VERSION = 1;
const BASE_URL = `https://nodejs.org/dist/v${NODE_VERSION}`;
const workerRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const runtimeDir = join(workerRoot, "runtime");
const executableName = process.platform === "win32" ? "node.exe" : "node";
const PROCESS_TIMEOUT_MS = 120_000;
const DOWNLOAD_TIMEOUT_MS = 120_000;
const MAX_COMMAND_OUTPUT_CHARS = 1024 * 1024;

function archiveFor(platform, arch) {
  if (!["x64", "arm64"].includes(arch) || !["win32", "linux", "darwin"].includes(platform)) {
    throw new Error(`Node runtime packaging does not support ${platform}/${arch}`);
  }
  const platformName = platform === "win32" ? "win" : platform;
  const extension = platform === "win32" ? "zip" : "tar.xz";
  return `node-v${NODE_VERSION}-${platformName}-${arch}.${extension}`;
}

async function sha256(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

async function isFile(path) {
  try {
    return (await stat(path)).isFile();
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

async function run(command, args, options = {}) {
  await new Promise((resolveProcess, reject) => {
    const child = spawn(command, args, { stdio: "inherit", windowsHide: true, ...options });
    let settled = false;
    let timedOut = false;
    const finish = (error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error) reject(error);
      else resolveProcess();
    };
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill();
    }, PROCESS_TIMEOUT_MS);
    child.once("error", finish);
    child.once("exit", (code, signal) => {
      if (timedOut) finish(new Error(`${command} timed out after ${PROCESS_TIMEOUT_MS}ms`));
      else if (code === 0) finish();
      else finish(new Error(`${command} failed with ${signal ? `signal ${signal}` : `exit code ${code}`}`));
    });
  });
}

async function output(command, args) {
  return await new Promise((resolveOutput, reject) => {
    const child = spawn(command, args, { stdio: ["ignore", "pipe", "inherit"], windowsHide: true });
    let stdout = "";
    let settled = false;
    let timedOut = false;
    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error) reject(error);
      else resolveOutput(value);
    };
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill();
    }, PROCESS_TIMEOUT_MS);
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
      if (stdout.length > MAX_COMMAND_OUTPUT_CHARS) {
        child.kill();
        finish(new Error(`${command} produced too much output`));
      }
    });
    child.once("error", finish);
    child.once("exit", (code, signal) => {
      if (timedOut) finish(new Error(`${command} timed out after ${PROCESS_TIMEOUT_MS}ms`));
      else if (code === 0) finish(null, stdout.trim());
      else finish(new Error(`${command} failed with ${signal ? `signal ${signal}` : `exit code ${code}`}`));
    });
  });
}

async function download(url, destination) {
  const response = await fetch(url, {
    redirect: "follow",
    signal: AbortSignal.timeout(DOWNLOAD_TIMEOUT_MS),
  });
  if (!response.ok || !response.body) {
    throw new Error(`Download failed for ${url}: HTTP ${response.status}`);
  }
  await pipeline(response.body, createWriteStream(destination, { flags: "wx" }));
}

function expectedArchiveHash(shasums, archiveName) {
  const matches = shasums.split(/\r?\n/).filter((line) => line.endsWith(`  ${archiveName}`));
  if (matches.length !== 1) throw new Error(`SHASUMS256.txt does not contain exactly one entry for ${archiveName}`);
  const match = /^([a-f0-9]{64})  (.+)$/.exec(matches[0]);
  if (!match || match[2] !== archiveName) throw new Error(`Invalid checksum entry for ${archiveName}`);
  return match[1];
}

async function cacheIsValid(expected) {
  const metadataPath = join(runtimeDir, "install-metadata.json");
  const executablePath = join(runtimeDir, executableName);
  const licensePath = join(runtimeDir, "LICENSE");
  try {
    const metadata = JSON.parse(await readFile(metadataPath, "utf8"));
    if (
      metadata.metadataVersion !== METADATA_VERSION ||
      metadata.nodeVersion !== NODE_VERSION ||
      metadata.platform !== process.platform ||
      metadata.arch !== process.arch ||
      metadata.archive !== expected ||
      !(await isFile(executablePath)) ||
      !(await isFile(licensePath)) ||
      (await sha256(executablePath)) !== metadata.executableSha256 ||
      (await sha256(licensePath)) !== metadata.licenseSha256
    ) return false;
    return await output(executablePath, ["--version"]) === `v${NODE_VERSION}`;
  } catch {
    return false;
  }
}

async function extract(archivePath, destination) {
  await mkdir(destination, { recursive: true });
  if (process.platform === "win32") {
    const script = "& { param($Archive, $Destination) Expand-Archive -LiteralPath $Archive -DestinationPath $Destination -Force }";
    await run("powershell.exe", ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script, archivePath, destination]);
  } else {
    await run("tar", ["-xJf", archivePath, "-C", destination]);
  }
}

async function install() {
  const archiveName = archiveFor(process.platform, process.arch);
  if (await cacheIsValid(archiveName)) {
    console.log(`Node v${NODE_VERSION} runtime is already installed for ${process.platform}/${process.arch}`);
    return;
  }

  const workDir = join(tmpdir(), `app-host-node-${randomUUID()}`);
  const stagingDir = join(workerRoot, `runtime.install-${randomUUID()}`);
  const backupDir = join(workerRoot, `runtime.backup-${randomUUID()}`);
  let movedExistingRuntime = false;
  try {
    await mkdir(workDir, { recursive: false });
    const shasumsPath = join(workDir, "SHASUMS256.txt");
    const archivePath = join(workDir, archiveName);
    const extractedDir = join(workDir, "extracted");
    await download(`${BASE_URL}/SHASUMS256.txt`, shasumsPath);
    await download(`${BASE_URL}/${archiveName}`, archivePath);

    const expectedHash = expectedArchiveHash(await readFile(shasumsPath, "utf8"), archiveName);
    const actualHash = await sha256(archivePath);
    if (actualHash !== expectedHash) {
      throw new Error(`SHA-256 mismatch for ${archiveName}: expected ${expectedHash}, got ${actualHash}`);
    }

    await extract(archivePath, extractedDir);
    const archiveRoot = join(extractedDir, archiveName.replace(/\.(zip|tar\.xz)$/, ""));
    const sourceExecutable = process.platform === "win32"
      ? join(archiveRoot, "node.exe")
      : join(archiveRoot, "bin", "node");
    const sourceLicense = join(archiveRoot, "LICENSE");
    if (!(await isFile(sourceExecutable)) || !(await isFile(sourceLicense))) {
      throw new Error(`${archiveName} is missing the expected Node executable or LICENSE`);
    }

    await mkdir(stagingDir, { recursive: false });
    const stagedExecutable = join(stagingDir, executableName);
    const stagedLicense = join(stagingDir, "LICENSE");
    await copyFile(sourceExecutable, stagedExecutable);
    await copyFile(sourceLicense, stagedLicense);
    if (process.platform !== "win32") await chmod(stagedExecutable, 0o755);
    const version = await output(stagedExecutable, ["--version"]);
    if (version !== `v${NODE_VERSION}`) throw new Error(`Extracted runtime reported ${version || "no version"}`);

    const metadata = {
      metadataVersion: METADATA_VERSION,
      nodeVersion: NODE_VERSION,
      platform: process.platform,
      arch: process.arch,
      archive: archiveName,
      archiveSha256: actualHash,
      executableSha256: await sha256(stagedExecutable),
      licenseSha256: await sha256(stagedLicense),
    };
    await writeFile(join(stagingDir, "install-metadata.json"), `${JSON.stringify(metadata, null, 2)}\n`, { flag: "wx" });

    if (await stat(runtimeDir).then(() => true, (error) => error?.code === "ENOENT" ? false : Promise.reject(error))) {
      await rename(runtimeDir, backupDir);
      movedExistingRuntime = true;
    }
    await rename(stagingDir, runtimeDir);
    await rm(backupDir, { recursive: true, force: true });
    movedExistingRuntime = false;
    console.log(`Installed Node v${NODE_VERSION} for ${process.platform}/${process.arch}`);
  } catch (error) {
    if (movedExistingRuntime && !(await stat(runtimeDir).then(() => true, () => false))) {
      try {
        await rename(backupDir, runtimeDir);
        movedExistingRuntime = false;
      } catch (restoreError) {
        throw new AggregateError([error, restoreError], `Installation failed and the previous runtime remains at ${backupDir}`);
      }
    }
    throw error;
  } finally {
    await rm(workDir, { recursive: true, force: true });
    await rm(stagingDir, { recursive: true, force: true });
    if (!movedExistingRuntime) await rm(backupDir, { recursive: true, force: true });
  }
}

install().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});
