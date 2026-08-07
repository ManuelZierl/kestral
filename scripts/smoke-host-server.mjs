import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

import { parseHostServerListenAddress } from "./host-server-listen-address.mjs";

const executable = resolve(process.argv[2] ?? "target/release/host-server");
const root = await mkdtemp(`${tmpdir()}/kestral-host-server-smoke-`);
const allowedOrigin = "http://localhost:1420";
const environment = {
  ...process.env,
  HOST_REMOTE_BIND: "127.0.0.1:0",
  HOST_REMOTE_ORIGIN: allowedOrigin,
  KESTRAL_DATA_DIR: `${root}/data`,
};
const pairing = spawnSync(executable, ["owner", "pair"], {
  cwd: root,
  env: environment,
  encoding: "utf8",
});
assert.equal(pairing.status, 0, `owner pairing command failed\n${pairing.stderr}`);
const pairingCode = pairing.stdout.match(/valid for 10 minutes\): ([A-Za-z0-9_-]+)/)?.[1];
assert.ok(pairingCode, "owner pairing command must print a one-time code");
let stderr = "";
let listening = false;

const child = spawn(executable, [], {
  cwd: root,
  env: environment,
  stdio: ["ignore", "ignore", "pipe"],
});

const exited = new Promise((resolveExit) => {
  child.once("exit", (code, signal) => resolveExit({ code, signal }));
});

const address = new Promise((resolveAddress, rejectAddress) => {
  const timeout = setTimeout(() => {
    rejectAddress(new Error(`host-server did not listen within 15 seconds\n${stderr}`));
  }, 15_000);

  child.once("error", (error) => {
    clearTimeout(timeout);
    rejectAddress(error);
  });
  child.stderr.on("data", (chunk) => {
    stderr += chunk.toString();
    const listenAddress = parseHostServerListenAddress(stderr);
    if (!listenAddress || listening) return;
    listening = true;
    clearTimeout(timeout);
    resolveAddress(listenAddress);
  });
  child.once("exit", (code, signal) => {
    if (listening) return;
    clearTimeout(timeout);
    rejectAddress(new Error(`host-server exited before listening (${code ?? signal})\n${stderr}`));
  });
});

async function stopChild() {
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  await Promise.race([
    exited,
    new Promise((resolveTimeout) => setTimeout(resolveTimeout, 2_000)),
  ]);
  if (child.exitCode === null && child.signalCode === null) {
    child.kill("SIGKILL");
    await exited;
  }
}

try {
  const baseUrl = await address;

  const liveReset = spawnSync(executable, ["owner", "reset", "--confirm"], {
    cwd: root,
    env: environment,
    encoding: "utf8",
  });
  assert.notEqual(liveReset.status, 0, "owner reset must not run beside a live backend");
  assert.match(liveReset.stderr, /stop the backend before resetting owner authentication/);

  const unauthenticated = await fetch(`${baseUrl}/api/health`, {
    headers: { Origin: allowedOrigin },
    signal: AbortSignal.timeout(5_000),
  });
  assert.equal(unauthenticated.status, 401, "health must require an owner session");

  const wrongOrigin = await fetch(`${baseUrl}/api/auth/status`, {
    headers: { Origin: "http://untrusted.example" },
    signal: AbortSignal.timeout(5_000),
  });
  assert.equal(wrongOrigin.status, 403, "health must reject an untrusted browser origin");

  const preflight = await fetch(`${baseUrl}/api/invoke/list_apps`, {
    method: "OPTIONS",
    headers: { Origin: allowedOrigin },
    signal: AbortSignal.timeout(5_000),
  });
  assert.equal(preflight.status, 204, "browser preflight must succeed");
  assert.equal(preflight.headers.get("access-control-allow-origin"), allowedOrigin);

  const status = await fetch(`${baseUrl}/api/auth/status`, {
    headers: { Origin: allowedOrigin },
    signal: AbortSignal.timeout(5_000),
  });
  assert.equal(status.status, 200, "authentication status must be public at the trusted origin");
  assert.deepEqual(await status.json(), { paired: false, authenticated: false });

  const apps = await fetch(`${baseUrl}/api/invoke/list_apps`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Origin: allowedOrigin,
    },
    body: "{}",
    signal: AbortSignal.timeout(5_000),
  });
  assert.equal(apps.status, 401, "command dispatch must reject a missing owner session");

  const eventStream = await fetch(`${baseUrl}/api/events/stream`, {
    headers: { Accept: "text/event-stream", Origin: allowedOrigin },
    signal: AbortSignal.timeout(5_000),
  });
  assert.equal(eventStream.status, 401, "event stream must reject a missing owner session");

  const oversizedPairing = await fetch(`${baseUrl}/api/auth/register/start`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Origin: allowedOrigin },
    body: JSON.stringify({ pairing_code: "x".repeat(1024 * 1024) }),
    signal: AbortSignal.timeout(5_000),
  });
  assert.equal(oversizedPairing.status, 413, "authentication bodies must remain size-limited");

  const invalidPairing = await fetch(`${baseUrl}/api/auth/register/start`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Origin: allowedOrigin },
    body: JSON.stringify({ pairing_code: "invalid" }),
    signal: AbortSignal.timeout(5_000),
  });
  assert.equal(invalidPairing.status, 401, "invalid pairing code must be rejected");

  const registration = await fetch(`${baseUrl}/api/auth/register/start`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Origin: allowedOrigin },
    body: JSON.stringify({ pairing_code: pairingCode }),
    signal: AbortSignal.timeout(5_000),
  });
  assert.equal(registration.status, 200, "SSH-created pairing code must start registration");
  const registrationBody = await registration.json();
  assert.equal(typeof registrationBody.ceremony_id, "string");
  assert.equal(typeof registrationBody.options?.publicKey?.challenge, "string");

  const replay = await fetch(`${baseUrl}/api/auth/register/start`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Origin: allowedOrigin },
    body: JSON.stringify({ pairing_code: pairingCode }),
    signal: AbortSignal.timeout(5_000),
  });
  assert.equal(replay.status, 401, "pairing code must be single use");
  assert.equal(
    existsSync(`${root}/host-data`),
    false,
    "KESTRAL_DATA_DIR must prevent metadata from leaking into the working directory",
  );

  console.log(`host-server remote smoke passed at ${baseUrl}`);
} finally {
  await stopChild();
  await rm(root, { recursive: true, force: true });
}
