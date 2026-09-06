import assert from "node:assert/strict";
import { mkdtemp, rm, truncate, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { checkProductBudgets, MIB } from "./check-product-budgets.mjs";

async function withTempDirectory(callback) {
  const directory = await mkdtemp(join(tmpdir(), "kestral-product-budget-"));
  try {
    await callback(directory);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

async function sparseFile(path, size) {
  await writeFile(path, "");
  await truncate(path, size);
}

test("accepts Linux release artifacts within the configured ceilings", async () => {
  await withTempDirectory(async (directory) => {
    await sparseFile(join(directory, "kestral-0.1.0-alpha.1-linux-x86_64.AppImage"), MIB);
    await sparseFile(join(directory, "kestral-0.1.0-alpha.1-linux-x86_64.deb"), MIB);
    await sparseFile(join(directory, "kestral-0.1.0-alpha.1-linux-x86_64-server.tar.gz"), MIB);
    await sparseFile(join(directory, "kestral-browser-client-0.1.0-alpha.1.zip"), MIB);

    const result = await checkProductBudgets("linux", directory);
    assert.deepEqual(result.failures, []);
    assert.equal(result.measurements.length, 4);
  });
});

test("rejects an unsupported platform even when called as a library", async () => {
  await withTempDirectory(async (directory) => {
    await assert.rejects(
      () => checkProductBudgets("macos", directory),
      /unsupported product budget platform/,
    );
  });
});

test("fails closed when a required release artifact is absent", async () => {
  await withTempDirectory(async (directory) => {
    const result = await checkProductBudgets("windows", directory);
    assert.equal(result.failures.length, 2);
    assert.match(result.failures[0], /missing/);
  });
});

test("fails closed when more than one artifact matches a release slot", async () => {
  await withTempDirectory(async (directory) => {
    await sparseFile(join(directory, "kestral-0.1.0-alpha.1-windows-x86_64-portable.zip"), MIB);
    await sparseFile(join(directory, "kestral-0.1.0-alpha.2-windows-x86_64-portable.zip"), MIB);
    await sparseFile(join(directory, "kestral-0.1.0-alpha.1-windows-x86_64-nsis.exe"), MIB);

    const result = await checkProductBudgets("windows", directory);
    assert.equal(result.failures.length, 1);
    assert.match(result.failures[0], /ambiguous/);
    assert.match(result.failures[0], /alpha\.1/);
    assert.match(result.failures[0], /alpha\.2/);
  });
});

test("reports an artifact that exceeds its ceiling", async () => {
  await withTempDirectory(async (directory) => {
    await sparseFile(join(directory, "kestral-0.1.0-alpha.1-windows-x86_64-portable.zip"), 301 * MIB);
    await sparseFile(join(directory, "kestral-0.1.0-alpha.1-windows-x86_64-nsis.exe"), MIB);

    const result = await checkProductBudgets("windows", directory);
    assert.equal(result.failures.length, 1);
    assert.match(result.failures[0], /portable\.zip/);
    assert.match(result.failures[0], /exceeds/);
  });
});
