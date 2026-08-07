import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { lstat, rm } from "node:fs/promises";
import { homedir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const hostDirectory = path.join(root, "host");
const loopbackHost = "127.0.0.1";

export const usage = `Usage: npm run dev:split -- [options]

Starts the Kestral backend and browser frontend together on loopback.

Options:
  --pair                    Create an owner pairing code before startup
  --clean                   Reset development and browser-local Kestral data, then pair
  --frontend-port <port>    Browser, API proxy, and HMR port (default: 1420)
  --backend-port <port>     Host-local backend port (default: 4310)
  --origin <origin>         Exact browser origin (default: http://localhost:<frontend-port>)
  --data-dir <path>         Host data directory (default: ~/.local/share/kestral-split-dev)
  --help                    Show this help

Examples:
  npm run dev:split
  npm run dev:split -- --pair
  npm run dev:split -- --clean
  npm run dev:split -- --frontend-port 8000 --backend-port 4311
`;

function optionValue(arguments_, index, option) {
  const value = arguments_[index + 1];
  if (!value || value.startsWith("--")) {
    throw new Error(`${option} requires a value`);
  }
  return value;
}

function parsePort(value, option) {
  if (!/^\d+$/.test(value)) {
    throw new Error(`${option} must be an integer between 1 and 65535`);
  }
  const port = Number(value);
  if (port < 1 || port > 65535) {
    throw new Error(`${option} must be an integer between 1 and 65535`);
  }
  return port;
}

function normalizeOrigin(value, frontendPort) {
  let origin;
  try {
    origin = new URL(value);
  } catch (error) {
    throw new Error(`--origin is invalid: ${error.message}`);
  }
  if (
    origin.protocol !== "http:"
    || !["localhost", "127.0.0.1", "[::1]"].includes(origin.hostname)
    || origin.username
    || origin.password
    || origin.pathname !== "/"
    || origin.search
    || origin.hash
  ) {
    throw new Error("--origin must be a plain HTTP loopback origin without a path, query, or fragment");
  }
  const originPort = Number(origin.port || "80");
  if (originPort !== frontendPort) {
    throw new Error(`--origin port ${originPort} does not match --frontend-port ${frontendPort}`);
  }
  return origin.origin;
}

export function parseDevSplitOptions(
  arguments_,
  environment = process.env,
  home = homedir(),
  workingDirectory = process.cwd(),
) {
  const values = new Map();
  let pair = false;
  let clean = false;
  let help = false;

  for (let index = 0; index < arguments_.length; index += 1) {
    const option = arguments_[index];
    if (option === "--pair") {
      if (pair) throw new Error("--pair may be specified only once");
      pair = true;
    } else if (option === "--clean") {
      if (clean) throw new Error("--clean may be specified only once");
      clean = true;
    } else if (option === "--help") {
      help = true;
    } else if (["--frontend-port", "--backend-port", "--origin", "--data-dir"].includes(option)) {
      if (values.has(option)) throw new Error(`${option} may be specified only once`);
      values.set(option, optionValue(arguments_, index, option));
      index += 1;
    } else {
      throw new Error(`unknown option: ${option}`);
    }
  }

  const frontendPort = parsePort(
    values.get("--frontend-port") ?? environment.KESTRAL_DEV_FRONTEND_PORT ?? "1420",
    "--frontend-port",
  );
  const backendPort = parsePort(
    values.get("--backend-port") ?? environment.KESTRAL_DEV_BACKEND_PORT ?? "4310",
    "--backend-port",
  );
  if (frontendPort === backendPort) {
    throw new Error("frontend and backend ports must differ");
  }

  const origin = normalizeOrigin(
    values.get("--origin") ?? environment.HOST_REMOTE_ORIGIN ?? `http://localhost:${frontendPort}`,
    frontendPort,
  );
  const configuredDataDirectory = values.get("--data-dir")
    ?? environment.KESTRAL_DATA_DIR
    ?? path.join(home, ".local", "share", "kestral-split-dev");
  if (!configuredDataDirectory.trim()) {
    throw new Error("--data-dir must not be empty");
  }

  return {
    pair: pair || clean,
    clean,
    help,
    frontendPort,
    backendPort,
    origin,
    dataDirectory: path.resolve(workingDirectory, configuredDataDirectory),
  };
}

export async function cleanDevDataDirectory(dataDirectory) {
  const resolvedDirectory = path.resolve(dataDirectory);
  const protectedDirectories = new Set([
    path.parse(resolvedDirectory).root,
    path.resolve(homedir()),
    root,
    hostDirectory,
  ]);
  if (protectedDirectories.has(resolvedDirectory)) {
    throw new Error(`refusing to clean protected directory '${resolvedDirectory}'`);
  }

  let metadata;
  try {
    metadata = await lstat(resolvedDirectory);
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw new Error(`inspect development data directory failed: ${error.message}`);
  }
  if (metadata.isSymbolicLink()) {
    throw new Error(`refusing to clean linked development data directory '${resolvedDirectory}'`);
  }
  if (!metadata.isDirectory()) {
    throw new Error(`development data path is not a directory: '${resolvedDirectory}'`);
  }

  try {
    await rm(resolvedDirectory, {
      recursive: true,
      force: false,
      maxRetries: 3,
      retryDelay: 100,
    });
  } catch (error) {
    throw new Error(`clean development data directory failed: ${error.message}`);
  }
  return true;
}

function commandName(command, arguments_) {
  return [command, ...arguments_].join(" ");
}

function runCommand(command, arguments_, options) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, arguments_, { ...options, stdio: "inherit" });
    child.once("error", (error) => reject(new Error(`failed to start ${command}: ${error.message}`)));
    child.once("exit", (code, signal) => {
      if (code === 0) {
        resolve();
      } else {
        const status = signal ? `signal ${signal}` : `exit code ${code}`;
        reject(new Error(`${commandName(command, arguments_)} failed with ${status}`));
      }
    });
  });
}

function startProcess(name, command, arguments_, options) {
  const child = spawn(command, arguments_, { ...options, stdio: "inherit" });
  const exited = new Promise((resolve) => {
    child.once("error", (error) => resolve({ name, error }));
    child.once("exit", (code, signal) => resolve({ name, code, signal }));
  });
  return { name, child, exited };
}

async function stopProcesses(processes) {
  for (const process_ of processes) {
    if (process_.child.exitCode === null && process_.child.signalCode === null) {
      process_.child.kill("SIGTERM");
    }
  }

  const allExited = Promise.all(processes.map((process_) => process_.exited));
  const timedOut = await Promise.race([
    allExited.then(() => false),
    new Promise((resolve) => setTimeout(() => resolve(true), 5000)),
  ]);
  if (!timedOut) return;

  for (const process_ of processes) {
    if (process_.child.exitCode === null && process_.child.signalCode === null) {
      process_.child.kill("SIGKILL");
    }
  }
  await allExited;
}

function executablePath(environment) {
  const targetDirectory = path.resolve(root, environment.CARGO_TARGET_DIR ?? "target");
  const profileDirectory = environment.CARGO_BUILD_TARGET
    ? path.join(targetDirectory, environment.CARGO_BUILD_TARGET, "debug")
    : path.join(targetDirectory, "debug");
  return path.join(profileDirectory, process.platform === "win32" ? "host-server.exe" : "host-server");
}

async function main() {
  const options = parseDevSplitOptions(process.argv.slice(2));
  if (options.help) {
    process.stdout.write(usage);
    return;
  }

  const npm = process.platform === "win32" ? "npm.cmd" : "npm";
  const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
  const backendEnvironment = {
    ...process.env,
    HOST_REMOTE_BIND: `${loopbackHost}:${options.backendPort}`,
    HOST_REMOTE_ORIGIN: options.origin,
    KESTRAL_DATA_DIR: options.dataDirectory,
  };
  const frontendEnvironment = {
    ...process.env,
    KESTRAL_HOST_API_PROXY_TARGET: `http://${loopbackHost}:${options.backendPort}`,
    ...(options.clean ? { VITE_KESTRAL_CLEAN_START_ID: randomUUID() } : {}),
  };

  console.log("Preparing Kestral split development...");
  await runCommand(npm, ["run", "provider-worker:package"], { cwd: hostDirectory });
  await runCommand(cargo, ["build", "-p", "host", "--bin", "host-server"], { cwd: root });

  if (options.clean) {
    const removed = await cleanDevDataDirectory(options.dataDirectory);
    console.log(removed
      ? `Removed all development data from ${options.dataDirectory}`
      : `Development data directory is already empty: ${options.dataDirectory}`);
  }

  const backendExecutable = executablePath(process.env);
  if (options.pair) {
    console.log("\nOwner pairing code:");
    await runCommand(backendExecutable, ["owner", "pair"], {
      cwd: root,
      env: backendEnvironment,
    });
  }

  console.log(`\nBrowser:  ${options.origin}`);
  console.log(`Backend: ${loopbackHost}:${options.backendPort} (host-local)`);
  console.log(`Data:    ${options.dataDirectory}`);
  console.log("Press Ctrl+C to stop both processes.\n");

  const backend = startProcess("backend", backendExecutable, [], {
    cwd: root,
    env: backendEnvironment,
  });
  const viteExecutable = path.join(hostDirectory, "node_modules", "vite", "bin", "vite.js");
  const frontend = startProcess(
    "frontend",
    process.execPath,
    [viteExecutable, "dev", "--host", loopbackHost, "--port", String(options.frontendPort), "--strictPort"],
    { cwd: hostDirectory, env: frontendEnvironment },
  );
  const processes = [backend, frontend];

  let resolveSignal;
  const receivedSignal = new Promise((resolve) => {
    resolveSignal = resolve;
  });
  const signalHandlers = new Map();
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    const handler = () => resolveSignal({ name: "launcher", signal });
    signalHandlers.set(signal, handler);
    process.once(signal, handler);
  }

  const outcome = await Promise.race([
    receivedSignal,
    ...processes.map((process_) => process_.exited),
  ]);
  for (const [signal, handler] of signalHandlers) process.removeListener(signal, handler);

  if (outcome.name !== "launcher") {
    const status = outcome.error?.message
      ?? (outcome.signal ? `signal ${outcome.signal}` : `exit code ${outcome.code}`);
    console.error(`${outcome.name} stopped unexpectedly (${status}); stopping split development.`);
    process.exitCode = 1;
  } else if (outcome.signal === "SIGINT") {
    process.exitCode = 130;
  } else {
    process.exitCode = 143;
  }
  await stopProcesses(processes);
}

const entryUrl = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : undefined;
if (entryUrl === import.meta.url) {
  main().catch((error) => {
    console.error(`split development failed: ${error.message}`);
    process.exitCode = 1;
  });
}
