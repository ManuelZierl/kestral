#!/usr/bin/env node

import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { main } from "../packages/create-kestral-app/cli.mjs";

export { createAppProject, parseArguments } from "../packages/create-kestral-app/cli.mjs";

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  await main();
}
