import { describe, expect, it } from "vitest";

import type { Artifact, ChatMessageView, LedgerRecord } from "$lib/api";
import {
  chatArtifactCards,
  chatPermissionProposals,
  isRunAvailable,
  MCP_CHAT_PREVIEW_CHARACTER_LIMIT,
  UNAVAILABLE_REFERENCE_MESSAGE,
} from "$lib/provenance/sessionReferences";

function message(overrides: Partial<ChatMessageView> = {}): ChatMessageView {
  return {
    id: "message-1",
    role: "assistant",
    text: "ok",
    run_id: null,
    artifact_ids: [],
    status: "completed",
    created_at: "2026-07-01T10:00:00.000Z",
    completed_at: "2026-07-01T10:00:01.000Z",
    ...overrides,
  };
}

function artifact(overrides: Partial<Artifact> = {}): Artifact {
  return {
    artifact_id: "artifact-1",
    artifact_type: "note-card",
    title: "Saved note",
    content: { text: "milk" },
    provenance: {
      run_id: "run-1",
      capability: { provider: "notes", capability: "create" },
      grant_id: "grant-1",
      produced_by: "notes",
      recorded_at: "2026-07-09T10:00:00Z",
    },
    ...overrides,
  };
}

function record(runId: string): LedgerRecord {
  return {
    sequence: 1,
    recorded_at: "2026-07-09T10:00:00Z",
    event: {
      kind: "run-started",
      run_id: runId,
      initiator: { kind: "app", app_id: "chat", reason: "test" },
      goal: "test",
    },
  };
}

describe("sessionReferences", () => {
  it("marks stale artifact references visibly", () => {
    const cards = chatArtifactCards(
      message({ artifact_ids: ["artifact-1", "artifact-2"] }),
      true,
      [artifact()],
    );

    expect(cards).toEqual([
      {
        id: "artifact-1",
        title: "Saved note",
        type: "note-card",
        preview: "milk",
        available: true,
      },
      {
        id: "artifact-2",
        title: "Unavailable reference",
        type: "Unavailable",
        preview: UNAVAILABLE_REFERENCE_MESSAGE,
        available: false,
      },
    ]);
  });

  it("keeps internal execution artifacts out of inline chat cards", () => {
    const cards = chatArtifactCards(
      message({ artifact_ids: ["transcript", "response", "artifact-1"] }),
      true,
      [
        artifact({
          artifact_id: "transcript",
          artifact_type: "agent-transcript",
          title: "Agent transcript",
        }),
        artifact({
          artifact_id: "response",
          artifact_type: "llm-response",
          title: "LLM response (stop)",
          content: { reasoning: "Internal reasoning", usage: { total_tokens: 42 } },
        }),
        artifact(),
      ],
    );

    expect(cards.map((card) => card.id)).toEqual(["artifact-1"]);
  });

  it("hides MCP result cards with activity details off and truncates them when on", () => {
    const mcpResult = artifact({
      artifact_type: "mcp-result-card",
      title: "search result",
      content: { tool: "search", result: "x".repeat(200) },
    });
    const referenced = message({ artifact_ids: [mcpResult.artifact_id] });

    expect(chatArtifactCards(referenced, true, [mcpResult], false)).toEqual([]);

    const cards = chatArtifactCards(referenced, true, [mcpResult], true);
    expect(cards).toHaveLength(1);
    expect(Array.from(cards[0].preview)).toHaveLength(MCP_CHAT_PREVIEW_CHARACTER_LIMIT + 1);
    expect(cards[0].preview.endsWith("…")).toBe(true);
  });

  it("accepts only provenance-stamped fixed-policy permission proposals", () => {
    const proposal = artifact({
      artifact_type: "permission-proposal",
      content: {
        holder: "chat",
        scope: {
          kind: "exact-capability",
          provider: "notes",
          capability: "notes.create",
        },
        data_scope: { kind: "none" },
        condition: "requires-approval",
        duration: { kind: "non-expiring" },
        reason: "Create the requested event",
      },
      provenance: {
        run_id: "run-1",
        capability: {
          provider: "com.ma-zierl.host.permissions",
          capability: "permissions.propose_grant",
        },
        grant_id: "grant-1",
        produced_by: "com.ma-zierl.host.permissions",
        recorded_at: "2026-07-09T10:00:00Z",
      },
    });

    expect(chatPermissionProposals(
      message({ artifact_ids: [proposal.artifact_id] }),
      true,
      [proposal],
    )).toEqual([{
      artifactId: "artifact-1",
      holder: "chat",
      provider: "notes",
      capability: "notes.create",
      reason: "Create the requested event",
    }]);
    expect(chatArtifactCards(
      message({ artifact_ids: [proposal.artifact_id] }),
      true,
      [proposal],
    )).toEqual([]);

    proposal.provenance.produced_by = "untrusted-app";
    expect(chatPermissionProposals(message({ artifact_ids: [proposal.artifact_id] }), true, [proposal]))
      .toEqual([]);
  });

  it("falls back only when a loaded durable ledger truly lacks the run", () => {
    expect(isRunAvailable(false, [record("run-1")], "run-1")).toBeNull();
    expect(isRunAvailable(true, [record("run-1")], "run-1")).toBe(true);
    expect(isRunAvailable(true, [record("run-1")], "run-2")).toBe(false);
  });
});
