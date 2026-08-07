import { runWorker } from "./runner.ts";
import { assertSupportedRuntime, clearAmbientEnvironment } from "./runtime.ts";

try {
  assertSupportedRuntime();
  clearAmbientEnvironment();
  runWorker().catch((error) => {
    process.stderr.write(`provider worker fatal error: ${error instanceof Error ? error.name : "unknown"}\n`);
    process.exitCode = 1;
  });
} catch {
  process.stderr.write("provider worker requires Node.js 22.19.0 or newer\n");
  process.exitCode = 1;
}
