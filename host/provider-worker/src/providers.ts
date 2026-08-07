import { createModels, createProvider, getSupportedThinkingLevels, type Api, type AuthContext, type Credential, type CredentialStore, type Model, type Models, type OAuthAuth, type OAuthCredential, type Provider, type ThinkingLevel } from "@earendil-works/pi-ai";
import { openAICompletionsApi } from "@earendil-works/pi-ai/api/openai-completions.lazy";
import { amazonBedrockProvider } from "@earendil-works/pi-ai/providers/amazon-bedrock";
import { anthropicProvider } from "@earendil-works/pi-ai/providers/anthropic";
import { googleProvider } from "@earendil-works/pi-ai/providers/google";
import { githubCopilotProvider } from "@earendil-works/pi-ai/providers/github-copilot";
import { mistralProvider } from "@earendil-works/pi-ai/providers/mistral";
import { openaiProvider } from "@earendil-works/pi-ai/providers/openai";
import { openaiCodexProvider } from "@earendil-works/pi-ai/providers/openai-codex";
import { openrouterProvider } from "@earendil-works/pi-ai/providers/openrouter";
// pi-ai's lazy wrappers deliberately hide these imports from bundlers. The worker is
// a standalone bundle without node_modules, so materialize the same adapters here.
import { anthropicOAuth } from "../../node_modules/@earendil-works/pi-ai/dist/utils/oauth/anthropic.js";
import { githubCopilotOAuth } from "../../node_modules/@earendil-works/pi-ai/dist/utils/oauth/github-copilot.js";
import { openaiCodexOAuth } from "../../node_modules/@earendil-works/pi-ai/dist/utils/oauth/openai-codex.js";
import { validateOAuthCredential, type OAuthProviderConfig, type ProviderConfig, type TextVerbosity } from "./protocol.ts";

const NO_AMBIENT_AUTH: AuthContext = { env: async () => undefined, fileExists: async () => false };
const ZERO_COST = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 };
const DISCOVERY_MODEL = "__kestral_discovery__";
const TEXT_VERBOSITY_LEVELS: TextVerbosity[] = ["low", "medium", "high"];

function conservativeModel(provider: Provider, id: string, baseUrl?: string): Model<Api> {
  const template = provider.getModels()[0];
  if (!template) throw new Error(`provider ${provider.id} has no known API for custom model ${id}`);
  return { id, name: id, api: template.api, provider: provider.id, baseUrl: baseUrl ?? provider.baseUrl ?? template.baseUrl, reasoning: false, input: ["text"], cost: ZERO_COST, contextWindow: 16_384, maxTokens: 4_096 };
}

function customOpenAiProvider(config: ProviderConfig, modelId: string): Provider<"openai-completions"> {
  const baseUrl = config.base_url ?? (config.kind === "ollama" ? "http://127.0.0.1:11434/v1" : undefined);
  if (!baseUrl) throw new Error("open-ai-compatible requires provider.base_url");
  const model: Model<"openai-completions"> = { id: modelId, name: modelId, api: "openai-completions", provider: config.kind, baseUrl, reasoning: false, input: ["text"], cost: ZERO_COST, contextWindow: 16_384, maxTokens: 4_096, compat: { supportsStore: false, supportsDeveloperRole: false, supportsReasoningEffort: false, supportsUsageInStreaming: false, maxTokensField: "max_tokens" } };
  return createProvider({
    id: config.kind,
    name: config.kind,
    baseUrl,
    models: [model],
    refreshModels: async () => discoverOpenAiCompatibleModels(config, model),
    api: openAICompletionsApi(),
    auth: { apiKey: { name: "Request API key", resolve: async ({ credential }) => ({ auth: { apiKey: credential?.key } }) } },
  });
}

function discoveredModel(template: Model<"openai-completions">, id: string): Model<"openai-completions"> {
  return { ...template, id, name: id };
}

async function discoverOpenAiCompatibleModels(
  config: ProviderConfig,
  template: Model<"openai-completions">,
): Promise<Model<"openai-completions">[]> {
  const configuredBase = config.base_url ?? "http://127.0.0.1:11434";
  const url = config.kind === "ollama"
    ? `${configuredBase.replace(/\/v1\/?$/, "").replace(/\/$/, "")}/api/tags`
    : `${configuredBase.replace(/\/$/, "")}/models`;
  const headers = new Headers({ accept: "application/json" });
  if (config.api_key) headers.set("authorization", `Bearer ${config.api_key}`);
  const response = await fetch(url, { headers, signal: AbortSignal.timeout(15_000) });
  if (!response.ok) throw new Error(`model discovery returned HTTP ${response.status}`);
  const body: unknown = await response.json();
  if (typeof body !== "object" || body === null || Array.isArray(body)) throw new Error("model discovery response must be an object");
  const records = config.kind === "ollama"
    ? (body as Record<string, unknown>).models
    : (body as Record<string, unknown>).data;
  if (!Array.isArray(records)) throw new Error("model discovery response has no model array");
  const ids = records.map((record) => {
    if (typeof record !== "object" || record === null || Array.isArray(record)) throw new Error("model discovery returned an invalid model");
    const value = config.kind === "ollama"
      ? (record as Record<string, unknown>).name
      : (record as Record<string, unknown>).id;
    if (typeof value !== "string" || value.length === 0) throw new Error("model discovery returned a model without an id");
    return value;
  });
  return [...new Set(ids)].sort().map((id) => discoveredModel(template, id));
}

function withBundledOAuth(provider: Provider, oauth: OAuthAuth): Provider {
  return { ...provider, auth: { ...provider.auth, oauth } };
}

function builtinProvider(config: ProviderConfig): Provider {
  switch (config.kind) {
    case "openai": return openaiProvider();
    case "openai-codex": return withBundledOAuth(openaiCodexProvider(), openaiCodexOAuth);
    case "anthropic": return withBundledOAuth(anthropicProvider(), anthropicOAuth);
    case "github-copilot": return withBundledOAuth(githubCopilotProvider(), githubCopilotOAuth);
    case "openrouter": return openrouterProvider();
    case "google": return googleProvider();
    case "mistral": return mistralProvider();
    case "amazon-bedrock": return amazonBedrockProvider();
    default: throw new Error(`provider ${config.kind} is not built in`);
  }
}

export function oauthAdapter(config: OAuthProviderConfig): OAuthAuth | undefined {
  return builtinProvider(config).auth.oauth;
}

function validateBedrockConfig(config: ProviderConfig): void {
  if (config.kind !== "amazon-bedrock") return;
  const env = config.env ?? {};
  const fileBacked = ["AWS_PROFILE", "AWS_CONFIG_FILE", "AWS_SHARED_CREDENTIALS_FILE", "AWS_WEB_IDENTITY_TOKEN_FILE"];
  if (fileBacked.some((name) => env[name] !== undefined)) throw new Error("amazon-bedrock file-backed credentials are not permitted");
  const staticCredentials = Boolean(env.AWS_ACCESS_KEY_ID && env.AWS_SECRET_ACCESS_KEY);
  const bearer = Boolean(config.api_key || env.AWS_BEARER_TOKEN_BEDROCK);
  const skipAuth = env.AWS_BEDROCK_SKIP_AUTH === "1";
  if (!staticCredentials && !bearer && !skipAuth) throw new Error("amazon-bedrock requires explicit request credentials or AWS_BEDROCK_SKIP_AUTH=1");
}

export class RequestCredentialStore implements CredentialStore {
  readonly #credentials = new Map<string, Credential>();
  readonly #chains = new Map<string, Promise<void>>();

  constructor(providerId: string, credential?: OAuthCredential) {
    if (credential) this.#credentials.set(providerId, validateOAuthCredential(credential));
  }

  #enqueue<T>(providerId: string, task: () => Promise<T>): Promise<T> {
    const previous = this.#chains.get(providerId) ?? Promise.resolve();
    const operation = (async () => {
      await previous.catch(() => undefined);
      return task();
    })();
    this.#chains.set(providerId, operation.then(() => undefined, () => undefined));
    return operation;
  }

  async read(providerId: string): Promise<Credential | undefined> {
    return this.#credentials.get(providerId);
  }

  modify(providerId: string, update: (current: Credential | undefined) => Promise<Credential | undefined>): Promise<Credential | undefined> {
    return this.#enqueue(providerId, async () => {
      const current = this.#credentials.get(providerId);
      const proposed = await update(current);
      if (proposed !== undefined) {
        if (proposed.type !== "oauth") throw new Error("request credential store accepts only OAuth credentials");
        this.#credentials.set(providerId, validateOAuthCredential(proposed, "OAuth provider credential"));
      }
      return this.#credentials.get(providerId);
    });
  }

  delete(providerId: string): Promise<void> {
    return this.#enqueue(providerId, async () => {
      this.#credentials.delete(providerId);
    });
  }
}

function canonicalJson(value: unknown): string {
  const sort = (entry: unknown): unknown => {
    if (Array.isArray(entry)) return entry.map(sort);
    if (typeof entry !== "object" || entry === null) return entry;
    return Object.fromEntries(Object.entries(entry).sort(([left], [right]) => left.localeCompare(right)).map(([key, child]) => [key, sort(child)]));
  };
  return JSON.stringify(sort(value));
}

export interface ProviderScope {
  models: Models;
  providerId: string;
  model?: Model<Api>;
  credentialStore: RequestCredentialStore;
  updatedCredential(): Promise<OAuthCredential | undefined>;
}

export type ProviderFactory = (config: ProviderConfig) => Provider;

function providerScope(models: Models, providerId: string, credentialStore: RequestCredentialStore, initial?: OAuthCredential, model?: Model<Api>): ProviderScope {
  const initialJson = initial === undefined ? undefined : canonicalJson(initial);
  return {
    models,
    providerId,
    credentialStore,
    ...(model ? { model } : {}),
    updatedCredential: async () => {
      const current = await credentialStore.read(providerId);
      if (current?.type !== "oauth" || canonicalJson(current) === initialJson) return undefined;
      return validateOAuthCredential(current, "OAuth provider credential");
    },
  };
}

export function createProviderScope(config: ProviderConfig, modelId?: string, providerFactory: ProviderFactory = builtinProvider): ProviderScope {
  validateBedrockConfig(config);
  if (config.kind === "ollama" || config.kind === "open-ai-compatible") {
    const provider = customOpenAiProvider(config, modelId ?? DISCOVERY_MODEL);
    if (config.oauth_credential) throw new Error(`provider ${config.kind} does not support OAuth credentials`);
    const credentialStore = new RequestCredentialStore(provider.id);
    const models = createModels({ authContext: NO_AMBIENT_AUTH, credentials: credentialStore });
    models.setProvider(provider);
    return providerScope(models, provider.id, credentialStore, undefined, provider.getModels()[0]);
  }
  let provider = providerFactory(config);
  if (config.oauth_credential && !provider.auth.oauth) throw new Error(`provider ${config.kind} does not support OAuth credentials`);
  const credentialStore = new RequestCredentialStore(provider.id, config.oauth_credential);
  const models = createModels({ authContext: NO_AMBIENT_AUTH, credentials: credentialStore });
  let selected = modelId ? provider.getModels().find((model) => model.id === modelId) : undefined;
  if (selected && config.base_url) selected = { ...selected, baseUrl: config.base_url };
  if (!selected && modelId) selected = conservativeModel(provider, modelId, config.base_url);
  if (selected && !provider.getModels().some((model) => model.id === selected?.id && model.baseUrl === selected?.baseUrl)) {
    provider = createProvider({ id: provider.id, name: provider.name, baseUrl: provider.baseUrl, headers: provider.headers, auth: provider.auth, models: [...provider.getModels().filter((model) => model.id !== selected?.id), selected], api: { stream: (model, context, options) => providerFactory(config).stream(model, context, options), streamSimple: (model, context, options) => providerFactory(config).streamSimple(model, context, options) } });
  }
  models.setProvider(provider);
  return providerScope(models, provider.id, credentialStore, config.oauth_credential, selected);
}

export function normalizedModels(scope: ProviderScope) {
  return scope.models.getModels(scope.providerId).filter((model) => model.id !== DISCOVERY_MODEL).map((model) => ({
    id: model.id,
    display_name: model.name,
    reasoning: model.reasoning,
    variants: getSupportedThinkingLevels(model).filter((level): level is ThinkingLevel => level !== "off"),
    text_verbosity: model.api === "openai-codex-responses" ? TEXT_VERBOSITY_LEVELS : [],
    context_window: model.contextWindow,
    max_output_tokens: model.maxTokens,
  }));
}
