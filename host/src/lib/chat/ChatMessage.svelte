<script lang="ts">
  import { onDestroy, tick } from "svelte";
  import type { ChatMessageView, JsonObject } from "$lib/api";
  import ArtifactInlineCard from "$lib/chat/ArtifactInlineCard.svelte";
  import PermissionProposalCard from "$lib/chat/PermissionProposalCard.svelte";
  import { renderMarkdown } from "$lib/chat/markdown";
  import ChatExtensionSlot from "$lib/chat/ChatExtensionSlot.svelte";
  import {
    MAX_TEXT_COMMENT_CHARACTERS,
    parseTextMarks,
    rangesContainSelection,
    readingOpportunityEvent,
    renderMarkdownWithMarks,
    textCommentEvent,
    textSelectionEvent,
    type MessageTextMarks,
    type TextMarkComment,
    type TextMarkRange,
    type TextSelectionRange,
  } from "$lib/chat/messageAnnotations";
  import type { ReadingOpportunityReport } from "$lib/chat/chatReadingOpportunity";
  import { splitMessageParts } from "$lib/chat/messageParts";
  import KestralMark from "$lib/shell/KestralMark.svelte";
  import LoadingIndicator from "$lib/shell/LoadingIndicator.svelte";
  import {
    chatArtifactCards,
    chatPermissionProposals,
    isRunAvailable,
  } from "$lib/provenance/sessionReferences";
  import { artifacts, artifactsLoaded } from "$lib/stores/artifacts";
  import { openArtifact } from "$lib/stores/navigation";
  import { currentTab, records, recordsLoaded } from "$lib/stores/hostState";

  interface Props {
    message: ChatMessageView;
    threadId?: string;
    threadResourceId?: string;
    assistantMessageNumber?: number;
    showMetadata?: boolean;
    showThinking?: boolean;
    /// Chat's single observation controller. A message registers its rendered
    /// body and forwards the aggregates it receives to the asking extension;
    /// it never observes anything itself.
    readingObservation?: {
      register: (
        messageId: string,
        element: HTMLElement,
        deliver: (report: ReadingOpportunityReport) => void,
      ) => void;
      unregister: (messageId: string) => void;
      setRequested: (messageId: string, requested: boolean) => void;
    };
  }

  let {
    message,
    threadId = "",
    threadResourceId = "",
    assistantMessageNumber = 1,
    showMetadata = false,
    showThinking = false,
    readingObservation,
  }: Props = $props();

  const artifactCards = $derived(
    chatArtifactCards(message, $artifactsLoaded, $artifacts, showMetadata),
  );
  const permissionProposals = $derived(
    chatPermissionProposals(message, $artifactsLoaded, $artifacts),
  );
  const runAvailable = $derived(isRunAvailable($recordsLoaded, $records, message.run_id));

  const parts = $derived(message.role === "assistant" ? splitMessageParts(message.text) : []);
  const partOffsets = $derived.by(() => {
    let offset = 0;
    return parts.map((part) => {
      const start = offset;
      offset += part.plainText.length;
      return start;
    });
  });

  interface TextAnnotations extends MessageTextMarks {
    appId: string;
    appName: string;
  }

  let textMarks = $state<Record<string, TextAnnotations>>({});
  let messageBody = $state<HTMLElement | null>(null);
  let selectionActionsElement = $state<HTMLElement | null>(null);
  let selectedRanges = $state<TextSelectionRange[]>([]);
  let selectedCommentTargets = $state<CommentTarget[]>([]);
  let commentEditor = $state<CommentEditor | null>(null);
  let commentInputElement = $state<HTMLTextAreaElement | null>(null);
  let commentReturnFocus: HTMLElement | null = null;
  let commentOperationTimer: ReturnType<typeof setTimeout> | null = null;
  let selectionStartedHere = false;
  let extensionSlot = $state<
    { sendExtensionEvent: (extensionKey: string, payload: JsonObject) => boolean } | undefined
  >();

  const annotators = $derived(
    Object.entries(textMarks).sort(([left], [right]) => left.localeCompare(right)),
  );
  const selectionActions = $derived(
    selectedRanges.length === 0
      ? []
      : annotators.map(([extensionKey, annotation]) => {
          const targetMarked = !rangesContainSelection(annotation.ranges, selectedRanges);
          return {
            extensionKey,
            appName: annotation.appName,
            label: targetMarked ? annotation.labels.mark : annotation.labels.unmark,
            targetMarked,
            deletesComments: !targetMarked && (annotation.comments ?? []).some((comment) =>
              comment.ranges.some((commentRange) => selectedRanges.some((range) =>
                range.part === commentRange.part &&
                range.start < commentRange.end &&
                range.end > commentRange.start
              ))
            ),
          };
        }),
  );
  const selectionCommentActions = $derived(
    selectedRanges.length === 0
      ? []
      : annotators.flatMap(([extensionKey, annotation]) => {
          if (!annotation.comments || !annotation.commentLabels) return [];
          if (!rangesContainSelection(annotation.ranges, selectedRanges)) return [];
          const existing = annotation.comments.find((comment) =>
            sameRanges(comment.ranges, selectedRanges)
          ) ?? null;
          return [{
            extensionKey,
            appName: annotation.appName,
            ranges: selectedRanges,
            comment: existing,
            label: existing ? annotation.commentLabels.edit : annotation.commentLabels.add,
          }];
        }),
  );

  interface CommentTarget {
    extensionKey: string;
    appName: string;
    ranges: TextSelectionRange[];
    comment: TextMarkComment | null;
    label: string;
  }

  interface CommentEditor extends CommentTarget {
    value: string;
    operationId: string | null;
    error: string | null;
    confirmDelete: boolean;
  }

  interface CommentActionRange extends TextMarkRange {
    interactive: true;
    sourcePart: number;
    commentId?: string;
    commentRange: TextMarkRange;
  }

  function sameRanges(left: TextMarkRange[], right: TextMarkRange[]): boolean {
    return left.length === right.length && left.every((range, index) =>
      range.part === right[index].part &&
      range.start === right[index].start &&
      range.end === right[index].end
    );
  }

  function handleExtensionState(
    extensionKey: string,
    appId: string,
    appName: string,
    payload: JsonObject,
  ): void {
    const marks = parseTextMarks(payload, parts);
    if (!marks) {
      // The owner replaced valid state with something this contract cannot
      // read. Keeping the previous marks would leave the user looking at
      // annotations their app no longer claims.
      handleInvalidExtensionState(extensionKey);
      return;
    }
    textMarks = { ...textMarks, [extensionKey]: { ...marks, appId, appName } };
    if (
      commentEditor?.extensionKey === extensionKey &&
      commentEditor.operationId &&
      marks.commentOperation?.id === commentEditor.operationId
    ) {
      if (marks.commentOperation.status === "completed") {
        closeCommentEditor();
      } else if (marks.commentOperation.status === "failed") {
        clearCommentOperationTimer();
        commentEditor = {
          ...commentEditor,
          operationId: null,
          error: marks.commentOperation.error ?? "Could not save the comment.",
        };
        void tick().then(() => commentInputElement?.focus());
      }
    }
  }

  function handleInvalidExtensionState(extensionKey: string): void {
    if (!(extensionKey in textMarks)) return;
    const next = { ...textMarks };
    delete next[extensionKey];
    textMarks = next;
  }

  /// Forward one bounded aggregate to every extension that asked for it. The
  /// report carries no geometry, so nothing here can leak scroll position.
  function deliverReadingReport(report: ReadingOpportunityReport): void {
    for (const [extensionKey, annotation] of annotators) {
      if (!annotation.observeReadingOpportunity) continue;
      extensionSlot?.sendExtensionEvent(
        extensionKey,
        readingOpportunityEvent({
          sessionId: report.sessionId,
          qualifiedVisibleMs: report.qualifiedVisibleMs,
          exposedMask: report.exposedMask,
          firstQualifiedAt: report.firstQualifiedAt,
          lastQualifiedAt: report.lastQualifiedAt,
          final: report.final,
        }),
      );
    }
  }

  const observationRequested = $derived(
    annotators.some(([, annotation]) => annotation.observeReadingOpportunity),
  );

  $effect(() => {
    const observation = readingObservation;
    const element = messageBody;
    if (!observation || !element || message.role !== "assistant") return;
    observation.register(message.id, element, deliverReadingReport);
    return () => observation.unregister(message.id);
  });

  $effect(() => {
    readingObservation?.setRequested(message.id, observationRequested);
  });

  function handleExtensionRemoved(extensionKey: string): void {
    // The frame that owed us a reply is gone; nothing will resolve a write
    // that was still in flight, so release the editor instead of leaving it
    // pending on an app that no longer exists.
    if (commentEditor?.extensionKey === extensionKey && commentEditor.operationId) {
      clearCommentOperationTimer();
      failPendingComment(
        `${commentEditor.appName} is no longer available. Your comment was not saved.`,
      );
    }
    if (!(extensionKey in textMarks)) return;
    const next = { ...textMarks };
    delete next[extensionKey];
    textMarks = next;
  }

  function selectedOffset(
    element: HTMLElement,
    container: Node,
    offset: number,
    fallback: number,
  ): number {
    if (!element.contains(container)) return fallback;
    const before = document.createRange();
    before.selectNodeContents(element);
    before.setEnd(container, offset);
    return before.toString().length;
  }

  function captureTextSelection(): void {
    const selection = window.getSelection();
    if (!selection || selection.isCollapsed || selection.rangeCount !== 1 || !messageBody) return;
    const selected = selection.getRangeAt(0);
    if (!selectionStartedHere && !messageBody.contains(selected.commonAncestorContainer)) return;

    const selectionStart = selectedOffset(
      messageBody,
      selected.startContainer,
      selected.startOffset,
      0,
    );
    const selectionEnd = selectedOffset(
      messageBody,
      selected.endContainer,
      selected.endOffset,
      parts.reduce((sum, part) => sum + part.plainText.length, 0),
    );
    const ranges: TextSelectionRange[] = [];
    for (const part of parts) {
      const partStart = partOffsets[part.index];
      const start = Math.max(0, Math.min(part.plainText.length, selectionStart - partStart));
      const end = Math.max(start, Math.min(part.plainText.length, selectionEnd - partStart));
      const text = part.plainText.slice(start, end);
      if (end > start && text.trim() !== "") ranges.push({ part: part.index, start, end, text });
    }
    selectedRanges = ranges;
    if (ranges.length > 0) selectedCommentTargets = [];
  }

  function trackTextSelection(node: HTMLElement): { destroy: () => void } {
    const clearBeforePointerSelection = (event: PointerEvent) => {
      if (
        event.target instanceof Node &&
        selectionActionsElement?.contains(event.target)
      ) return;
      selectionStartedHere = event.target instanceof Node && node.contains(event.target);
      selectedRanges = [];
    };
    const finishPointerSelection = () => {
      captureTextSelection();
      selectionStartedHere = false;
    };
    const captureKeyboardSelection = () => {
      const selection = window.getSelection();
      if (selection?.isCollapsed && !selectionActionsElement?.contains(document.activeElement)) {
        selectedRanges = [];
        return;
      }
      captureTextSelection();
    };
    document.addEventListener("pointerdown", clearBeforePointerSelection);
    document.addEventListener("pointerup", finishPointerSelection);
    document.addEventListener("selectionchange", captureTextSelection);
    document.addEventListener("keyup", captureKeyboardSelection);
    node.addEventListener("click", handleMarkedClick);
    node.addEventListener("keydown", handleMarkedKeydown);
    return {
      destroy: () => {
        document.removeEventListener("pointerdown", clearBeforePointerSelection);
        document.removeEventListener("pointerup", finishPointerSelection);
        document.removeEventListener("selectionchange", captureTextSelection);
        document.removeEventListener("keyup", captureKeyboardSelection);
        node.removeEventListener("click", handleMarkedClick);
        node.removeEventListener("keydown", handleMarkedKeydown);
      },
    };
  }

  function applyTextSelection(extensionKey: string, targetMarked: boolean): void {
    if (selectedRanges.length === 0) return;
    extensionSlot?.sendExtensionEvent(
      extensionKey,
      textSelectionEvent(selectedRanges, targetMarked),
    );
    selectedRanges = [];
    window.getSelection()?.removeAllRanges();
  }

  function selectionRange(range: TextMarkRange): TextSelectionRange {
    return {
      ...range,
      text: parts[range.part]?.plainText.slice(range.start, range.end) ?? "",
    };
  }

  function commentActionRanges(annotation: TextAnnotations, part: number): CommentActionRange[] {
    if (!annotation.comments || !annotation.commentLabels) return [];
    const comments = annotation.comments.flatMap((comment) =>
      comment.ranges
        .filter((range) => range.part === part)
        .map((range) => ({ comment, range }))
    );
    const actions: CommentActionRange[] = comments.map(({ comment, range }) => ({
      part,
      start: partOffsets[part] + range.start,
      end: partOffsets[part] + range.end,
      interactive: true,
      sourcePart: part,
      commentId: comment.id,
      commentRange: range,
    }));

    for (const range of annotation.ranges.filter((candidate) => candidate.part === part)) {
      const boundaries = new Set([range.start, range.end]);
      for (const { range: commentRange } of comments) {
        if (commentRange.start < range.end && commentRange.end > range.start) {
          boundaries.add(Math.max(range.start, commentRange.start));
          boundaries.add(Math.min(range.end, commentRange.end));
        }
      }
      const ordered = [...boundaries].sort((left, right) => left - right);
      for (let index = 0; index < ordered.length - 1; index += 1) {
        const start = ordered[index];
        const end = ordered[index + 1];
        if (comments.some(({ range }) => range.start <= start && range.end >= end)) continue;
        actions.push({
          part,
          start: partOffsets[part] + start,
          end: partOffsets[part] + end,
          interactive: true,
          sourcePart: part,
          commentRange: { part, start, end },
        });
      }
    }
    return actions;
  }

  function commentTargetsForSegment(part: number, start: number, end: number): CommentTarget[] {
    return annotators.flatMap(([extensionKey, annotation]) => {
      if (!annotation.comments || !annotation.commentLabels) return [];
      const existing = annotation.comments.find((comment) =>
        comment.ranges.some((range) =>
          range.part === part && range.start <= start && range.end >= end
        )
      ) ?? null;
      const marked = annotation.ranges.some((candidate) =>
        candidate.part === part && candidate.start <= start && candidate.end >= end
      );
      if (!marked) return [];
      return [{
        extensionKey,
        appName: annotation.appName,
        ranges: existing
          ? existing.ranges.map(selectionRange)
          : [selectionRange({ part, start, end })],
        comment: existing,
        label: existing ? annotation.commentLabels.edit : annotation.commentLabels.add,
      }];
    });
  }

  function activateMarkedText(target: EventTarget | null): void {
    if (!(target instanceof HTMLElement)) return;
    const mark = target.closest<HTMLElement>("mark[data-chat-text-actions]");
    const part = Number(mark?.dataset.chatTextPart);
    const start = Number(mark?.dataset.chatTextLogicalStart ?? mark?.dataset.chatTextStart);
    const end = Number(mark?.dataset.chatTextLogicalEnd ?? mark?.dataset.chatTextEnd);
    if (!mark || !Number.isInteger(part) || !Number.isInteger(start) || !Number.isInteger(end)) return;
    const targets = commentTargetsForSegment(part, start, end);
    if (targets.length === 0) return;
    selectedRanges = [];
    window.getSelection()?.removeAllRanges();
    commentReturnFocus = mark;
    if (targets.length === 1) {
      openCommentEditor(targets[0]);
    } else {
      selectedCommentTargets = targets;
    }
  }

  function handleMarkedClick(event: MouseEvent): void {
    activateMarkedText(event.target);
  }

  function handleMarkedKeydown(event: KeyboardEvent): void {
    if (event.key !== "Enter" && event.key !== " ") return;
    if (!(event.target instanceof HTMLElement) || !event.target.matches("mark[data-chat-text-actions]")) return;
    event.preventDefault();
    activateMarkedText(event.target);
  }

  function openCommentEditor(target: CommentTarget, returnFocus?: HTMLElement): void {
    if (returnFocus) commentReturnFocus = returnFocus;
    selectedCommentTargets = [];
    selectedRanges = [];
    window.getSelection()?.removeAllRanges();
    commentEditor = {
      ...target,
      value: target.comment?.text ?? "",
      operationId: null,
      error: null,
      confirmDelete: false,
    };
    void tick().then(() => commentInputElement?.focus());
  }

  function closeCommentEditor(): void {
    clearCommentOperationTimer();
    const closedRange = commentEditor?.ranges[0];
    commentEditor = null;
    selectedCommentTargets = [];
    const focusTarget = commentReturnFocus;
    commentReturnFocus = null;
    void tick().then(() => {
      if (focusTarget?.isConnected) {
        focusTarget.focus();
        return;
      }
      if (!closedRange || !messageBody) return;
      const part = messageBody.querySelector<HTMLElement>(`[data-message-part="${closedRange.part}"]`);
      const replacement = [...(part?.querySelectorAll<HTMLElement>("mark[data-chat-text-actions]") ?? [])]
        .find((mark) =>
          Number(mark.dataset.chatTextLogicalStart ?? mark.dataset.chatTextStart) <= closedRange.start &&
          Number(mark.dataset.chatTextLogicalEnd ?? mark.dataset.chatTextEnd) >= closedRange.start
        );
      replacement?.focus();
    });
  }

  function randomId(): string {
    return crypto.randomUUID();
  }

  /// A comment write is completed by the extension frame, which is userland
  /// code the host does not control: it can be disabled mid-edit, fail to
  /// load, or simply never answer. Every wait therefore has to end on its own.
  const COMMENT_OPERATION_TIMEOUT_MS = 15_000;

  function failPendingComment(reason: string): void {
    if (!commentEditor?.operationId) return;
    commentEditor = { ...commentEditor, operationId: null, error: reason };
    void tick().then(() => commentInputElement?.focus());
  }

  /// Hand one comment write to the extension frame and guarantee the editor
  /// leaves its pending state, whether the frame answers, is already gone, or
  /// stays silent.
  function beginCommentOperation(operationId: string, payload: JsonObject): void {
    clearCommentOperationTimer();
    const delivered = extensionSlot?.sendExtensionEvent(
      commentEditor!.extensionKey,
      payload,
    );
    if (!delivered) {
      failPendingComment(
        `${commentEditor!.appName} isn’t available right now. Your comment was not saved.`,
      );
      return;
    }
    commentOperationTimer = setTimeout(() => {
      commentOperationTimer = null;
      if (commentEditor?.operationId !== operationId) return;
      failPendingComment(
        `${commentEditor.appName} did not respond. Your comment was not saved.`,
      );
    }, COMMENT_OPERATION_TIMEOUT_MS);
  }

  function clearCommentOperationTimer(): void {
    if (commentOperationTimer === null) return;
    clearTimeout(commentOperationTimer);
    commentOperationTimer = null;
  }

  function saveComment(): void {
    if (!commentEditor || commentEditor.operationId) return;
    const text = commentEditor.value.trim();
    if (text === "") {
      commentEditor = { ...commentEditor, error: "Enter a comment before saving." };
      return;
    }
    const operationId = randomId();
    const commentId = commentEditor.comment?.id ?? randomId();
    commentEditor = { ...commentEditor, value: text, operationId, error: null, confirmDelete: false };
    beginCommentOperation(
      operationId,
      textCommentEvent(operationId, "upsert", commentId, commentEditor.ranges, text),
    );
  }

  function deleteComment(): void {
    const comment = commentEditor?.comment;
    if (!commentEditor || !comment || commentEditor.operationId) return;
    if (!commentEditor.confirmDelete) {
      commentEditor = { ...commentEditor, confirmDelete: true };
      return;
    }
    const operationId = randomId();
    commentEditor = { ...commentEditor, operationId, error: null };
    beginCommentOperation(
      operationId,
      textCommentEvent(operationId, "delete", comment.id, commentEditor.ranges),
    );
  }

  // Escape always closes. Cancelling a pending write only abandons this
  // editor's interest in the reply — a late reply is ignored because the
  // matching editor is gone — and it must never be possible to trap the user
  // in a dialog waiting on an app that may never answer.
  function handleCommentEditorKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") closeCommentEditor();
  }

  onDestroy(() => {
    clearCommentOperationTimer();
  });

  const roleLabel = $derived(
    message.role === "user"
      ? "You"
      : message.role === "assistant"
        ? "Assistant"
        : message.role === "tool-status"
          ? "Tool"
          : "System",
  );
  const statusLabel = $derived(
    message.status === "failed"
      ? "Failed"
      : message.status === "cancelled"
        ? "Cancelled"
      : message.status === "interrupted"
        ? "Interrupted"
        : message.status === "pending"
          ? "Running"
          : message.role === "tool-status"
            ? "Tool used"
            : null,
  );

  function openRun() {
    currentTab.set("system");
  }
</script>

<article class="entry {message.role} {message.status ?? ''}">
  {#if message.role === "assistant"}
    <div class="avatar" aria-hidden="true">
      {#if message.status === "pending"}
        <LoadingIndicator size={1.9} markSize={1.2} inheritColor />
      {:else}
        <KestralMark size="1.2rem" />
      {/if}
    </div>
  {/if}

  <div class="content">
    {#if message.role !== "user"}
      <span class="author">{roleLabel}</span>
    {/if}

    <div class="bubble">
      {#if message.status === "pending" && message.role !== "user"}
        <div class="status status-pending" role="status" aria-live="polite">Thinking</div>
      {:else if message.status === "failed" || message.status === "interrupted" || message.status === "cancelled"}
        <div class="status status-{message.status}" role="status" aria-live="polite">
          {statusLabel}
        </div>
      {:else if message.role === "tool-status" && statusLabel}
        <div class="status status-completed" role="status" aria-live="polite">{statusLabel}</div>
      {/if}
      {#if message.role === "assistant"}
        <!-- Assistant replies are Markdown; renderMarkdown escapes first, so
             this {@html} only ever contains tags the renderer itself emits. -->
        {#if annotators.length > 0 && parts.length > 0}
          <div class="text markdown" bind:this={messageBody} use:trackTextSelection>
            {@html renderMarkdownWithMarks(message.text, annotators.flatMap(([, annotation]) => [
              ...annotation.ranges.map((range) => ({
                ...range,
                start: partOffsets[range.part] + range.start,
                end: partOffsets[range.part] + range.end,
              })),
              ...parts.flatMap((part) => commentActionRanges(annotation, part.index)),
            ]))}
          </div>
        {:else}
          <div class="text markdown">{@html renderMarkdown(message.text)}</div>
        {/if}
      {:else}
        <p class="text">{message.text}</p>
      {/if}

      {#if selectionActions.length > 0 || selectionCommentActions.length > 0}
        <div
          class="viewport-actions"
          bind:this={selectionActionsElement}
          aria-label="Selected text actions"
        >
          {#each selectionActions as action (action.extensionKey)}
            <button
              type="button"
              class="selection-action"
              onpointerdown={(event) => event.preventDefault()}
              onclick={() => applyTextSelection(action.extensionKey, action.targetMarked)}
            >
              {action.label}{action.deletesComments ? " and delete attached comment" : ""}
              {#if selectionActions.length > 1}<span> with {action.appName}</span>{/if}
            </button>
          {/each}
          {#each selectionCommentActions as action (action.extensionKey)}
            <button
              type="button"
              class="selection-action secondary"
              onpointerdown={(event) => event.preventDefault()}
              onclick={(event) => openCommentEditor(action, event.currentTarget)}
            >
              {action.label}{#if selectionCommentActions.length > 1}<span> with {action.appName}</span>{/if}
            </button>
          {/each}
          <span class="selection-copy-hint">Copy still works normally.</span>
        </div>
      {/if}

      {#if selectedCommentTargets.length > 0}
        <div class="viewport-actions" aria-label="Marked text actions">
          {#each selectedCommentTargets as target (target.extensionKey)}
            <button
              type="button"
              class="selection-action"
              onclick={(event) => openCommentEditor(target, event.currentTarget)}
            >
              {target.label}{#if selectedCommentTargets.length > 1}<span> with {target.appName}</span>{/if}
            </button>
          {/each}
          <button type="button" class="tray-cancel" onclick={() => selectedCommentTargets = []}>Cancel</button>
        </div>
      {/if}

      {#if commentEditor}
        <dialog
          open
          class="comment-editor"
          aria-label={commentEditor.comment ? "Edit reading comment" : "Add reading comment"}
          onkeydown={handleCommentEditorKeydown}
        >
          <label>
            <span>Comment</span>
            <textarea
              bind:this={commentInputElement}
              bind:value={commentEditor.value}
              maxlength={MAX_TEXT_COMMENT_CHARACTERS}
              rows="3"
              disabled={commentEditor.operationId !== null}
            ></textarea>
          </label>
          <div class="comment-editor-actions">
            <button type="button" class="selection-action" disabled={commentEditor.operationId !== null} onclick={saveComment}>
              {commentEditor.operationId ? "Saving…" : "Save comment"}
            </button>
            {#if commentEditor.comment}
              <button type="button" class="delete-comment" disabled={commentEditor.operationId !== null} onclick={deleteComment}>
                {commentEditor.confirmDelete ? "Confirm delete" : "Delete comment"}
              </button>
            {/if}
            <button type="button" class="tray-cancel" onclick={closeCommentEditor}>Cancel</button>
          </div>
          {#if commentEditor.error}
            <p class="comment-error" role="alert">{commentEditor.error}</p>
          {/if}
        </dialog>
      {/if}

      {#if showThinking && message.role === "assistant" && message.reasoning}
        <details class="reasoning">
          <summary>Thinking</summary>
          <div class="reasoning-text">{message.reasoning}</div>
        </details>
      {/if}

      <!-- Extensions act on the response text, so mount them only once the
           response is complete — a pending message would hand them a partial
           or empty text. A message without a host timestamp cannot satisfy the
           v5 context, so it gets no extension rather than a fabricated time. -->
      {#if message.role === "assistant" && message.status !== "pending" && message.text.trim() !== "" && message.created_at}
        <ChatExtensionSlot
          bind:this={extensionSlot}
          pointName="message-actions"
          context={{
            thread_id: threadId,
            resource_id: threadResourceId,
            message_id: message.id,
            assistant_message_number: assistantMessageNumber,
            assistant_response_excerpt: message.text.slice(0, 500),
            assistant_response_text: message.text,
            created_at: message.created_at,
            // Assistant responses have always been persisted only after their
            // text was complete, so a message stored before `completed_at`
            // existed still reports its true full-response time here.
            completed_at: message.completed_at ?? message.created_at,
            role: "assistant",
            part_count: parts.length,
            parts: parts.map((part) => ({
              index: part.index,
              excerpt: part.excerpt,
              plain_text: part.plainText,
            })),
          }}
          onExtensionState={handleExtensionState}
          onExtensionRemoved={handleExtensionRemoved}
        />
      {/if}

      {#each artifactCards as artifact (artifact.id)}
        <ArtifactInlineCard {artifact} openArtifacts={() => openArtifact(artifact.id)} />
      {/each}

      {#each permissionProposals as proposal (proposal.artifactId)}
        <PermissionProposalCard {proposal} />
      {/each}

      {#if showMetadata && message.run_id}
        {#if runAvailable === true}
          <details class="run-details">
            <summary>Details</summary>
            <div class="run-meta">
              <code>{message.run_id}</code>
              <button type="button" class="artifact-link" onclick={openRun}>View activity in System</button>
            </div>
          </details>
        {:else if runAvailable === false}
          <p class="metadata-unavailable">Activity details are unavailable for this message.</p>
        {/if}
      {/if}
    </div>
  </div>
</article>

<style>
  .entry {
    display: flex;
    gap: 0.85rem;
    align-items: flex-start;
  }
  .entry.user {
    flex-direction: row-reverse;
  }
  .entry.tool-status {
    opacity: 1;
  }
  .entry.pending .bubble {
    opacity: 0.7;
  }
  .avatar {
    flex-shrink: 0;
    width: 1.9rem;
    height: 1.9rem;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--color-text);
    color: var(--color-accent-contrast);
    margin-top: 0.1rem;
  }
  .content {
    min-width: 0;
    max-width: min(44rem, 90%);
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .entry.user .content {
    align-items: flex-end;
  }
  .author {
    font-size: 0.78rem;
    font-weight: 600;
    color: var(--color-text);
  }
  .bubble {
    display: grid;
    gap: 0.7rem;
    word-break: break-word;
  }
  .entry.user .bubble {
    background: var(--color-surface-muted);
    border-radius: 18px 18px 4px 18px;
    padding: 0.75rem 0.95rem;
  }
  .entry.assistant .bubble {
    background: transparent;
  }
  .entry.tool-status .bubble {
    background: var(--color-surface-muted);
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: 0.7rem 0.9rem;
    font-size: 0.88rem;
    color: var(--color-text-muted);
  }
  .entry.failed .bubble,
  .entry.refused .bubble,
  .entry.cancelled .bubble {
    background: var(--color-warning-soft);
    border: 1px solid var(--color-warning-border);
    border-radius: 12px;
    padding: 0.75rem 0.95rem;
  }
  .text {
    margin: 0;
    white-space: pre-line;
    line-height: 1.6;
    color: var(--color-text);
    font-size: 0.95rem;
  }
  .entry.user .text {
    color: var(--color-text);
  }
  /* Rendered Markdown for assistant replies. Tight, readable spacing that
     matches the conversation rhythm rather than document defaults. */
  .markdown {
    line-height: 1.6;
    color: var(--color-text);
    font-size: 0.95rem;
  }
  .markdown :global(p) {
    margin: 0 0 0.7rem;
  }
  .markdown :global(p:last-child) {
    margin-bottom: 0;
  }
  .markdown :global(h1),
  .markdown :global(h2),
  .markdown :global(h3),
  .markdown :global(h4),
  .markdown :global(h5),
  .markdown :global(h6) {
    margin: 0.9rem 0 0.5rem;
    line-height: 1.3;
    font-weight: 600;
  }
  .markdown :global(h1) { font-size: 1.25rem; }
  .markdown :global(h2) { font-size: 1.15rem; }
  .markdown :global(h3) { font-size: 1.05rem; }
  .markdown :global(h4),
  .markdown :global(h5),
  .markdown :global(h6) { font-size: 0.98rem; }
  .markdown :global(ul),
  .markdown :global(ol) {
    margin: 0 0 0.7rem;
    padding-left: 1.4rem;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .markdown :global(a) {
    color: var(--color-accent);
    text-decoration: underline;
  }
  .markdown :global(strong) {
    font-weight: 600;
  }
  .markdown :global(code) {
    font-family: Consolas, "SF Mono", monospace;
    font-size: 0.85em;
    background: var(--color-surface-muted);
    padding: 0.1rem 0.35rem;
    border-radius: 6px;
    overflow-wrap: anywhere;
  }
  .markdown :global(pre) {
    margin: 0 0 0.7rem;
    padding: 0.8rem 0.9rem;
    background: var(--color-surface-muted);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    overflow-x: auto;
  }
  .markdown :global(pre code) {
    background: transparent;
    padding: 0;
    font-size: 0.82rem;
    line-height: 1.5;
    white-space: pre;
  }
  /* Tables scroll locally when their content is wider than the message. */
  .markdown :global(.markdown-table) {
    max-width: 100%;
    margin: 0 0 0.7rem;
    overflow-x: auto;
  }
  .markdown :global(table) {
    width: max-content;
    min-width: 100%;
    border-collapse: collapse;
  }
  .markdown :global(th),
  .markdown :global(td) {
    padding: 0.45rem 0.6rem;
    border: 1px solid var(--color-border-subtle);
    text-align: left;
    vertical-align: top;
    word-break: normal;
  }
  .markdown :global(th) {
    background: var(--color-surface-muted);
    font-weight: 600;
  }
  .markdown :global(.align-center) { text-align: center; }
  .markdown :global(.align-right) { text-align: right; }
  .markdown :global(blockquote) {
    margin: 0 0 0.7rem;
    padding: 0.2rem 0 0.2rem 0.9rem;
    border-left: 3px solid var(--color-border-strong);
    color: var(--color-text-soft);
  }
  .markdown :global(hr) {
    border: none;
    border-top: 1px solid var(--color-border-subtle);
    margin: 0.9rem 0;
  }
  .markdown :global(mark[data-chat-text-mark]) {
    color: inherit;
    background: var(--color-success-soft);
    border-bottom: 2px solid var(--color-success-text);
    padding-block: 0.05em;
  }
  .markdown :global(mark[data-chat-text-comment]) {
    background: var(--color-comment-soft);
    border-bottom: 2px dotted var(--color-comment-border);
    color: var(--color-comment-text);
  }
  .markdown :global(mark[data-chat-text-actions]) {
    cursor: pointer;
  }
  .markdown :global(mark[data-chat-text-actions]:focus-visible) {
    outline: 2px solid var(--color-focus-ring);
    outline-offset: 2px;
  }
  .viewport-actions,
  .comment-editor {
    position: fixed;
    z-index: 40;
    inset-inline-start: 50%;
    inset-block-end: 1rem;
    transform: translateX(-50%);
    width: max-content;
    max-width: calc(100vw - 2rem);
    max-height: calc(100vh - 2rem);
    max-height: calc(100dvh - 2rem);
    overflow: auto;
    border: 1px solid var(--color-border-strong);
    background: var(--color-surface);
  }
  .viewport-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.4rem 0.65rem;
    justify-content: center;
    padding: 0.5rem 0.65rem;
    border-radius: 0.75rem;
  }
  .selection-action {
    min-height: 2rem;
    border: 1px solid var(--color-accent);
    border-radius: 999px;
    padding: 0.25em 0.7em;
    background: var(--color-accent);
    color: var(--color-accent-contrast);
    font: inherit;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
  }
  .selection-action.secondary {
    background: var(--color-surface);
    color: var(--color-accent);
  }
  .selection-action:focus-visible {
    outline: 2px solid var(--color-focus-ring);
    outline-offset: 2px;
  }
  .selection-copy-hint {
    color: var(--color-text-muted);
    font-size: 0.78rem;
  }
  .tray-cancel,
  .delete-comment {
    min-height: 2rem;
    border: 1px solid var(--color-border);
    border-radius: 999px;
    padding: 0.25em 0.7em;
    background: var(--color-surface);
    color: var(--color-text);
    font: inherit;
    font-size: 0.8rem;
    cursor: pointer;
  }
  .delete-comment {
    color: var(--color-danger-text);
    border-color: var(--color-danger-border);
  }
  .tray-cancel:focus-visible,
  .delete-comment:focus-visible,
  .comment-editor textarea:focus-visible {
    outline: 2px solid var(--color-focus-ring);
    outline-offset: 2px;
  }
  .comment-editor {
    width: min(28rem, calc(100vw - 2rem));
    display: grid;
    gap: 0.65rem;
    padding: 0.8rem;
    border-radius: 0.85rem;
    margin: 0;
    color: var(--color-text);
  }
  .comment-editor-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.5rem 0.75rem;
  }
  .comment-editor label {
    display: grid;
    gap: 0.3rem;
    color: var(--color-text);
    font-size: 0.85rem;
    font-weight: 600;
  }
  .comment-editor textarea {
    width: 100%;
    max-height: 10rem;
    resize: vertical;
    border: 1px solid var(--color-border-strong);
    border-radius: 0.6rem;
    padding: 0.6rem;
    background: var(--color-surface);
    color: var(--color-text);
    font: inherit;
    font-weight: 400;
  }
  .comment-editor button:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .comment-error {
    margin: 0;
    color: var(--color-danger-text);
    font-size: 0.82rem;
  }
  .status {
    width: fit-content;
    border-radius: 999px;
    padding: 0.2rem 0.55rem;
    font-size: 0.7rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.02em;
  }
  .status-failed {
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .status-refused {
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .status-cancelled {
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .status-pending {
    background: var(--color-surface-muted);
    color: var(--color-text-muted);
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }
  .status-pending::before {
    content: "";
    width: 0.4rem;
    height: 0.4rem;
    border-radius: 50%;
    background: var(--color-text-muted);
    animation: pulse 1.1s infinite ease-in-out;
  }
  .status-completed {
    background: var(--color-success-soft);
    color: var(--color-success-text);
  }
  @keyframes pulse {
    0%, 100% { opacity: 0.3; }
    50% { opacity: 1; }
  }
  .artifact-link {
    width: fit-content;
    /* 24 CSS px minimum touch target (WCAG 2.2 SC 2.5.8); centering the text
       in a taller flex box keeps the link's position visually unchanged. */
    min-height: 1.5rem;
    display: inline-flex;
    align-items: center;
    border: none;
    background: transparent;
    color: var(--color-accent);
    padding: 0;
    font-size: 0.82rem;
    font-weight: 500;
    cursor: pointer;
  }
  .artifact-link:hover {
    text-decoration: underline;
  }
  .warning {
    margin: 0;
    color: var(--color-warning-text);
    background: var(--color-warning-soft);
    border: 1px solid var(--color-warning-border);
    border-radius: 10px;
    padding: 0.7rem 0.85rem;
    font-size: 0.85rem;
    line-height: 1.45;
  }
  .metadata-unavailable {
    margin: 0;
    color: var(--color-text-faint);
    font-size: 0.8rem;
  }
  .run-details {
    font-size: 0.82rem;
    color: var(--color-text-faint);
  }
  .reasoning {
    max-width: 65ch;
    color: var(--color-text-muted);
    font-size: 0.86rem;
  }
  .reasoning summary {
    width: fit-content;
    cursor: pointer;
    font-weight: 600;
  }
  .reasoning-text {
    margin-top: 0.5rem;
    padding: 0.65rem 0.75rem;
    border-left: 2px solid var(--color-border-strong);
    white-space: pre-wrap;
    line-height: 1.55;
  }
  .run-details summary {
    cursor: pointer;
    color: var(--color-text-soft);
  }
  .run-meta {
    margin-top: 0.5rem;
    display: flex;
    gap: 0.8rem;
    align-items: center;
    flex-wrap: wrap;
  }
  code {
    font-family: Consolas, "SF Mono", monospace;
    font-size: 0.78rem;
    color: var(--color-text-soft);
    background: var(--color-surface-muted);
    padding: 0.2rem 0.4rem;
    border-radius: 6px;
    overflow-wrap: anywhere;
  }
  .entry.user code {
    background: var(--color-surface-hover);
  }
</style>
