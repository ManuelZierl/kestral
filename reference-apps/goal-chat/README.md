# Goal Chat

Goal Chat is a backend-free Kestral app for conversations organized around an evolving goal rather than only a chronological transcript.

Each model turn receives the user-owned goal, the model's current goal interpretation, the consolidated working solution, open questions, decisions, constraints, assumptions, and the 30 most recent messages. It returns a normal visible reply plus a JSON-schema-structured replacement for the model-owned working state.

The distinction between **your stated goal** and **model interpretation** is deliberate: the first user turn seeds the user-owned field, after which only the user can edit it. The model may revise its interpretation but cannot silently rewrite the user's stated intent.

## Architecture

- `backend.kind = none`: there is no app process or server.
- The sandboxed surface invokes Kestral's ordinary `llm-provider/llm.generate` capability.
- Conversation messages and working state use Kestral host-managed app data.
- Model calls request JSON-schema structured output.
- The user message is persisted before the provider call, while model-owned working state is replaced only after a valid model result.

## Install

Install this directory as a local Kestral app package. It already contains `app.json` and the checksummed `ui/index.html` package payload.

The app requests the normal `llm.generate` permission; model access is mediated and attributable through Kestral like any other cross-app action.

## Test

```sh
npm test
```

## Current scope

This implementation intentionally keeps one active goal-oriented conversation. **New conversation** clears that conversation and its state after confirmation. Multiple named conversations and goal trees are natural follow-ups, but are not required to test the central interaction model.
