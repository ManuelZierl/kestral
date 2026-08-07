import { describe, expect, it } from "vitest";

import type { ChatThread } from "$lib/api";
import {
  deleteThreadAndChooseNext,
  describeChatActionError,
  renameThreadSummary,
  upsertThreadSummary,
  type ChatWorkspaceState,
} from "$lib/chat/chatThreadsModel";

function thread(id: string, title = "Chat"): ChatThread {
  return {
    id,
    resource_id: `resource-${id}`,
    revision: 0,
    title,
    created_at: "2026-07-08T10:00:00Z",
    updated_at: "2026-07-08T10:00:00Z",
    messages: [],
    injected_contexts: [],
  };
}

describe("chatThreadsModel", () => {
  it("deletes a thread and picks the next active thread", () => {
    const initial: ChatWorkspaceState = {
      activeThreadId: "thread-1",
      threadSummaries: [
        { ...thread("thread-1"), message_count: 2 },
        { ...thread("thread-2"), message_count: 1 },
      ],
    };
    const next = deleteThreadAndChooseNext(initial, "thread-1");
    expect(next.threadSummaries).toHaveLength(1);
    expect(next.activeThreadId).toBe("thread-2");
  });

  it("upserts thread summaries by update time", () => {
    const first = thread("thread-1");
    const second = { ...thread("thread-2"), updated_at: "2026-07-08T11:00:00Z" };
    const next = upsertThreadSummary([
      { ...first, message_count: 0 },
      { ...second, message_count: 0 },
    ], { ...first, updated_at: "2026-07-08T12:00:00Z" });
    expect(next[0].id).toBe("thread-1");
  });

  it("rename reorders immediately, matching the backend's updated_at bump", () => {
    const summaries = [
      { ...thread("thread-2"), updated_at: "2026-07-08T11:00:00Z", message_count: 0 },
      { ...thread("thread-1"), message_count: 0 },
    ];
    const next = renameThreadSummary(summaries, "thread-1", "Renamed", "2026-07-08T12:00:00Z");
    expect(next[0].id).toBe("thread-1");
    expect(next[0].title).toBe("Renamed");
  });

  it("describes thread action failures in product language", () => {
    expect(describeChatActionError("rename", new Error("boom"))).toBe(
      "Couldn't rename the chat. Try again.",
    );
    expect(describeChatActionError("send", new Error("kernel busy"))).toBe(
      "Couldn't send your message. The host is busy — try again.",
    );
    expect(describeChatActionError("create", "kernel busy")).toContain("host is busy");
    expect(describeChatActionError("profile", new Error("boom"))).toBe(
      "Couldn't change the assistant profile. Try again.",
    );
    expect(describeChatActionError("engine", new Error("boom"))).toBe(
      "Couldn't change the agent engine. Try again.",
    );
  });
});
