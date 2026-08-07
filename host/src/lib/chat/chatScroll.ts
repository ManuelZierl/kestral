import type { ChatMessageView } from "$lib/api";

export const CHAT_SCROLL_BOTTOM_THRESHOLD_PX = 64;

export interface ScrollMetrics {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

export interface ChatScrollUpdateInput {
  currentThreadId: string | null;
  lastThreadId: string | null;
  messageCount: number;
  lastMessageCount: number;
  contentVersion: string;
  lastContentVersion: string;
  userPinnedToBottom: boolean;
  hasUnreadBelow: boolean;
  forceScrollToBottom: boolean;
}

export interface ChatScrollUpdateDecision {
  shouldScrollToBottom: boolean;
  hasUnreadBelow: boolean;
}

export function isNearBottomPosition(
  metrics: ScrollMetrics,
  threshold = CHAT_SCROLL_BOTTOM_THRESHOLD_PX,
): boolean {
  return metrics.scrollHeight - metrics.clientHeight - metrics.scrollTop <= threshold;
}

export function getChatContentVersion(messages: ChatMessageView[]): string {
  return JSON.stringify(
    messages.map(({ id, status, text, artifact_ids, run_id }) => ({
      id,
      status,
      text,
      artifact_ids: [...artifact_ids].sort(),
      run_id,
    })),
  );
}

export function deriveScrollUpdate(
  input: ChatScrollUpdateInput,
): ChatScrollUpdateDecision {
  const threadChanged = input.currentThreadId !== input.lastThreadId;
  const contentChanged = input.contentVersion !== input.lastContentVersion;
  const messageCountChanged = input.messageCount !== input.lastMessageCount;

  if (input.forceScrollToBottom || threadChanged) {
    return { shouldScrollToBottom: true, hasUnreadBelow: false };
  }

  if (!contentChanged && !messageCountChanged) {
    return { shouldScrollToBottom: false, hasUnreadBelow: input.hasUnreadBelow };
  }

  if (input.userPinnedToBottom) {
    return { shouldScrollToBottom: true, hasUnreadBelow: false };
  }

  return { shouldScrollToBottom: false, hasUnreadBelow: true };
}
