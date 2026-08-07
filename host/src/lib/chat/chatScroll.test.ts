import { describe, expect, it } from "vitest";

import {
  CHAT_SCROLL_BOTTOM_THRESHOLD_PX,
  deriveScrollUpdate,
  getChatContentVersion,
  isNearBottomPosition,
} from "$lib/chat/chatScroll";

const baseInput = {
  currentThreadId: "thread-1",
  lastThreadId: "thread-1",
  messageCount: 2,
  lastMessageCount: 2,
  contentVersion: "same",
  lastContentVersion: "same",
  userPinnedToBottom: true,
  hasUnreadBelow: false,
  forceScrollToBottom: false,
};

describe("chatScroll", () => {
  it("detects when the log is near the bottom", () => {
    expect(
      isNearBottomPosition({
        scrollTop: 500,
        scrollHeight: 1000,
        clientHeight: 440,
      }),
    ).toBe(true);

    expect(
      isNearBottomPosition(
        {
          scrollTop: 500,
          scrollHeight: 1000,
          clientHeight: 400,
        },
        CHAT_SCROLL_BOTTOM_THRESHOLD_PX,
      ),
    ).toBe(false);
  });

  it("scrolls to bottom when the active thread changes", () => {
    expect(
      deriveScrollUpdate({
        ...baseInput,
        currentThreadId: "thread-2",
      }),
    ).toEqual({ shouldScrollToBottom: true, hasUnreadBelow: false });
  });

  it("keeps the view pinned when new content arrives at the bottom", () => {
    expect(
      deriveScrollUpdate({
        ...baseInput,
        messageCount: 3,
        contentVersion: "new",
      }),
    ).toEqual({ shouldScrollToBottom: true, hasUnreadBelow: false });
  });

  it("shows unread state when content arrives while scrolled up", () => {
    expect(
      deriveScrollUpdate({
        ...baseInput,
        userPinnedToBottom: false,
        messageCount: 3,
        contentVersion: "new",
      }),
    ).toEqual({ shouldScrollToBottom: false, hasUnreadBelow: true });
  });

  it("preserves unread state when nothing changed", () => {
    expect(
      deriveScrollUpdate({
        ...baseInput,
        userPinnedToBottom: false,
        hasUnreadBelow: true,
      }),
    ).toEqual({ shouldScrollToBottom: false, hasUnreadBelow: true });
  });

  it("treats edited message content as new content", () => {
    const previous = getChatContentVersion([
      {
        id: "message-1",
        role: "assistant",
        text: "hello",
        run_id: null,
        artifact_ids: [],
        status: "completed",
        created_at: "2026-07-01T10:00:00.000Z",
        completed_at: "2026-07-01T10:00:01.000Z",
      },
    ]);
    const next = getChatContentVersion([
      {
        id: "message-1",
        role: "assistant",
        text: "hello there",
        run_id: null,
        artifact_ids: [],
        status: "completed",
        created_at: "2026-07-01T10:00:00.000Z",
        completed_at: "2026-07-01T10:00:01.000Z",
      },
    ]);

    expect(previous).not.toBe(next);
  });

  it("stays correct across a long conversation: pinned follows, scrolled-up accumulates unread", () => {
    // Simulate a 200-message conversation arriving one message at a time.
    let state = { hasUnreadBelow: false, lastCount: 0, lastVersion: "v0" };
    for (let count = 1; count <= 100; count += 1) {
      const decision = deriveScrollUpdate({
        ...baseInput,
        messageCount: count,
        lastMessageCount: state.lastCount,
        contentVersion: `v${count}`,
        lastContentVersion: state.lastVersion,
        userPinnedToBottom: true,
        hasUnreadBelow: state.hasUnreadBelow,
      });
      // While the user stays at the bottom, every message keeps the view pinned.
      expect(decision).toEqual({ shouldScrollToBottom: true, hasUnreadBelow: false });
      state = { hasUnreadBelow: decision.hasUnreadBelow, lastCount: count, lastVersion: `v${count}` };
    }

    // The user scrolls up to reread; the rest of the conversation must not yank them down.
    for (let count = 101; count <= 200; count += 1) {
      const decision = deriveScrollUpdate({
        ...baseInput,
        messageCount: count,
        lastMessageCount: state.lastCount,
        contentVersion: `v${count}`,
        lastContentVersion: state.lastVersion,
        userPinnedToBottom: false,
        hasUnreadBelow: state.hasUnreadBelow,
      });
      expect(decision.shouldScrollToBottom).toBe(false);
      expect(decision.hasUnreadBelow).toBe(true);
      state = { hasUnreadBelow: decision.hasUnreadBelow, lastCount: count, lastVersion: `v${count}` };
    }

    // Sending a message forces the jump back down regardless of position.
    expect(
      deriveScrollUpdate({
        ...baseInput,
        messageCount: 201,
        lastMessageCount: 200,
        contentVersion: "v201",
        lastContentVersion: "v200",
        userPinnedToBottom: false,
        hasUnreadBelow: true,
        forceScrollToBottom: true,
      }),
    ).toEqual({ shouldScrollToBottom: true, hasUnreadBelow: false });
  });

  it("ignores artifact id ordering when content is otherwise unchanged", () => {
    const previous = getChatContentVersion([
      {
        id: "message-1",
        role: "assistant",
        text: "hello",
        run_id: null,
        artifact_ids: ["artifact-2", "artifact-1"],
        status: "completed",
        created_at: "2026-07-01T10:00:00.000Z",
        completed_at: "2026-07-01T10:00:01.000Z",
      },
    ]);
    const next = getChatContentVersion([
      {
        id: "message-1",
        role: "assistant",
        text: "hello",
        run_id: null,
        artifact_ids: ["artifact-1", "artifact-2"],
        status: "completed",
        created_at: "2026-07-01T10:00:00.000Z",
        completed_at: "2026-07-01T10:00:01.000Z",
      },
    ]);

    expect(previous).toBe(next);
  });
});
