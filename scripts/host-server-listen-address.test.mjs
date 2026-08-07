import assert from "node:assert/strict";
import test from "node:test";

import { parseHostServerListenAddress } from "./host-server-listen-address.mjs";

test("waits for the complete listen message before returning its address", () => {
  let stderr = "Kestral backend listening on http://127";
  assert.equal(parseHostServerListenAddress(stderr), undefined);

  stderr += ".0.0.1:4310\n";
  assert.equal(parseHostServerListenAddress(stderr), "http://127.0.0.1:4310");
});
