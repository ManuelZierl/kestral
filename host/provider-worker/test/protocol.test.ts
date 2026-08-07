import assert from "node:assert/strict";
import test from "node:test";
import { parseCommand, ProtocolError } from "../src/protocol.ts";

const provider = { kind: "open-ai-compatible", base_url: "http://localhost:1234/v1", api_key: "secret", env: { REGION: "local" } };

test("parses a strict generate command", () => {
  const command = parseCommand({ command: "generate", request_id: "r1", provider, model: "local-model", messages: [{ role: "user", content: "hello" }], tools: [{ type: "function", function: { name: "lookup", description: "Lookup", parameters: { type: "object", properties: { id: { type: "string" } }, required: ["id"] } } }], text_verbosity: "high" });
  assert.equal(command.command, "generate");
  assert.equal(command.request_id, "r1");
  assert.equal(command.command === "generate" && command.text_verbosity, "high");
  assert.throws(() => parseCommand({ command: "generate", request_id: "r1", provider, model: "m", messages: [], text_verbosity: "maximum" }), /text_verbosity is invalid/);
});

test("rejects unknown fields", () => {
  assert.throws(() => parseCommand({ command: "shutdown", request_id: "r1", extra: true }), ProtocolError);
  assert.throws(() => parseCommand({ command: "models-list", request_id: "r1", provider, model: "stale-model" }), /unknown field model/);
});

test("rejects malformed nested messages and schemas", () => {
  assert.throws(() => parseCommand({ command: "generate", request_id: "r1", provider, model: "m", messages: [{ role: "user", content: "x", extra: true }] }), /unknown field/);
  assert.throws(() => parseCommand({ command: "generate", request_id: "r1", provider, model: "m", messages: [], tools: [{ type: "function", function: { name: "bad", description: "", parameters: { type: "wat" } } }] }), /type is invalid/);
});

test("parses cancel and rejects unsupported commands", () => {
  assert.deepEqual(parseCommand({ command: "cancel", request_id: "c1", target_request_id: "r1" }), { command: "cancel", request_id: "c1", target_request_id: "r1" });
  assert.throws(() => parseCommand({ command: "execute", request_id: "r1" }), /unknown command/);
});

test("parses strict OAuth commands and new provider kinds", () => {
  assert.deepEqual(parseCommand({ command: "oauth-login", request_id: "oauth-1", provider: { kind: "openai-codex", base_url: "https://example.com" } }), {
    command: "oauth-login",
    request_id: "oauth-1",
    provider: { kind: "openai-codex", base_url: "https://example.com" },
  });
  assert.deepEqual(parseCommand({ command: "oauth-prompt-response", request_id: "response-1", target_request_id: "oauth-1", prompt_id: "prompt-1", value: "browser" }), {
    command: "oauth-prompt-response",
    request_id: "response-1",
    target_request_id: "oauth-1",
    prompt_id: "prompt-1",
    value: "browser",
  });
  const models = parseCommand({ command: "models-list", request_id: "models-1", provider: { kind: "github-copilot" } });
  assert.equal(models.command === "models-list" && models.provider.kind, "github-copilot");
});

test("rejects malformed OAuth commands", () => {
  assert.throws(() => parseCommand({ command: "oauth-login", request_id: "oauth-1", provider: { kind: "anthropic", api_key: "not-accepted" } }), /unknown field/);
  assert.throws(() => parseCommand({ command: "oauth-prompt-response", request_id: "response-1", target_request_id: "oauth-1", prompt_id: "prompt-1" }), /requires value or cancelled/);
  assert.throws(() => parseCommand({ command: "oauth-prompt-response", request_id: "response-1", target_request_id: "oauth-1", prompt_id: "prompt-1", value: "x", cancelled: true }), /both value and cancelled/);
  assert.throws(() => parseCommand({ command: "oauth-prompt-response", request_id: "response-1", target_request_id: "oauth-1", prompt_id: "prompt-1", value: "x".repeat(16_385) }), /too long/);
  assert.throws(() => parseCommand({ command: "oauth-login", request_id: "oauth-1", provider: { kind: "anthropic", base_url: `https://example.com/${"x".repeat(8_193)}` } }), /too long/);
});

test("parses bounded OAuth credentials with provider-specific fields", () => {
  const command = parseCommand({
    command: "generate",
    request_id: "generate-oauth",
    provider: {
      kind: "github-copilot",
      oauth_credential: {
        type: "oauth",
        access: "access-token",
        refresh: "refresh-token",
        expires: 1234,
        enterpriseUrl: "example.ghe.com",
        availableModelIds: ["model-a"],
      },
    },
    model: "model-a",
    messages: [],
  });
  assert(command.command === "generate");
  assert.deepEqual(command.provider.oauth_credential, {
    type: "oauth",
    access: "access-token",
    refresh: "refresh-token",
    expires: 1234,
    enterpriseUrl: "example.ghe.com",
    availableModelIds: ["model-a"],
  });
});

test("rejects unsafe or malformed OAuth credentials", () => {
  const generate = (oauth_credential: unknown, extra: Record<string, unknown> = {}) => parseCommand({
    command: "generate",
    request_id: "generate-oauth",
    provider: { kind: "anthropic", oauth_credential, ...extra },
    model: "model-a",
    messages: [],
  });
  const valid = { type: "oauth", access: "access", refresh: "refresh", expires: 1234 };
  assert.throws(() => generate(valid, { api_key: "key" }), /mutually exclusive/);
  assert.throws(() => generate({ ...valid, access: "" }), /non-empty string/);
  assert.throws(() => generate({ ...valid, expires: Number.POSITIVE_INFINITY }), /finite non-negative/);
  assert.throws(() => generate({ ...valid, metadata: { constructor: "unsafe" } }), /dangerous field constructor/);
  assert.throws(() => generate(JSON.parse('{"type":"oauth","access":"access","refresh":"refresh","expires":1,"metadata":{"__proto__":"unsafe"}}')), /dangerous field __proto__/);
  assert.throws(() => generate({ ...valid, metadata: { value: "x".repeat(16_385) } }), /too long/);
  let nested: Record<string, unknown> = { value: true };
  for (let index = 0; index < 13; index += 1) nested = { nested };
  assert.throws(() => generate({ ...valid, metadata: nested }), /deeply nested/);
  const cyclic: Record<string, unknown> = { ...valid };
  cyclic.metadata = cyclic;
  assert.throws(() => generate(cyclic), /must not be cyclic/);
  const pollutedPrototype = { ...valid, metadata: { safe: true } };
  Object.setPrototypeOf(pollutedPrototype.metadata, { polluted: true });
  assert.throws(() => generate(pollutedPrototype), /plain JSON object/);
  const customArray = ["safe"] as string[] & { extra?: string };
  customArray.extra = "unsafe";
  assert.throws(() => generate({ ...valid, metadata: customArray }), /invalid array fields/);
});
