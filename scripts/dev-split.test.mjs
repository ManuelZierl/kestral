import assert from "node:assert/strict";
import { lstat, mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

import { cleanDevDataDirectory, parseDevSplitOptions } from "./dev-split.mjs";

const home = "/home/tester";
const workingDirectory = "/workspace/host";

test("split development defaults to the localhost origin required by WebAuthn", () => {
  assert.deepEqual(parseDevSplitOptions([], {}, home, workingDirectory), {
    pair: false,
    clean: false,
    help: false,
    frontendPort: 1420,
    backendPort: 4310,
    origin: "http://localhost:1420",
    dataDirectory: "/home/tester/.local/share/kestral-split-dev",
  });
});

test("split development accepts explicit options and cleanup implies pairing", () => {
  assert.deepEqual(
    parseDevSplitOptions([
      "--clean",
      "--frontend-port", "8000",
      "--backend-port", "4311",
      "--origin", "http://127.0.0.1:8000",
      "--data-dir", "profiles/remote-dev",
    ], {}, home, workingDirectory),
    {
      pair: true,
      clean: true,
      help: false,
      frontendPort: 8000,
      backendPort: 4311,
      origin: "http://127.0.0.1:8000",
      dataDirectory: "/workspace/host/profiles/remote-dev",
    },
  );
});

test("split development reads reusable options from the environment", () => {
  assert.deepEqual(parseDevSplitOptions([], {
    KESTRAL_DEV_FRONTEND_PORT: "5173",
    KESTRAL_DEV_BACKEND_PORT: "4312",
    HOST_REMOTE_ORIGIN: "http://localhost:5173",
    KESTRAL_DATA_DIR: "/srv/kestral-dev",
  }, home, workingDirectory), {
    pair: false,
    clean: false,
    help: false,
    frontendPort: 5173,
    backendPort: 4312,
    origin: "http://localhost:5173",
    dataDirectory: "/srv/kestral-dev",
  });
});

test("split development rejects unsafe or contradictory options", () => {
  assert.throws(
    () => parseDevSplitOptions(["--origin", "http://remote-host:1420"], {}, home, workingDirectory),
    /plain HTTP loopback origin/,
  );
  assert.throws(
    () => parseDevSplitOptions(["--frontend-port", "8000", "--origin", "http://localhost:1420"], {}, home, workingDirectory),
    /does not match/,
  );
  assert.throws(
    () => parseDevSplitOptions(["--backend-port", "1420"], {}, home, workingDirectory),
    /ports must differ/,
  );
  assert.throws(
    () => parseDevSplitOptions(["--wat"], {}, home, workingDirectory),
    /unknown option/,
  );
  assert.throws(
    () => parseDevSplitOptions(["--clean", "--clean"], {}, home, workingDirectory),
    /only once/,
  );
});

test("clean split development removes the complete selected data directory", async () => {
  const parent = await mkdtemp(path.join(tmpdir(), "kestral-dev-clean-test-"));
  const dataDirectory = path.join(parent, "data");
  try {
    await mkdir(path.join(dataDirectory, "apps"), { recursive: true });
    await writeFile(path.join(dataDirectory, "kernel-state-v1.json"), "stale state");
    await writeFile(path.join(dataDirectory, "apps", "record.json"), "stale app");

    assert.equal(await cleanDevDataDirectory(dataDirectory), true);
    await assert.rejects(lstat(dataDirectory), { code: "ENOENT" });
    assert.equal(await cleanDevDataDirectory(dataDirectory), false);
  } finally {
    await rm(parent, { recursive: true, force: true });
  }
});

test("clean split development refuses protected roots", async () => {
  await assert.rejects(
    cleanDevDataDirectory(path.parse(process.cwd()).root),
    /refusing to clean protected directory/,
  );
});

test("clean split development refuses a linked data directory", { skip: process.platform === "win32" }, async () => {
  const parent = await mkdtemp(path.join(tmpdir(), "kestral-dev-link-test-"));
  const target = path.join(parent, "target");
  const linkedData = path.join(parent, "linked-data");
  try {
    await mkdir(target);
    await writeFile(path.join(target, "keep.txt"), "keep");
    await symlink(target, linkedData, "dir");

    await assert.rejects(cleanDevDataDirectory(linkedData), /linked development data directory/);
    assert.equal((await lstat(path.join(target, "keep.txt"))).isFile(), true);
  } finally {
    await rm(parent, { recursive: true, force: true });
  }
});
