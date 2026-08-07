import { spawn } from "node:child_process";
import { resolve } from "node:path";

const executable = resolve(
  "provider-worker",
  "runtime",
  process.platform === "win32" ? "node.exe" : "node",
);
const child = spawn(executable, process.argv.slice(2), {
  stdio: "inherit",
  windowsHide: true,
});
child.once("error", (error) => {
  console.error(`Unable to start bundled Node runtime: ${error.message}`);
  process.exitCode = 1;
});
child.once("exit", (code, signal) => {
  if (signal) {
    console.error(`Bundled Node runtime exited from signal ${signal}`);
    process.exitCode = 1;
    return;
  }
  process.exitCode = code ?? 1;
});
