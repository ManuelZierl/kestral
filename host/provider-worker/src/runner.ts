import { createInterface } from "node:readline";
import { parseCommand, ProtocolError, requestIdHint } from "./protocol.ts";
import { WorkerService, type Emit } from "./service.ts";

export async function runWorker(): Promise<void> {
  const emit: Emit = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);
  const service = new WorkerService();
  const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
  emit({ type: "ready", protocol_version: 2 });
  const pending = new Set<Promise<void>>();
  let shutdown = false;
  for await (const line of input) {
    if (shutdown || !line.trim()) continue;
    let raw: unknown;
    try {
      raw = JSON.parse(line);
      const command = parseCommand(raw);
      if (command.command === "cancel") {
        await service.handle(command, emit);
        continue;
      }
      if (command.command === "shutdown") {
        shutdown = await service.handle(command, emit);
        input.close();
        continue;
      }
      const task = service.handle(command, emit).then(() => undefined).catch((error) => {
        process.stderr.write(`provider worker internal command failure: ${error instanceof Error ? error.name : "unknown"}\n`);
      });
      pending.add(task);
      void task.finally(() => pending.delete(task));
    } catch (error) {
      emit({ type: "failed", request_id: requestIdHint(raw), code: error instanceof ProtocolError ? error.code : "invalid_json", message: error instanceof ProtocolError ? error.message : "input is not valid JSON" });
    }
  }
  await Promise.allSettled(pending);
}
