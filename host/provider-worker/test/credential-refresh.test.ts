import assert from "node:assert/strict";
import test from "node:test";
import {
  createAssistantMessageEventStream,
  createProvider,
  type AssistantMessage,
  type OAuthAuth,
  type OAuthCredential,
  type Provider,
  type SimpleStreamOptions,
} from "@earendil-works/pi-ai";
import { createProviderScope, RequestCredentialStore, type ProviderFactory } from "../src/providers.ts";
import { WorkerService, type ProviderScopeFactory } from "../src/service.ts";

type Event = Record<string, unknown>;

const INPUT_CREDENTIAL: OAuthCredential = {
  type: "oauth",
  access: "input-access-token",
  refresh: "input-refresh-token",
  expires: 0,
  accountId: "account-1",
};

const ROTATED_CREDENTIAL: OAuthCredential = {
  type: "oauth",
  access: "rotated-access-token",
  refresh: "rotated-refresh-token",
  expires: 4_102_444_800_000,
  accountId: "account-1",
};

function assistantMessage(stopReason: "stop" | "error", errorMessage?: string): AssistantMessage {
  return {
    role: "assistant",
    content: stopReason === "stop" ? [{ type: "text", text: "ok" }] : [],
    api: "openai-completions",
    provider: "anthropic",
    model: "fake-model",
    usage: { input: 1, output: 1, cacheRead: 0, cacheWrite: 0, totalTokens: 2, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } },
    stopReason,
    ...(errorMessage ? { errorMessage } : {}),
    timestamp: 1,
  };
}

function fakeProvider(fail: boolean, usedApiKeys: Array<string | undefined>, failModelRefresh = false): Provider {
  const oauth: OAuthAuth = {
    name: "Fake OAuth",
    login: async () => INPUT_CREDENTIAL,
    refresh: async () => ROTATED_CREDENTIAL,
    toAuth: async (credential) => ({ apiKey: credential.access }),
  };
  const model = {
    id: "fake-model",
    name: "Fake model",
    api: "openai-completions" as const,
    provider: "anthropic",
    baseUrl: "https://example.invalid",
    reasoning: false,
    input: ["text" as const],
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: 1_000,
    maxTokens: 100,
  };
  const streamSimple = (_model: typeof model, _context: unknown, options?: SimpleStreamOptions) => {
    usedApiKeys.push(options?.apiKey);
    const stream = createAssistantMessageEventStream();
    queueMicrotask(() => {
      if (fail) {
        const error = assistantMessage("error", "provider failed with rotated-access-token and rotated-refresh-token");
        stream.push({ type: "error", reason: "error", error });
      } else {
        const message = assistantMessage("stop");
        stream.push({ type: "done", reason: "stop", message });
      }
    });
    return stream;
  };
  return createProvider({
    id: "anthropic",
    name: "Fake provider",
    auth: { oauth },
    models: [model],
    ...(failModelRefresh ? { refreshModels: async () => { throw new Error("model refresh failed after token rotation"); } } : {}),
    api: {
      stream: (selected, context, options) => streamSimple(selected as typeof model, context, options),
      streamSimple: (selected, context, options) => streamSimple(selected as typeof model, context, options),
    },
  });
}

function serviceFor(fail: boolean, usedApiKeys: Array<string | undefined>, failModelRefresh = false): WorkerService {
  const providerFactory: ProviderFactory = () => fakeProvider(fail, usedApiKeys, failModelRefresh);
  const scopeFactory: ProviderScopeFactory = (config, model) => createProviderScope(config, model, providerFactory);
  return new WorkerService(undefined, scopeFactory);
}

function generateCommand() {
  return {
    command: "generate" as const,
    request_id: "generate-oauth",
    provider: { kind: "anthropic" as const, oauth_credential: INPUT_CREDENTIAL },
    model: "fake-model",
    messages: [{ role: "user" as const, content: "hello" }],
  };
}

test("returns refreshed OAuth credential after successful generation", async () => {
  const usedApiKeys: Array<string | undefined> = [];
  const events: Event[] = [];
  await serviceFor(false, usedApiKeys).handle(generateCommand(), (event) => events.push(event));
  assert.deepEqual(usedApiKeys, ["rotated-access-token"]);
  const completed = events.at(-1);
  assert.deepEqual({ ...completed, response: undefined }, {
    type: "completed",
    request_id: "generate-oauth",
    response: undefined,
    credential: ROTATED_CREDENTIAL,
  });
  const response = completed?.response as Record<string, unknown>;
  assert.deepEqual(response.message, { role: "assistant", content: "ok" });
  assert.equal(response.finish_reason, "stop");
  assert.deepEqual(response.usage, {
    prompt_tokens: 1,
    completion_tokens: 1,
    total_tokens: 2,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    cost: 0,
  });
  assert.deepEqual(Object.keys(response.provider_metrics as object), ["total_latency_ms"]);
  assert.ok(Number.isInteger(
    (response.provider_metrics as { total_latency_ms: number }).total_latency_ms,
  ));
});

test("returns refreshed OAuth credential after generation failure and redacts it", async () => {
  const usedApiKeys: Array<string | undefined> = [];
  const events: Event[] = [];
  await serviceFor(true, usedApiKeys).handle(generateCommand(), (event) => events.push(event));
  assert.deepEqual(usedApiKeys, ["rotated-access-token"]);
  assert.deepEqual(events.at(-1), {
    type: "failed",
    request_id: "generate-oauth",
    code: "provider_error",
    message: "provider failed with [redacted] and [redacted]",
    credential: ROTATED_CREDENTIAL,
  });
});

test("models event returns OAuth refresh without contacting a provider service", async () => {
  const events: Event[] = [];
  await serviceFor(false, []).handle({
    command: "models-list",
    request_id: "models-oauth",
    provider: { kind: "anthropic", oauth_credential: INPUT_CREDENTIAL },
  }, (event) => events.push(event));
  assert.equal(events[0]?.type, "models");
  assert.deepEqual(events[0]?.credential, ROTATED_CREDENTIAL);
});

test("failed models event returns an earlier OAuth refresh", async () => {
  const events: Event[] = [];
  await serviceFor(false, [], true).handle({
    command: "models-refresh",
    request_id: "models-oauth-failed",
    provider: { kind: "anthropic", oauth_credential: INPUT_CREDENTIAL },
  }, (event) => events.push(event));
  assert.deepEqual(events, [{
    type: "failed",
    request_id: "models-oauth-failed",
    code: "models_error",
    message: "Model refresh failed for anthropic",
    credential: ROTATED_CREDENTIAL,
  }]);
});

test("request credential store serializes modifications and retains latest value", async () => {
  const store = new RequestCredentialStore("anthropic", INPUT_CREDENTIAL);
  const order: string[] = [];
  let releaseFirst: (() => void) | undefined;
  const firstGate = new Promise<void>((resolve) => { releaseFirst = resolve; });
  const first = store.modify("anthropic", async () => {
    order.push("first-start");
    await firstGate;
    order.push("first-end");
    return ROTATED_CREDENTIAL;
  });
  const secondCredential = { ...ROTATED_CREDENTIAL, access: "second-access-token" };
  const second = store.modify("anthropic", async (current) => {
    order.push(`second-${current?.type === "oauth" ? current.access : "missing"}`);
    return secondCredential;
  });
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(order, ["first-start"]);
  releaseFirst?.();
  await Promise.all([first, second]);
  assert.deepEqual(order, ["first-start", "first-end", "second-rotated-access-token"]);
  assert.deepEqual(await store.read("anthropic"), secondCredential);
});
