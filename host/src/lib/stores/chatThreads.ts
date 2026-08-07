import {
  cancelChatMessage,
  createChatThread,
  deleteChatThread,
  getChatThread,
  listChatThreads,
  renameChatThread,
  removeChatContribution,
  setChatThreadProfile,
  setChatModelProfile,
  setChatAgentEngine,
  sendChatMessage,
  attachChatArtifact,
  type ChatThread,
  type ChatThreadSummary,
} from "$lib/api";
import {
  deleteThreadAndChooseNext,
  upsertThreadSummary,
} from "$lib/chat/chatThreadsModel";
import type { ChatContribution, ChatProfileReceipt } from "$lib/api";
import { synchronizeArtifactReferences } from "$lib/stores/artifacts";
import { writable, get } from "svelte/store";

const ACTIVE_THREAD_STORAGE_KEY = "kernel.active-chat-thread";
const PENDING_SEND_STORAGE_KEY = "kernel.pending-chat-sends";

export const chatThreads = writable<ChatThreadSummary[]>([]);
export const activeChatThread = writable<ChatThread | null>(null);
export const activeChatThreadId = writable<string | null>(null);
/// Threads with an assistant run in flight, so the UI can mark them in the
/// thread list — not just the conversation currently on screen.
export const sendingChatThreadIds = writable<ReadonlySet<string>>(new Set());
export interface StreamingChatReply {
  text: string;
  reasoning: string;
}
export const streamingChatReplies = writable<ReadonlyMap<string, StreamingChatReply>>(new Map());
export interface ChatDraft {
  text: string;
  contributions: ChatContribution[];
}
export const chatDrafts = writable<ReadonlyMap<string, ChatDraft>>(new Map());
const threadMutationEpochs = new Map<string, number>();
const threadMutationsInFlight = new Map<string, number>();

function beginThreadMutation(threadId: string): () => void {
  threadMutationEpochs.set(threadId, (threadMutationEpochs.get(threadId) ?? 0) + 1);
  threadMutationsInFlight.set(threadId, (threadMutationsInFlight.get(threadId) ?? 0) + 1);
  let finished = false;
  return () => {
    if (finished) return;
    finished = true;
    threadMutationEpochs.set(threadId, (threadMutationEpochs.get(threadId) ?? 0) + 1);
    const remaining = (threadMutationsInFlight.get(threadId) ?? 1) - 1;
    if (remaining > 0) threadMutationsInFlight.set(threadId, remaining);
    else threadMutationsInFlight.delete(threadId);
  };
}

function applyThreadResult(threadId: string, thread: ChatThread, updateDraft: boolean): void {
  chatThreads.update((current) => upsertThreadSummary(current, thread));
  if (get(activeChatThreadId) === threadId) {
    activeChatThread.set(thread);
    persistActiveThreadId(threadId);
  }
  if (updateDraft) {
    setChatDraft(threadId, {
      text: get(chatDrafts).get(threadId)?.text ?? "",
      contributions: thread.contributions ?? [],
    });
  }
}

function updateChatDraft(threadId: string, update: (draft: ChatDraft) => ChatDraft) {
  chatDrafts.update((current) => {
    const next = new Map(current);
    next.set(threadId, update(next.get(threadId) ?? { text: "", contributions: [] }));
    return next;
  });
}

export function setChatDraft(threadId: string, draft: ChatDraft) {
  chatDrafts.update((current) => {
    const next = new Map(current);
    next.set(threadId, draft);
    return next;
  });
}

export function setChatDraftText(threadId: string, text: string) {
  updateChatDraft(threadId, (draft) => ({ ...draft, text }));
}

export function clearChatDraft(threadId: string) {
  chatDrafts.update((current) => {
    const next = new Map(current);
    next.delete(threadId);
    return next;
  });
}

function updateStreamingReply(threadId: string, value: StreamingChatReply | null) {
  streamingChatReplies.update((current) => {
    const next = new Map(current);
    if (value) next.set(threadId, value);
    else next.delete(threadId);
    return next;
  });
}

function markThreadSending(threadId: string, sending: boolean) {
  sendingChatThreadIds.update((current) => {
    const next = new Set(current);
    if (sending) {
      next.add(threadId);
    } else {
      next.delete(threadId);
    }
    return next;
  });
}

function restoreActiveThreadId(): string | null {
  if (typeof localStorage === "undefined") return null;
  return localStorage.getItem(ACTIVE_THREAD_STORAGE_KEY);
}

function persistActiveThreadId(threadId: string | null) {
  if (typeof localStorage === "undefined") return;
  if (threadId === null) {
    localStorage.removeItem(ACTIVE_THREAD_STORAGE_KEY);
    return;
  }
  localStorage.setItem(ACTIVE_THREAD_STORAGE_KEY, threadId);
}

export async function refreshChatThreads() {
  const summaries = await listChatThreads();
  chatThreads.set(summaries);
  const currentId = get(activeChatThreadId) ?? restoreActiveThreadId();
  const nextId = summaries.some((thread) => thread.id === currentId)
    ? currentId
    : (summaries[0]?.id ?? null);
  if (nextId) {
    await selectChatThread(nextId);
  } else {
    activeChatThread.set(null);
    activeChatThreadId.set(null);
    persistActiveThreadId(null);
  }
}

// Selections race: the 1.5s poll re-selects the current thread while user
// clicks pick another. Only the newest request may apply its response, or a
// slow stale fetch silently jumps the UI back to the old thread.
let selectRequestSequence = 0;

export async function selectChatThread(threadId: string) {
  const requestId = ++selectRequestSequence;
  const mutationEpoch = threadMutationEpochs.get(threadId) ?? 0;
  const thread = await getChatThread(threadId);
  try {
    await synchronizeArtifactReferences(thread.messages.flatMap((message) => message.artifact_ids));
  } catch {
    // Keep the thread usable while the normal host poll retries. The artifact
    // store remains not-loaded, so the UI does not mislabel unresolved ids as
    // removed or inconsistent.
  }
  if (
    requestId !== selectRequestSequence ||
    mutationEpoch !== (threadMutationEpochs.get(threadId) ?? 0) ||
    threadMutationsInFlight.has(threadId)
  ) {
    return;
  }
  activeChatThread.set(thread);
  activeChatThreadId.set(threadId);
  persistActiveThreadId(threadId);
  chatThreads.update((current) => upsertThreadSummary(current, thread));
  setChatDraft(threadId, {
    text: get(chatDrafts).get(threadId)?.text ?? "",
    contributions: thread.contributions ?? get(chatDrafts).get(threadId)?.contributions ?? [],
  });
}

let ensureChatThreadInFlight: Promise<void> | null = null;
let createNewChatThreadInFlight: Promise<ChatThread> | null = null;

async function ensureChatThreadOnce() {
  const currentThreadId = get(activeChatThreadId) ?? restoreActiveThreadId();
  const currentThread = get(activeChatThread);
  const summaries = await listChatThreads();
  chatThreads.set(summaries);

  const nextId = summaries.some((thread) => thread.id === currentThreadId)
    ? currentThreadId
    : (summaries[0]?.id ?? null);

  if (currentThread && nextId === currentThread.id) {
    persistActiveThreadId(nextId);
    return;
  }

  if (nextId) {
    await selectChatThread(nextId);
    return;
  }

  await createNewChatThread();
}

export function ensureChatThread(): Promise<void> {
  if (ensureChatThreadInFlight) return ensureChatThreadInFlight;
  ensureChatThreadInFlight = ensureChatThreadOnce().finally(() => {
    ensureChatThreadInFlight = null;
  });
  return ensureChatThreadInFlight;
}

function isUntouchedChatDraft(thread: ChatThread): boolean {
  const draft = get(chatDrafts).get(thread.id);
  return thread.revision === 0
    && thread.messages.length === 0
    && !draft?.text
    && (draft?.contributions.length ?? 0) === 0;
}

async function createNewChatThreadOnce(): Promise<ChatThread> {
  const current = get(activeChatThread);
  if (current && isUntouchedChatDraft(current)) return current;

  const summaries = await listChatThreads();
  for (const summary of summaries) {
    if (summary.message_count !== 0 || summary.id === current?.id) continue;
    const candidate = await getChatThread(summary.id);
    if (!isUntouchedChatDraft(candidate)) continue;
    await selectChatThread(candidate.id);
    const selected = get(activeChatThread);
    if (selected && isUntouchedChatDraft(selected)) return selected;
  }

  const thread = await createChatThread();
  selectRequestSequence += 1;
  activeChatThread.set(thread);
  activeChatThreadId.set(thread.id);
  persistActiveThreadId(thread.id);
  chatThreads.update((currentThreads) => upsertThreadSummary(currentThreads, thread));
  setChatDraft(thread.id, { text: "", contributions: thread.contributions ?? [] });
  return thread;
}

export function createNewChatThread(): Promise<ChatThread> {
  if (createNewChatThreadInFlight) return createNewChatThreadInFlight;

  const creation = createNewChatThreadOnce();
  const trackedCreation = creation.finally(() => {
    if (createNewChatThreadInFlight === trackedCreation) createNewChatThreadInFlight = null;
  });
  createNewChatThreadInFlight = trackedCreation;
  return trackedCreation;
}

export async function renameExistingChatThread(threadId: string, title: string) {
  const finishMutation = beginThreadMutation(threadId);
  try {
    const thread = await renameChatThread(threadId, title);
    applyThreadResult(threadId, thread, false);
    return thread;
  } finally {
    finishMutation();
  }
}

export async function deleteExistingChatThread(threadId: string) {
  await deleteChatThread(threadId);
  clearChatDraft(threadId);
  const nextState = deleteThreadAndChooseNext(
    {
      activeThreadId: get(activeChatThreadId),
      threadSummaries: get(chatThreads),
    },
    threadId,
  );
  chatThreads.set(nextState.threadSummaries);
  if (nextState.activeThreadId) {
    await selectChatThread(nextState.activeThreadId);
    return;
  }
  activeChatThread.set(null);
  activeChatThreadId.set(null);
  persistActiveThreadId(null);
}

/// Idempotency keys for sends whose response was lost. Keyed by thread; a
/// resend of the same text reuses the key, so the host can recognize a
/// retry of already-executed work instead of running it twice.
type PendingSend = { requestId: string; message: string };

interface StoredPendingSends {
  version: 1;
  sends: Record<string, PendingSend>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parsePendingSends(value: string): Record<string, PendingSend> {
  const parsed: unknown = JSON.parse(value);
  if (!isRecord(parsed) || Object.keys(parsed).sort().join(",") !== "sends,version" || parsed.version !== 1 || !isRecord(parsed.sends)) {
    throw new Error("Pending Chat sends use an unsupported storage format.");
  }
  const sends: Record<string, PendingSend> = {};
  for (const [threadId, pending] of Object.entries(parsed.sends)) {
    if (
      !threadId
      || !isRecord(pending)
      || Object.keys(pending).sort().join(",") !== "message,requestId"
      || typeof pending.requestId !== "string"
      || !pending.requestId
      || typeof pending.message !== "string"
    ) {
      throw new Error("Pending Chat send recovery data is incomplete.");
    }
    sends[threadId] = { requestId: pending.requestId, message: pending.message };
  }
  return sends;
}

function readPendingSends(): Record<string, PendingSend> {
  if (typeof localStorage === "undefined") return {};
  const stored = localStorage.getItem(PENDING_SEND_STORAGE_KEY);
  return stored ? parsePendingSends(stored) : {};
}

function pendingSend(threadId: string): PendingSend | undefined {
  return readPendingSends()[threadId];
}

function persistPendingSend(threadId: string, value: PendingSend | null) {
  if (typeof localStorage === "undefined") return;
  const sends = readPendingSends();
  if (value) sends[threadId] = value;
  else delete sends[threadId];
  const stored: StoredPendingSends = { version: 1, sends };
  localStorage.setItem(PENDING_SEND_STORAGE_KEY, JSON.stringify(stored));
}

function requestIdFor(threadId: string, message: string): string {
  const failed = pendingSend(threadId);
  if (failed && failed.message === message) {
    return failed.requestId;
  }
  return crypto.randomUUID();
}

export async function sendMessageToActiveThread(
  message: string,
) {
  const threadId = get(activeChatThreadId);
  if (!threadId) {
    throw new Error("No active chat thread");
  }
  const composedMessage = message;
  const requestId = requestIdFor(threadId, composedMessage);
  const finishMutation = beginThreadMutation(threadId);
  const optimisticMessageId = `pending-user-${requestId}`;
  try {
    persistPendingSend(threadId, { requestId, message: composedMessage });
    markThreadSending(threadId, true);
    updateStreamingReply(threadId, { text: "", reasoning: "" });
    // Show the user's message immediately rather than waiting for the whole
    // assistant turn to finish. The authoritative thread from the backend
    // replaces this optimistic copy on success; it is rolled back on failure.
    if (get(activeChatThreadId) === threadId) {
      activeChatThread.update((thread) =>
        thread && thread.id === threadId
          ? {
              ...thread,
              messages: [
                ...thread.messages,
                {
                  id: optimisticMessageId,
                  role: "user",
                  text: composedMessage,
                  reasoning: null,
                  run_id: null,
                  artifact_ids: [],
                  status: "pending",
                  client_request_id: requestId,
                  created_at: new Date().toISOString(),
                  completed_at: null,
                },
              ],
            }
          : thread,
      );
    }
    const result = await sendChatMessage(threadId, composedMessage, requestId, (event) => {
      if (event.kind === "llm-stream-start") {
        updateStreamingReply(threadId, { text: "", reasoning: "" });
        return;
      }
      const current = get(streamingChatReplies).get(threadId) ?? { text: "", reasoning: "" };
      updateStreamingReply(threadId, {
        text: current.text + event.content,
        reasoning: current.reasoning + event.reasoning,
      });
    });
    try {
      await synchronizeArtifactReferences(
        result.thread.messages.flatMap((message) => message.artifact_ids),
      );
    } catch {
      // The completed message is durable. Show it and let host polling retry
      // artifact synchronization without turning a transport failure into a
      // failed send or an unsafe resend.
    }
    persistPendingSend(threadId, null);
    applyThreadResult(threadId, result.thread, true);
  } catch (error) {
    // Roll back the optimistic user message so a failed send leaves no ghost
    // message behind (the caller restores the draft text for a retry).
    if (get(activeChatThreadId) === threadId) {
      activeChatThread.update((thread) =>
        thread && thread.id === threadId
          ? {
              ...thread,
              messages: thread.messages.filter((message) => message.id !== optimisticMessageId),
            }
          : thread,
      );
    }
    throw error;
  } finally {
    updateStreamingReply(threadId, null);
    markThreadSending(threadId, false);
    finishMutation();
  }
}

export async function selectAssistantProfile(threadId: string, appId: string, profileName: string) {
  const finishMutation = beginThreadMutation(threadId);
  try {
    const thread = await setChatThreadProfile(threadId, appId, profileName);
    applyThreadResult(threadId, thread, true);
  } finally {
    finishMutation();
  }
}

export async function selectModelProfile(threadId: string, profileRef: string | null) {
  const finishMutation = beginThreadMutation(threadId);
  try {
    const thread = await setChatModelProfile(threadId, profileRef);
    applyThreadResult(threadId, thread, false);
  } finally {
    finishMutation();
  }
}

export async function selectChatAgentEngine(threadId: string, appId: string | null) {
  const finishMutation = beginThreadMutation(threadId);
  try {
    const thread = await setChatAgentEngine(threadId, appId);
    applyThreadResult(threadId, thread, false);
  } finally {
    finishMutation();
  }
}

export async function attachChatArtifactToThread(threadId: string, artifactId: string, title: string) {
  const finishMutation = beginThreadMutation(threadId);
  try {
    const { thread } = await attachChatArtifact(threadId, artifactId, title);
    applyThreadResult(threadId, thread, true);
  } finally {
    finishMutation();
  }
}

export async function removeChatContributionFromThread(
  threadId: string,
  sourceAppId: string,
  kind: ChatContribution["kind"],
  itemId: string,
) {
  const finishMutation = beginThreadMutation(threadId);
  try {
    const thread = await removeChatContribution(threadId, sourceAppId, kind, itemId);
    applyThreadResult(threadId, thread, true);
  } finally {
    finishMutation();
  }
}

export async function cancelMessageForActiveThread() {
  const threadId = get(activeChatThreadId);
  if (!threadId) throw new Error("No active chat thread");
  await cancelChatMessage(threadId);
}
