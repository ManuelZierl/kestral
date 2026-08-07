import assert from "node:assert/strict";
import test from "node:test";
import type { AuthLoginCallbacks, OAuthAuth, OAuthCredential } from "@earendil-works/pi-ai";
import { createProviderScope, normalizedModels, oauthAdapter } from "../src/providers.ts";
import { WorkerService, type OAuthAdapterFactory } from "../src/service.ts";

type Event = Record<string, unknown>;

function fakeAdapter(login: (callbacks: AuthLoginCallbacks) => Promise<OAuthCredential>): OAuthAuth {
  return {
    name: "Fake OAuth",
    login,
    refresh: async (credential) => credential,
    toAuth: async (credential) => ({ apiKey: credential.access }),
  };
}

async function nextTurn(): Promise<void> {
  await new Promise<void>((resolve) => setImmediate(resolve));
}

test("bridges OAuth events and prompts without persisting credentials", async () => {
  const factory: OAuthAdapterFactory = () => fakeAdapter(async (callbacks) => {
    callbacks.notify({ type: "auth_url", url: "https://example.com/login", instructions: "Open the login page" });
    callbacks.notify({ type: "device_code", userCode: "ABCD-EFGH", verificationUri: "https://example.com/device", intervalSeconds: 5, expiresInSeconds: 600 });
    callbacks.notify({ type: "progress", message: "Waiting" });
    const account = await callbacks.prompt({ type: "select", message: "Choose account", options: [{ id: "personal", label: "Personal" }] });
    const secret = await callbacks.prompt({ type: "secret", message: "One-time secret", placeholder: "code" });
    return { type: "oauth", access: `access-${account}-${secret}`, refresh: "refresh-token", expires: 1234, accountId: "account-1" };
  });
  const service = new WorkerService(factory);
  const events: Event[] = [];
  const login = service.handle({ command: "oauth-login", request_id: "login-1", provider: { kind: "anthropic" } }, (event) => events.push(event));
  await nextTurn();

  assert.deepEqual(events.slice(0, 4).map((event) => event.type), ["oauth-event", "oauth-event", "oauth-event", "oauth-prompt"]);
  assert.deepEqual(events[1]?.event, { type: "device_code", user_code: "ABCD-EFGH", verification_uri: "https://example.com/device", interval_seconds: 5, expires_in_seconds: 600 });
  const selectPrompt = events[3];
  await service.handle({ command: "oauth-prompt-response", request_id: "response-1", target_request_id: "login-1", prompt_id: selectPrompt?.prompt_id as string, value: "personal" }, (event) => events.push(event));
  await nextTurn();
  const secretPrompt = events.find((event) => event.type === "oauth-prompt" && (event.prompt as Event).type === "secret");
  assert(secretPrompt);
  await service.handle({ command: "oauth-prompt-response", request_id: "response-2", target_request_id: "login-1", prompt_id: secretPrompt.prompt_id as string, value: "123456" }, (event) => events.push(event));
  await login;

  assert.deepEqual(events.at(-1), {
    type: "oauth-completed",
    request_id: "login-1",
    credential: { type: "oauth", access: "access-personal-123456", refresh: "refresh-token", expires: 1234, accountId: "account-1" },
  });
  assert.equal(events.some((event) => event.type === "acknowledged"), false);
  assert.equal(events.some((event) => event.type === "failed"), false);
});

test("isolates concurrent login prompts by request and prompt id", async () => {
  const service = new WorkerService(() => fakeAdapter(async (callbacks) => {
    const value = await callbacks.prompt({ type: "text", message: "Name" });
    return { type: "oauth", access: `access-${value}`, refresh: `refresh-${value}`, expires: 1234 };
  }));
  const events: Event[] = [];
  const emit = (event: Event) => events.push(event);
  const first = service.handle({ command: "oauth-login", request_id: "first", provider: { kind: "anthropic" } }, emit);
  const second = service.handle({ command: "oauth-login", request_id: "second", provider: { kind: "openai-codex" } }, emit);
  await nextTurn();
  const firstPrompt = events.find((event) => event.type === "oauth-prompt" && event.request_id === "first");
  const secondPrompt = events.find((event) => event.type === "oauth-prompt" && event.request_id === "second");
  assert(firstPrompt && secondPrompt);

  await service.handle({ command: "oauth-prompt-response", request_id: "wrong", target_request_id: "first", prompt_id: secondPrompt.prompt_id as string, value: "wrong" }, emit);
  assert.equal(events.at(-1)?.code, "invalid_prompt_response");
  await service.handle({ command: "oauth-prompt-response", request_id: "answer-second", target_request_id: "second", prompt_id: secondPrompt.prompt_id as string, value: "two" }, emit);
  await service.handle({ command: "oauth-prompt-response", request_id: "answer-first", target_request_id: "first", prompt_id: firstPrompt.prompt_id as string, value: "one" }, emit);
  await Promise.all([first, second]);

  const credentials = events.filter((event) => event.type === "oauth-completed").map((event) => event.credential as OAuthCredential);
  assert.deepEqual(credentials.map((credential) => credential.access).sort(), ["access-one", "access-two"]);
});

test("validates select responses and provider output", async () => {
  const service = new WorkerService(() => fakeAdapter(async (callbacks) => {
    await callbacks.prompt({ type: "select", message: "Choose", options: [{ id: "valid", label: "Valid" }] });
    callbacks.notify({ type: "auth_url", url: "file:///not-allowed" });
    return { type: "oauth", access: "access-token", refresh: "refresh-token", expires: 1234 };
  }));
  const events: Event[] = [];
  const login = service.handle({ command: "oauth-login", request_id: "login", provider: { kind: "anthropic" } }, (event) => events.push(event));
  await nextTurn();
  const prompt = events.find((event) => event.type === "oauth-prompt");
  assert(prompt);
  await service.handle({ command: "oauth-prompt-response", request_id: "bad-option", target_request_id: "login", prompt_id: prompt.prompt_id as string, value: "invalid" }, (event) => events.push(event));
  assert.equal(events.at(-1)?.code, "invalid_prompt_response");
  await service.handle({ command: "oauth-prompt-response", request_id: "good-option", target_request_id: "login", prompt_id: prompt.prompt_id as string, value: "valid" }, (event) => events.push(event));
  await login;
  assert.equal(events.at(-1)?.type, "failed");
  assert.equal(events.at(-1)?.code, "oauth_error");
  assert.equal(String(events.at(-1)?.message).includes("access-token"), false);
  assert.equal(String(events.at(-1)?.message).includes("refresh-token"), false);
});

test("redacts returned access and refresh tokens from completion errors", async () => {
  const service = new WorkerService(() => fakeAdapter(async () => ({
    type: "oauth",
    access: "returned-access-token",
    refresh: "returned-refresh-token",
    expires: 1234,
  })));
  const events: Event[] = [];
  await service.handle({ command: "oauth-login", request_id: "login", provider: { kind: "anthropic" } }, (event) => {
    if (event.type === "oauth-completed") throw new Error("sink rejected returned-access-token and returned-refresh-token");
    events.push(event);
  });
  assert.deepEqual(events, [{ type: "failed", request_id: "login", code: "oauth_error", message: "sink rejected [redacted] and [redacted]" }]);
});

test("cancel and shutdown abort login prompts", async () => {
  const service = new WorkerService(() => fakeAdapter(async (callbacks) => {
    await callbacks.prompt({ type: "manual_code", message: "Paste code" });
    return { type: "oauth", access: "access", refresh: "refresh", expires: 1234 };
  }));
  const events: Event[] = [];
  const emit = (event: Event) => events.push(event);
  const cancelled = service.handle({ command: "oauth-login", request_id: "cancelled", provider: { kind: "anthropic" } }, emit);
  await nextTurn();
  await service.handle({ command: "cancel", request_id: "cancel-command", target_request_id: "cancelled" }, emit);
  await cancelled;
  assert(events.some((event) => event.type === "failed" && event.request_id === "cancelled" && event.code === "cancelled"));

  const shutdownLogin = service.handle({ command: "oauth-login", request_id: "shutdown-login", provider: { kind: "anthropic" } }, emit);
  await nextTurn();
  assert.equal(await service.handle({ command: "shutdown", request_id: "shutdown" }, emit), true);
  await shutdownLogin;
  assert(events.some((event) => event.type === "failed" && event.request_id === "shutdown-login" && event.code === "cancelled"));
});

test("fails login when the selected provider has no OAuth adapter", async () => {
  const events: Event[] = [];
  await new WorkerService(() => undefined).handle({ command: "oauth-login", request_id: "login", provider: { kind: "openai" } }, (event) => events.push(event));
  assert.deepEqual(events, [{ type: "failed", request_id: "login", code: "oauth_error", message: "provider openai has no OAuth adapter" }]);
});

test("production selection exposes only built-in OAuth adapters", () => {
  assert.equal(oauthAdapter({ kind: "anthropic" })?.name, "Anthropic (Claude Pro/Max)");
  assert.equal(oauthAdapter({ kind: "openai-codex" })?.name, "OpenAI (ChatGPT Plus/Pro)");
  assert.equal(oauthAdapter({ kind: "github-copilot" })?.name, "GitHub Copilot");
  assert.equal(oauthAdapter({ kind: "openai" }), undefined);
});

test("OAuth generation scopes include Codex and Copilot built-ins", () => {
  const credential = { type: "oauth" as const, access: "access", refresh: "refresh", expires: Date.now() + 60_000 };
  const codex = createProviderScope({ kind: "openai-codex", oauth_credential: credential });
  assert(codex.models.getModels("openai-codex").length > 0);
  assert(codex.models.getModels("openai-codex").every((model) => model.baseUrl === "https://chatgpt.com/backend-api"));
  assert(createProviderScope({ kind: "github-copilot", oauth_credential: credential }).models.getModels("github-copilot").length > 0);
});

test("Codex model catalog is available before ChatGPT login", () => {
  const scope = createProviderScope({ kind: "openai-codex" });
  assert(scope.models.getModels("openai-codex").some((model) => model.id === "gpt-5.4-mini"));
});

test("Codex model discovery includes supported thinking variants", () => {
  const models = normalizedModels(createProviderScope({ kind: "openai-codex" }));
  const sol = models.find((model) => model.id === "gpt-5.6-sol");
  assert(sol);
  assert.deepEqual(sol.variants, ["minimal", "low", "medium", "high", "xhigh", "max"]);
  assert.deepEqual(sol.text_verbosity, ["low", "medium", "high"]);
});

test("provider catalog discovery never includes a configured model from another provider", async () => {
  const events: Event[] = [];
  await new WorkerService().handle({
    command: "models-list",
    request_id: "models",
    provider: { kind: "anthropic" },
  }, (event) => events.push(event));

  const result = events.find((event) => event.type === "models");
  assert(result);
  assert.equal((result.models as { id: string }[]).some((model) => model.id === "gpt-5.4-mini"), false);
});
