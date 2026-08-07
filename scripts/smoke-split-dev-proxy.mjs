import assert from "node:assert/strict";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { createServer } from "../host/node_modules/vite/dist/node/index.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const browserOrigin = "http://localhost:1420";

const backend = http.createServer((request, response) => {
  if (request.url !== "/api/probe") {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, { "Content-Type": "application/json" });
  response.end(JSON.stringify({ origin: request.headers.origin }));
});
await new Promise((resolve, reject) => {
  backend.once("error", reject);
  backend.listen(0, "127.0.0.1", resolve);
});
const backendAddress = backend.address();
assert(backendAddress && typeof backendAddress !== "string");
process.env.KESTRAL_HOST_API_PROXY_TARGET = `http://127.0.0.1:${backendAddress.port}`;

const vite = await createServer({
  root: path.join(root, "host"),
  configFile: path.join(root, "host", "vite.config.js"),
  logLevel: "silent",
  server: { host: "127.0.0.1", port: 0, strictPort: false },
});

try {
  await vite.listen();
  const viteAddress = vite.httpServer?.address();
  assert(viteAddress && typeof viteAddress !== "string");
  const gateway = `http://127.0.0.1:${viteAddress.port}`;

  const canonicalRedirect = await fetch(gateway, { redirect: "manual" });
  assert.equal(canonicalRedirect.status, 307);
  assert.equal(canonicalRedirect.headers.get("location"), `http://localhost:${viteAddress.port}/`);
  const canonicalGateway = `http://localhost:${viteAddress.port}`;

  const frontend = await fetch(canonicalGateway);
  assert.equal(frontend.status, 200);
  assert.match(await frontend.text(), /<html/);

  const proxied = await fetch(`${canonicalGateway}/api/probe`, {
    headers: { Origin: browserOrigin },
  });
  assert.equal(proxied.status, 200);
  assert.deepEqual(await proxied.json(), { origin: browserOrigin });
  console.log(`split development gateway smoke passed at ${gateway}`);
} finally {
  await vite.close();
  await new Promise((resolve, reject) => {
    backend.close((error) => error ? reject(error) : resolve());
  });
}
