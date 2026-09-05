#!/usr/bin/env node

import { stat } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const MIB = 1024 * 1024;

export const PRODUCT_BUDGETS = Object.freeze({
  linux: Object.freeze([
    { suffix: "-linux-x86_64.AppImage", maxBytes: 220 * MIB },
    { suffix: "-linux-x86_64.deb", maxBytes: 180 * MIB },
    { suffix: "-linux-x86_64-server.tar.gz", maxBytes: 150 * MIB },
    { suffix: "-browser-client-", maxBytes: 25 * MIB, contains: true, extension: ".zip" },
  ]),
  windows: Object.freeze([
    { suffix: "-windows-x86_64-portable.zip", maxBytes: 300 * MIB },
    { suffix: "-windows-x86_64-nsis.exe", maxBytes: 300 * MIB },
  ]),
});

function formatMiB(bytes) {
  return `${(bytes / MIB).toFixed(1)} MiB`;
}

function usage() {
  return "Usage: node scripts/check-product-budgets.mjs --platform <linux|windows> [--dir target/release/public]";
}

export function parseArguments(argv) {
  const result = { platform: null, directory: "target/release/public" };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--platform") {
      result.platform = argv[++index] ?? null;
    } else if (argument === "--dir") {
      result.directory = argv[++index] ?? null;
    } else if (argument === "--help" || argument === "-h") {
      return { help: true };
    } else {
      throw new Error(`unknown argument '${argument}'`);
    }
  }
  if (!Object.hasOwn(PRODUCT_BUDGETS, result.platform ?? "")) {
    throw new Error("--platform must be 'linux' or 'windows'");
  }
  if (!result.directory) throw new Error("--dir requires a value");
  return result;
}

async function listFiles(directory) {
  const { readdir } = await import("node:fs/promises");
  return readdir(directory);
}

function matchingFile(files, budget) {
  if (budget.contains) {
    return files.find((name) => name.includes(budget.suffix) && name.endsWith(budget.extension ?? ""));
  }
  return files.find((name) => name.endsWith(budget.suffix));
}

export async function checkProductBudgets(platform, directory) {
  const root = resolve(directory);
  const files = await listFiles(root);
  const measurements = [];
  const failures = [];

  for (const budget of PRODUCT_BUDGETS[platform]) {
    const file = matchingFile(files, budget);
    const label = budget.contains ? `*${budget.suffix}*${budget.extension ?? ""}` : `*${budget.suffix}`;
    if (!file) {
      failures.push(`required ${platform} release artifact '${label}' is missing`);
      continue;
    }
    const info = await stat(resolve(root, file));
    measurements.push({ file, bytes: info.size, maxBytes: budget.maxBytes });
    if (info.size > budget.maxBytes) {
      failures.push(`${file}: ${formatMiB(info.size)} exceeds ${formatMiB(budget.maxBytes)}`);
    }
  }

  return { measurements, failures };
}

async function main() {
  try {
    const options = parseArguments(process.argv.slice(2));
    if (options.help) {
      console.log(usage());
      return;
    }
    const result = await checkProductBudgets(options.platform, options.directory);
    for (const measurement of result.measurements) {
      console.log(`${measurement.file}: ${formatMiB(measurement.bytes)} / ${formatMiB(measurement.maxBytes)}`);
    }
    if (result.failures.length > 0) {
      for (const failure of result.failures) console.error(`product budget: ${failure}`);
      process.exitCode = 1;
      return;
    }
    console.log(`Product size budgets passed for ${options.platform}.`);
  } catch (error) {
    console.error(`product budget: ${error instanceof Error ? error.message : String(error)}`);
    console.error(usage());
    process.exitCode = 1;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  await main();
}
