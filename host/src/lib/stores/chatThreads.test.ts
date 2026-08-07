import { beforeEach, describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";

import type { Artifact, ChatStreamEvent, ChatThread, ChatThreadSummary } from "$lib/api";

const backend = vi.hoisted(() => ({
  nextId: 1,
  threads: [] as ChatThread[],
  artifacts: [] as Artifact[],
}));

function cloneThread(thread: ChatThread): ChatThread {
  return JSON.parse(JSON.stringify(thread)) as ChatThread;
}

function summarize(thread: ChatThread): ChatThreadSummary {
  return {
    id: thread.id,
    title: thread.title,
    created_at: thread.created_at,
    updated_at: thread.updated_at,
    message_count: thread.messages.length,
  };
}

vi.mock("$lib/api", () => ({
  listChatThreads: vi.fn(async () =>
    backend.threads
      .map(summarize)
      .sort((left, right) => right.updated_at.localeCompare(left.updated_at)),
  ),
  listArtifacts: vi.fn(async () => backend.artifacts),
  getChatThread: vi.fn(async (threadId: string) => {
    const thread = backend.threads.find((candidate) => candidate.id === threadId);
    if (!thread) throw new Error(`unknown chat thread: ${threadId}`);
    return cloneThread(thread);
  }),
  createChatThread: vi.fn(async () => {
    const now = `2026-07-09T10:00:0${backend.nextId}Z`;
    const thread: ChatThread = {
      id: `thread-${backend.nextId++}`,
      resource_id: `chat-thread-${backend.nextId}`,
      revision: 0,
      title: "New chat",
      created_at: now,
      updated_at: now,
      messages: [],
      injected_contexts: [],
    };
    backend.threads.unshift(thread);
    return cloneThread(thread);
  }),
  renameChatThread: vi.fn(async (threadId: string, title: string) => {
    const thread = backend.threads.find((candidate) => candidate.id === threadId);
    if (!thread) throw new Error(`unknown chat thread: ${threadId}`);
    thread.title = title;
    thread.updated_at = `2026-07-09T11:00:0${backend.nextId}Z`;
    thread.revision += 1;
    return cloneThread(thread);
  }),
  deleteChatThread: vi.fn(async (threadId: string) => {
    backend.threads = backend.threads.filter((thread) => thread.id !== threadId);
  }),
  sendChatMessage: vi.fn(async (
    threadId: string,
    message: string,
    requestId: string,
    onStream: (event: ChatStreamEvent) => void,
  ) => {
    onStream({ kind: "llm-stream-delta", content: "reply", reasoning: "" });
    const thread = backend.threads.find((candidate) => candidate.id === threadId);
    if (!thread) throw new Error(`unknown chat thread: ${threadId}`);
    thread.messages.push({
      id: `message-${thread.messages.length + 1}`,
      role: "user",
      text: message,
      run_id: null,
      artifact_ids: [],
      status: "completed",
      created_at: "2026-07-09T09:00:00.000Z",
      completed_at: "2026-07-09T09:00:01.000Z",
      client_request_id: requestId,
    });
    thread.updated_at = `2026-07-09T12:00:0${backend.nextId}Z`;
    return { thread: cloneThread(thread) };
  }),
  setChatThreadProfile: vi.fn(async (threadId: string) => {
    const thread = backend.threads.find((candidate) => candidate.id === threadId);
    if (!thread) throw new Error(`unknown chat thread: ${threadId}`);
    return cloneThread(thread);
  }),
  setChatModelProfile: vi.fn(async (threadId: string) => {
    const thread = backend.threads.find((candidate) => candidate.id === threadId);
    if (!thread) throw new Error(`unknown chat thread: ${threadId}`);
    return cloneThread(thread);
  }),
  setChatAgentEngine: vi.fn(async (threadId: string) => {
    const thread = backend.threads.find((candidate) => candidate.id === threadId);
    if (!thread) throw new Error(`unknown chat thread: ${threadId}`);
    return cloneThread(thread);
  }),
  attachChatArtifact: vi.fn(async (threadId: string) => {
    const thread = backend.threads.find((candidate) => candidate.id === threadId);
    if (!thread) throw new Error(`unknown chat thread: ${threadId}`);
    return { thread: cloneThread(thread), contribution: {} };
  }),
  removeChatContribution: vi.fn(async (threadId: string) => {
    const thread = backend.threads.find((candidate) => candidate.id === threadId);
    if (!thread) throw new Error(`unknown chat thread: ${threadId}`);
    return cloneThread(thread);
  }),
  cancelChatMessage: vi.fn(async () => undefined),
}));

import {
  createChatThread,
  getChatThread,
  listArtifacts,
  listChatThreads,
  sendChatMessage,
  setChatThreadProfile,
} from "$lib/api";
import { artifacts, artifactsLoaded } from "$lib/stores/artifacts";
import {
  activeChatThread,
  activeChatThreadId,
  chatThreads,
  createNewChatThread,
  deleteExistingChatThread,
  ensureChatThread,
  renameExistingChatThread,
  parsePendingSends,
  selectAssistantProfile,
  selectChatThread,
  sendMessageToActiveThread,
  chatDrafts,
  setChatDraftText,
} from "$lib/stores/chatThreads";

describe("chatThreads store", () => {
  beforeEach(() => {
    backend.nextId = 1;
    backend.threads = [];
    backend.artifacts = [];
    artifacts.set([]);
    artifactsLoaded.set(false);
    chatThreads.set([]);
    activeChatThread.set(null);
    activeChatThreadId.set(null);
    chatDrafts.set(new Map());
    vi.mocked(createChatThread).mockClear();

    const storage = new Map<string, string>();
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: {
        getItem: (key: string) => storage.get(key) ?? null,
        setItem: (key: string, value: string) => storage.set(key, value),
        removeItem: (key: string) => storage.delete(key),
      },
    });
  });

  it("rejects corrupt or unknown pending-send recovery formats", () => {
    expect(() => parsePendingSends("{}")).toThrow("unsupported storage format");
    expect(() => parsePendingSends(JSON.stringify({ version: 2, sends: {} }))).toThrow(
      "unsupported storage format",
    );
    expect(() => parsePendingSends(JSON.stringify({
      version: 1,
      sends: { "thread-1": { requestId: "", message: "hello" } },
    }))).toThrow("incomplete");
  });

  it("supports create, switch, rename, and delete thread flows", async () => {
    await createNewChatThread();
    await sendMessageToActiveThread("First chat");
    await createNewChatThread();

    const summaries = get(chatThreads);
    expect(summaries).toHaveLength(2);

    await selectChatThread(summaries[1].id);
    expect(get(activeChatThreadId)).toBe(summaries[1].id);

    await renameExistingChatThread(summaries[1].id, "Renamed thread");
    expect(get(activeChatThread)?.title).toBe("Renamed thread");
    expect(get(activeChatThread)?.revision).toBe(1);
    expect(get(chatThreads).some((thread) => thread.title === "Renamed thread")).toBe(true);

    await deleteExistingChatThread(summaries[1].id);
    expect(get(chatThreads)).toHaveLength(1);
    expect(get(activeChatThreadId)).toBe(get(chatThreads)[0]?.id ?? null);
  });

  it("reuses the current untouched new chat", async () => {
    await createNewChatThread();
    await createNewChatThread();

    expect(createChatThread).toHaveBeenCalledOnce();
    expect(backend.threads).toHaveLength(1);
    expect(get(chatThreads)).toHaveLength(1);
  });

  it("selects an inactive untouched draft instead of creating another", async () => {
    await createNewChatThread();
    await sendMessageToActiveThread("Older conversation");
    const olderThreadId = get(activeChatThreadId)!;
    await createNewChatThread();
    const draftThreadId = get(activeChatThreadId)!;
    await selectChatThread(olderThreadId);

    await createNewChatThread();

    expect(get(activeChatThreadId)).toBe(draftThreadId);
    expect(createChatThread).toHaveBeenCalledTimes(2);
    expect(backend.threads).toHaveLength(2);
    expect(get(chatThreads)).toHaveLength(2);
  });

  it("shares concurrent new-chat scans instead of creating duplicate drafts", async () => {
    let releaseList!: () => void;
    const listGate = new Promise<void>((resolve) => { releaseList = resolve; });
    const realList = vi.mocked(listChatThreads).getMockImplementation()!;
    vi.mocked(listChatThreads).mockImplementationOnce(async () => {
      await listGate;
      return realList();
    });

    const first = createNewChatThread();
    const second = createNewChatThread();
    releaseList();
    await Promise.all([first, second]);

    expect(createChatThread).toHaveBeenCalledOnce();
    expect(backend.threads).toHaveLength(1);
  });

  it("keeps a configured empty chat distinct from the next new chat", async () => {
    await createNewChatThread();
    await sendMessageToActiveThread("Older conversation");
    const olderThreadId = get(activeChatThreadId)!;
    await createNewChatThread();
    const configuredDraftId = get(activeChatThreadId)!;
    await renameExistingChatThread(configuredDraftId, "Configured draft");
    await selectChatThread(olderThreadId);

    await createNewChatThread();

    expect(createChatThread).toHaveBeenCalledTimes(3);
    expect(get(chatThreads)).toHaveLength(3);
    expect(get(activeChatThreadId)).not.toBe(configuredDraftId);
  });

  it("reloads a persisted thread before creating a new one", async () => {
    backend.threads = [
      {
        id: "thread-persisted",
        resource_id: "chat-thread-persisted",
        revision: 1,
        title: "Saved thread",
        created_at: "2026-07-09T09:00:00Z",
        updated_at: "2026-07-09T09:05:00Z",
        injected_contexts: [],
        messages: [
          {
            id: "message-1",
            role: "assistant",
            text: "Welcome back",
            run_id: null,
            artifact_ids: [],
            status: "completed",
            created_at: "2026-07-09T09:00:00.000Z",
            completed_at: "2026-07-09T09:00:01.000Z",
          },
        ],
      },
    ];
    globalThis.localStorage.setItem("kernel.active-chat-thread", "thread-persisted");

    await ensureChatThread();

    expect(get(activeChatThreadId)).toBe("thread-persisted");
    expect(get(activeChatThread)?.messages[0]?.text).toBe("Welcome back");
    expect(get(chatThreads)).toHaveLength(1);
    expect(backend.threads).toHaveLength(1);
  });

  it("shares concurrent ensure work instead of creating duplicate threads", async () => {
    await Promise.all([ensureChatThread(), ensureChatThread(), ensureChatThread()]);

    expect(createChatThread).toHaveBeenCalledOnce();
    expect(backend.threads).toHaveLength(1);
    expect(get(activeChatThreadId)).toBe("thread-1");
  });

  it("does not switch back to the sending thread when the user moved on mid-send", async () => {
    await createNewChatThread();
    await sendMessageToActiveThread("seed first chat");
    const firstId = get(activeChatThreadId)!;
    await createNewChatThread();
    const secondId = get(activeChatThreadId)!;
    await selectChatThread(firstId);

    let releaseSend!: () => void;
    const gate = new Promise<void>((resolve) => {
      releaseSend = resolve;
    });
    const realSend = vi.mocked(sendChatMessage).getMockImplementation()!;
    vi.mocked(sendChatMessage).mockImplementationOnce(async (threadId, message, requestId, onStream) => {
      await gate;
      return realSend(threadId, message, requestId, onStream);
    });

    const pendingSend = sendMessageToActiveThread("hello");
    await selectChatThread(secondId);
    releaseSend();
    await pendingSend;

    expect(get(activeChatThreadId)).toBe(secondId);
    expect(get(activeChatThread)?.id).toBe(secondId);
    // The sent message still landed in the summary list.
    expect(get(chatThreads).find((thread) => thread.id === firstId)?.message_count).toBe(2);
  });

  it("preserves text entered while a successful send is in flight", async () => {
    await createNewChatThread();
    const threadId = get(activeChatThreadId)!;
    let releaseSend!: () => void;
    const gate = new Promise<void>((resolve) => {
      releaseSend = resolve;
    });
    const realSend = vi.mocked(sendChatMessage).getMockImplementation()!;
    vi.mocked(sendChatMessage).mockImplementationOnce(async (...args) => {
      await gate;
      return realSend(...args);
    });

    const send = sendMessageToActiveThread("first request");
    setChatDraftText(threadId, "next thought");
    releaseSend();
    await send;

    expect(get(chatDrafts).get(threadId)?.text).toBe("next thought");
  });

  it("synchronizes new tool artifacts before exposing the completed reply", async () => {
    await createNewChatThread();
    let finishArtifacts!: (value: Artifact[]) => void;
    vi.mocked(listArtifacts).mockReturnValueOnce(
      new Promise((resolve) => { finishArtifacts = resolve; }),
    );
    const thread = cloneThread(backend.threads[0]);
    thread.messages.push({
      id: "assistant-1",
      role: "assistant",
      text: "Created the note.",
      run_id: "run-1",
      artifact_ids: ["artifact-new"],
      status: "completed",
      created_at: "2026-07-09T09:00:00.000Z",
      completed_at: "2026-07-09T09:00:01.000Z",
    });
    vi.mocked(sendChatMessage).mockResolvedValueOnce({ thread });

    const send = sendMessageToActiveThread("create a note");
    await vi.waitFor(() => expect(listArtifacts).toHaveBeenCalledOnce());
    expect(get(artifactsLoaded)).toBe(false);
    expect(get(activeChatThread)?.messages.some((message) => message.id === "assistant-1")).toBe(false);

    finishArtifacts([{
      artifact_id: "artifact-new",
      artifact_type: "note",
      title: "New note",
      content: {},
      provenance: {
        run_id: "run-1",
        capability: { provider: "notes", capability: "create" },
        grant_id: "grant-1",
        produced_by: "notes",
        recorded_at: "2026-07-25T00:00:00Z",
      },
    }]);
    await send;

    expect(get(artifacts).map((artifact) => artifact.artifact_id)).toEqual(["artifact-new"]);
    expect(get(activeChatThread)?.messages.some((message) => message.id === "assistant-1")).toBe(true);
  });

  it("reuses the request id when retrying a failed send of the same text", async () => {
    await createNewChatThread();

    vi.mocked(sendChatMessage).mockRejectedValueOnce(new Error("transport lost"));
    await expect(sendMessageToActiveThread("hello")).rejects.toThrow("transport lost");
    const failedRequestId = vi.mocked(sendChatMessage).mock.calls.at(-1)![2];

    // Retrying the identical text must reuse the key so the host can
    // recognize already-executed work.
    await sendMessageToActiveThread("hello");
    expect(vi.mocked(sendChatMessage).mock.calls.at(-1)![2]).toBe(failedRequestId);

    // A different message is new work and gets a fresh key.
    await sendMessageToActiveThread("something else");
    expect(vi.mocked(sendChatMessage).mock.calls.at(-1)![2]).not.toBe(failedRequestId);
  });

  it("restores a retry request id from local storage after module state is lost", async () => {
    await createNewChatThread();
    const threadId = get(activeChatThreadId)!;
    globalThis.localStorage.setItem(
      "kernel.pending-chat-sends",
      JSON.stringify({
        version: 1,
        sends: { [threadId]: { requestId: "durable-request", message: "hello" } },
      }),
    );

    await sendMessageToActiveThread("hello");

    expect(vi.mocked(sendChatMessage).mock.calls.at(-1)![2]).toBe("durable-request");
    expect(globalThis.localStorage.getItem("kernel.pending-chat-sends")).toBe(
      JSON.stringify({ version: 1, sends: {} }),
    );
  });

  it("sends only the visible message through the private Chat command", async () => {
    await createNewChatThread();

    await sendMessageToActiveThread("continue");

    expect(vi.mocked(sendChatMessage).mock.calls.at(-1)?.[1]).toBe("continue");
    expect(vi.mocked(sendChatMessage).mock.calls.at(-1)).toHaveLength(4);
  });

  it("drops an out-of-order selection response instead of jumping threads", async () => {
    await createNewChatThread();
    await sendMessageToActiveThread("seed first chat");
    const firstId = get(activeChatThreadId)!;
    await createNewChatThread();
    const secondId = get(activeChatThreadId)!;

    let releaseFirst!: () => void;
    const gate = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });
    const realGet = vi.mocked(getChatThread).getMockImplementation()!;
    vi.mocked(getChatThread).mockImplementationOnce(async (threadId) => {
      await gate;
      return realGet(threadId);
    });

    const staleSelect = selectChatThread(firstId);
    await selectChatThread(secondId);
    releaseFirst();
    await staleSelect;

    expect(get(activeChatThreadId)).toBe(secondId);
    expect(get(activeChatThread)?.id).toBe(secondId);
  });

  it("does not let an older hydration overwrite a completed send", async () => {
    await createNewChatThread();
    const threadId = get(activeChatThreadId)!;
    const staleThread = cloneThread(get(activeChatThread)!);
    let releaseHydration!: () => void;
    const gate = new Promise<void>((resolve) => { releaseHydration = resolve; });
    vi.mocked(getChatThread).mockImplementationOnce(async () => {
      await gate;
      return staleThread;
    });

    const hydration = selectChatThread(threadId);
    await sendMessageToActiveThread("new message");
    releaseHydration();
    await hydration;

    expect(get(activeChatThread)?.messages).toHaveLength(1);
    expect(get(activeChatThread)?.messages[0]?.text).toBe("new message");
  });

  it("does not show a profile response for a thread the user left", async () => {
    await createNewChatThread();
    await sendMessageToActiveThread("seed first chat");
    const firstId = get(activeChatThreadId)!;
    await createNewChatThread();
    const secondId = get(activeChatThreadId)!;
    await selectChatThread(firstId);
    const firstThread = cloneThread(get(activeChatThread)!);
    let releaseProfile!: () => void;
    const gate = new Promise<void>((resolve) => { releaseProfile = resolve; });
    vi.mocked(setChatThreadProfile).mockImplementationOnce(async () => {
      await gate;
      return firstThread;
    });

    const profileChange = selectAssistantProfile(firstId, "assistant-app", "default");
    await selectChatThread(secondId);
    releaseProfile();
    await profileChange;

    expect(get(activeChatThreadId)).toBe(secondId);
    expect(get(activeChatThread)?.id).toBe(secondId);
  });

  it("keeps draft text isolated per thread", async () => {
    await createNewChatThread();
    const firstId = get(activeChatThreadId)!;
    setChatDraftText(firstId, "Please review this");

    await createNewChatThread();
    const secondId = get(activeChatThreadId)!;
    expect(get(chatDrafts).get(secondId)?.contributions).toEqual([]);
    await selectChatThread(firstId);

    const draft = get(chatDrafts).get(firstId)!;
    await sendMessageToActiveThread(draft.text);

    expect(vi.mocked(sendChatMessage).mock.calls.at(-1)?.[1]).toBe("Please review this");
  });
});
