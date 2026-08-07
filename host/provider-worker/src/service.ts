import { randomUUID } from "node:crypto";
import { type AssistantMessage, type AuthEvent, type AuthPrompt, type OAuthAuth, type OAuthCredential } from "@earendil-works/pi-ai";
import { toHostResponse, toPiContext } from "./conversion.ts";
import { createProviderScope, normalizedModels, oauthAdapter, type ProviderScope } from "./providers.ts";
import { validateOAuthCredential, type InboundCommand, type OAuthProviderConfig, type ProviderConfig } from "./protocol.ts";

export type Emit = (message: Record<string, unknown>) => void;
export type OAuthAdapterFactory = (provider: OAuthProviderConfig) => OAuthAuth | undefined;
export type ProviderScopeFactory = (provider: ProviderConfig, model?: string) => ProviderScope;

const MAX_EVENT_URL_LENGTH = 8_192;
const MAX_EVENT_TEXT_LENGTH = 4_096;
const MAX_PLACEHOLDER_LENGTH = 2_048;
const MAX_OPTION_COUNT = 100;
const MAX_OPTION_ID_LENGTH = 256;
const MAX_OPTION_LABEL_LENGTH = 512;

interface PendingPrompt {
  allowedValues?: ReadonlySet<string>;
  resolve(value: string): void;
  reject(error: Error): void;
}

function abortError(message = "request cancelled"): Error {
  return Object.assign(new Error(message), { name: "AbortError" });
}

function boundedProviderString(value: unknown, label: string, maximum = MAX_EVENT_TEXT_LENGTH, allowEmpty = false): string {
  if (typeof value !== "string" || (!allowEmpty && value.length === 0) || value.length > maximum) {
    throw new Error(`OAuth provider returned invalid ${label}`);
  }
  return value;
}

function boundedProviderNumber(value: unknown, label: string): number | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) throw new Error(`OAuth provider returned invalid ${label}`);
  return value;
}

function httpUrl(value: unknown, label: string): string {
  const parsed = boundedProviderString(value, label, MAX_EVENT_URL_LENGTH);
  let url: URL;
  try { url = new URL(parsed); } catch { throw new Error(`OAuth provider returned invalid ${label}`); }
  if (url.protocol !== "http:" && url.protocol !== "https:") throw new Error(`OAuth provider returned invalid ${label}`);
  return parsed;
}

function oauthEvent(event: AuthEvent): Record<string, unknown> {
  if (event.type === "auth_url") {
    const instructions = event.instructions === undefined ? undefined : boundedProviderString(event.instructions, "auth instructions");
    return { type: "auth_url", url: httpUrl(event.url, "auth URL"), ...(instructions === undefined ? {} : { instructions }) };
  }
  if (event.type === "device_code") {
    const interval_seconds = boundedProviderNumber(event.intervalSeconds, "device-code interval");
    const expires_in_seconds = boundedProviderNumber(event.expiresInSeconds, "device-code expiry");
    return {
      type: "device_code",
      user_code: boundedProviderString(event.userCode, "device code", 512),
      verification_uri: httpUrl(event.verificationUri, "verification URL"),
      ...(interval_seconds === undefined ? {} : { interval_seconds }),
      ...(expires_in_seconds === undefined ? {} : { expires_in_seconds }),
    };
  }
  if (event.type === "progress") return { type: "progress", message: boundedProviderString(event.message, "progress message", MAX_EVENT_TEXT_LENGTH, true) };
  throw new Error("OAuth provider returned an unsupported event");
}

function oauthPrompt(prompt: AuthPrompt): { output: Record<string, unknown>; allowedValues?: ReadonlySet<string> } {
  const message = boundedProviderString(prompt.message, "prompt message");
  if (prompt.type === "select") {
    if (!Array.isArray(prompt.options) || prompt.options.length === 0 || prompt.options.length > MAX_OPTION_COUNT) {
      throw new Error("OAuth provider returned invalid prompt options");
    }
    const ids = new Set<string>();
    const options = prompt.options.map((option) => {
      const id = boundedProviderString(option.id, "prompt option id", MAX_OPTION_ID_LENGTH);
      if (ids.has(id)) throw new Error("OAuth provider returned duplicate prompt option ids");
      ids.add(id);
      const description = option.description === undefined ? undefined : boundedProviderString(option.description, "prompt option description");
      return { id, label: boundedProviderString(option.label, "prompt option label", MAX_OPTION_LABEL_LENGTH), ...(description === undefined ? {} : { description }) };
    });
    return { output: { type: "select", message, options }, allowedValues: ids };
  }
  if (prompt.type === "text" || prompt.type === "secret" || prompt.type === "manual_code") {
    const placeholder = prompt.placeholder === undefined ? undefined : boundedProviderString(prompt.placeholder, "prompt placeholder", MAX_PLACEHOLDER_LENGTH, true);
    return { output: { type: prompt.type, message, ...(placeholder === undefined ? {} : { placeholder }) } };
  }
  throw new Error("OAuth provider returned an unsupported prompt");
}

function redactedError(error: unknown, secrets: readonly string[] = []): string {
  const message = error instanceof Error ? error.message : "provider request failed";
  let redacted = message.replace(/(?:sk|key|token|secret|bearer)[-_a-z0-9]{8,}/gi, "[redacted]");
  for (const secret of secrets.filter((value) => value.length >= 4)) redacted = redacted.replaceAll(secret, "[redacted]");
  return redacted.replace(/(https?:\/\/)[^/@\s]+@/gi, "$1[redacted]@").replace(/([?&][^=\s]+)=([^&\s]+)/g, "$1=[redacted]").slice(0, 500);
}

function providerSecrets(provider: ProviderConfig | OAuthProviderConfig): string[] {
  if (!("api_key" in provider) && !("env" in provider) && !("oauth_credential" in provider)) return [];
  return [provider.api_key ?? "", ...Object.values(provider.env ?? {}), ...credentialStrings(provider.oauth_credential)];
}

function credentialStrings(credential: OAuthCredential | undefined): string[] {
  if (!credential) return [];
  const strings: string[] = [];
  const visit = (value: unknown): void => {
    if (typeof value === "string") strings.push(value);
    else if (Array.isArray(value)) value.forEach(visit);
    else if (typeof value === "object" && value !== null) Object.values(value).forEach(visit);
  };
  visit(credential);
  return strings;
}

function errorCode(error: unknown): string {
  if (error instanceof Error && error.name === "AbortError") return "cancelled";
  return "provider_error";
}

async function cleanupRequestResources(requestId: string): Promise<void> {
  const { cleanupSessionResources } = await import("../../node_modules/@earendil-works/pi-ai/dist/session-resources.js");
  cleanupSessionResources(requestId);
}

export function providerPayload(
  schema: Record<string, unknown> | undefined,
  textVerbosity: "low" | "medium" | "high" | undefined,
  payload: unknown,
  api: string,
): unknown | undefined {
  if (schema === undefined && textVerbosity === undefined) return undefined;
  if (typeof payload !== "object" || payload === null || Array.isArray(payload)) {
    throw new Error("provider payload is not an object");
  }
  const object = { ...payload } as Record<string, unknown>;
  if (schema !== undefined) {
    if (api === "openai-completions") {
      object.response_format = {
        type: "json_schema",
        json_schema: { name: "response", strict: true, schema },
      };
    } else if (api === "openai-responses" || api === "openai-codex-responses") {
      const text = typeof object.text === "object" && object.text !== null && !Array.isArray(object.text)
        ? object.text as Record<string, unknown>
        : {};
      object.text = { ...text, format: { type: "json_schema", name: "response", strict: true, schema } };
    } else {
      throw new Error(`structured output is unsupported for ${api}`);
    }
  }
  if (textVerbosity !== undefined) {
    if (api !== "openai-codex-responses") {
      throw new Error(`text verbosity is unsupported for ${api}`);
    }
    const text = typeof object.text === "object" && object.text !== null && !Array.isArray(object.text)
      ? object.text as Record<string, unknown>
      : {};
    object.text = { ...text, verbosity: textVerbosity };
  }
  return object;
}

export class WorkerService {
  readonly #active = new Map<string, AbortController>();
  readonly #pendingPrompts = new Map<string, Map<string, PendingPrompt>>();
  readonly #oauthAdapter: OAuthAdapterFactory;
  readonly #providerScope: ProviderScopeFactory;
  #shuttingDown = false;

  constructor(oauthAdapterFactory: OAuthAdapterFactory = oauthAdapter, providerScopeFactory: ProviderScopeFactory = createProviderScope) {
    this.#oauthAdapter = oauthAdapterFactory;
    this.#providerScope = providerScopeFactory;
  }

  #rejectPrompts(requestId: string, error: Error): void {
    const prompts = this.#pendingPrompts.get(requestId);
    if (!prompts) return;
    this.#pendingPrompts.delete(requestId);
    for (const prompt of prompts.values()) prompt.reject(error);
  }

  #prompt(requestId: string, prompt: AuthPrompt, emit: Emit): Promise<string> {
    const parsed = oauthPrompt(prompt);
    const promptId = randomUUID();
    return new Promise<string>((resolve, reject) => {
      const prompts = this.#pendingPrompts.get(requestId) ?? new Map<string, PendingPrompt>();
      this.#pendingPrompts.set(requestId, prompts);
      const settle = (action: () => void) => {
        prompt.signal?.removeEventListener("abort", onAbort);
        prompts.delete(promptId);
        if (prompts.size === 0) this.#pendingPrompts.delete(requestId);
        action();
      };
      const onAbort = () => settle(() => reject(abortError("OAuth prompt cancelled")));
      prompts.set(promptId, {
        allowedValues: parsed.allowedValues,
        resolve: (value) => settle(() => resolve(value)),
        reject: (error) => settle(() => reject(error)),
      });
      if (prompt.signal?.aborted) {
        onAbort();
        return;
      }
      prompt.signal?.addEventListener("abort", onAbort, { once: true });
      emit({ type: "oauth-prompt", request_id: requestId, prompt_id: promptId, prompt: parsed.output });
    });
  }

  #handlePromptResponse(command: Extract<InboundCommand, { command: "oauth-prompt-response" }>, emit: Emit): void {
    const prompt = this.#pendingPrompts.get(command.target_request_id)?.get(command.prompt_id);
    if (!prompt) {
      emit({ type: "failed", request_id: command.request_id, code: "invalid_prompt_response", message: "OAuth prompt is not active for target request" });
      return;
    }
    if (command.cancelled) prompt.reject(abortError("OAuth prompt cancelled by user"));
    else if (typeof command.value !== "string") {
      emit({ type: "failed", request_id: command.request_id, code: "invalid_prompt_response", message: "OAuth prompt response has no value" });
      return;
    } else if (prompt.allowedValues && !prompt.allowedValues.has(command.value)) {
      emit({ type: "failed", request_id: command.request_id, code: "invalid_prompt_response", message: "OAuth prompt response is not a listed option" });
      return;
    } else prompt.resolve(command.value);
  }

  async #login(command: Extract<InboundCommand, { command: "oauth-login" }>, controller: AbortController, emit: Emit): Promise<void> {
    let credentialSecrets: string[] = [];
    try {
      const adapter = this.#oauthAdapter(command.provider);
      if (!adapter) throw new Error(`provider ${command.provider.kind} has no OAuth adapter`);
      const rawCredential = await adapter.login({
        signal: controller.signal,
        notify: (event) => emit({ type: "oauth-event", request_id: command.request_id, event: oauthEvent(event) }),
        prompt: (prompt) => this.#prompt(command.request_id, prompt, emit),
      });
      credentialSecrets = credentialStrings(rawCredential);
      if (controller.signal.aborted) throw abortError();
      emit({ type: "oauth-completed", request_id: command.request_id, credential: validateOAuthCredential(rawCredential, "OAuth provider credential") });
    } catch (error) {
      const cancelled = controller.signal.aborted || errorCode(error) === "cancelled";
      emit({ type: "failed", request_id: command.request_id, code: cancelled ? "cancelled" : "oauth_error", message: cancelled ? "request cancelled" : redactedError(error, credentialSecrets) });
    } finally {
      this.#rejectPrompts(command.request_id, abortError("OAuth login ended"));
    }
  }

  async handle(command: InboundCommand, emit: Emit): Promise<boolean> {
    if (command.command === "oauth-prompt-response") {
      this.#handlePromptResponse(command, emit);
      return false;
    }
    if (command.command === "cancel") {
      const controller = this.#active.get(command.target_request_id);
      controller?.abort();
      this.#rejectPrompts(command.target_request_id, abortError());
      emit({ type: "acknowledged", request_id: command.request_id, command: "cancel", target_request_id: command.target_request_id, accepted: controller !== undefined });
      return false;
    }
    if (command.command === "shutdown") {
      this.#shuttingDown = true;
      for (const controller of this.#active.values()) controller.abort();
      for (const requestId of this.#pendingPrompts.keys()) this.#rejectPrompts(requestId, abortError());
      emit({ type: "acknowledged", request_id: command.request_id, command: "shutdown" });
      return true;
    }
    if (this.#shuttingDown) {
      emit({ type: "failed", request_id: command.request_id, code: "shutting_down", message: "worker is shutting down" });
      return false;
    }
    if (command.command === "models-list" || command.command === "models-refresh") {
      let scope: ProviderScope | undefined;
      try {
        scope = this.#providerScope(command.provider);
        if (command.provider.oauth_credential) {
          const authModel = scope.model ?? scope.models.getModels(scope.providerId)[0];
          if (authModel) await scope.models.getAuth(authModel);
        }
        if (command.command === "models-refresh") await scope.models.refresh(scope.providerId);
        const credential = await scope.updatedCredential();
        emit({ type: "models", request_id: command.request_id, models: normalizedModels(scope), ...(credential ? { credential } : {}) });
      } catch (error) {
        const credential = await scope?.updatedCredential();
        emit({ type: "failed", request_id: command.request_id, code: "models_error", message: redactedError(error, [...providerSecrets(command.provider), ...credentialStrings(credential)]), ...(credential ? { credential } : {}) });
      }
      return false;
    }
    if (this.#active.has(command.request_id)) {
      emit({ type: "failed", request_id: command.request_id, code: "duplicate_request", message: "request_id is already active" });
      return false;
    }
    const controller = new AbortController();
    this.#active.set(command.request_id, controller);
    if (command.command === "oauth-login") {
      try {
        await this.#login(command, controller, emit);
      } finally {
        this.#active.delete(command.request_id);
      }
      return false;
    }
    let scope: ProviderScope | undefined;
    try {
      scope = this.#providerScope(command.provider, command.model);
      if (!scope.model) throw new Error(`unknown model ${command.model}`);
      const context = toPiContext(command.messages, command.tools, scope.model);
      const startedAt = performance.now();
      const stream = scope.models.streamSimple(scope.model, context, {
        signal: controller.signal,
        apiKey: command.provider.api_key,
        env: command.provider.env,
        reasoning: command.reasoning,
        sessionId: command.request_id,
        temperature: command.temperature,
        maxTokens: command.max_output_tokens,
        timeoutMs: command.timeout_ms,
        maxRetries: 0,
        onPayload: (payload, model) => providerPayload(command.response_format, command.text_verbosity, payload, model.api),
      });
      let final: AssistantMessage | undefined;
      let firstTokenAt: number | undefined;
      for await (const event of stream) {
        if (event.type === "text_delta") {
          if (event.delta && firstTokenAt === undefined) firstTokenAt = performance.now();
          emit({ type: "stream-delta", request_id: command.request_id, content: event.delta, reasoning: "" });
        }
        if (event.type === "thinking_delta") {
          if (event.delta && firstTokenAt === undefined) firstTokenAt = performance.now();
          emit({ type: "stream-delta", request_id: command.request_id, content: "", reasoning: event.delta });
        }
        if (event.type === "done") final = event.message;
        if (event.type === "error") throw Object.assign(new Error(event.error.errorMessage ?? "provider stream failed"), { name: event.reason === "aborted" ? "AbortError" : "ProviderError" });
      }
      final ??= await stream.result();
      const credential = await scope.updatedCredential();
      const response = toHostResponse(final);
      response.provider_metrics = {
        ...(firstTokenAt === undefined
          ? {}
          : { time_to_first_token_ms: Math.round(firstTokenAt - startedAt) }),
        total_latency_ms: Math.round(performance.now() - startedAt),
      };
      emit({ type: "completed", request_id: command.request_id, response, ...(credential ? { credential } : {}) });
    } catch (error) {
      const cancelled = controller.signal.aborted || errorCode(error) === "cancelled";
      const credential = await scope?.updatedCredential();
      emit({ type: "failed", request_id: command.request_id, code: cancelled ? "cancelled" : errorCode(error), message: cancelled ? "request cancelled" : redactedError(error, [...providerSecrets(command.provider), ...credentialStrings(credential)]), ...(credential ? { credential } : {}) });
    } finally {
      await cleanupRequestResources(command.request_id);
      this.#active.delete(command.request_id);
    }
    return false;
  }
}
