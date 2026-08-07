import type { ChatThread, ChatThreadSummary } from "$lib/api";

export interface ChatWorkspaceState {
  activeThreadId: string | null;
  threadSummaries: ChatThreadSummary[];
}

export function deleteThreadAndChooseNext(
  state: ChatWorkspaceState,
  deletedThreadId: string,
): ChatWorkspaceState {
  const threadSummaries = state.threadSummaries.filter((thread) => thread.id !== deletedThreadId);
  const activeThreadId =
    state.activeThreadId === deletedThreadId ? (threadSummaries[0]?.id ?? null) : state.activeThreadId;
  return { activeThreadId, threadSummaries };
}

export function upsertThreadSummary(
  threadSummaries: ChatThreadSummary[],
  thread: ChatThread,
): ChatThreadSummary[] {
  const nextSummary: ChatThreadSummary = {
    id: thread.id,
    title: thread.title,
    created_at: thread.created_at,
    updated_at: thread.updated_at,
    message_count: thread.messages.length,
  };
  return sortByLatest([nextSummary, ...threadSummaries.filter((item) => item.id !== thread.id)]);
}

/// Rename locally with the same reordering the backend will apply (it bumps
/// `updated_at`), so the list moves once now instead of jumping again when
/// the next poll returns the authoritative timestamp.
export function renameThreadSummary(
  threadSummaries: ChatThreadSummary[],
  threadId: string,
  title: string,
  renamedAt: string,
): ChatThreadSummary[] {
  return sortByLatest(
    threadSummaries.map((thread) =>
      thread.id === threadId ? { ...thread, title, updated_at: renamedAt } : thread,
    ),
  );
}

function sortByLatest(threadSummaries: ChatThreadSummary[]): ChatThreadSummary[] {
  return [...threadSummaries].sort((left, right) => right.updated_at.localeCompare(left.updated_at));
}

export type ChatThreadAction =
  | "create"
  | "open"
  | "rename"
  | "delete"
  | "send"
  | "cancel"
  | "profile"
  | "model-profile"
  | "engine";

export function describeChatActionError(action: ChatThreadAction, error: unknown): string {
  const failure =
    action === "create"
      ? "Couldn't create a new chat."
      : action === "open"
        ? "Couldn't open that chat."
        : action === "rename"
          ? "Couldn't rename the chat."
          : action === "delete"
            ? "Couldn't delete the chat."
            : action === "cancel"
              ? "Couldn't stop the reply."
              : action === "profile"
                ? "Couldn't change the assistant profile."
                : action === "model-profile"
                  ? "Couldn't change the model profile."
                : action === "engine"
                  ? "Couldn't change the agent engine."
                  : "Couldn't send your message.";
  const busy = String(error).includes("kernel busy");
  return busy ? `${failure} The host is busy — try again.` : `${failure} Try again.`;
}
