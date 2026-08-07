import type { Api, AssistantMessage, Context, Message, Model, Tool, TSchema } from "@earendil-works/pi-ai";
import type { HostMessage, HostTool, HostToolCall } from "./protocol.ts";

const EMPTY_COST = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 };

export function toPiContext(messages: HostMessage[], tools: HostTool[] | undefined, model: Model<Api>): Context {
  const system = messages.filter((message) => message.role === "system").map((message) => message.content).join("\n\n");
  const toolNames = new Map<string, string>();
  for (const message of messages) {
    if (message.role === "assistant") for (const call of message.tool_calls ?? []) toolNames.set(call.id, call.function.name);
  }
  const converted: Message[] = [];
  messages.forEach((message, index) => {
    const timestamp = index;
    if (message.role === "system") return;
    if (message.role === "user") {
      converted.push({ role: "user", content: message.content, timestamp });
      return;
    }
    if (message.role === "tool") {
      converted.push({ role: "toolResult", toolCallId: message.tool_call_id, toolName: message.name ?? toolNames.get(message.tool_call_id) ?? "unknown", content: [{ type: "text", text: message.content }], isError: false, timestamp });
      return;
    }
    const content: AssistantMessage["content"] = [];
    if (message.content) content.push({ type: "text", text: message.content });
    for (const call of message.tool_calls ?? []) content.push({ type: "toolCall", id: call.id, name: call.function.name, arguments: JSON.parse(call.function.arguments) as Record<string, unknown> });
    converted.push({ role: "assistant", content, api: model.api, provider: model.provider, model: model.id, usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { ...EMPTY_COST } }, stopReason: message.tool_calls?.length ? "toolUse" : "stop", timestamp });
  });
  const piTools = tools?.map((tool): Tool => ({ name: tool.function.name, description: tool.function.description, parameters: tool.function.parameters as TSchema }));
  return { ...(system ? { systemPrompt: system } : {}), messages: converted, ...(piTools === undefined ? {} : { tools: piTools }) };
}

export interface HostLlmResponse {
  message: { role: "assistant"; content: string; tool_calls?: HostToolCall[] };
  reasoning?: string;
  usage?: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
    cache_read_tokens: number;
    cache_write_tokens: number;
    cost?: number;
  };
  provider_metrics?: { time_to_first_token_ms?: number; total_latency_ms: number };
  finish_reason: string;
}

export function mapStopReason(reason: AssistantMessage["stopReason"]): string {
  switch (reason) {
    case "toolUse": return "tool_calls";
    case "length": return "length";
    case "aborted": return "cancelled";
    case "error": return "error";
    case "stop": return "stop";
  }
}

export function toHostResponse(message: AssistantMessage): HostLlmResponse {
  const content = message.content.filter((part) => part.type === "text").map((part) => part.text).join("");
  const reasoning = message.content.filter((part) => part.type === "thinking").map((part) => part.thinking).join("");
  const tool_calls = message.content.filter((part) => part.type === "toolCall").map((part): HostToolCall => ({ id: part.id, type: "function", function: { name: part.name, arguments: JSON.stringify(part.arguments) } }));
  const usage = message.usage;
  return {
    message: { role: "assistant", content, ...(tool_calls.length ? { tool_calls } : {}) },
    ...(reasoning ? { reasoning } : {}),
    usage: {
      prompt_tokens: usage.input + usage.cacheRead + usage.cacheWrite,
      completion_tokens: usage.output,
      total_tokens: usage.totalTokens,
      cache_read_tokens: usage.cacheRead,
      cache_write_tokens: usage.cacheWrite,
      ...(Number.isFinite(usage.cost.total) ? { cost: usage.cost.total } : {}),
    },
    finish_reason: mapStopReason(message.stopReason),
  };
}
