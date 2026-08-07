---
title: Agent workers
layout: default
parent: Extending Kestral
nav_order: 2
---

# Agent worker protocol
{: .no_toc }

1. TOC
{:toc}

An `agent-worker` backend is the public, language-neutral host adapter for a
headless agent app. Kestral Pi is one implementation. The adapter is not a
kernel primitive: the worker has no kernel handle, credential, or ambient app
authority, and every model or tool callback is mediated as a child Run under
the initiating app's grants.

## Package contract

The package backend declares `kind: "agent-worker"`, `protocol_version: 1`, and
a checksum-pinned worker entry. Its manifest must declare exactly one
`agent.run` capability and an `agent-transcript` artifact type. It has no need
for self-grants: authority comes from the app that invokes `agent.run`.

The host starts one worker process per `agent.run` invocation. Messages are
newline-delimited JSON on standard input and output. Unknown fields, malformed
JSON, an unexpected message order, or a protocol mismatch fail the invocation.
The worker must emit this first:

```json
{"type":"ready","protocol_version":1}
```

The exact `agent.run` capability input may pin `profile`, `model`, `reasoning`,
`temperature`, and `max_output_tokens`. Its optional `tools` policy accepts
`exclude_providers` and an exact `allow_capabilities` list. The host applies
that policy to the initiating app's live grant-aware capability catalog before
starting the worker. The worker receives only the resulting tool definitions;
neither policy field can add authority.

## Host commands

Every command has a unique non-empty `request_id`.

| `command` | Required fields | Purpose |
|---|---|---|
| `agent-run` | `request_id`, `messages`, `tools`, `max_turns` | Starts the job. Optional fields are `system_prompt`, `model`, and `reasoning`. The supplied tools are the initiating app's currently granted capabilities, not the worker app's authority. |
| `tool-result` | `request_id`, `target_request_id`, `tool_call_id`, `outcome`, `content` | Answers a worker tool request. `outcome` is `completed`, `refused`, or `failed`. |
| `llm-completed` | `request_id`, `call_id`, `response` | Answers a model callback with an assistant message, optional reasoning, and `finish_reason`. |
| `llm-failed` | `request_id`, `call_id`, `message` | Fails one model callback without fabricating a model result. |
| `cancel` | `request_id`, `target_request_id` | Requests cancellation of an active job. |
| `shutdown` | `request_id` | Requests orderly worker shutdown. |

Messages use roles `system`, `user`, `assistant`, and `tool`. Tools use the
OpenAI-compatible function shape:

```json
{
  "type": "function",
  "function": {
    "name": "files__read",
    "description": "Read a registered file",
    "parameters": {"type":"object","properties":{},"additionalProperties":false}
  }
}
```

`max_turns` is an integer from 1 through 10. One job accepts at most 1,024
messages, 256 tools, and 128 tool calls per assistant message. IDs and ordinary
string fields are bounded to 16 KiB; message and tool-result content is bounded
to 2 MiB. The host separately caps model-visible completed tool content to 32
KiB and marks it as untrusted.

## Worker events

| `type` | Required fields | Host behavior |
|---|---|---|
| `llm-request` | `request_id`, `call_id`, `model`, `messages`, `tools` | Invokes `llm.generate` through a caller-attributed child Run. `reasoning` is optional. |
| `tool-request` | `request_id`, `tool_call_id`, `tool_name`, `arguments` | Resolves the advertised tool name and invokes the capability through the full action path. |
| `agent-event` | `request_id`, `event` | Emits transient progress. `tool_call_id` and `tool_name` are optional. |
| `completed` | `request_id`, `text`, `finish_reason`, `turns`, `transcript` | Proposes the final result and transcript artifact. `reasoning` is optional; `finish_reason` is `stop` or `max-turns`. |
| `failed` | `request_id`, `code`, `message` | Fails the invocation visibly. |
| `acknowledged` | `request_id`, `command` | Confirms `cancel` or `shutdown`; `target_request_id` is present for cancellation. |

The host supplies model generation and the initial granted tool list. Agent
runtime capabilities named `agent.run` are excluded from that list to prevent
recursive runtime dispatch; the worker cannot discover or invoke any other
capability. Child invocation dispatch uses eight workers, a queue of 32
requests, and a limit of four outstanding requests per initiating app. Queue or
quota saturation fails visibly and creates no additional OS thread.

Chat can select a supported engine per thread and pins the engine receipt for
that send. If a selected engine or exact contract match is unavailable later,
Chat falls back to its plain grant-aware path and records the fallback reason.

## Lifecycle and cancellation

The ready deadline is 10 seconds, the idle deadline is 120 seconds, stdout
lines are capped at 2 MiB, and captured stderr is capped at 16 KiB. A callback
resets the idle deadline. Cancellation sends `cancel`, waits one second, and
then terminates the process if needed. Shutdown has a two-second deadline.
Late output cannot commit after cancellation because kernel finalization
revalidates the Run, grant, app identity, and deadline.

Protocol version 1 ships with the host adapter contract. Host and worker changes
that alter this shape require a coordinated protocol-version release; there is
no permissive fallback.
