<script lang="ts">
  import { onMount, tick } from "svelte";
  import {
    getChatPromptPreview,
    availableCapabilitiesFor,
    type CapabilityUseView,
    type ChatPromptPreview,
    type ChatMessageView,
    type ChatThreadSummary,
    type Provenance,
    type GrantCondition,
  } from "$lib/api";
  import { mostInteractiveCondition } from "$lib/apps/capabilityAccess";
  import ChatMessage from "$lib/chat/ChatMessage.svelte";
  import {
    attachChatReadingObserver,
    createReadingOpportunityTracker,
    type ChatReadingObserver,
    type ReadingOpportunityReport,
  } from "$lib/chat/chatReadingOpportunity";
  import {
    deriveScrollUpdate,
    getChatContentVersion,
    isNearBottomPosition,
  } from "$lib/chat/chatScroll";
  import { describeChatActionError } from "$lib/chat/chatThreadsModel";
  import ChatExtensionSlot from "$lib/chat/ChatExtensionSlot.svelte";
  import KestralMark from "$lib/shell/KestralMark.svelte";
  import { hostInitialized, refreshHost } from "$lib/stores/hostState";
  import { pendingChromeRequests } from "$lib/stores/chromeState";
  import { apps } from "$lib/stores/apps";
  import { appConfigEntry, hostConfig } from "$lib/stores/config";
  import { openAppSettings } from "$lib/stores/navigation";
  import { grantsRevision } from "$lib/stores/grants";
  import {
    activeChatThread,
    chatDrafts,
    chatThreads,
    cancelMessageForActiveThread,
    createNewChatThread,
    clearChatDraft,
    deleteExistingChatThread,
    renameExistingChatThread,
    removeChatContributionFromThread,
    selectChatAgentEngine,
    selectModelProfile,
    selectAssistantProfile,
    selectChatThread,
    setChatDraft,
    setChatDraftText,
    sendMessageToActiveThread,
    sendingChatThreadIds,
    streamingChatReplies,
  } from "$lib/stores/chatThreads";

  let log = $state<HTMLElement | null>(null);
  let composer = $state<HTMLTextAreaElement | null>(null);
  let renamingThreadId = $state<string | null>(null);
  let renameDraft = $state("");
  let renameInput = $state<HTMLInputElement | null>(null);
  let deleteConfirmThreadId = $state<string | null>(null);
  let threadActionError = $state<string | null>(null);
  let sendError = $state<string | null>(null);
  let availableTools = $state<CapabilityUseView[]>([]);
  let toolsError = $state<string | null>(null);
  let toolsOpen = $state(false);
  let modelContextOpen = $state(false);
  let threadListOpen = $state(false);
  let userPinnedToBottom = true;
  let hasUnreadBelow = $state(false);
  let lastThreadId: string | null = null;
  let lastMessageCount = 0;
  let lastContentVersion = "";
  let forceScrollToBottom = false;
  let scrollRequestVersion = 0;
  // One observation controller for the whole log — not one per message and not
  // one per extension frame. It stays idle until a message asks for it.
  interface ReadingRegion {
    element: HTMLElement;
    deliver: (report: ReadingOpportunityReport) => void;
  }
  const readingRegions = new Map<string, ReadingRegion>();
  const readingRequests = new Set<string>();
  let readingObserver: ChatReadingObserver | null = null;
  const readingTracker = createReadingOpportunityTracker({
    onReport: (report) => readingRegions.get(report.messageId)?.deliver(report),
  });
  const readingObservation = {
    register(
      messageId: string,
      element: HTMLElement,
      deliver: (report: ReadingOpportunityReport) => void,
    ): void {
      readingRegions.set(messageId, { element, deliver });
      readingObserver?.register(messageId, element);
    },
    unregister(messageId: string): void {
      readingRegions.delete(messageId);
      readingRequests.delete(messageId);
      readingObserver?.unregister(messageId);
    },
    setRequested(messageId: string, requested: boolean): void {
      if (requested) readingRequests.add(messageId);
      else readingRequests.delete(messageId);
      readingObserver?.refresh();
    },
  };
  let promptPreview = $state<ChatPromptPreview | null>(null);
  let promptPreviewError = $state<string | null>(null);
  let promptPreviewStatus = $state<"idle" | "loading" | "error" | "ready">("idle");
  let promptPreviewRequestVersion = 0;
  let availableProfiles = $state<import("$lib/api").ChatProfileView[]>([]);
  let availableModelProfiles = $state<import("$lib/api").ChatModelProfileView[]>([]);
  let availableAgentEngines = $state<import("$lib/api").ChatAgentEngineView[]>([]);
  let chatChoicesError = $state<string | null>(null);
  let chatChoicesRequestVersion = 0;
  let chatChoicesLoadInFlight: Promise<void> | null = null;
  let chatChoicesReloadRequested = false;
  let toolsRequestVersion = 0;
  let toolsLoadInFlight: Promise<void> | null = null;
  let toolsReloadRequested = false;
  let selectedAssistantProfile = $derived($activeChatThread?.assistant_profile_ref ?? "standard");
  const usableProfiles = $derived(availableProfiles.filter((profile) => profile.availability === "available"));
  const usableModelProfiles = $derived(availableModelProfiles.filter((profile) => profile.available));
  const usableAgentEngines = $derived(availableAgentEngines.filter((engine) => engine.available));
  const selectedModelProfile = $derived.by(() => {
    const receipt = $activeChatThread?.model_profile_receipt;
    if (!receipt) return null;
    return availableModelProfiles.find((profile) =>
      profile.source_app_id === receipt.source_app_id && profile.profile_id === receipt.profile_id
    ) ?? null;
  });
  const selectedModelProfileIsCurrent = $derived(
    !!selectedModelProfile
      && selectedModelProfile.available
      && selectedModelProfile.profile_digest === $activeChatThread?.model_profile_receipt?.profile_digest,
  );
  const activeTools = $derived.by(() => {
    const receipt = $activeChatThread?.model_profile_receipt;
    if (!receipt || !selectedModelProfileIsCurrent) return availableTools;
    const allowed = new Set(receipt.tool_refs);
    return availableTools.filter((tool) => allowed.has(`${tool.provider_app_id}/${tool.capability}`));
  });

  const toolGroups = $derived.by(() => {
    const groups = new Map<
      string,
      {
        id: string;
        name: string;
        tools: { capability: string; description: string; grantCondition: GrantCondition }[];
      }
    >();
    for (const tool of activeTools) {
      const group = groups.get(tool.provider_app_id) ?? {
        id: tool.provider_app_id,
        name: tool.provider_display_name,
        tools: [] as {
          capability: string;
          description: string;
          grantCondition: GrantCondition;
        }[],
      };
      group.tools.push({
        capability: tool.capability,
        description: tool.description,
        grantCondition: mostInteractiveCondition(tool),
      });
      groups.set(tool.provider_app_id, group);
    }
    return Array.from(groups.values())
      .map((group) => ({
        ...group,
        tools: [...group.tools].sort((left, right) => left.capability.localeCompare(right.capability)),
      }))
      .sort((left, right) => left.name.localeCompare(right.name));
  });
  const waitingForApproval = $derived($pendingChromeRequests > 0);
  const chatInstalled = $derived($apps.some((app) => app.manifest.app_id === "chat"));
  const modelProviderConfigured = $derived(
    $hostConfig?.host.default_llm_profile != null || selectedModelProfileIsCurrent,
  );
  const showMetadata = $derived(appConfigEntry($hostConfig, "chat").settings.show_metadata === true);
  const showThinking = $derived(appConfigEntry($hostConfig, "chat").settings.show_thinking === true);
  // The header speaks about THIS conversation; runs in other threads are
  // marked in the thread list instead.
  const activeThreadWorking = $derived(
    $activeChatThread !== null && $sendingChatThreadIds.has($activeChatThread.id),
  );
  const isEmpty = $derived(!$activeChatThread || $activeChatThread.messages.length === 0);
  const streamingReply = $derived(
    $activeChatThread ? ($streamingChatReplies.get($activeChatThread.id) ?? null) : null,
  );
  const activeDraft = $derived(
    $activeChatThread
      ? ($chatDrafts.get($activeChatThread.id) ?? { text: "", contributions: [] })
      : { text: "", contributions: [] },
  );

  // Chat is only mounted while it is the active destination, so the component's
  // own lifetime is the "is the user in Chat" gate. Leaving the tab tears this
  // down, which finalizes every open observation session.
  $effect(() => {
    const root = log;
    if (!root) return;
    readingObserver = attachChatReadingObserver({
      root,
      tracker: readingTracker,
      isRequested: (messageId) => readingRequests.has(messageId),
      isActive: () => true,
    });
    // Messages that mounted before the log element existed still need the
    // observer to start watching them.
    for (const [messageId, region] of readingRegions) {
      readingObserver.register(messageId, region.element);
    }
    return () => {
      readingObserver?.destroy();
      readingObserver = null;
    };
  });

  // Switching threads ends observation of the responses that are going away;
  // their unflushed interval is reported before the elements unmount.
  $effect(() => {
    const threadId = $activeChatThread?.id ?? null;
    return () => {
      if (threadId !== null) readingTracker.flush(true);
    };
  });

  $effect(() => {
    const thread = $activeChatThread;
    const threadId = thread?.id ?? null;
    const messages = thread?.messages ?? [];
    const messageCount = messages.length;
    // Fold the in-flight streaming reply into the content version so this
    // effect re-runs (and keeps the view pinned to the bottom) as the
    // assistant's answer grows — not only when the finalized message lands.
    const streaming = threadId ? $streamingChatReplies.get(threadId) : null;
    const contentVersion =
      getChatContentVersion(messages) +
      (streaming ? `|streaming:${streaming.text.length}:${streaming.reasoning.length}` : "");
    const decision = deriveScrollUpdate({
      currentThreadId: threadId,
      lastThreadId,
      messageCount,
      lastMessageCount,
      contentVersion,
      lastContentVersion,
      userPinnedToBottom,
      hasUnreadBelow,
      forceScrollToBottom,
    });

    lastThreadId = threadId;
    lastMessageCount = messageCount;
    lastContentVersion = contentVersion;
    forceScrollToBottom = false;
    hasUnreadBelow = decision.hasUnreadBelow;

    if (!threadId) {
      userPinnedToBottom = true;
      return;
    }

    if (decision.shouldScrollToBottom) {
      void scrollToBottom();
    }
  });

  $effect(() => {
    renamingThreadId;
    renameInput?.focus();
    renameInput?.select();
  });

  onMount(() => {
    resizeComposer();
    if (activeDraft.contributions.length > 0) composer?.focus();
  });

  $effect(() => {
    activeDraft.text;
    activeDraft.contributions;
    resizeComposer();
  });

  $effect(() => {
    $grantsRevision;
    $hostConfig;
    const installed = $apps.some((app) => app.manifest.app_id === "chat");
    if ($hostInitialized && installed) {
      void loadChatConfiguration();
    } else {
      availableTools = [];
      availableProfiles = [];
      availableModelProfiles = [];
      availableAgentEngines = [];
      toolsError = null;
      chatChoicesError = null;
      chatChoicesRequestVersion += 1;
      toolsRequestVersion += 1;
    }
  });

  $effect(() => {
    $grantsRevision;
    $hostConfig;
    if (modelContextOpen) void loadPromptPreview();
  });

  async function loadChatConfiguration() {
    await loadChatChoices();
    await loadAvailableTools();
  }

  function loadAvailableTools(): Promise<void> {
    toolsRequestVersion += 1;
    if (toolsLoadInFlight) {
      toolsReloadRequested = true;
      return toolsLoadInFlight;
    }
    const load = loadAvailableToolsUntilCurrent().finally(() => {
      if (toolsLoadInFlight === load) toolsLoadInFlight = null;
    });
    toolsLoadInFlight = load;
    return load;
  }

  async function loadAvailableToolsUntilCurrent() {
    for (;;) {
      toolsReloadRequested = false;
      const requestVersion = toolsRequestVersion;
      try {
        const tools = await retryKernelBusy(() => availableCapabilitiesFor("chat"));
        if (requestVersion !== toolsRequestVersion) {
          if (!toolsReloadRequested) return;
          continue;
        }
        availableTools = tools;
        toolsError = null;
      } catch (error) {
        if (requestVersion !== toolsRequestVersion) {
          if (!toolsReloadRequested) return;
          continue;
        }
        console.error(error);
        toolsError = "Couldn't load the tool list. Try again.";
      }
      if (!toolsReloadRequested) return;
    }
  }

  async function loadPromptPreview() {
    const requestVersion = ++promptPreviewRequestVersion;
    promptPreviewStatus = "loading";
    promptPreviewError = null;
    promptPreview = null;
    try {
      const preview = await getChatPromptPreview(undefined, $activeChatThread?.id);
      if (requestVersion !== promptPreviewRequestVersion) return;
      promptPreview = preview;
      promptPreviewStatus = "ready";
    } catch (error) {
      if (requestVersion !== promptPreviewRequestVersion) return;
      promptPreview = null;
      promptPreviewStatus = "error";
      promptPreviewError = String(error);
    }
  }

  function loadChatChoices(): Promise<void> {
    chatChoicesRequestVersion += 1;
    if (chatChoicesLoadInFlight) {
      chatChoicesReloadRequested = true;
      return chatChoicesLoadInFlight;
    }
    const load = loadChatChoicesUntilCurrent().finally(() => {
      if (chatChoicesLoadInFlight === load) chatChoicesLoadInFlight = null;
    });
    chatChoicesLoadInFlight = load;
    return load;
  }

  async function loadChatChoicesUntilCurrent() {
    const api = await import("$lib/api");
    for (;;) {
      chatChoicesReloadRequested = false;
      const requestVersion = chatChoicesRequestVersion;
      const profiles = await settled(() => retryKernelBusy(api.listChatProfiles));
      if (requestVersion !== chatChoicesRequestVersion) {
        if (!chatChoicesReloadRequested) return;
        continue;
      }
      const modelProfiles = await settled(() => retryKernelBusy(api.listChatModelProfiles));
      if (requestVersion !== chatChoicesRequestVersion) {
        if (!chatChoicesReloadRequested) return;
        continue;
      }
      const engines = await settled(() => retryKernelBusy(api.listChatAgentEngines));
      if (requestVersion !== chatChoicesRequestVersion) {
        if (!chatChoicesReloadRequested) return;
        continue;
      }
      if (profiles.status === "fulfilled") availableProfiles = profiles.value;
      if (modelProfiles.status === "fulfilled") availableModelProfiles = modelProfiles.value;
      if (engines.status === "fulfilled") availableAgentEngines = engines.value;
      const failures = [profiles, modelProfiles, engines]
        .filter((result): result is PromiseRejectedResult => result.status === "rejected");
      for (const failure of failures) console.error(failure.reason);
      chatChoicesError = failures.length > 0 ? "Couldn't load all Chat choices. Try again." : null;
      if (!chatChoicesReloadRequested) return;
    }
  }

  async function settled<T>(operation: () => Promise<T>): Promise<PromiseSettledResult<T>> {
    try {
      return { status: "fulfilled", value: await operation() };
    } catch (reason) {
      return { status: "rejected", reason };
    }
  }

  async function retryKernelBusy<T>(operation: () => Promise<T>): Promise<T> {
    for (let attempt = 1; ; attempt += 1) {
      try {
        return await operation();
      } catch (error) {
        if (attempt >= 3 || !String(error).includes("kernel busy")) throw error;
        await new Promise((resolve) => setTimeout(resolve, 250));
      }
    }
  }

  function toggleTools() {
    toolsOpen = !toolsOpen;
    if (toolsOpen) void loadAvailableTools();
  }

  function toggleModelContext() {
    modelContextOpen = !modelContextOpen;
  }

  async function sendCurrentMessage() {
    const threadId = $activeChatThread?.id;
    // Gate on this specific thread being in flight, not a component-wide flag —
    // otherwise a slow reply in one thread would block sending in every other.
    if (!threadId || $sendingChatThreadIds.has(threadId) || !chatInstalled) return;
    const submittedDraft = activeDraft;
    if (submittedDraft.text.trim() === "" && submittedDraft.contributions.length === 0) return;
    forceScrollToBottom = true;
    sendError = null;
    // Clear before the await: the user may already be typing the next
    // message while the assistant works, and that text must survive.
    clearChatDraft(threadId);
    deleteConfirmThreadId = null;
    try {
      await sendMessageToActiveThread(submittedDraft.text);
    } catch (error) {
      const currentDraft = $chatDrafts.get(threadId);
      setChatDraft(threadId, {
        text: [submittedDraft.text, currentDraft?.text ?? ""].filter((text) => text !== "").join("\n\n"),
        contributions: [...submittedDraft.contributions, ...(currentDraft?.contributions ?? [])],
      });
      forceScrollToBottom = false;
      sendError = describeChatActionError("send", error);
      console.error(error);
    } finally {
      await refreshHost();
    }
  }

  async function send(event: SubmitEvent) {
    event.preventDefault();
    await sendCurrentMessage();
  }

  function isNearBottom(log: HTMLElement): boolean {
    return isNearBottomPosition({
      scrollTop: log.scrollTop,
      scrollHeight: log.scrollHeight,
      clientHeight: log.clientHeight,
    });
  }

  async function scrollToBottom({ smooth = false }: { smooth?: boolean } = {}) {
    const requestVersion = ++scrollRequestVersion;
    await tick();
    if (requestVersion !== scrollRequestVersion || !log) return;
    log.scrollTo({ top: log.scrollHeight, behavior: smooth ? "smooth" : "auto" });
    userPinnedToBottom = true;
    hasUnreadBelow = false;
  }

  function handleLogScroll() {
    if (!log) return;
    userPinnedToBottom = isNearBottom(log);
    if (userPinnedToBottom) {
      hasUnreadBelow = false;
    }
  }

  function handleComposerKeydown(event: KeyboardEvent) {
    if (event.key !== "Enter" || event.shiftKey || event.isComposing) return;
    event.preventDefault();
    void sendCurrentMessage();
  }

  const COMPOSER_MAX_HEIGHT_REM = 11; // must match `textarea { max-height: 11rem }` below

  function resizeComposer() {
    if (!composer) return;
    const rootFontSizePx = parseFloat(getComputedStyle(document.documentElement).fontSize);
    const maxHeightPx = COMPOSER_MAX_HEIGHT_REM * rootFontSizePx;
    composer.style.height = "0px";
    const nextHeight = Math.min(composer.scrollHeight, maxHeightPx);
    composer.style.height = `${nextHeight}px`;
    composer.style.overflowY = composer.scrollHeight > maxHeightPx ? "auto" : "hidden";
  }

  function updateDraftText(text: string) {
    const threadId = $activeChatThread?.id;
    if (threadId) setChatDraftText(threadId, text);
  }

  async function removeContribution(contribution: import("$lib/api").ChatContribution) {
    const threadId = $activeChatThread?.id;
    if (!threadId) return;
    try {
      await removeChatContributionFromThread(
        threadId,
        contribution.source_app_id,
        contribution.kind,
        contribution.item_id,
      );
      sendError = null;
    } catch (error) {
      sendError = `Couldn't remove the attached context. ${String(error)}`;
    }
  }

  function contributionStatus(contribution: import("$lib/api").ChatContribution): string {
    return `${contribution.lifecycle} · ${contribution.completeness}`;
  }

  function canUseContribution(contribution: import("$lib/api").ChatContribution): boolean {
    return contribution.kind === "text-snapshot" && contribution.lifecycle === "accepted" && contribution.completeness === "complete";
  }

  function useContributionText(contribution: import("$lib/api").ChatContribution) {
    if (!canUseContribution(contribution)) return;
    const text = typeof contribution.body === "object" && contribution.body && "text" in contribution.body
      ? String((contribution.body as Record<string, unknown>).text ?? "")
      : contribution.title;
    updateDraftText([activeDraft.text.trim(), text].filter(Boolean).join("\n\n"));
  }

  async function handleProfileChange(event: Event) {
    const target = event.currentTarget as HTMLSelectElement;
    const [appId, profileName] = target.value.split("/");
    const threadId = $activeChatThread?.id;
    if (!threadId || !appId || !profileName) return;
    try {
      await selectAssistantProfile(threadId, appId, profileName);
      sendError = null;
    } catch (error) {
      sendError = describeChatActionError("profile", error);
    }
  }

  async function handleAgentEngineChange(event: Event) {
    const select = event.currentTarget as HTMLSelectElement;
    const value = select.value;
    if (!$activeChatThread) return;
    try {
      await selectChatAgentEngine($activeChatThread.id, value === "plain-llm" ? null : value);
      sendError = null;
    } catch (error) {
      sendError = describeChatActionError("engine", error);
    }
  }

  async function handleModelProfileChange(event: Event) {
    const value = (event.currentTarget as HTMLSelectElement).value;
    if (!$activeChatThread) return;
    try {
      await selectModelProfile($activeChatThread.id, value === "chat-default" ? null : value);
      sendError = null;
      await loadAvailableTools();
    } catch (error) {
      sendError = describeChatActionError("model-profile", error);
    }
  }

  async function acceptUpdatedModelProfile() {
    if (!$activeChatThread || !selectedModelProfile) return;
    try {
      await selectModelProfile(
        $activeChatThread.id,
        `${selectedModelProfile.source_app_id}/${selectedModelProfile.profile_id}`,
      );
      sendError = null;
    } catch (error) {
      sendError = describeChatActionError("model-profile", error);
    }
  }

  async function handleNewChat() {
    threadActionError = null;
    try {
      await createNewChatThread();
    } catch (error) {
      threadActionError = describeChatActionError("create", error);
    }
  }

  async function handleSelectThread(threadId: string) {
    threadActionError = null;
    try {
      await selectChatThread(threadId);
      threadListOpen = false;
    } catch (error) {
      threadActionError = describeChatActionError("open", error);
    }
  }

  function beginRename(threadId: string, title: string) {
    deleteConfirmThreadId = null;
    renamingThreadId = threadId;
    renameDraft = title;
  }

  async function commitRename(threadId: string) {
    const title = renameDraft.trim();
    if (title === "") {
      renamingThreadId = null;
      return;
    }
    try {
      await renameExistingChatThread(threadId, title);
      // Success closes the editor; failure keeps it open so the typed
      // title survives for a retry.
      renamingThreadId = null;
      threadActionError = null;
    } catch (error) {
      threadActionError = describeChatActionError("rename", error);
    }
  }

  function cancelRename() {
    renamingThreadId = null;
  }

  function handleRenameKeydown(event: KeyboardEvent, threadId: string) {
    if (event.key === "Enter") {
      event.preventDefault();
      void commitRename(threadId);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      cancelRename();
    }
  }

  function requestDelete(threadId: string) {
    renamingThreadId = null;
    deleteConfirmThreadId = threadId;
  }

  function cancelDelete() {
    deleteConfirmThreadId = null;
  }

  async function handleDelete(threadId: string) {
    try {
      await deleteExistingChatThread(threadId);
      // Success closes the confirmation; failure keeps it open for a retry.
      deleteConfirmThreadId = null;
      threadActionError = null;
    } catch (error) {
      threadActionError = describeChatActionError("delete", error);
      return;
    }
    if (!$activeChatThread) {
      await handleNewChat();
    }
  }

  async function cancelActiveMessage() {
    try {
      await cancelMessageForActiveThread();
      sendError = null;
    } catch (error) {
      sendError = describeChatActionError("cancel", error);
    }
  }

  function threadMeta(thread: ChatThreadSummary): string {
    const countLabel = thread.message_count === 1 ? "1 message" : `${thread.message_count} messages`;
    return `${relativeTime(thread.updated_at)} · ${countLabel}`;
  }

  function grantConditionLabel(condition: GrantCondition): string {
    return condition === "silent"
      ? "Allowed"
      : condition === "notify"
        ? "Notify"
        : "Approval";
  }

  function assistantMessageNumber(messages: ChatMessageView[], index: number): number {
    return messages.slice(0, index + 1).filter((message) => message.role === "assistant").length;
  }

  function receiptFor(message: ChatMessageView) {
    const clientRequestId = message.client_request_id;
    if (!clientRequestId || !$activeChatThread?.prompt_receipts) return null;
    return $activeChatThread.prompt_receipts[clientRequestId] ?? null;
  }

  function promptReceiptLabel(messages: ChatMessageView[], index: number): string | null {
    const receipt = receiptFor(messages[index]);
    if (!receipt) return null;

    for (let previousIndex = index - 1; previousIndex >= 0; previousIndex -= 1) {
      const previousReceipt = receiptFor(messages[previousIndex]);
      if (!previousReceipt) continue;
      return previousReceipt.system_prompt_digest === receipt.system_prompt_digest
        ? null
        : "System prompt changed";
    }

    return "System prompt used";
  }

  function relativeTime(iso: string): string {
    const then = new Date(iso).getTime();
    const diff = Date.now() - then;
    const mins = Math.floor(diff / 60000);
    if (mins < 1) return "just now";
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    if (days < 7) return `${days}d ago`;
    return new Date(iso).toLocaleDateString();
  }
</script>

<section class="chat">
  <aside class="threads">
    <div class="threads-header">
      <button type="button" class="new-chat" onclick={() => void handleNewChat()}>
        <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
          <path d="M12 5v14M5 12h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none" />
        </svg>
        New chat
      </button>
    </div>
    <button
      type="button"
      class="mobile-list-toggle"
      aria-expanded={threadListOpen}
      aria-controls="chat-thread-list"
      onclick={() => (threadListOpen = !threadListOpen)}
    >
      All chats
    </button>
    {#if threadActionError}
      <div class="thread-error" role="alert">
        <span>{threadActionError}</span>
        <button type="button" class="mini-btn ghost" onclick={() => (threadActionError = null)}>Dismiss</button>
      </div>
    {/if}
    <div id="chat-thread-list" class="thread-list" class:mobile-open={threadListOpen}>
      {#each $chatThreads as thread (thread.id)}
        <div class="thread-item" class:active={$activeChatThread?.id === thread.id}>
          {#if renamingThreadId === thread.id}
            <div class="thread-rename">
              <input
                bind:this={renameInput}
                bind:value={renameDraft}
                onkeydown={(event) => handleRenameKeydown(event, thread.id)}
                aria-label="rename chat"
              />
              <div class="thread-rename-actions">
                <button type="button" class="mini-btn" onclick={() => void commitRename(thread.id)}>Save</button>
                <button type="button" class="mini-btn ghost" onclick={cancelRename}>Cancel</button>
              </div>
            </div>
          {:else}
            <button type="button" class="thread-button" onclick={() => void handleSelectThread(thread.id)}>
              <span class="thread-title">
                {thread.title}
                {#if $sendingChatThreadIds.has(thread.id)}
                  <span class="thread-working" aria-hidden="true" title="Working on a reply"></span>
                  <span class="sr-only">Working on a reply</span>
                {/if}
              </span>
              <span class="thread-time">{threadMeta(thread)}</span>
            </button>
            {#if deleteConfirmThreadId === thread.id}
              <div class="thread-confirm-delete">
                <span>Delete this chat?</span>
                <div class="thread-confirm-actions">
                  <button type="button" class="mini-btn danger" onclick={() => void handleDelete(thread.id)}>Delete</button>
                  <button type="button" class="mini-btn ghost" onclick={cancelDelete}>Cancel</button>
                </div>
              </div>
            {:else}
              <div class="thread-actions">
                <button
                  type="button"
                  class="icon-btn"
                  aria-label="rename chat"
                  onclick={() => beginRename(thread.id, thread.title)}
                >
                  <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
                    <path
                      d="M4 20h4L18.5 9.5a2.12 2.12 0 0 0-3-3L5 17v3z"
                      stroke="currentColor"
                      stroke-width="1.6"
                      fill="none"
                      stroke-linejoin="round"
                    />
                  </svg>
                </button>
                <button
                  type="button"
                  class="icon-btn danger"
                  aria-label="delete chat"
                  onclick={() => requestDelete(thread.id)}
                >
                  <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
                    <path
                      d="M5 7h14M10 7V5h4v2M6 7l1 13h10l1-13"
                      stroke="currentColor"
                      stroke-width="1.6"
                      fill="none"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    />
                  </svg>
                </button>
              </div>
            {/if}
          {/if}
        </div>
      {/each}
    </div>
  </aside>

  <div class="conversation">
    <header class="conversation-header">
      <h2>{$activeChatThread?.title ?? "New chat"}</h2>
      <div class="header-right">
        {#if activeThreadWorking || waitingForApproval}
          <span class="running {waitingForApproval ? 'waiting' : ''}">
            <span class="dots"><i></i><i></i><i></i></span>
            {waitingForApproval ? "Waiting for approval" : "Working"}
          </span>
        {/if}
        <button
          type="button"
          class="tools-toggle"
          aria-expanded={toolsOpen}
          aria-controls="chat-tools-inspector"
          onclick={toggleTools}
        >
          Tools
          <svg viewBox="0 0 24 24" width="13" height="13" aria-hidden="true" class:open={toolsOpen}>
            <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2" fill="none" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <button
          type="button"
          class="tools-toggle"
          aria-expanded={modelContextOpen}
          aria-controls="chat-model-context-inspector"
          onclick={toggleModelContext}
        >
          Model context
          <svg viewBox="0 0 24 24" width="13" height="13" aria-hidden="true" class:open={modelContextOpen}>
            <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2" fill="none" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
      </div>
    </header>

    {#if activeThreadWorking || waitingForApproval}
      <div class="activity-banner" class:waiting={waitingForApproval} role="status" aria-live="polite">
        <strong>{waitingForApproval ? "Waiting for approval" : "Working"}</strong>
        <span>
          {waitingForApproval
            ? "Respond to the approval prompt to continue."
            : "Working on your request…"}
        </span>
        <button type="button" class="cancel-run" onclick={() => void cancelActiveMessage()}>Cancel</button>
      </div>
    {/if}

    {#if $activeChatThread}
      {#key `${$activeChatThread.resource_id}:${$activeChatThread.revision}`}
        <!-- Extension contexts are init-only. Remount on thread or revision
             changes so inline apps never retain stale conversation context. -->
        <section class="chat-extensions" aria-label="Conversation app actions">
          <ChatExtensionSlot pointName="thread-actions" context={{ thread_id: $activeChatThread.id, resource_id: $activeChatThread.resource_id, revision: $activeChatThread.revision }} />
          <ChatExtensionSlot pointName="composer-context" context={{ thread_id: $activeChatThread.id, selection: "", request_id: `${$activeChatThread.id}:${$activeChatThread.revision}`.slice(0, 128) }} />
          <ChatExtensionSlot pointName="composer-actions" context={{ thread_id: $activeChatThread.id, draft_id: $activeChatThread.id, action: "review" }} />
        </section>
      {/key}
    {/if}

    {#if toolsOpen}
      <section id="chat-tools-inspector" class="tools-inspector">
        {#if toolsError}
          <p class="tools-empty" role="alert">
            {toolsError}
            <button type="button" class="mini-btn" onclick={() => void loadAvailableTools()}>
              Retry
            </button>
          </p>
        {:else if toolGroups.length === 0}
          <p class="tools-empty">No tools available right now.</p>
        {:else}
          {#each toolGroups as group}
            <div class="tool-group">
              <span class="tool-group-label">{group.name}</span>
              <div class="tool-list">
                {#each group.tools as tool}
                  <span
                    class="tool-chip"
                    class:approval={tool.grantCondition === "requires-approval"}
                    class:notify={tool.grantCondition === "notify"}
                  >
                    {tool.capability}
                    <small>{grantConditionLabel(tool.grantCondition)}</small>
                    <small>{tool.description}</small>
                    <code>{group.id}/{tool.capability}</code>
                  </span>
                {/each}
              </div>
            </div>
          {/each}
        {/if}
      </section>
    {/if}

    {#if modelContextOpen}
      <section
        id="chat-model-context-inspector"
        class="tools-inspector"
        aria-labelledby="chat-model-context-title"
      >
        <h3 id="chat-model-context-title">Model context</h3>
        <p class="tools-empty">Stored app context is revalidated against its original Run and grant before every send. Tools remain independently grant-controlled.</p>
        <p class="tools-empty">The selected provider or delegated agent may apply behavior outside the payload Kestral sends.</p>
        {#if promptPreviewStatus === "loading"}<p role="status">Refreshing prompt context…</p>{/if}
        {#if promptPreviewStatus === "error"}<p class="send-error" role="alert">{promptPreviewError}</p>{/if}
          {#if promptPreview}
          <section class="tool-group">
            <h4 class="tool-group-label">Current mode</h4>
            <p>{promptPreview.runtime.mode} · {promptPreview.runtime.connector_kind} · {promptPreview.runtime.model_id}</p>
            <p>{promptPreview.runtime.host_version}</p>
          </section>
          <section class="tool-group">
            <h4 class="tool-group-label">Current authoritative prompt layers</h4>
            {#each promptPreview.layers as layer}
              <div class="tool-chip inline-layer">
                <strong>{layer.title}</strong>
                <small>{layer.kind}</small>
                <small>{layer.included ? "included" : "excluded"}</small>
                <span>{layer.source ?? "No source"}</span>
                <pre>{layer.content}</pre>
              </div>
            {/each}
          </section>
          <section class="tool-group">
            <h4 class="tool-group-label">Stored app context</h4>
            {#each $activeChatThread?.injected_contexts ?? [] as context}
              <div class="tool-chip inline-layer">
                <strong>{context.source_app_id} · {context.source_app_version}</strong>
                <small>{context.item_id} · revision {context.revision}</small>
                <small>Source Run {context.source_run_id}</small>
                <pre>{context.content}</pre>
              </div>
            {:else}
              <p class="tools-empty">No app has stored model context for this thread.</p>
            {/each}
          </section>
          <section class="tool-group">
            <h4 class="tool-group-label">Tools</h4>
            {#each toolGroups as group}
              <div class="tool-chip inline-layer">
                <strong>{group.name}</strong>
                {#each group.tools as tool}
                  <span>{tool.capability}: {tool.description} ({grantConditionLabel(tool.grantCondition)})</span>
                  <code>{group.id}/{tool.capability}</code>
                {/each}
              </div>
            {:else}
              <p class="tools-empty">No tools are currently available.</p>
            {/each}
          </section>
        {/if}
      </section>
    {/if}

    <div class="log-shell">
      <div class="log" bind:this={log} onscroll={handleLogScroll}>
        {#if isEmpty}
          <section class="empty-state">
            <div class="greeting">
              <div class="greeting-avatar" aria-hidden="true">
                <KestralMark size="1.375rem" />
              </div>
              <h1>Ask anything</h1>
              <p>…or start with one of these ideas.</p>
            </div>
            <div class="suggestions">
              <button type="button" class="suggestion" onclick={() => updateDraftText("Draft a friendly message to reschedule lunch.")}>
                <span class="suggestion-title">Draft a message</span>
                <span class="suggestion-sub">Find the right words</span>
              </button>
              <button type="button" class="suggestion" onclick={() => updateDraftText("Explain why the sky changes color at sunset.")}>
                <span class="suggestion-title">Explain a topic</span>
                <span class="suggestion-sub">Make something clearer</span>
              </button>
              <button type="button" class="suggestion" onclick={() => updateDraftText("Brainstorm five simple weekend project ideas.")}>
                <span class="suggestion-title">Brainstorm ideas</span>
                <span class="suggestion-sub">Explore a few directions</span>
              </button>
            </div>
          </section>
        {:else}
          <div class="messages">
            {#each ($activeChatThread?.messages ?? []) as message, messageIndex (message.id)}
              {#if showMetadata || message.role !== "tool-status"}
                <div class="message-stack">
                  <ChatMessage
                    message={message}
                    threadId={$activeChatThread?.id ?? ""}
                    threadResourceId={$activeChatThread?.resource_id ?? ""}
                    assistantMessageNumber={assistantMessageNumber($activeChatThread?.messages ?? [], messageIndex)}
                    {showMetadata}
                    {showThinking}
                    {readingObservation}
                  />
                  {#if message.role === "user"}
                    {@const receipt = receiptFor(message)}
                    {@const receiptLabel = promptReceiptLabel($activeChatThread?.messages ?? [], messageIndex)}
                    {#if receipt && (receiptLabel || receipt.injected_context)}
                      <details class="prompt-receipt">
                        <summary>
                          {receiptLabel ?? "App context used"}
                          {#if receipt.injected_context}
                            · {receipt.injected_context.exact_message === null ? "app metadata only" : "exact app text recorded"}
                          {/if}
                        </summary>
                        <p class="receipt-meta">{receipt.created_at} · {receipt.system_prompt_digest}</p>
                        {#if receiptLabel}
                          <pre>{receipt.system_prompt}</pre>
                          {#each receipt.layers as layer}
                            <section class="receipt-layer">
                              <strong>{layer.title}</strong>
                              <span>{layer.kind}</span>
                              <span>{layer.source ?? "No source"}</span>
                              <pre>{layer.content}</pre>
                            </section>
                          {/each}
                        {/if}
                        {#if receipt.injected_context}
                          <section class="receipt-layer injected-context-receipt">
                            <strong>Grant-authorized app context</strong>
                            <span>Message digest {receipt.injected_context.message_digest}</span>
                            {#each receipt.injected_context.entries as entry}
                              <div class="injected-context-source">
                                <span>{entry.source_app_name} {entry.source_app_version} · {entry.source_app_id}</span>
                                <span>{entry.item_id} · revision {entry.revision}</span>
                                <span>Run {entry.source_run_id} · grant {entry.grant_id}</span>
                                <span>Content digest {entry.content_digest}</span>
                              </div>
                            {/each}
                            {#if receipt.injected_context.exact_message !== null}
                              <pre>{receipt.injected_context.exact_message}</pre>
                            {:else}
                              <p class="tools-empty">Exact text was not recorded for this request.</p>
                            {/if}
                          </section>
                        {/if}
                      </details>
                    {/if}
                  {/if}
                </div>
              {/if}
            {/each}
            {#if activeThreadWorking}
              <ChatMessage
                message={{
                  id: "active-assistant-work",
                  role: "assistant",
                  text: streamingReply?.text ?? "",
                  reasoning: streamingReply?.reasoning || null,
                  run_id: null,
                  artifact_ids: [],
                  status: "pending",
                  // The in-flight reply is not a persisted message yet, so it
                  // carries no host timestamp. Extensions are gated on one, and
                  // inventing a time here would be the only thing that let this
                  // placeholder look like a completed response.
                  created_at: "",
                  completed_at: null,
                }}
                threadId={$activeChatThread?.id ?? ""}
                threadResourceId={$activeChatThread?.resource_id ?? ""}
                {showMetadata}
                {showThinking}
              />
            {/if}
          </div>
        {/if}
      </div>

      {#if hasUnreadBelow}
        <button
          type="button"
          class="jump-to-bottom"
          aria-label="Jump to bottom"
          onclick={() => void scrollToBottom({ smooth: true })}
        >
          New messages
        </button>
      {/if}
    </div>

    <form class="composer" onsubmit={send}>
      {#if $hostConfig && !modelProviderConfigured}
        <div class="provider-setup" role="status">
          <span>No model provider is configured. Chat will return setup guidance until you choose one.</span>
          <button
            type="button"
            onclick={() => openAppSettings("llm-provider", "LLM Provider")}
          >
            Configure model provider
          </button>
        </div>
      {/if}
      {#if sendError}
        <p class="send-error" role="alert">{sendError}</p>
      {/if}
      {#if activeDraft.contributions.length > 0}
        <div class="draft-contexts" aria-label="Attached contributions">
          {#each activeDraft.contributions as contribution (`${contribution.source_app_id}/${contribution.kind}/${contribution.item_id}`)}
            <details class="draft-context-chip">
              <summary>
                <strong>{contribution.title}</strong>
                <span>{contribution.kind}</span>
                <span>{contribution.source_app_id}/{contribution.item_id}</span>
                <span>{contributionStatus(contribution)}</span>
              </summary>
              <pre>{JSON.stringify(contribution.body, null, 2)}</pre>
              <div class="thread-actions-inline">
                <button type="button" class="mini-btn ghost" onclick={() => void removeContribution(contribution)}>Remove</button>
                {#if canUseContribution(contribution)}
                  <button type="button" class="mini-btn ghost" onclick={() => useContributionText(contribution)}>Use selected snapshot</button>
                {/if}
              </div>
            </details>
          {/each}
        </div>
      {/if}
      {#if chatChoicesError}
        <p class="send-error" role="alert">
          {chatChoicesError}
          <button type="button" class="mini-btn" onclick={() => void loadChatChoices()}>Retry</button>
        </p>
      {/if}
      {#if usableProfiles.length > 1}
        <label class="profile-select">
          <span>Assistant profile</span>
          <select value={selectedAssistantProfile} onchange={handleProfileChange} aria-describedby="profile-help">
            {#each usableProfiles as profile}
              <option value={`${profile.app_id}/${profile.profile_name}`}>
                {profile.app_display_name} / {profile.title}
              </option>
            {/each}
          </select>
        </label>
        <div id="profile-help" class="profile-help">
          {#each usableProfiles as profile}
            <p>
              <strong>{profile.app_display_name} / {profile.title}</strong>
              <span>{profile.description}</span>
            </p>
          {/each}
          {#each availableProfiles.filter((profile) => profile.availability !== "available") as profile}
            <p class="profile-unavailable">
              <strong>{profile.app_display_name} / {profile.title}</strong>
              <span>{profile.availability_reason ?? "Unavailable"}</span>
              <span>{profile.suggested_capability_refs.length} suggested capabilities; {profile.suggested_agent_engine_contract ?? "no engine"}</span>
            </p>
          {/each}
        </div>
      {/if}
      {#if availableModelProfiles.length > 0 || $activeChatThread?.model_profile_receipt}
        <section class="model-profile-choice" aria-labelledby="model-profile-label">
          <label class="profile-select">
            <span id="model-profile-label">Model profile</span>
            <select
              value={$activeChatThread?.model_profile_receipt
                ? `${$activeChatThread.model_profile_receipt.source_app_id}/${$activeChatThread.model_profile_receipt.profile_id}`
                : "chat-default"}
              onchange={handleModelProfileChange}
              aria-describedby="model-profile-help"
            >
              <option value="chat-default">Chat default</option>
              {#each usableModelProfiles as profile}
                <option value={`${profile.source_app_id}/${profile.profile_id}`}>
                  {profile.source_app_name} · {profile.title} · {profile.model}
                </option>
              {/each}
            </select>
          </label>
          <div id="model-profile-help" class="model-profile-help">
            {#if $activeChatThread?.model_profile_receipt && !selectedModelProfileIsCurrent}
              <p role="status">
                This profile changed or is unavailable. Chat will use its default until you review it.
                {#if selectedModelProfile?.available}
                  <button type="button" class="mini-btn" onclick={() => void acceptUpdatedModelProfile()}>
                    Use updated profile
                  </button>
                {/if}
              </p>
            {:else if selectedModelProfile}
              <p>
                <strong>{selectedModelProfile.model}</strong>
                via {selectedModelProfile.connector_id}. {selectedModelProfile.effective_tool_refs.length}
                of {selectedModelProfile.tool_refs.length} profile tools available.
              </p>
              {#if selectedModelProfile.unavailable_tool_refs.length > 0}
                <p class="profile-warning" role="status">
                  Not granted to Chat and excluded: {selectedModelProfile.unavailable_tool_refs.join(", ")}.
                </p>
              {/if}
            {:else}
              <p>Uses the default provider and every tool currently granted to Chat.</p>
            {/if}
          </div>
        </section>
      {/if}
      {#if usableAgentEngines.length > 0 || $activeChatThread?.chat_agent_engine_ref}
        <label class="profile-select">
          <span>Agent engine</span>
          <select value={$activeChatThread?.chat_agent_engine_ref ?? "plain-llm"} onchange={handleAgentEngineChange}>
            <option value="plain-llm">Plain LLM</option>
            {#each usableAgentEngines as engine}
              <option value={engine.app_id}>{engine.display_name} / {engine.version}</option>
            {/each}
          </select>
        </label>
        {#if $activeChatThread?.chat_agent_engine_state?.status}
          <p class="profile-help" role="status">
            <strong>{$activeChatThread.chat_agent_engine_state.status === "fallback" ? "Using Plain LLM." : $activeChatThread.chat_agent_engine_state.status}</strong>
            {#if $activeChatThread.chat_agent_engine_state.fallback_reason}
              {$activeChatThread.chat_agent_engine_state.fallback_reason}
            {/if}
          </p>
        {/if}
      {/if}
      <div class="composer-box">
        <textarea
          bind:this={composer}
          value={activeDraft.text}
          placeholder="Message Chat…"
          aria-label="chat message"
          onkeydown={handleComposerKeydown}
          oninput={(event) => {
            updateDraftText(event.currentTarget.value);
            resizeComposer();
          }}
          rows="1"
        ></textarea>
        <button
          type="submit"
          class="send-btn"
          disabled={activeThreadWorking || !chatInstalled || !$activeChatThread || (activeDraft.text.trim() === "" && activeDraft.contributions.length === 0) || activeDraft.contributions.some((contribution) => contribution.completeness === "unavailable" || (contribution.kind === "resource-ref" && contribution.lifecycle !== "accepted"))}
          aria-label="send message"
        >
          <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
            <path d="M12 19V5M5 12l7-7 7 7" stroke="currentColor" stroke-width="2.2" fill="none" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
      </div>
      {#if activeDraft.contributions.some((contribution) => contribution.completeness === "unavailable" || (contribution.kind === "resource-ref" && contribution.lifecycle !== "accepted"))}
        <p class="send-error" role="alert">Resolve unavailable resource references before sending.</p>
      {/if}
      <p class="composer-hint">Chat may make mistakes. Verify important information.</p>
    </form>
  </div>
</section>

<style>
  .chat {
    flex: 1;
    min-height: 0;
    margin: -1.2rem -1.35rem -1.35rem;
    display: grid;
    grid-template-columns: minmax(15rem, 19rem) minmax(0, 1fr);
    background: var(--color-surface-raised);
    overflow: hidden;
  }

  .threads {
    min-height: 0;
    border-right: 1px solid var(--color-border-subtle);
    background: var(--color-surface-muted);
    display: flex;
    flex-direction: column;
  }
  .threads-header {
    padding: 0.85rem 0.75rem 0.5rem;
  }
  .new-chat {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 0.55rem;
    justify-content: center;
    border: 1px solid var(--color-border-subtle);
    border-radius: 12px;
    background: var(--color-surface-raised);
    color: var(--color-text);
    padding: 0.7rem 0.9rem;
    font-size: 0.92rem;
    font-weight: 600;
    cursor: pointer;
    transition: background 120ms ease, border-color 120ms ease;
  }
  .new-chat:hover {
    background: var(--color-surface-hover);
    border-color: var(--color-border-hover);
  }
  .mobile-list-toggle {
    display: none;
  }
  .thread-error {
    margin: 0 0.5rem 0.35rem;
    padding: 0.5rem 0.6rem;
    border: 1px solid var(--color-warning-border);
    border-radius: 10px;
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
    font-size: 0.78rem;
    display: grid;
    gap: 0.35rem;
    justify-items: start;
  }
  .thread-list {
    min-height: 0;
    overflow-y: auto;
    padding: 0.5rem 0.5rem 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  .thread-item {
    position: relative;
    border-radius: 10px;
    transition: background 120ms ease;
  }
  .thread-item:hover {
    background: var(--color-surface-hover);
  }
  .thread-item.active {
    background: var(--color-surface-hover);
  }
  .thread-button {
    width: 100%;
    border: none;
    background: transparent;
    text-align: left;
    padding: 0.6rem 0.7rem;
    cursor: pointer;
    display: grid;
    gap: 0.15rem;
    padding-right: 3.4rem;
  }
  .thread-title {
    color: var(--color-text);
    font-size: 0.88rem;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .thread-working {
    display: inline-block;
    width: 0.45rem;
    height: 0.45rem;
    margin-left: 0.35rem;
    border-radius: 50%;
    background: var(--color-accent);
    vertical-align: middle;
    animation: thread-pulse 1.1s infinite ease-in-out;
  }
  @keyframes thread-pulse {
    0%,
    100% {
      opacity: 0.35;
    }
    50% {
      opacity: 1;
    }
  }
  /* Visually-hidden but screen-reader-readable text: the pulsing dot itself
     is aria-hidden and its `title` alone is not reliably announced. */
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .thread-time {
    color: var(--color-text-faint);
    font-size: 0.72rem;
  }
  .thread-actions {
    position: absolute;
    top: 50%;
    right: 0.4rem;
    transform: translateY(-50%);
    display: flex;
    gap: 0.1rem;
    opacity: 0;
    transition: opacity 120ms ease;
  }
  .thread-item:hover .thread-actions,
  .thread-item:focus-within .thread-actions,
  .thread-item.active .thread-actions {
    opacity: 1;
  }
  @media (hover: none), (pointer: coarse) {
    .thread-actions {
      opacity: 1;
    }
  }
  .thread-item input {
    width: 100%;
    border: 1px solid var(--color-border-hover);
    border-radius: 8px;
    padding: 0.5rem 0.6rem;
    font-size: 0.88rem;
    background: var(--color-surface-raised);
  }
  .thread-rename {
    display: grid;
    gap: 0.45rem;
    padding: 0.45rem;
  }
  .thread-rename-actions,
  .thread-confirm-actions {
    display: flex;
    gap: 0.35rem;
  }
  .mini-btn {
    border: 1px solid var(--color-border-hover);
    border-radius: 999px;
    background: var(--color-surface-raised);
    color: var(--color-text);
    padding: 0.25rem 0.6rem;
    /* The 0.72rem type would otherwise leave this box under the WCAG 2.5.8
       24x24 floor; these are the rename/delete confirm controls, where a
       mis-tap is a destructive action. */
    min-height: 2rem;
    font-size: 0.72rem;
    font-weight: 600;
    cursor: pointer;
  }
  .mini-btn.ghost {
    color: var(--color-text-soft);
  }
  .mini-btn.danger {
    border-color: var(--color-warning-border);
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .thread-confirm-delete {
    display: grid;
    gap: 0.4rem;
    padding: 0 0.7rem 0.6rem;
    color: var(--color-text-soft);
    font-size: 0.75rem;
  }
  .icon-btn {
    border: none;
    background: transparent;
    color: var(--color-text-soft);
    width: 1.7rem;
    height: 1.7rem;
    border-radius: 7px;
    display: grid;
    place-items: center;
    cursor: pointer;
  }
  .icon-btn:hover {
    background: var(--color-surface-hover);
    color: var(--color-text);
  }
  .icon-btn.danger:hover {
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }

  .conversation {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--color-surface-raised);
  }
  .conversation-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.85rem 1.25rem;
    border-bottom: 1px solid var(--color-border-subtle);
    min-height: 3.4rem;
    box-sizing: border-box;
  }
  .conversation-header h2 {
    flex: 1 1 10rem;
    min-width: 0;
    margin: 0;
    font-size: 0.98rem;
    font-weight: 600;
    color: var(--color-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .header-right {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex: 0 1 auto;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .running {
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    border-radius: 999px;
    padding: 0.3rem 0.7rem;
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
    font-size: 0.78rem;
    font-weight: 500;
    white-space: nowrap;
  }
  .running.waiting {
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .dots {
    display: inline-flex;
    gap: 0.18rem;
  }
  .dots i {
    width: 0.32rem;
    height: 0.32rem;
    border-radius: 50%;
    background: currentColor;
    display: inline-block;
    animation: blink 1.2s infinite ease-in-out;
  }
  .dots i:nth-child(2) {
    animation-delay: 0.2s;
  }
  .dots i:nth-child(3) {
    animation-delay: 0.4s;
  }
  @keyframes blink {
    0%, 60%, 100% { opacity: 0.25; }
    30% { opacity: 1; }
  }
  .tools-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    border: 1px solid var(--color-border-subtle);
    border-radius: 999px;
    background: var(--color-surface-raised);
    color: var(--color-text-soft);
    padding: 0.35rem 0.7rem;
    font-size: 0.8rem;
    cursor: pointer;
  }
  .tools-toggle:hover {
    background: var(--color-surface-muted);
  }
  .tools-toggle svg {
    transition: transform 160ms ease;
  }
  .tools-toggle svg.open {
    transform: rotate(180deg);
  }

  .tools-inspector {
    min-height: 0;
    max-height: min(50vh, 30rem);
    max-height: min(50dvh, 30rem);
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: 0.65rem 1.25rem;
    border-bottom: 1px solid var(--color-border-subtle);
    background: var(--color-surface-muted);
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .activity-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.8rem;
    padding: 0.6rem 1.25rem;
    border-bottom: 1px solid var(--color-border-subtle);
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
    font-size: 0.82rem;
  }
  .activity-banner.waiting {
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .activity-banner strong {
    font-size: 0.8rem;
  }
  .cancel-run {
    border: 1px solid currentColor;
    border-radius: 999px;
    background: transparent;
    color: inherit;
    min-height: 1.75rem;
    padding: 0.25rem 0.65rem;
  }
  .tool-group {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex-wrap: wrap;
  }
  .tool-group-label {
    color: var(--color-text-faint);
    font-size: 0.78rem;
    font-weight: 600;
    margin: 0;
  }
  .tool-list {
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
  }
  .tool-chip {
    border-radius: 999px;
    background: var(--color-success-soft);
    color: var(--color-success-text);
    padding: 0.22rem 0.65rem;
    font-size: 0.76rem;
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }
  .tool-chip.notify {
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
  }
  .tool-chip.approval {
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .tool-chip small {
    font-size: 0.68rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.02em;
  }
  .tool-chip code {
    max-width: 100%;
    color: currentColor;
    font-size: 0.68rem;
    overflow-wrap: anywhere;
    white-space: normal;
  }
  .tool-chip.inline-layer {
    width: 100%;
    min-width: 0;
    border-radius: 0.75rem;
    display: grid;
    align-items: start;
    white-space: normal;
  }
  .inline-layer pre,
  .prompt-receipt pre,
  .receipt-layer pre {
    max-width: 100%;
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .tools-empty {
    margin: 0;
    color: var(--color-text-faint);
    font-size: 0.82rem;
  }
  .log-shell {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  .log {
    height: 100%;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }
  .message-stack {
    min-width: 0;
  }
  .prompt-receipt {
    width: min(100%, 48rem);
    min-width: 0;
    margin: -0.45rem auto 0.7rem;
    padding-inline: 1rem;
    color: var(--color-text-muted);
    font-size: 0.78rem;
  }
  .prompt-receipt summary {
    width: fit-content;
    min-height: 1.75rem;
    cursor: pointer;
    color: var(--color-text-soft);
  }
  .receipt-meta {
    overflow-wrap: anywhere;
  }
  .receipt-layer {
    min-width: 0;
    margin-top: 0.65rem;
    padding: 0.65rem;
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.65rem;
    display: grid;
    gap: 0.35rem;
  }
  .injected-context-source {
    min-width: 0;
    display: grid;
    gap: 0.2rem;
    padding-block: 0.35rem;
    border-block-start: 1px solid var(--color-border-subtle);
    overflow-wrap: anywhere;
  }
  .injected-context-receipt pre {
    max-height: min(40dvh, 24rem);
    overflow: auto;
  }
  .jump-to-bottom {
    position: absolute;
    left: 50%;
    bottom: 1rem;
    transform: translateX(-50%);
    border: 1px solid var(--color-border-hover);
    border-radius: 999px;
    background: var(--color-surface-raised);
    color: var(--color-text);
    padding: 0.45rem 0.8rem;
    font-size: 0.78rem;
    font-weight: 600;
    box-shadow: 0 8px 20px var(--color-shadow-strong);
    cursor: pointer;
  }
  .jump-to-bottom:hover {
    background: var(--color-surface-raised);
    border-color: var(--color-border-hover);
  }
  .empty-state {
    margin: auto;
    width: 100%;
    max-width: 46rem;
    padding: 2rem 1.25rem;
    display: grid;
    gap: 2rem;
    justify-items: center;
    text-align: center;
  }
  .greeting {
    display: grid;
    gap: 0.6rem;
    justify-items: center;
  }
  .greeting-avatar {
    width: 3rem;
    height: 3rem;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: linear-gradient(135deg, var(--color-accent), var(--color-accent-strong));
    color: var(--color-accent-contrast);
  }
  .greeting h1 {
    margin: 0;
    font-size: clamp(1.35rem, 1.1rem + 1.2vw, 1.7rem);
    font-weight: 600;
    color: var(--color-text);
    letter-spacing: -0.01em;
  }
  .greeting p {
    margin: 0;
    color: var(--color-text-soft);
    font-size: 0.95rem;
  }
  .suggestions {
    display: grid;
    /* Intrinsic: as many columns as fit, down to one, no breakpoint. */
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 12rem), 1fr));
    gap: 0.75rem;
    width: 100%;
  }
  .suggestion {
    border: 1px solid var(--color-border-subtle);
    border-radius: 14px;
    background: var(--color-surface-raised);
    padding: 0.9rem 1rem;
    text-align: left;
    cursor: pointer;
    display: grid;
    gap: 0.25rem;
    transition: background 120ms ease, border-color 120ms ease;
  }
  .suggestion:hover {
    background: var(--color-surface-muted);
    border-color: var(--color-border-hover);
  }
  .suggestion-title {
    color: var(--color-text);
    font-weight: 600;
    font-size: 0.9rem;
  }
  .suggestion-sub {
    color: var(--color-text-faint);
    font-size: 0.78rem;
  }

  .messages {
    width: 100%;
    max-width: 46rem;
    margin: 0 auto;
    padding: 1.5rem 1.25rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 1.4rem;
  }

  .composer {
    border-top: 1px solid var(--color-border-subtle);
    padding: 0.85rem 1.25rem 1rem;
    background: var(--color-surface-raised);
    display: grid;
    justify-items: center;
    gap: 0.4rem;
  }
  .composer-box {
    width: 100%;
    max-width: 46rem;
    display: flex;
    align-items: flex-end;
    gap: 0.5rem;
    border: 1px solid var(--color-border-hover);
    border-radius: 26px;
    padding: 0.5rem 0.5rem 0.5rem 1.1rem;
    background: var(--color-surface-raised);
    transition: border-color 120ms ease, box-shadow 120ms ease;
  }
  .provider-setup {
    width: 100%;
    max-width: 46rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 0.55rem 0.85rem;
    padding: 0.6rem 0.75rem;
    border: 1px solid var(--color-accent-border);
    border-radius: 12px;
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
    font-size: 0.82rem;
  }
  .provider-setup button {
    min-height: 2rem;
    border: 1px solid var(--color-accent);
    border-radius: 999px;
    padding: 0.3em 0.8em;
    background: var(--color-accent);
    color: var(--color-accent-contrast);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }
  .draft-contexts {
    width: 100%;
    max-width: 46rem;
    display: flex;
    flex-wrap: nowrap;
    gap: 0.45rem;
    overflow-x: auto;
    scrollbar-width: thin;
  }
  .draft-context-chip {
    flex: 0 0 auto;
    min-width: 0;
    max-width: min(100%, 22rem);
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    border: 1px solid var(--color-accent-border);
    border-radius: 12px;
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
    padding: 0.35rem 0.4rem 0.35rem 0.65rem;
    font-size: 0.75rem;
  }
  .profile-select {
    display: grid;
    gap: 0.25rem;
    width: min(100%, 18rem);
  }
  .profile-select span {
    color: var(--color-text-faint);
    font-size: 0.72rem;
    font-weight: 600;
  }
  .profile-select select {
    width: 100%;
    min-width: 0;
    border: 1px solid var(--color-border-hover);
    border-radius: 12px;
    background: var(--color-surface-raised);
    color: var(--color-text);
    padding: 0.45rem 0.65rem;
  }
  .model-profile-choice {
    width: min(100%, 46rem);
    display: flex;
    align-items: end;
    justify-content: center;
    gap: 0.75rem;
    flex-wrap: wrap;
  }
  .model-profile-help {
    flex: 1 1 18rem;
    max-width: 32rem;
    color: var(--color-text-muted);
    font-size: 0.78rem;
    line-height: 1.45;
  }
  .model-profile-help p {
    margin: 0;
  }
  .model-profile-help .profile-warning {
    color: var(--color-warning-text);
  }
  .draft-context-chip strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text);
    font-size: 0.8rem;
  }
  .composer-box:focus-within {
    border-color: var(--color-accent);
    box-shadow: 0 2px 10px var(--color-shadow-soft);
  }
  textarea {
    flex: 1;
    border: none;
    outline: none;
    resize: none;
    background: transparent;
    font: inherit;
    font-size: 0.96rem;
    line-height: 1.5;
    color: var(--color-text);
    max-height: 11rem;
    padding: 0.45rem 0;
  }
  textarea::placeholder {
    color: var(--color-text-faint);
  }
  .send-btn {
    flex-shrink: 0;
    width: 2.5rem;
    height: 2.5rem;
    border: none;
    border-radius: 50%;
    background: var(--color-text);
    color: var(--color-accent-contrast);
    display: grid;
    place-items: center;
    cursor: pointer;
    transition: background 120ms ease, opacity 120ms ease;
  }
  .send-btn:hover:not(:disabled) {
    background: var(--color-accent-strong);
  }
  .send-btn:disabled {
    background: var(--color-border-hover);
    color: var(--color-accent-contrast);
    cursor: default;
  }
  .composer-hint {
    margin: 0;
    color: var(--color-text-faint);
    font-size: 0.72rem;
  }
  .send-error {
    width: 100%;
    max-width: 46rem;
    margin: 0;
    padding: 0.55rem 0.8rem;
    border: 1px solid var(--color-warning-border);
    border-radius: 12px;
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
    font-size: 0.82rem;
  }

  @media (max-width: 69em) {
    .chat {
      grid-template-columns: 15rem minmax(0, 1fr);
    }
  }
  /* Shrink the thread list before the conversation, so narrow windows do
     not scroll horizontally earlier than necessary. */
  @media (max-width: 56em) {
    .chat {
      grid-template-columns: minmax(10rem, 12rem) minmax(0, 1fr);
    }
  }
  /* Phone-width: stack. The thread list becomes a horizontal strip above
     the conversation so the conversation keeps the full width. */
  @media (max-width: 40em) {
    .chat {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: auto minmax(0, 1fr);
    }
    .threads {
      flex-direction: row;
      align-items: center;
      flex-wrap: wrap;
      border-right: none;
      border-bottom: 1px solid var(--color-border-subtle);
    }
    .thread-error {
      order: 1;
      flex-basis: 100%;
      margin: 0 0.6rem 0.4rem;
    }
    .threads-header {
      flex-shrink: 0;
      padding: 0.5rem 0.4rem 0.5rem 0.6rem;
    }
    .thread-list {
      flex-direction: row;
      align-items: center;
      overflow-x: auto;
      overflow-y: hidden;
      padding: 0.5rem 0.6rem 0.5rem 0.2rem;
      gap: 0.3rem;
    }
    .thread-item {
      flex: 0 0 auto;
      max-width: 13rem;
    }
    .conversation-header,
    .composer,
    .tools-inspector,
    .activity-banner {
      padding-inline: 0.85rem;
    }
  }
  @media (max-width: 30em) {
    .threads {
      position: relative;
      flex-wrap: wrap;
      gap: 0.35rem;
      padding: 0.45rem 0.6rem;
    }
    .threads-header {
      padding: 0;
    }
    .new-chat,
    .mobile-list-toggle {
      min-height: 2.75rem;
      border: 1px solid var(--color-border-subtle);
      border-radius: 10px;
      background: var(--color-surface-raised);
      color: var(--color-text);
      padding: 0.55rem 0.7rem;
      font: inherit;
      font-size: 0.84rem;
      font-weight: 600;
    }
    .mobile-list-toggle {
      display: block;
    }
    .thread-list {
      display: none;
      flex: 1 0 100%;
      max-height: 12rem;
      overflow-y: auto;
      padding: 0.25rem 0;
    }
    .thread-list.mobile-open {
      display: flex;
      flex-direction: column;
      align-items: stretch;
    }
    .thread-item {
      width: 100%;
      max-width: none;
    }
    .conversation-header,
    .composer,
    .tools-inspector,
    .activity-banner,
    .messages {
      padding-inline: 0.65rem;
    }
    .conversation-header {
      gap: 0.5rem;
    }
    .header-right {
      width: 100%;
      justify-content: flex-start;
    }
    .composer-box {
      flex-wrap: wrap;
      border-radius: 1rem;
    }
    .composer-box textarea {
      order: 1;
      flex-basis: calc(100% - 3rem);
      min-width: 0;
    }
    .composer-box .send-btn {
      order: 2;
    }
    .running {
      display: none;
    }
  }
</style>
