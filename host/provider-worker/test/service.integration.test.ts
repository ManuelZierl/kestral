import assert from "node:assert/strict";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { test } from "node:test";
import { once } from "node:events";
import { providerPayload, WorkerService } from "../src/service.ts";

async function withServer(
  handle: (request: IncomingMessage, response: ServerResponse) => void,
  run: (baseUrl: string) => Promise<void>,
): Promise<void> {
  const server = createServer(handle);
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address === "object");
  try {
    await run(`http://127.0.0.1:${address.port}/v1`);
  } finally {
    server.close();
    await once(server, "close");
  }
}

test("streams a pi-ai OpenAI-compatible completion and normalizes usage", async () => {
  await withServer((request, response) => {
    assert.equal(request.url, "/v1/chat/completions");
    assert.equal(request.headers.authorization, "Bearer integration-secret");
    let body = "";
    request.setEncoding("utf8");
    request.on("data", (chunk: string) => body += chunk);
    request.on("end", () => {
      const payload = JSON.parse(body) as Record<string, unknown>;
      assert.equal(payload.stream, true);
      assert.equal(payload.stream_options, undefined);
      response.writeHead(200, { "content-type": "text/event-stream" });
      response.write('data: {"choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}\n\n');
      response.write('data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4,"prompt_tokens_details":{"cached_tokens":2}}}\n\n');
      response.end("data: [DONE]\n\n");
    });
  }, async (baseUrl) => {
    const events: Record<string, unknown>[] = [];
    await new WorkerService().handle({
      command: "generate",
      request_id: "generate-1",
      provider: { kind: "open-ai-compatible", base_url: baseUrl, api_key: "integration-secret" },
      model: "fixture-model",
      messages: [{ role: "user", content: "hello" }],
      timeout_ms: 5_000,
    }, (event) => events.push(event));

    assert(events.some((event) => event.type === "stream-delta" && event.content === "Hello"));
    const completed = events.find((event) => event.type === "completed");
    assert(completed);
    const response = completed.response as Record<string, unknown>;
    assert.deepEqual(response.message, { role: "assistant", content: "Hello" });
    assert.deepEqual(response.usage, {
      prompt_tokens: 3,
      completion_tokens: 1,
      total_tokens: 4,
      cache_read_tokens: 2,
      cache_write_tokens: 0,
      cost: 0,
    });
    const metrics = response.provider_metrics as Record<string, unknown>;
    assert.equal(typeof metrics.time_to_first_token_ms, "number");
    assert.equal(typeof metrics.total_latency_ms, "number");
    assert.ok(Number.isInteger(metrics.time_to_first_token_ms));
    assert.ok(Number.isInteger(metrics.total_latency_ms));
    assert.ok((metrics.total_latency_ms as number) >= (metrics.time_to_first_token_ms as number));
  });
});

test("refreshes Ollama models through the custom pi provider", async () => {
  await withServer((request, response) => {
    assert.equal(request.url, "/api/tags");
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ models: [{ name: "qwen3" }, { name: "llama3.2" }] }));
  }, async (baseUrl) => {
    const events: Record<string, unknown>[] = [];
    await new WorkerService().handle({
      command: "models-refresh",
      request_id: "models-1",
      provider: { kind: "ollama", base_url: baseUrl },
    }, (event) => events.push(event));

    assert.deepEqual(events, [{
      type: "models",
      request_id: "models-1",
      models: [
        { id: "llama3.2", display_name: "llama3.2", reasoning: false, variants: [], text_verbosity: [], context_window: 16_384, max_output_tokens: 4_096 },
        { id: "qwen3", display_name: "qwen3", reasoning: false, variants: [], text_verbosity: [], context_window: 16_384, max_output_tokens: 4_096 },
      ],
    }]);
  });
});

test("applies Codex text verbosity without replacing structured output", () => {
  assert.deepEqual(
    providerPayload(
      { type: "object" },
      "high",
      { text: { format: { type: "text" } }, input: [] },
      "openai-codex-responses",
    ),
    {
      text: {
        format: { type: "json_schema", name: "response", strict: true, schema: { type: "object" } },
        verbosity: "high",
      },
      input: [],
    },
  );
  assert.throws(
    () => providerPayload(undefined, "high", {}, "anthropic-messages"),
    /text verbosity is unsupported/,
  );
});
