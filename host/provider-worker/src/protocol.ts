export const PROVIDER_KINDS = [
  "ollama",
  "open-ai-compatible",
  "openai",
  "openai-codex",
  "anthropic",
  "github-copilot",
  "openrouter",
  "google",
  "mistral",
  "amazon-bedrock",
] as const;

export type ProviderKind = (typeof PROVIDER_KINDS)[number];
export type TextVerbosity = "low" | "medium" | "high";

export interface ProviderConfig {
  kind: ProviderKind;
  base_url?: string;
  api_key?: string;
  oauth_credential?: OAuthCredential;
  env?: Record<string, string>;
}

export interface OAuthProviderConfig {
  kind: ProviderKind;
  base_url?: string;
}

export interface HostToolCall {
  id: string;
  type: "function";
  function: { name: string; arguments: string };
}

export type HostMessage =
  | { role: "system"; content: string }
  | { role: "user"; content: string }
  | { role: "assistant"; content: string; tool_calls?: HostToolCall[] }
  | { role: "tool"; content: string; tool_call_id: string; name?: string };

export interface HostTool {
  type: "function";
  function: { name: string; description: string; parameters: Record<string, unknown> };
}

export type InboundCommand =
  | {
      command: "generate";
      request_id: string;
      provider: ProviderConfig;
      model: string;
      messages: HostMessage[];
      tools?: HostTool[];
      response_format?: Record<string, unknown>;
      reasoning?: "minimal" | "low" | "medium" | "high" | "xhigh" | "max";
      text_verbosity?: TextVerbosity;
      temperature?: number;
      max_output_tokens?: number;
      timeout_ms?: number;
    }
  | { command: "models-list"; request_id: string; provider: ProviderConfig }
  | { command: "models-refresh"; request_id: string; provider: ProviderConfig }
  | { command: "oauth-login"; request_id: string; provider: OAuthProviderConfig }
  | { command: "oauth-prompt-response"; request_id: string; target_request_id: string; prompt_id: string; value?: string; cancelled?: true }
  | { command: "cancel"; request_id: string; target_request_id: string }
  | { command: "shutdown"; request_id: string };

export class ProtocolError extends Error {
  readonly code = "invalid_request";
}

type JsonObject = Record<string, unknown>;
const MAX_REQUEST_ID_LENGTH = 128;
const MAX_PROMPT_VALUE_LENGTH = 16_384;
const MAX_URL_LENGTH = 8_192;
const MAX_CREDENTIAL_TOKEN_LENGTH = 262_144;
const MAX_CREDENTIAL_EXTRA_STRING_LENGTH = 16_384;
const MAX_CREDENTIAL_JSON_LENGTH = 1_048_576;
const MAX_CREDENTIAL_DEPTH = 12;
const MAX_CREDENTIAL_NODES = 4_096;
const MAX_CREDENTIAL_KEYS = 256;
const MAX_CREDENTIAL_ARRAY_LENGTH = 1_024;
const MAX_CREDENTIAL_KEY_LENGTH = 256;
const DANGEROUS_KEYS = new Set(["__proto__", "prototype", "constructor"]);

function object(value: unknown, label: string): JsonObject {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new ProtocolError(`${label} must be an object`);
  }
  return value as JsonObject;
}

function exact(value: JsonObject, allowed: readonly string[], label: string): void {
  const unknown = Object.keys(value).find((key) => !allowed.includes(key));
  if (unknown) throw new ProtocolError(`${label} contains unknown field ${unknown}`);
}

function string(value: unknown, label: string, allowEmpty = false): string {
  if (typeof value !== "string" || (!allowEmpty && value.length === 0)) {
    throw new ProtocolError(`${label} must be a non-empty string`);
  }
  return value;
}

function boundedString(value: unknown, label: string, maximum: number, allowEmpty = false): string {
  const parsed = string(value, label, allowEmpty);
  if (parsed.length > maximum) throw new ProtocolError(`${label} is too long`);
  return parsed;
}

function optionalNumber(value: unknown, label: string, integer = false): number | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== "number" || !Number.isFinite(value) || (integer && !Number.isInteger(value))) {
    throw new ProtocolError(`${label} must be a finite${integer ? " integer" : " number"}`);
  }
  return value;
}

function parseHttpUrl(value: unknown, label: string): string {
  const parsed = boundedString(value, label, MAX_URL_LENGTH);
  let url: URL;
  try { url = new URL(parsed); } catch { throw new ProtocolError(`${label} must be an absolute HTTP URL`); }
  if (url.protocol !== "http:" && url.protocol !== "https:") throw new ProtocolError(`${label} must be an HTTP URL`);
  return parsed;
}

function parseProviderKind(value: unknown): ProviderKind {
  const kind = boundedString(value, "provider.kind", 64) as ProviderKind;
  if (!(PROVIDER_KINDS as readonly string[]).includes(kind)) throw new ProtocolError(`unsupported provider kind ${kind}`);
  return kind;
}

export function validateOAuthCredential(value: unknown, label = "provider.oauth_credential"): OAuthCredential {
  const root = object(value, label);
  const prototype = Object.getPrototypeOf(root);
  if (prototype !== Object.prototype && prototype !== null) throw new ProtocolError(`${label} must be a plain JSON object`);
  const mandatory = (key: string): unknown => {
    const descriptor = Object.getOwnPropertyDescriptor(root, key);
    if (!descriptor || !("value" in descriptor) || !descriptor.enumerable) throw new ProtocolError(`${label}.${key} must be a JSON data property`);
    return descriptor.value;
  };
  if (mandatory("type") !== "oauth") throw new ProtocolError(`${label}.type must be oauth`);
  boundedString(mandatory("access"), `${label}.access`, MAX_CREDENTIAL_TOKEN_LENGTH);
  boundedString(mandatory("refresh"), `${label}.refresh`, MAX_CREDENTIAL_TOKEN_LENGTH);
  const expires = mandatory("expires");
  if (typeof expires !== "number" || !Number.isFinite(expires) || expires < 0) throw new ProtocolError(`${label}.expires must be a finite non-negative number`);

  let nodes = 0;
  const seen = new Set<object>();
  const validateJson = (entry: unknown, path: string, depth: number): void => {
    nodes += 1;
    if (nodes > MAX_CREDENTIAL_NODES) throw new ProtocolError(`${label} contains too many values`);
    if (depth > MAX_CREDENTIAL_DEPTH) throw new ProtocolError(`${label} is too deeply nested`);
    if (entry === null || typeof entry === "boolean") return;
    if (typeof entry === "number") {
      if (!Number.isFinite(entry)) throw new ProtocolError(`${path} must be a finite number`);
      return;
    }
    if (typeof entry === "string") {
      if (entry.length > MAX_CREDENTIAL_EXTRA_STRING_LENGTH) throw new ProtocolError(`${path} is too long`);
      return;
    }
    if (typeof entry !== "object") throw new ProtocolError(`${path} must be JSON-safe`);
    if (seen.has(entry)) throw new ProtocolError(`${label} must not be cyclic`);
    seen.add(entry);
    if (Array.isArray(entry)) {
      if (Object.getPrototypeOf(entry) !== Array.prototype) throw new ProtocolError(`${path} must be a plain JSON array`);
      if (entry.length > MAX_CREDENTIAL_ARRAY_LENGTH) throw new ProtocolError(`${path} contains too many entries`);
      const keys = Reflect.ownKeys(entry);
      if (keys.some((key) => {
        if (key === "length") return false;
        if (typeof key !== "string" || !/^(0|[1-9][0-9]*)$/.test(key)) return true;
        const index = Number(key);
        return !Number.isSafeInteger(index) || index >= entry.length;
      })) throw new ProtocolError(`${path} contains invalid array fields`);
      for (let index = 0; index < entry.length; index += 1) {
        const descriptor = Object.getOwnPropertyDescriptor(entry, String(index));
        if (!descriptor || !("value" in descriptor) || !descriptor.enumerable) throw new ProtocolError(`${path} must not be sparse or contain accessors`);
        validateJson(descriptor.value, `${path}[${index}]`, depth + 1);
      }
    } else {
      const entryPrototype = Object.getPrototypeOf(entry);
      if (entryPrototype !== Object.prototype && entryPrototype !== null) throw new ProtocolError(`${path} must be a plain JSON object`);
      const keys = Reflect.ownKeys(entry);
      if (keys.length > MAX_CREDENTIAL_KEYS) throw new ProtocolError(`${path} contains too many fields`);
      for (const key of keys) {
        if (typeof key !== "string") throw new ProtocolError(`${path} contains a non-string field name`);
        if (key.length === 0 || key.length > MAX_CREDENTIAL_KEY_LENGTH) throw new ProtocolError(`${path} contains an invalid field name`);
        if (DANGEROUS_KEYS.has(key)) throw new ProtocolError(`${path} contains dangerous field ${key}`);
        const descriptor = Object.getOwnPropertyDescriptor(entry, key);
        if (!descriptor || !("value" in descriptor) || !descriptor.enumerable) throw new ProtocolError(`${path}.${key} must be a JSON data property`);
        if (depth === 0 && (key === "type" || key === "access" || key === "refresh" || key === "expires")) continue;
        validateJson(descriptor.value, `${path}.${key}`, depth + 1);
      }
    }
    seen.delete(entry);
  };
  validateJson(root, label, 0);
  let serialized: string;
  try { serialized = JSON.stringify(root); } catch { throw new ProtocolError(`${label} must be JSON-safe`); }
  if (serialized.length > MAX_CREDENTIAL_JSON_LENGTH) throw new ProtocolError(`${label} is too large`);
  return JSON.parse(serialized) as OAuthCredential;
}

function parseProvider(value: unknown): ProviderConfig {
  const input = object(value, "provider");
  exact(input, ["kind", "base_url", "api_key", "oauth_credential", "env"], "provider");
  const kind = parseProviderKind(input.kind);
  const base_url = input.base_url === undefined ? undefined : parseHttpUrl(input.base_url, "provider.base_url");
  const api_key = input.api_key === undefined ? undefined : string(input.api_key, "provider.api_key", true);
  const oauth_credential = input.oauth_credential === undefined ? undefined : validateOAuthCredential(input.oauth_credential);
  if (api_key !== undefined && oauth_credential !== undefined) throw new ProtocolError("provider.api_key and provider.oauth_credential are mutually exclusive");
  let env: Record<string, string> | undefined;
  if (input.env !== undefined) {
    const raw = object(input.env, "provider.env");
    env = {};
    for (const [key, entry] of Object.entries(raw)) {
      if (!key || typeof entry !== "string") throw new ProtocolError("provider.env must map non-empty names to strings");
      env[key] = entry;
    }
  }
  return { kind, ...(base_url === undefined ? {} : { base_url }), ...(api_key === undefined ? {} : { api_key }), ...(oauth_credential === undefined ? {} : { oauth_credential }), ...(env === undefined ? {} : { env }) };
}

function parseOAuthProvider(value: unknown): OAuthProviderConfig {
  const input = object(value, "provider");
  exact(input, ["kind", "base_url"], "provider");
  const kind = parseProviderKind(input.kind);
  const base_url = input.base_url === undefined ? undefined : parseHttpUrl(input.base_url, "provider.base_url");
  return { kind, ...(base_url === undefined ? {} : { base_url }) };
}

function parseToolCall(value: unknown, label: string): HostToolCall {
  const input = object(value, label);
  exact(input, ["id", "type", "function"], label);
  if (input.type !== "function") throw new ProtocolError(`${label}.type must be function`);
  const fn = object(input.function, `${label}.function`);
  exact(fn, ["name", "arguments"], `${label}.function`);
  const args = string(fn.arguments, `${label}.function.arguments`, true);
  try {
    const parsed = JSON.parse(args);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) throw new Error();
  } catch { throw new ProtocolError(`${label}.function.arguments must encode a JSON object`); }
  return { id: string(input.id, `${label}.id`), type: "function", function: { name: string(fn.name, `${label}.function.name`), arguments: args } };
}

function parseMessage(value: unknown, index: number): HostMessage {
  const label = `messages[${index}]`;
  const input = object(value, label);
  const role = string(input.role, `${label}.role`);
  if (role === "system" || role === "user") {
    exact(input, ["role", "content"], label);
    return { role, content: string(input.content, `${label}.content`, true) };
  }
  if (role === "assistant") {
    exact(input, ["role", "content", "tool_calls"], label);
    const tool_calls = input.tool_calls === undefined ? undefined : parseArray(input.tool_calls, `${label}.tool_calls`).map((call, i) => parseToolCall(call, `${label}.tool_calls[${i}]`));
    return { role, content: string(input.content, `${label}.content`, true), ...(tool_calls === undefined ? {} : { tool_calls }) };
  }
  if (role === "tool") {
    exact(input, ["role", "content", "tool_call_id", "name"], label);
    const name = input.name === undefined ? undefined : string(input.name, `${label}.name`);
    return { role, content: string(input.content, `${label}.content`, true), tool_call_id: string(input.tool_call_id, `${label}.tool_call_id`), ...(name === undefined ? {} : { name }) };
  }
  throw new ProtocolError(`${label}.role is unsupported`);
}

function validateJsonSchema(value: unknown, label: string, seen = new Set<unknown>()): asserts value is Record<string, unknown> {
  const schema = object(value, label);
  if (seen.has(schema)) throw new ProtocolError(`${label} must not be cyclic`);
  seen.add(schema);
  if (schema.type !== undefined) {
    const valid = ["null", "boolean", "object", "array", "number", "integer", "string"];
    const types = Array.isArray(schema.type) ? schema.type : [schema.type];
    if (types.length === 0 || types.some((type) => typeof type !== "string" || !valid.includes(type))) throw new ProtocolError(`${label}.type is invalid`);
  }
  if (schema.properties !== undefined) {
    const properties = object(schema.properties, `${label}.properties`);
    for (const [name, child] of Object.entries(properties)) validateJsonSchema(child, `${label}.properties.${name}`, seen);
  }
  if (schema.items !== undefined && !Array.isArray(schema.items)) validateJsonSchema(schema.items, `${label}.items`, seen);
  if (schema.required !== undefined && (!Array.isArray(schema.required) || schema.required.some((entry) => typeof entry !== "string"))) {
    throw new ProtocolError(`${label}.required must be an array of strings`);
  }
  seen.delete(schema);
}

function parseTool(value: unknown, index: number): HostTool {
  const label = `tools[${index}]`;
  const input = object(value, label);
  exact(input, ["type", "function"], label);
  if (input.type !== "function") throw new ProtocolError(`${label}.type must be function`);
  const fn = object(input.function, `${label}.function`);
  exact(fn, ["name", "description", "parameters"], `${label}.function`);
  validateJsonSchema(fn.parameters, `${label}.function.parameters`);
  return { type: "function", function: { name: string(fn.name, `${label}.function.name`), description: string(fn.description, `${label}.function.description`, true), parameters: fn.parameters } };
}

function parseArray(value: unknown, label: string): unknown[] {
  if (!Array.isArray(value)) throw new ProtocolError(`${label} must be an array`);
  return value;
}

export function requestIdHint(value: unknown): string {
  if (typeof value === "object" && value !== null && !Array.isArray(value) && typeof (value as JsonObject).request_id === "string") return ((value as JsonObject).request_id as string).slice(0, MAX_REQUEST_ID_LENGTH);
  return "";
}

export function parseCommand(value: unknown): InboundCommand {
  const input = object(value, "command");
  const command = string(input.command, "command.command");
  const request_id = boundedString(input.request_id, "command.request_id", MAX_REQUEST_ID_LENGTH);
  if (command === "generate") {
    exact(input, ["command", "request_id", "provider", "model", "messages", "tools", "response_format", "reasoning", "text_verbosity", "temperature", "max_output_tokens", "timeout_ms"], "generate");
    const temperature = optionalNumber(input.temperature, "generate.temperature");
    const max_output_tokens = optionalNumber(input.max_output_tokens, "generate.max_output_tokens", true);
    const timeout_ms = optionalNumber(input.timeout_ms, "generate.timeout_ms", true);
    if (max_output_tokens !== undefined && max_output_tokens <= 0) throw new ProtocolError("generate.max_output_tokens must be positive");
    if (timeout_ms !== undefined && timeout_ms <= 0) throw new ProtocolError("generate.timeout_ms must be positive");
    const tools = input.tools === undefined ? undefined : parseArray(input.tools, "generate.tools").map(parseTool);
    let response_format: Record<string, unknown> | undefined;
    if (input.response_format !== undefined) {
      validateJsonSchema(input.response_format, "generate.response_format");
      response_format = input.response_format;
    }
    const reasoning = input.reasoning === undefined ? undefined : string(input.reasoning, "generate.reasoning");
    if (reasoning !== undefined && !["minimal", "low", "medium", "high", "xhigh", "max"].includes(reasoning)) {
      throw new ProtocolError("generate.reasoning is invalid");
    }
    const text_verbosity = input.text_verbosity === undefined ? undefined : string(input.text_verbosity, "generate.text_verbosity");
    if (text_verbosity !== undefined && !["low", "medium", "high"].includes(text_verbosity)) {
      throw new ProtocolError("generate.text_verbosity is invalid");
    }
    return { command, request_id, provider: parseProvider(input.provider), model: string(input.model, "generate.model"), messages: parseArray(input.messages, "generate.messages").map(parseMessage), ...(tools === undefined ? {} : { tools }), ...(response_format === undefined ? {} : { response_format }), ...(reasoning === undefined ? {} : { reasoning: reasoning as "minimal" | "low" | "medium" | "high" | "xhigh" | "max" }), ...(text_verbosity === undefined ? {} : { text_verbosity: text_verbosity as TextVerbosity }), ...(temperature === undefined ? {} : { temperature }), ...(max_output_tokens === undefined ? {} : { max_output_tokens }), ...(timeout_ms === undefined ? {} : { timeout_ms }) };
  }
  if (command === "models-list" || command === "models-refresh") {
    exact(input, ["command", "request_id", "provider"], command);
    return { command, request_id, provider: parseProvider(input.provider) };
  }
  if (command === "oauth-login") {
    exact(input, ["command", "request_id", "provider"], command);
    return { command, request_id, provider: parseOAuthProvider(input.provider) };
  }
  if (command === "oauth-prompt-response") {
    exact(input, ["command", "request_id", "target_request_id", "prompt_id", "value", "cancelled"], command);
    const target_request_id = boundedString(input.target_request_id, `${command}.target_request_id`, MAX_REQUEST_ID_LENGTH);
    const prompt_id = boundedString(input.prompt_id, `${command}.prompt_id`, 128);
    const value = input.value === undefined ? undefined : boundedString(input.value, `${command}.value`, MAX_PROMPT_VALUE_LENGTH, true);
    if (input.cancelled !== undefined && typeof input.cancelled !== "boolean") throw new ProtocolError(`${command}.cancelled must be a boolean`);
    if (input.cancelled === true && value !== undefined) throw new ProtocolError(`${command} cannot contain both value and cancelled`);
    if (input.cancelled !== true && value === undefined) throw new ProtocolError(`${command} requires value or cancelled`);
    return { command, request_id, target_request_id, prompt_id, ...(value === undefined ? {} : { value }), ...(input.cancelled === true ? { cancelled: true as const } : {}) };
  }
  if (command === "cancel") {
    exact(input, ["command", "request_id", "target_request_id"], "cancel");
    return { command, request_id, target_request_id: boundedString(input.target_request_id, "cancel.target_request_id", MAX_REQUEST_ID_LENGTH) };
  }
  if (command === "shutdown") {
    exact(input, ["command", "request_id"], "shutdown");
    return { command, request_id };
  }
  throw new ProtocolError(`unknown command ${command}`);
}
import type { OAuthCredential } from "@earendil-works/pi-ai";
