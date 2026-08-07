import { render, screen, fireEvent } from "@testing-library/svelte";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { get, type Writable } from "svelte/store";

import type { ChatCompositionReceipt, ChatMessageView, ChatPromptPreview, ChatThread, ChatThreadSummary, HostConfig, InstalledApp } from "$lib/api";
import ChatSurface from "./ChatSurface.svelte";

function receipt(
  systemPromptDigest: string,
  systemPrompt: string,
  createdAt: string,
  layers: ChatCompositionReceipt["layers"] = [],
  injectedContext: ChatCompositionReceipt["injected_context"] = null,
): ChatCompositionReceipt {
  return {
    system_prompt_digest: systemPromptDigest,
    assistant_profile_ref: "chat/standard",
    assistant_profile_digest: "standard",
    enabled_skill_digests: [],
    context_block_digests: [],
    attachment_refs: [],
    available_capability_refs: [],
    provider_profile_ref: "provider/profile",
    model_profile: null,
    agent_engine_ref: null,
    agent_engine_version: null,
    agent_engine_features: [],
    assistant_capability_refs: [],
    created_at: createdAt,
    system_prompt: systemPrompt,
    layers,
    injected_context: injectedContext,
  };
}

function promptPreview(modelId: string, layerTitle: string): ChatPromptPreview {
  return {
    system_prompt: "system",
    digest: `digest-${modelId}`,
    layers: [{ id: layerTitle, kind: "runtime-context", title: layerTitle, source: null, content: "content", editable: false, included: true }],
    available_skills: [],
    runtime: {
      host_version: "1.0.0",
      mode: "chat",
      model_id: modelId,
      connector_kind: "openai",
      app_inventory: null,
      connection_details: null,
    },
  };
}

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return { ...actual, availableCapabilitiesFor: vi.fn(async () => []), listChatProfiles: vi.fn(async () => []), listChatModelProfiles: vi.fn(async () => []), listChatAgentEngines: vi.fn(async () => []), getChatPromptPreview: vi.fn(async () => ({
    system_prompt: "system",
    digest: "digest",
    layers: [],
    available_skills: [],
    runtime: { host_version: "1.0.0", mode: "chat", model_id: "model", connector_kind: "openai" },
  })), validateExtensionContext: vi.fn(async () => {}) };
});

vi.mock("$lib/stores/hostState", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/stores/hostState")>();
  return { ...actual, refreshHost: vi.fn(async () => {}) };
});

vi.mock("$lib/stores/chatThreads", async () => {
  const { writable } = await import("svelte/store");
  const chatDrafts = writable(new Map());
  return {
    chatThreads: writable([]),
    activeChatThread: writable(null),
    activeChatThreadId: writable(null),
    chatDrafts,
    sendingChatThreadIds: writable(new Set()),
    streamingChatReplies: writable(new Map()),
    ensureChatThread: vi.fn(async () => {}),
    createNewChatThread: vi.fn(async () => {}),
    deleteExistingChatThread: vi.fn(async () => {}),
    renameExistingChatThread: vi.fn(async () => {}),
    selectChatThread: vi.fn(async () => {}),
    sendMessageToActiveThread: vi.fn(async () => {}),
    cancelMessageForActiveThread: vi.fn(async () => {}),
    setChatDraft: vi.fn((threadId, draft) => chatDrafts.update((current) => new Map(current).set(threadId, draft))),
    setChatDraftText: vi.fn((threadId, text) => chatDrafts.update((current) => {
      const next = new Map(current);
      const draft = next.get(threadId) ?? { text: "", contributions: [] };
      next.set(threadId, { ...draft, text });
      return next;
    })),
    clearChatDraft: vi.fn((threadId) => chatDrafts.update((current) => {
      const next = new Map(current);
      next.delete(threadId);
      return next;
    })),
    removeChatContributionFromThread: vi.fn(),
    selectAssistantProfile: vi.fn(async () => {}),
    selectModelProfile: vi.fn(async () => {}),
    selectChatAgentEngine: vi.fn(async () => {}),
  };
});

const chatStores = await import("$lib/stores/chatThreads");
const chatThreads = chatStores.chatThreads as Writable<ChatThreadSummary[]>;
const activeChatThread = chatStores.activeChatThread as Writable<ChatThread | null>;
const activeChatThreadId = chatStores.activeChatThreadId as Writable<string | null>;
const chatDrafts = chatStores.chatDrafts as Writable<ReadonlyMap<string, import("$lib/stores/chatThreads").ChatDraft>>;
const sendingChatThreadIds = chatStores.sendingChatThreadIds as Writable<ReadonlySet<string>>;
const ensureChatThread = vi.mocked(chatStores.ensureChatThread);
const sendMessageToActiveThread = vi.mocked(chatStores.sendMessageToActiveThread);
const renameExistingChatThread = vi.mocked(chatStores.renameExistingChatThread);
const deleteExistingChatThread = vi.mocked(chatStores.deleteExistingChatThread);
const createNewChatThread = vi.mocked(chatStores.createNewChatThread);
const selectChatAgentEngine = vi.mocked(chatStores.selectChatAgentEngine);
const selectModelProfile = vi.mocked(chatStores.selectModelProfile);

const { pendingChromeRequests } = await import("$lib/stores/chromeState");
const api = await import("$lib/api");
const availableCapabilitiesFor = vi.mocked(api.availableCapabilitiesFor);
const getChatPromptPreview = vi.mocked(api.getChatPromptPreview);
const validateExtensionContext = vi.mocked(api.validateExtensionContext);
const { apps } = await import("$lib/stores/apps");
const { hostConfig } = await import("$lib/stores/config");
const { hostInitialized } = await import("$lib/stores/hostState");
const { currentTab } = await import("$lib/stores/hostState");
const { appSettingsTarget } = await import("$lib/stores/navigation");

function installedChat(): InstalledApp {
  return {
    content_hash: "hash",
    installed_at: "2026-07-15T00:00:00Z",
    manifest: {
      app_id: "chat",
      version: "1.0.0",
      display_name: "Chat",
      description: "test",
      capabilities: [],
      surfaces: [],
      agents: [],
      skills: [],
      assistant_profiles: [],
      automations: [],
      connectors: [],
      config_declarations: [],
      artifact_types: [],
      extension_points: [],
      extension_contributions: [],
      grant_requests: [],
      event_subscriptions: [],
    },
  };
}

function unconfiguredHostConfig(): HostConfig {
  return {
    version: 2,
    host: {
      default_llm_provider: "llm-provider",
      default_llm_profile: null,
      cloud_llm_egress_accepted_profiles: [],
      app_data_backup_retention: 1,
    },
    apps: {},
    connectors: {},
    mcp_servers: {},
    mcp_exports: {},
    mcp_export_transitions: {},
    mcp_gateway: {
      enabled: false,
      bind_address: "127.0.0.1:8137",
      allowed_origins: [],
      oauth_enabled: false,
    },
  };
}

function chatMessage(
  overrides: Partial<ChatMessageView> & Pick<ChatMessageView, "id" | "role">,
): ChatMessageView {
  return {
    text: "",
    run_id: null,
    artifact_ids: [],
    status: null,
    created_at: "2026-07-01T10:00:00.000Z",
    completed_at: "2026-07-01T10:00:01.000Z",
    ...overrides,
  };
}

function thread(overrides: Partial<ChatThread> = {}): ChatThread {
  return {
    id: "thread-1",
    resource_id: "chat-thread-1",
    revision: 0,
    title: "Trip planning",
    created_at: "2026-07-01T10:00:00Z",
    updated_at: "2026-07-01T10:05:00Z",
    messages: [],
    injected_contexts: [],
    ...overrides,
  };
}

function summary(source: ChatThread): ChatThreadSummary {
  return {
    id: source.id,
    title: source.title,
    created_at: source.created_at,
    updated_at: source.updated_at,
    message_count: source.messages.length,
  };
}

function seedActiveThread(active: ChatThread = thread()) {
  chatThreads.set([summary(active)]);
  activeChatThread.set(active);
  activeChatThreadId.set(active.id);
}

beforeAll(() => {
  // jsdom has no layout engine; the sticky-bottom logic calls scrollTo.
  Element.prototype.scrollTo = vi.fn();
});

beforeEach(() => {
  hostConfig.set(null);
  vi.clearAllMocks();
  availableCapabilitiesFor.mockResolvedValue([]);
  chatThreads.set([]);
  activeChatThread.set(null);
  activeChatThreadId.set(null);
  chatDrafts.set(new Map());
  sendingChatThreadIds.set(new Set());
  pendingChromeRequests.set(0);
  currentTab.set("chat");
  appSettingsTarget.set(null);
  apps.set([installedChat()]);
  hostInitialized.set(true);
});

describe("ChatSurface tool discovery", () => {
  it("waits for authoritative host initialization before loading Chat configuration", async () => {
    hostInitialized.set(false);
    render(ChatSurface);

    await vi.waitFor(() => {
      expect(api.listChatProfiles).not.toHaveBeenCalled();
      expect(availableCapabilitiesFor).not.toHaveBeenCalled();
    });

    hostInitialized.set(true);

    await vi.waitFor(() => expect(api.listChatProfiles).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(availableCapabilitiesFor).toHaveBeenCalledOnce());
  });

  it("waits until Chat is installed before loading capabilities", async () => {
    apps.set([]);
    render(ChatSurface);
    await vi.waitFor(() => expect(availableCapabilitiesFor).not.toHaveBeenCalled());

    apps.set([installedChat()]);

    await vi.waitFor(() => {
      expect(availableCapabilitiesFor).toHaveBeenCalledOnce();
      expect(availableCapabilitiesFor).toHaveBeenCalledWith("chat");
    });
  });

  it("keeps send disabled until Chat is installed", async () => {
    seedActiveThread();
    apps.set([]);
    render(ChatSurface);
    const composer = screen.getByLabelText("chat message");
    const send = screen.getByLabelText("send message") as HTMLButtonElement;

    await fireEvent.input(composer, { target: { value: "hello" } });
    expect(send.disabled).toBe(true);
    await fireEvent.keyDown(composer, { key: "Enter" });
    expect(sendMessageToActiveThread).not.toHaveBeenCalled();

    apps.set([installedChat()]);
    await vi.waitFor(() => expect(send.disabled).toBe(false));
  });

  it("retries transient kernel contention while loading Chat choices", async () => {
    vi.spyOn(api, "listChatProfiles")
      .mockRejectedValueOnce(new Error("kernel busy: another host operation owns the kernel"))
      .mockResolvedValue([]);
    vi.spyOn(api, "listChatAgentEngines").mockResolvedValue([]);

    render(ChatSurface);

    await vi.waitFor(() => expect(api.listChatProfiles).toHaveBeenCalledTimes(2));
    expect(screen.queryByText("Couldn't load all Chat choices. Try again.")).toBeNull();
  });

  it("loads Chat choices sequentially so split transport reads do not contend with each other", async () => {
    let resolveProfiles!: (value: import("$lib/api").ChatProfileView[]) => void;
    vi.spyOn(api, "listChatProfiles").mockReturnValueOnce(
      new Promise((resolve) => { resolveProfiles = resolve; }),
    );

    render(ChatSurface);

    await vi.waitFor(() => expect(api.listChatProfiles).toHaveBeenCalledOnce());
    expect(api.listChatModelProfiles).not.toHaveBeenCalled();
    expect(api.listChatAgentEngines).not.toHaveBeenCalled();

    resolveProfiles([]);

    await vi.waitFor(() => expect(api.listChatModelProfiles).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(api.listChatAgentEngines).toHaveBeenCalledOnce());
  });

  it("coalesces a newer Chat refresh while the current choice read is unresolved", async () => {
    let resolveProfiles!: (value: import("$lib/api").ChatProfileView[]) => void;
    vi.spyOn(api, "listChatProfiles")
      .mockReturnValueOnce(new Promise((resolve) => { resolveProfiles = resolve; }))
      .mockResolvedValueOnce([]);

    render(ChatSurface);
    await vi.waitFor(() => expect(api.listChatProfiles).toHaveBeenCalledOnce());

    apps.set([installedChat()]);
    resolveProfiles([]);

    await vi.waitFor(() => expect(api.listChatProfiles).toHaveBeenCalledTimes(2));
    expect(api.listChatModelProfiles).toHaveBeenCalledOnce();
    expect(api.listChatAgentEngines).toHaveBeenCalledOnce();
  });

  it("retries transient kernel contention while loading tools", async () => {
    availableCapabilitiesFor
      .mockRejectedValueOnce(new Error("kernel busy: another host operation owns the kernel"))
      .mockResolvedValueOnce([]);

    render(ChatSurface);

    await vi.waitFor(() => expect(availableCapabilitiesFor).toHaveBeenCalledTimes(2));
    await fireEvent.click(screen.getByRole("button", { name: /Tools/ }));
    expect(screen.queryByText("Couldn't load the tool list. Try again.")).toBeNull();
  });
});

describe("ChatSurface prompt context", () => {
  it("ignores an older prompt preview that resolves after the current thread", async () => {
    let resolveFirst!: (preview: ChatPromptPreview) => void;
    let resolveSecond!: (preview: ChatPromptPreview) => void;
    getChatPromptPreview
      .mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }))
      .mockImplementationOnce(() => new Promise((resolve) => { resolveSecond = resolve; }));
    seedActiveThread();
    render(ChatSurface);

    await fireEvent.click(screen.getByRole("button", { name: "Model context" }));
    await vi.waitFor(() => expect(getChatPromptPreview).toHaveBeenCalledTimes(1));
    activeChatThread.set(thread({ id: "thread-2", resource_id: "chat-thread-2" }));
    await vi.waitFor(() => expect(getChatPromptPreview).toHaveBeenCalledTimes(2));

    resolveSecond(promptPreview("current-model", "Current layer"));
    expect(await screen.findByText("Current layer")).toBeTruthy();
    resolveFirst(promptPreview("stale-model", "Stale layer"));
    await vi.waitFor(() => expect(screen.queryByText("Stale layer")).toBeNull());
    expect(screen.getByText((content) => content.includes("current-model"))).toBeTruthy();
  });

  it("removes stale prompt details when the latest refresh fails", async () => {
    getChatPromptPreview
      .mockResolvedValueOnce(promptPreview("first-model", "Old layer"))
      .mockRejectedValueOnce(new Error("preview unavailable"));
    seedActiveThread();
    render(ChatSurface);

    await fireEvent.click(screen.getByRole("button", { name: "Model context" }));
    expect(await screen.findByText("Old layer")).toBeTruthy();
    activeChatThread.set(thread({ id: "thread-2", resource_id: "chat-thread-2" }));

    expect((await screen.findByRole("alert")).textContent).toContain("preview unavailable");
    expect(screen.queryByText("Old layer")).toBeNull();
  });

  it("shows inspector context separately from tools and keeps the first prompt receipt", async () => {
    const active = thread({
      messages: [
        chatMessage({ id: "user-1", role: "user", text: "Hello", run_id: null, artifact_ids: [], status: null, client_request_id: "req-1" }),
        chatMessage({ id: "assistant-1", role: "assistant", text: "Hi", run_id: "run-1", artifact_ids: [], status: "completed" }),
      ],
      prompt_receipts: {
        "req-1": receipt(
          "digest-1",
          "exact prompt",
          "2026-07-26T10:00:00Z",
          [{ id: "protocol", kind: "protocol", title: "Host protocol", source: null, content: "exact prompt" }],
        ),
      },
    });
    seedActiveThread(active);
    render(ChatSurface);

    await fireEvent.click(screen.getByRole("button", { name: "Model context" }));
    expect(await screen.findByText("Current authoritative prompt layers")).toBeTruthy();
    expect(screen.getByRole("region", { name: "Model context" })).toBeTruthy();
    expect(screen.getByText(/Stored app context is revalidated against its original Run and grant/)).toBeTruthy();
    expect(screen.getAllByText("exact prompt").length).toBeGreaterThan(1);
    expect(screen.getByText((content) => content.includes("digest-1"))).toBeTruthy();
    expect(screen.getByText("System prompt used")).toBeTruthy();
  });

  it("shows the host-recorded exact app context for its request", async () => {
    const appContext = "[Authorized app context]\nPlease review the marked claim.";
    const active = thread({
      messages: [
        chatMessage({ id: "user-1", role: "user", text: "Continue", client_request_id: "req-1" }),
        chatMessage({ id: "assistant-1", role: "assistant", text: "Reviewed", status: "completed" }),
      ],
      prompt_receipts: {
        "req-1": receipt("digest-1", "same prompt", "2026-08-02T10:00:00Z", [], {
          message_digest: "context-digest",
          entries: [{
            source_app_id: "org.example.reading",
            source_app_name: "Reading Insights",
            source_app_version: "1.0.0",
            item_id: "assistant-0",
            revision: 3,
            source_run_id: "run-context",
            grant_id: "grant-context",
            content_digest: "content-digest",
          }],
          exact_message: appContext,
        }),
      },
    });
    seedActiveThread(active);
    render(ChatSurface);

    await fireEvent.click(screen.getByText(/System prompt used.*exact app text recorded/));
    expect(screen.getByText("Grant-authorized app context")).toBeTruthy();
    expect(screen.getByText(/Reading Insights 1\.0\.0 · org\.example\.reading/)).toBeTruthy();
    expect(screen.getByText((_content, element) =>
      element?.tagName === "PRE" && element.textContent === appContext
    )).toBeTruthy();
  });

  it("labels metadata-only app-context receipts without exposing exact text", async () => {
    const active = thread({
      messages: [
        chatMessage({ id: "user-1", role: "user", text: "Continue", client_request_id: "req-1" }),
        chatMessage({ id: "assistant-1", role: "assistant", text: "Continued", status: "completed" }),
      ],
      prompt_receipts: {
        "req-1": receipt("digest-1", "same prompt", "2026-08-02T10:00:00Z", [], {
          message_digest: "context-digest",
          entries: [{
            source_app_id: "org.example.reading",
            source_app_name: "Reading Insights",
            source_app_version: "1.0.0",
            item_id: "assistant-0",
            revision: 3,
            source_run_id: "run-context",
            grant_id: "grant-context",
            content_digest: "content-digest",
          }],
          exact_message: null,
        }),
      },
    });
    seedActiveThread(active);
    render(ChatSurface);

    await fireEvent.click(screen.getByText(/System prompt used.*app metadata only/));
    expect(screen.getByText("Exact text was not recorded for this request.")).toBeTruthy();
    expect(screen.getByText(/Content digest content-digest/)).toBeTruthy();
  });

  it("shows prompt changes without repeating an unchanged receipt", () => {
    const active = thread({
      messages: [
        chatMessage({ id: "user-1", role: "user", text: "One", run_id: null, artifact_ids: [], status: null, client_request_id: "req-1" }),
        chatMessage({ id: "assistant-1", role: "assistant", text: "First", run_id: "run-1", artifact_ids: [], status: "completed" }),
        chatMessage({ id: "user-2", role: "user", text: "Two", run_id: null, artifact_ids: [], status: null, client_request_id: "req-2" }),
        chatMessage({ id: "assistant-2", role: "assistant", text: "Second", run_id: "run-2", artifact_ids: [], status: "completed" }),
        chatMessage({ id: "user-3", role: "user", text: "Three", run_id: null, artifact_ids: [], status: null, client_request_id: "req-3" }),
        chatMessage({ id: "assistant-3", role: "assistant", text: "Third", run_id: "run-3", artifact_ids: [], status: "completed" }),
      ],
      prompt_receipts: {
        "req-1": receipt("digest-a", "Prompt A", "2026-07-26T10:00:00Z"),
        "req-2": receipt("digest-a", "Prompt A", "2026-07-26T10:01:00Z"),
        "req-3": receipt("digest-b", "Prompt B", "2026-07-26T10:02:00Z"),
      },
    });
    seedActiveThread(active);
    render(ChatSurface);

    expect(screen.getAllByText("System prompt used")).toHaveLength(1);
    expect(screen.getAllByText("System prompt changed")).toHaveLength(1);
    expect(screen.queryByText("System prompt sent")).toBeNull();
    expect(screen.getAllByText("Prompt A")).toHaveLength(1);
    expect(screen.getAllByText("Prompt B")).toHaveLength(1);
  });
});

describe("ChatSurface assistant profile selector", () => {
  it("shows the live app title and description only when multiple usable profiles exist", async () => {
    availableCapabilitiesFor.mockResolvedValue([]);
    vi.spyOn(api, "listChatProfiles").mockResolvedValue([
      {
        app_id: "com.example.writer",
        profile_name: "assistant",
        version: "2.0.0",
        digest: "d".repeat(64),
        reviewed_skill_digests: [],
        capability_refs: ["notes/create"],
        engine_contract: "agent.run",
        status: "available",
        app_display_name: "Writer Kit",
        title: "Writer Assistant",
        description: "Draft responses",
        suggested_capability_refs: ["notes/create"],
        suggested_agent_engine_contract: "agent.run",
        availability: "available",
        availability_reason: null,
      },
      {
        app_id: "com.example.writer",
        profile_name: "strict",
        version: "2.0.0",
        digest: "e".repeat(64),
        reviewed_skill_digests: [],
        capability_refs: [],
        engine_contract: null,
        status: "available",
        app_display_name: "Writer Kit",
        title: "Strict",
        description: "Narrower draft style",
        suggested_capability_refs: [],
        suggested_agent_engine_contract: null,
        availability: "available",
        availability_reason: null,
      },
    ] as import("$lib/api").ChatProfileView[]);
    seedActiveThread();
    render(ChatSurface);

    expect(await screen.findByRole("combobox", { name: "Assistant profile" })).toBeTruthy();
    expect(
      screen.getByRole("option", { name: /Writer Kit\s*\/\s*Writer Assistant/ }),
    ).toBeTruthy();
    expect(screen.getByText("Draft responses")).toBeTruthy();
  });
});

describe("ChatSurface agent engine selector", () => {
  it("shows Plain LLM alongside one granted engine", async () => {
    vi.spyOn(api, "listChatAgentEngines").mockResolvedValueOnce([{
      app_id: "com.example.agent",
      display_name: "Example Agent",
      version: "1.0.0",
      contract: "agent.run",
      features: [],
      available: true,
      availability_reason: null,
    }]);
    seedActiveThread();
    render(ChatSurface);

    const selector = await screen.findByRole("combobox", { name: "Agent engine" });
    expect(screen.getByRole("option", { name: "Plain LLM" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "Example Agent / 1.0.0" })).toBeTruthy();

    await fireEvent.change(selector, { target: { value: "com.example.agent" } });
    expect(selectChatAgentEngine).toHaveBeenCalledWith("thread-1", "com.example.agent");
  });

  it("reports an engine selection failure without hiding the current conversation", async () => {
    vi.spyOn(api, "listChatAgentEngines").mockResolvedValueOnce([{
      app_id: "com.example.agent",
      display_name: "Example Agent",
      version: "1.0.0",
      contract: "agent.run",
      features: [],
      available: true,
      availability_reason: null,
    }]);
    selectChatAgentEngine.mockRejectedValueOnce(new Error("kernel busy"));
    seedActiveThread();
    render(ChatSurface);

    const selector = await screen.findByRole("combobox", { name: "Agent engine" });
    await fireEvent.change(selector, { target: { value: "com.example.agent" } });

    expect(await screen.findByText(/Couldn't change the agent engine/)).toBeTruthy();
    expect(screen.getAllByText("Trip planning")).toHaveLength(2);
  });
});

describe("ChatSurface model profile selector", () => {
  it("shows effective authority and selects a reusable model profile", async () => {
    const modelProfile: import("$lib/api").ChatModelProfileView = {
      source_app_id: "com.example.model-setup",
      source_app_name: "Model Setup",
      source_app_version: "0.1.0",
      profile_id: "focused-work",
      profile_digest: "a".repeat(64),
      title: "Focused work",
      description: "Use only note reads.",
      connector_id: "llm-provider/local",
      model: "model-a",
      reasoning: "high",
      temperature: 0.2,
      max_output_tokens: 4096,
      tool_refs: ["notes/read", "notes/write"],
      effective_tool_refs: ["notes/read"],
      unavailable_tool_refs: ["notes/write"],
      available: true,
      availability_reason: null,
    };
    vi.mocked(api.listChatModelProfiles).mockResolvedValue([modelProfile]);
    seedActiveThread(thread({ model_profile_ref: null, model_profile_receipt: null }));
    render(ChatSurface);

    const selector = await screen.findByRole("combobox", { name: "Model profile" });
    expect(screen.getByRole("option", { name: "Chat default" })).toBeTruthy();
    expect(screen.getByRole("option", { name: /Focused work.*model-a/ })).toBeTruthy();
    await fireEvent.change(selector, { target: { value: "com.example.model-setup/focused-work" } });
    expect(selectModelProfile).toHaveBeenCalledWith(
      "thread-1",
      "com.example.model-setup/focused-work",
    );
  });

  it("explains when configured tools are outside Chat's grants", async () => {
    const receipt = {
      source_app_id: "com.example.model-setup",
      source_app_version: "0.1.0",
      profile_id: "focused-work",
      profile_digest: "a".repeat(64),
      title: "Focused work",
      connector_id: "llm-provider/local",
      model: "model-a",
      reasoning: null,
      temperature: null,
      max_output_tokens: null,
      tool_refs: ["notes/read", "notes/write"],
    } satisfies import("$lib/api").ChatModelProfileReceipt;
    vi.mocked(api.listChatModelProfiles).mockResolvedValue([{
      ...receipt,
      source_app_name: "Model Setup",
      description: "Use only note reads.",
      effective_tool_refs: ["notes/read"],
      unavailable_tool_refs: ["notes/write"],
      available: true,
      availability_reason: null,
    }]);
    seedActiveThread(thread({
      model_profile_ref: "focused-work",
      model_profile_receipt: receipt,
    }));
    render(ChatSurface);

    await screen.findByRole("option", { name: /Model Setup.*Focused work.*model-a/ });
    const selector = await screen.findByRole("combobox", { name: "Model profile" }) as HTMLSelectElement;
    expect(selector.value).toBe("com.example.model-setup/focused-work");
    expect(await screen.findByText(/Not granted to Chat and excluded: notes\/write/)).toBeTruthy();
    expect(screen.getByText(/1 of 2 profile tools available/)).toBeTruthy();
  });
});

describe("ChatSurface composer", () => {
  it("supplies the active resource to the thread-actions contract", async () => {
    seedActiveThread();

    render(ChatSurface);

    await vi.waitFor(() => {
      expect(validateExtensionContext).toHaveBeenCalledWith("chat", "thread-actions", {
        thread_id: "thread-1",
        resource_id: "chat-thread-1",
        revision: 0,
      });
    });

    activeChatThread.set(thread({ revision: 1 }));
    await vi.waitFor(() => {
      expect(validateExtensionContext).toHaveBeenCalledWith("chat", "thread-actions", {
        thread_id: "thread-1",
        resource_id: "chat-thread-1",
        revision: 1,
      });
    });
  });

  it("supplies the declared composer-context contract", async () => {
    seedActiveThread();

    render(ChatSurface);

    await vi.waitFor(() => {
      expect(validateExtensionContext).toHaveBeenCalledWith("chat", "composer-context", {
        thread_id: "thread-1",
        selection: "",
        request_id: "thread-1:0",
      });
    });
  });

  it("sends on Enter, keeps Shift+Enter and IME composition as newline", async () => {
    seedActiveThread();
    render(ChatSurface);
    const composer = screen.getByLabelText("chat message");

    await fireEvent.input(composer, { target: { value: "hello" } });
    await fireEvent.keyDown(composer, { key: "Enter", shiftKey: true });
    expect(sendMessageToActiveThread).not.toHaveBeenCalled();

    await fireEvent.keyDown(composer, { key: "Enter", isComposing: true });
    expect(sendMessageToActiveThread).not.toHaveBeenCalled();

    await fireEvent.keyDown(composer, { key: "Enter" });
    await vi.waitFor(() => {
      expect(sendMessageToActiveThread).toHaveBeenCalledWith("hello");
    });
    expect((composer as HTMLTextAreaElement).value).toBe("");
  });

  it("restores the draft and shows a compact error when sending fails", async () => {
    seedActiveThread();
    sendMessageToActiveThread.mockRejectedValueOnce(new Error("kernel busy"));
    render(ChatSurface);
    const composer = screen.getByLabelText("chat message");

    await fireEvent.input(composer, { target: { value: "important message" } });
    await fireEvent.keyDown(composer, { key: "Enter" });

    expect(await screen.findByText(/Couldn't send your message/)).toBeTruthy();
    expect((composer as HTMLTextAreaElement).value).toBe("important message");
  });

  it("restores failed context without discarding text typed during the send", async () => {
    seedActiveThread();
    let rejectSend!: (error: Error) => void;
    sendMessageToActiveThread.mockReturnValueOnce(new Promise((_, reject) => { rejectSend = reject; }));
    render(ChatSurface);
    const composer = screen.getByLabelText("chat message");

    await fireEvent.input(composer, { target: { value: "first request" } });
    await fireEvent.keyDown(composer, { key: "Enter" });
    await fireEvent.input(composer, { target: { value: "next thought" } });
    rejectSend(new Error("transport lost"));

    await vi.waitFor(() => {
      expect((composer as HTMLTextAreaElement).value).toBe("first request\n\nnext thought");
    });
  });

  it("disables send without an active thread", () => {
    render(ChatSurface);
    const send = screen.getByLabelText("send message") as HTMLButtonElement;
    expect(send.disabled).toBe(true);
  });

  it("keeps attached draft context out of the textarea", async () => {
    seedActiveThread();
    chatDrafts.set(new Map([["thread-1", {
      text: "Add my question here",
      contributions: [{
        source_app_id: "notes",
        source_app_version: "1.0.0",
        source_contract: 1,
        item_id: "context-1",
        revision: 1,
        digest: "digest",
        completeness: "complete",
        lifecycle: "accepted",
        kind: "text-snapshot",
        title: "Planning",
        body: { text: "Book the train" },
        created_at: "2026-07-26T10:00:00Z",
        updated_at: "2026-07-26T10:00:00Z",
      }],
    }]]));
    render(ChatSurface);

    expect((screen.getByLabelText("chat message") as HTMLTextAreaElement).value).toBe("Add my question here");
    expect((screen.getByLabelText("chat message") as HTMLTextAreaElement).value).not.toContain("Book the train");
  });

  it("offers use selected snapshot only for retained text snapshots", async () => {
    seedActiveThread();
    chatDrafts.set(new Map([["thread-1", {
      text: "",
      contributions: [{
        source_app_id: "notes",
        source_app_version: "1.0.0",
        source_contract: 1,
        item_id: "context-1",
        revision: 1,
        digest: "digest",
        completeness: "complete",
        lifecycle: "accepted",
        kind: "text-snapshot",
        title: "Planning",
        body: { text: "Remember the train" },
        created_at: "2026-07-26T10:00:00Z",
        updated_at: "2026-07-26T10:00:00Z",
      }],
    }]]));
    render(ChatSurface);

    expect(screen.getByRole("button", { name: "Use selected snapshot" })).toBeTruthy();
  });
});

describe("ChatSurface approval state", () => {
  it("shows a waiting-for-approval banner while trusted chrome has a prompt open", async () => {
    seedActiveThread();
    render(ChatSurface);
    expect(screen.queryByText("Waiting for approval")).toBeNull();

    pendingChromeRequests.set(1);
    await vi.waitFor(() => {
      expect(screen.getAllByText("Waiting for approval").length).toBeGreaterThan(0);
    });
    expect(screen.getByText("Respond to the approval prompt to continue.")).toBeTruthy();
  });
});

describe("ChatSurface thread management", () => {
  it("leaves initial thread creation to host startup", async () => {
    render(ChatSurface);

    await vi.waitFor(() => expect(ensureChatThread).not.toHaveBeenCalled());
    expect(screen.queryByText("Couldn't open that chat. Try again.")).toBeNull();
  });

  it("renames a thread and closes the editor on success", async () => {
    seedActiveThread();
    render(ChatSurface);

    await fireEvent.click(screen.getByLabelText("rename chat"));
    const input = screen.getByLabelText("rename chat") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "Summer trip" } });
    await fireEvent.keyDown(input, { key: "Enter" });

    await vi.waitFor(() => {
      expect(renameExistingChatThread).toHaveBeenCalledWith("thread-1", "Summer trip");
    });
    expect(screen.queryByRole("textbox", { name: "rename chat" })).toBeNull();
  });

  it("keeps the rename editor open with an error when renaming fails", async () => {
    seedActiveThread();
    renameExistingChatThread.mockRejectedValueOnce(new Error("kernel busy"));
    render(ChatSurface);

    await fireEvent.click(screen.getByLabelText("rename chat"));
    const input = screen.getByLabelText("rename chat") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "Summer trip" } });
    await fireEvent.keyDown(input, { key: "Enter" });

    expect(await screen.findByText(/Couldn't rename the chat/)).toBeTruthy();
    expect(screen.getByLabelText("rename chat")).toBeTruthy();
    expect(input.value).toBe("Summer trip");
  });

  it("cancels a rename with Escape without saving", async () => {
    seedActiveThread();
    render(ChatSurface);

    await fireEvent.click(screen.getByLabelText("rename chat"));
    const input = screen.getByLabelText("rename chat") as HTMLInputElement;
    await fireEvent.keyDown(input, { key: "Escape" });

    expect(renameExistingChatThread).not.toHaveBeenCalled();
    expect(screen.queryByRole("textbox", { name: "rename chat" })).toBeNull();
  });

  it("deletes only after confirmation and keeps the confirm open on failure", async () => {
    seedActiveThread();
    deleteExistingChatThread.mockRejectedValueOnce(new Error("kernel busy"));
    render(ChatSurface);

    await fireEvent.click(screen.getByLabelText("delete chat"));
    expect(screen.getByText("Delete this chat?")).toBeTruthy();
    expect(deleteExistingChatThread).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(await screen.findByText(/Couldn't delete the chat/)).toBeTruthy();
    expect(screen.getByText("Delete this chat?")).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await vi.waitFor(() => {
      expect(deleteExistingChatThread).toHaveBeenCalledTimes(2);
    });
  });

  it("surfaces a compact error when creating a chat fails permanently", async () => {
    seedActiveThread();
    createNewChatThread.mockRejectedValueOnce(new Error("chat storage unavailable"));
    render(ChatSurface);

    await fireEvent.click(screen.getByRole("button", { name: "New chat" }));
    expect(await screen.findByText(/Couldn't create a new chat/)).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText(/Couldn't create a new chat/)).toBeNull();
  });

  it("marks threads that are still working on a reply", async () => {
    const active = thread();
    seedActiveThread(active);
    sendingChatThreadIds.set(new Set([active.id]));
    render(ChatSurface);

    await vi.waitFor(() => {
      expect(screen.getByTitle("Working on a reply")).toBeTruthy();
    });
    expect(get(sendingChatThreadIds).has(active.id)).toBe(true);
  });
});

describe("ChatSurface empty state", () => {
  it("links directly to model-provider settings when no default is configured", async () => {
    hostConfig.set(unconfiguredHostConfig());
    seedActiveThread();
    render(ChatSurface);

    await fireEvent.click(screen.getByRole("button", { name: "Configure model provider" }));

    expect(get(currentTab)).toBe("settings");
    expect(get(appSettingsTarget)).toMatchObject({
      appId: "llm-provider",
      displayName: "LLM Provider",
    });
  });

  it("shows generic suggestions independent of installed app tools", () => {
    availableCapabilitiesFor.mockResolvedValue([{
      provider_app_id: "com.example.tasks",
      provider_display_name: "Tasks",
      capability: "task.create",
      description: "Create a task",
      input_schema: {},
      authorizations: [{ data_scope: { kind: "none" }, condition: "notify" }],
    }]);
    seedActiveThread();
    const { container } = render(ChatSurface);

    expect(screen.getByText("Ask anything")).toBeTruthy();
    expect(container.querySelector('.greeting-avatar svg[aria-label="Kestral"]')).toBeTruthy();
    expect(screen.getByText("Draft a message")).toBeTruthy();
    expect(screen.queryByText("Create a note")).toBeNull();
  });

  it("renders conversation messages once they exist", () => {
    seedActiveThread(
      thread({
        messages: [
          chatMessage({ id: "m1", role: "user", text: "What's on my list?" }),
        ],
      }),
    );
    render(ChatSurface);

    expect(screen.queryByText("Ask anything")).toBeNull();
    expect(screen.getByText("What's on my list?")).toBeTruthy();
  });
});
