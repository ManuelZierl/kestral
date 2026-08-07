import assert from "node:assert/strict";
import test from "node:test";
import type { AssistantMessage, Model } from "@earendil-works/pi-ai";
import { mapStopReason, toHostResponse, toPiContext } from "../src/conversion.ts";

const model: Model<"openai-completions"> = { id: "m", name: "M", api: "openai-completions", provider: "test", baseUrl: "http://localhost/v1", reasoning: false, input: ["text"], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 1000, maxTokens: 100 };

test("maps system, prior tool calls, and tool results", () => {
  const context = toPiContext([
    { role: "system", content: "safe" },
    { role: "user", content: "find it" },
    { role: "assistant", content: "", tool_calls: [{ id: "call-1", type: "function", function: { name: "find", arguments: "{\"q\":\"x\"}" } }] },
    { role: "tool", content: "found", tool_call_id: "call-1" },
  ], undefined, model);
  assert.equal(context.systemPrompt, "safe");
  assert.equal(context.messages[2]?.role, "toolResult");
  assert.equal(context.messages[2]?.role === "toolResult" && context.messages[2].toolName, "find");
});

test("maps final content, reasoning, tools, usage, and stop reason", () => {
  const message: AssistantMessage = { role: "assistant", api: model.api, provider: model.provider, model: model.id, timestamp: 1, stopReason: "toolUse", content: [{ type: "thinking", thinking: "why" }, { type: "text", text: "answer" }, { type: "toolCall", id: "c", name: "run", arguments: { n: 1 } }], usage: { input: 2, output: 3, cacheRead: 4, cacheWrite: 5, totalTokens: 14, cost: { input: 0.1, output: 0.2, cacheRead: 0, cacheWrite: 0, total: 0.3 } } };
  assert.deepEqual(toHostResponse(message), { message: { role: "assistant", content: "answer", tool_calls: [{ id: "c", type: "function", function: { name: "run", arguments: "{\"n\":1}" } }] }, reasoning: "why", usage: { prompt_tokens: 11, completion_tokens: 3, total_tokens: 14, cache_read_tokens: 4, cache_write_tokens: 5, cost: 0.3 }, finish_reason: "tool_calls" });
  assert.equal(mapStopReason("aborted"), "cancelled");
});
