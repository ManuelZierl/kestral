import assert from "node:assert/strict";
import test from "node:test";
import { clearAmbientEnvironment, supportsRuntime } from "../src/runtime.ts";

test("enforces the pi-ai Node runtime floor", () => {
  assert.equal(supportsRuntime("22.18.0"), false);
  assert.equal(supportsRuntime("22.19.0"), true);
  assert.equal(supportsRuntime("23.0.0"), true);
  assert.equal(supportsRuntime("invalid"), false);
});

test("clears ambient auth while preserving Windows runtime variables", () => {
  const windowsEnvironment = {
    SystemRoot: "C:\\Windows",
    WINDIR: "C:\\Windows",
    OPENAI_API_KEY: "secret",
    HTTPS_PROXY: "http://proxy.example",
  };
  clearAmbientEnvironment(windowsEnvironment, "win32");
  assert.deepEqual(windowsEnvironment, {
    SystemRoot: "C:\\Windows",
    WINDIR: "C:\\Windows",
  });

  const unixEnvironment = { SystemRoot: "/not-used", OPENAI_API_KEY: "secret" };
  clearAmbientEnvironment(unixEnvironment, "linux");
  assert.deepEqual(unixEnvironment, {});
});
