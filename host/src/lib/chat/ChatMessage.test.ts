import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { get } from "svelte/store";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    listGrants: vi.fn(async () => []),
    submitPermissionProposal: vi.fn(async () => ({
      status: "issued" as const,
      grant_id: "grant-new",
      effective_condition: "requires-approval" as const,
    })),
  };
});

import type { Artifact, ChatMessageView, GrantView, LedgerRecord } from "$lib/api";
import { artifacts, artifactsLoaded } from "$lib/stores/artifacts";
import { grants, grantsLoaded } from "$lib/stores/grants";
import { currentTab, records, recordsLoaded } from "$lib/stores/hostState";
import { artifactTarget } from "$lib/stores/navigation";
import { TEXT_ANNOTATION_CONTRACT, TEXT_MARKS_KIND } from "./messageAnnotations";
import type { ReadingOpportunityReport } from "./chatReadingOpportunity";
import { splitMessageParts } from "./messageParts";
import ChatMessage from "./ChatMessage.svelte";
import * as api from "$lib/api";

// Stub the extension slot: these tests drive the text-marks contract directly
// through the slot's props instead of mounting sandboxed iframes.
const slotMock = vi.hoisted(() => ({
  props: null as Record<string, any> | null,
  // Returns whether a live frame received the event, matching the real slot.
  // `true` is the default so tests exercise the delivered path unless they
  // explicitly simulate a missing/removed extension frame.
  // The parameters are declared so `mock.calls` keeps its argument types.
  sendExtensionEvent: vi.fn((_extensionKey: string, _payload: Record<string, any>) => true),
}));
vi.mock("./ChatExtensionSlot.svelte", () => ({
  default: (_anchor: unknown, props: Record<string, any>) => {
    slotMock.props = props;
    return { sendExtensionEvent: slotMock.sendExtensionEvent };
  },
}));

function message(overrides: Partial<ChatMessageView> = {}): ChatMessageView {
  return {
    id: "msg-1",
    role: "assistant",
    text: "Hello!",
    run_id: null,
    artifact_ids: [],
    status: null,
    created_at: "2026-07-01T10:00:00.000Z",
    completed_at: "2026-07-01T10:00:02.000Z",
    ...overrides,
  };
}

function artifact(id: string): Artifact {
  return {
    artifact_id: id,
    artifact_type: "note",
    title: "Groceries",
    content: { body: "Milk" },
    provenance: {
      run_id: "run-1",
      capability: { provider: "notes", capability: "create" },
      grant_id: "grant-1",
      produced_by: "chat",
      recorded_at: "2026-07-01T10:00:00Z",
    },
  };
}

function runRecord(runId: string): LedgerRecord {
  return {
    sequence: 1,
    recorded_at: "2026-07-01T10:00:00Z",
    event: { kind: "run-started", run_id: runId } as LedgerRecord["event"],
  };
}

function grant(overrides: Partial<GrantView> = {}): GrantView {
  return {
    grant_id: "grant-active",
    holder: "chat",
    holder_display_name: "Chat",
    scope: { kind: "exact-capability", provider: "notes", capability: "notes.create" },
    data_scope: { kind: "none" },
    condition: "requires-approval",
    issued_at: "2026-07-01T10:00:00Z",
    expires_at: null,
    status: "active",
    origin: "user-added",
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  artifacts.set([]);
  artifactsLoaded.set(false);
  grants.set([]);
  grantsLoaded.set(true);
  records.set([]);
  recordsLoaded.set(false);
  artifactTarget.set(null);
  currentTab.set("chat");
  slotMock.props = null;
  slotMock.sendExtensionEvent.mockClear();
});

describe("ChatMessage", () => {
  it("renders a plain user message without status chips", () => {
    render(ChatMessage, { message: message({ role: "user", text: "hi there", status: "pending" }) });
    expect(screen.getByText("hi there")).toBeTruthy();
    expect(screen.queryByText("Thinking")).toBeNull();
    expect(screen.queryByText("Failed")).toBeNull();
  });

  it("keeps provider reasoning collapsed until requested", () => {
    const { container } = render(ChatMessage, {
      message: message({ reasoning: "I compared the available options." }),
      showThinking: true,
    });

    const details = container.querySelector<HTMLDetailsElement>("details.reasoning");
    expect(details).toBeTruthy();
    expect(details!.open).toBe(false);
    expect(screen.getByText("I compared the available options.")).toBeTruthy();
  });

  it("shows a compact state for pending, failed, interrupted, cancelled, and tool use", () => {
    const { container: pendingContainer, unmount: unmountPending } = render(ChatMessage, {
      message: message({ status: "pending", text: "" }),
    });
    expect(screen.getByText("Thinking")).toBeTruthy();
    expect(pendingContainer.querySelector(".avatar .speed-line")).toBeTruthy();
    const pendingBird = pendingContainer.querySelector<SVGGElement>(".avatar .bird");
    expect(pendingContainer.querySelector(".avatar .loading.inherit-color")).toBeTruthy();
    expect(pendingBird?.getAttribute("transform")).toBe(
      "translate(206 206) scale(0.7549342105263157) translate(-256 -256)",
    );
    unmountPending();

    const { unmount: unmountFailed } = render(ChatMessage, {
      message: message({ status: "failed", text: "The provider is unreachable." }),
    });
    expect(screen.getByText("Failed")).toBeTruthy();
    unmountFailed();

    const { unmount: unmountInterrupted } = render(ChatMessage, {
      message: message({ status: "interrupted", text: "The request was interrupted." }),
    });
    expect(screen.getByText("Interrupted")).toBeTruthy();
    unmountInterrupted();

    const { unmount: unmountCancelled } = render(ChatMessage, {
      message: message({ status: "cancelled", text: "You cancelled this request." }),
    });
    expect(screen.getByText("Cancelled")).toBeTruthy();
    unmountCancelled();

    render(ChatMessage, {
      showMetadata: true,
      message: message({ role: "tool-status", text: "Created note Groceries" }),
    });
    expect(screen.getByText("Tool used")).toBeTruthy();
    expect(screen.getByText("Tool")).toBeTruthy();
  });

  it("renders inline artifact cards for artifacts that still exist", () => {
    artifacts.set([artifact("artifact-1")]);
    artifactsLoaded.set(true);
    render(ChatMessage, { message: message({ artifact_ids: ["artifact-1"] }) });

    expect(screen.getByText("Groceries")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open in Artifacts"})).toBeTruthy();
  });

  it("shows MCP result cards only with activity details enabled", async () => {
    const result = artifact("mcp-result");
    result.artifact_type = "mcp-result-card";
    result.title = "search result";
    result.content = { tool: "search", result: "x".repeat(200) };
    artifacts.set([result]);
    artifactsLoaded.set(true);
    const view = render(ChatMessage, {
      message: message({ artifact_ids: [result.artifact_id] }),
    });

    expect(screen.queryByText("search result")).toBeNull();

    await view.rerender({
      message: message({ artifact_ids: [result.artifact_id] }),
      showMetadata: true,
    });
    expect(screen.getByText("search result")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open in Artifacts" })).toBeTruthy();
  });

  it("opens the exact artifact from an inline card", async () => {
    artifacts.set([artifact("artifact-1")]);
    artifactsLoaded.set(true);
    render(ChatMessage, { message: message({ artifact_ids: ["artifact-1"] }) });

    await fireEvent.click(screen.getByRole("button", { name: "Open in Artifacts" }));

    expect(get(currentTab)).toBe("stuff");
    expect(get(artifactTarget)?.artifactId).toBe("artifact-1");
  });

  it("submits a verified permission proposal from a host-owned card", async () => {
    const proposal = artifact("proposal-1");
    proposal.artifact_type = "permission-proposal";
    proposal.title = "Permission request";
    proposal.content = {
      holder: "chat",
      scope: {
        kind: "exact-capability",
        provider: "notes",
        capability: "notes.create",
      },
      data_scope: { kind: "none" },
      condition: "requires-approval",
      duration: { kind: "non-expiring" },
      reason: "Create the event requested by the user",
    };
    proposal.provenance = {
      run_id: "run-proposal",
      capability: {
        provider: "com.ma-zierl.host.permissions",
        capability: "permissions.propose_grant",
      },
      grant_id: "grant-proposal-tool",
      produced_by: "com.ma-zierl.host.permissions",
      recorded_at: "2026-07-01T10:00:00Z",
    };
    artifacts.set([proposal]);
    artifactsLoaded.set(true);
    render(ChatMessage, { message: message({ artifact_ids: ["proposal-1"] }) });

    expect(screen.getByText("By default, Kestral will ask before every use of this capability.")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Review and grant" }));

    expect(api.submitPermissionProposal).toHaveBeenCalledWith("proposal-1");
    expect(await screen.findByText("Permission granted. Each use will ask for approval.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open in Artifacts" })).toBeNull();
  });

  it("does not reactivate a permission proposal after its grant reloads", () => {
    const proposal = artifact("proposal-active");
    proposal.artifact_type = "permission-proposal";
    proposal.content = {
      holder: "chat",
      scope: {
        kind: "exact-capability",
        provider: "notes",
        capability: "notes.create",
      },
      data_scope: { kind: "none" },
      condition: "requires-approval",
      duration: { kind: "non-expiring" },
      reason: "Create an event",
    };
    proposal.provenance = {
      run_id: "run-proposal",
      capability: {
        provider: "com.ma-zierl.host.permissions",
        capability: "permissions.propose_grant",
      },
      grant_id: "grant-proposal-tool",
      produced_by: "com.ma-zierl.host.permissions",
      recorded_at: "2026-07-01T10:00:00Z",
    };
    artifacts.set([proposal]);
    artifactsLoaded.set(true);
    grants.set([grant()]);

    render(ChatMessage, { message: message({ artifact_ids: [proposal.artifact_id] }) });

    expect(screen.getByText("This approval-required permission is already active.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Review and grant" })).toBeNull();
    expect(api.submitPermissionProposal).not.toHaveBeenCalled();
  });

  it("keeps review available when only an inactive grant matches", () => {
    const proposal = artifact("proposal-revoked");
    proposal.artifact_type = "permission-proposal";
    proposal.content = {
      holder: "chat",
      scope: {
        kind: "exact-capability",
        provider: "notes",
        capability: "notes.create",
      },
      data_scope: { kind: "none" },
      condition: "requires-approval",
      duration: { kind: "non-expiring" },
      reason: "Create an event",
    };
    proposal.provenance = {
      run_id: "run-proposal",
      capability: {
        provider: "com.ma-zierl.host.permissions",
        capability: "permissions.propose_grant",
      },
      grant_id: "grant-proposal-tool",
      produced_by: "com.ma-zierl.host.permissions",
      recorded_at: "2026-07-01T10:00:00Z",
    };
    artifacts.set([proposal]);
    artifactsLoaded.set(true);
    grants.set([grant({ status: "revoked" })]);

    render(ChatMessage, { message: message({ artifact_ids: [proposal.artifact_id] }) });

    expect(screen.getByRole("button", { name: "Review and grant" })).toBeTruthy();
  });

  it("warns when another grant already makes the capability less interactive", async () => {
    vi.mocked(api.submitPermissionProposal).mockResolvedValueOnce({
      status: "already-active",
      grant_id: "grant-silent",
      effective_condition: "silent",
    });
    const proposal = artifact("proposal-silent");
    proposal.artifact_type = "permission-proposal";
    proposal.content = {
      holder: "chat",
      scope: {
        kind: "exact-capability",
        provider: "notes",
        capability: "notes.create",
      },
      data_scope: { kind: "none" },
      condition: "requires-approval",
      duration: { kind: "non-expiring" },
      reason: "Create an event",
    };
    proposal.provenance = {
      run_id: "run-proposal",
      capability: {
        provider: "com.ma-zierl.host.permissions",
        capability: "permissions.propose_grant",
      },
      grant_id: "grant-proposal-tool",
      produced_by: "com.ma-zierl.host.permissions",
      recorded_at: "2026-07-01T10:00:00Z",
    };
    artifacts.set([proposal]);
    artifactsLoaded.set(true);
    render(ChatMessage, { message: message({ artifact_ids: ["proposal-silent"] }) });

    await fireEvent.click(screen.getByRole("button", { name: "Review and grant" }));

    expect(await screen.findByText(/already allows this capability with no approval or notice/)).toBeTruthy();
  });

  it("marks unavailable artifacts instead of rendering a broken link", () => {
    artifacts.set([]);
    artifactsLoaded.set(true);
    render(ChatMessage, { message: message({ artifact_ids: ["gone-artifact"] }) });

    expect(screen.getAllByText("Unavailable reference").length).toBeGreaterThan(0);
    expect(screen.queryByRole("button", { name: "Open in Artifacts"})).toBeNull();
  });

  it("does not call a new artifact unavailable while references are synchronizing", () => {
    artifacts.set([]);
    artifactsLoaded.set(false);
    render(ChatMessage, { message: message({ artifact_ids: ["new-artifact"] }) });

    expect(screen.queryByText("Unavailable reference")).toBeNull();
  });

  it("hides the run id behind a collapsed details section", () => {
    records.set([runRecord("run-42")]);
    recordsLoaded.set(true);
    const { container } = render(ChatMessage, {
      message: message({ run_id: "run-42" }),
      showMetadata: true,
    });

    const details = container.querySelector("details");
    expect(details).toBeTruthy();
    expect(details!.open).toBe(false);
    expect(screen.getByText("Details")).toBeTruthy();
    expect(screen.getByText("run-42")).toBeTruthy();
  });

  it("explains unavailable runs instead of rendering a dead inspect action", () => {
    records.set([]);
    recordsLoaded.set(true);
    render(ChatMessage, {
      message: message({ run_id: "run-from-last-session" }),
      showMetadata: true,
    });

    expect(screen.getByText("Activity details are unavailable for this message.")).toBeTruthy();
    expect(screen.queryByText("Details")).toBeNull();
  });

  it("hides reasoning and run metadata by default", () => {
    records.set([runRecord("run-42")]);
    recordsLoaded.set(true);
    render(ChatMessage, {
      message: message({ reasoning: "Internal reasoning", run_id: "run-42" }),
    });

    expect(screen.queryByText("Internal reasoning")).toBeNull();
    expect(screen.queryByText("Details")).toBeNull();
  });

  it("keeps thinking separate from run metadata", () => {
    records.set([runRecord("run-42")]);
    recordsLoaded.set(true);
    render(ChatMessage, {
      message: message({ reasoning: "Internal reasoning", run_id: "run-42" }),
      showThinking: true,
      showMetadata: false,
    });

    expect(screen.getByText("Internal reasoning")).toBeTruthy();
    expect(screen.queryByText("Details")).toBeNull();
  });
});

describe("ChatMessage text marks", () => {
  const THREE_PARTS = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";

  function marksPayload(ranges: { part: number; start: number; end: number }[] = []) {
    return {
      kind: TEXT_MARKS_KIND,
      contract: TEXT_ANNOTATION_CONTRACT,
      groups: ranges.map((range, index) => ({ id: `group-${index + 1}`, ranges: [range] })),
      labels: { mark: "Mark as read", unmark: "Mark as unread" },
      state_revision: 3,
    };
  }

  it("hands the canonical parts to the extension slot", () => {
    render(ChatMessage, { message: message({ text: THREE_PARTS }) });
    expect(slotMock.props?.context.part_count).toBe(3);
    expect(slotMock.props?.context.parts).toEqual([
      { index: 0, excerpt: "First paragraph.", plain_text: "First paragraph." },
      { index: 1, excerpt: "Second paragraph.", plain_text: "Second paragraph." },
      { index: 2, excerpt: "Third paragraph.", plain_text: "Third paragraph." },
    ]);
  });

  it("renders one message body and no controls until an extension publishes marks", () => {
    const { container } = render(ChatMessage, { message: message({ text: THREE_PARTS }) });
    expect(container.querySelectorAll("mark[data-chat-text-mark]")).toHaveLength(0);
    expect(container.querySelector(".markdown.parts")).toBeNull();
    expect(screen.getByText("Second paragraph.")).toBeTruthy();
  });

  it("renders exact marks on the response without checkbox controls", async () => {
    const { container } = render(ChatMessage, { message: message({ text: THREE_PARTS }) });
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      marksPayload([{ part: 1, start: 0, end: 6 }]),
    );
    await tick();

    expect(container.querySelector("mark[data-chat-text-mark]")?.textContent).toBe("Second");
    expect(container.querySelectorAll("button.part-mark")).toHaveLength(0);
    expect(container.querySelectorAll(".markdown p")[1].textContent).toBe("Second paragraph.");
  });

  it("keeps native selection passive until the user chooses the mark action", async () => {
    const { container } = render(ChatMessage, { message: message({ text: THREE_PARTS }) });
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      marksPayload(),
    );
    await tick();

    const part = container.querySelectorAll<HTMLElement>(".markdown p")[1];
    const text = part.firstChild!;
    const range = document.createRange();
    range.setStart(text, 0);
    range.setEnd(text, 6);
    window.getSelection()!.removeAllRanges();
    window.getSelection()!.addRange(range);
    await fireEvent.pointerUp(document.body);

    expect(slotMock.sendExtensionEvent).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Mark as read" })).toBeTruthy();
    expect(screen.getByText("Copy still works normally.")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Mark as read" }));

    expect(slotMock.sendExtensionEvent).toHaveBeenCalledWith("app/surface", {
      kind: "message-text-selection",
      contract: TEXT_ANNOTATION_CONTRACT,
      ranges: [{ part: 1, start: 0, end: 6, text: "Second" }],
      marked: true,
    });
  });

  it("offers the unmark action when the whole selection is already marked", async () => {
    const { container } = render(ChatMessage, { message: message({ text: THREE_PARTS }) });
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      marksPayload([{ part: 1, start: 0, end: 6 }]),
    );
    await tick();

    const part = container.querySelectorAll<HTMLElement>(".markdown p")[1];
    const text = part.querySelector("mark[data-chat-text-mark]")!.firstChild!;
    const range = document.createRange();
    range.setStart(text, 0);
    range.setEnd(text, 6);
    window.getSelection()!.removeAllRanges();
    window.getSelection()!.addRange(range);
    await fireEvent(document, new Event("selectionchange"));

    await fireEvent.click(screen.getByRole("button", { name: "Mark as unread" }));
    expect(slotMock.sendExtensionEvent).toHaveBeenCalledWith(
      "app/surface",
      expect.objectContaining({ marked: false }),
    );
  });

  it("opens the add comment editor when marked text is clicked", async () => {
    const { container } = render(ChatMessage, { message: message({ text: THREE_PARTS }) });
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      {
        ...marksPayload([{ part: 1, start: 0, end: 6 }]),
        comments: [],
        comment_labels: { add: "Add comment", edit: "Edit comment" },
      },
    );
    await tick();

    const mark = container.querySelector<HTMLElement>("mark[data-chat-text-mark]")!;
    expect(mark.getAttribute("role")).toBe("button");
    expect(mark.getAttribute("aria-label")).toBe("Marked text. Activate to add a comment.");
    await fireEvent.click(mark);

    const dialog = screen.getByRole("dialog", { name: "Add reading comment" });
    expect(screen.getByRole("textbox", { name: "Comment" })).toBeTruthy();
    expect(dialog.textContent).not.toContain("Reading Insights");
    expect(dialog.textContent).not.toContain("Second");
  });

  it("supports keyboard activation for adding and editing comments", async () => {
    render(ChatMessage, { message: message({ text: THREE_PARTS }) });
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      {
        ...marksPayload([
          { part: 1, start: 0, end: 6 },
          { part: 2, start: 0, end: 5 },
        ]),
        comments: [{ id: "comment-1", ranges: [{ part: 2, start: 0, end: 5 }], text: "Review this" }],
        comment_labels: { add: "Add comment", edit: "Edit comment" },
      },
    );
    await tick();

    await fireEvent.keyDown(
      screen.getByRole("button", { name: "Marked text. Activate to add a comment." }),
      { key: "Enter" },
    );
    expect(screen.getByRole("dialog", { name: "Add reading comment" })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await tick();

    await fireEvent.keyDown(
      screen.getByRole("button", { name: "Commented text. Activate to edit comment." }),
      { key: " " },
    );
    expect(screen.getByRole("dialog", { name: "Edit reading comment" })).toBeTruthy();
  });

  it("opens a comment editor from a logical comment span and waits for app confirmation", async () => {
    const { container } = render(ChatMessage, { message: message({ text: THREE_PARTS }) });
    const state = {
      ...marksPayload([{ part: 1, start: 0, end: 6 }]),
      comments: [{ id: "comment-1", ranges: [{ part: 1, start: 0, end: 6 }], text: "Review this" }],
      comment_labels: { add: "Add comment", edit: "Edit comment" },
    };
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      state,
    );
    await tick();

    const mark = container.querySelector<HTMLElement>("mark[data-chat-text-comment]")!;
    expect(mark.getAttribute("role")).toBe("button");
    await fireEvent.click(mark);
    const dialog = screen.getByRole("dialog", { name: "Edit reading comment" });
    expect(dialog).toBeTruthy();
    const input = screen.getByRole("textbox", { name: "Comment" });
    await fireEvent.input(input, { target: { value: "Review this claim" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save comment" }));

    const [extensionKey, event] = slotMock.sendExtensionEvent.mock.calls.at(-1)!;
    expect(extensionKey).toBe("app/surface");
    expect(event).toMatchObject({
      kind: "message-text-comment",
      contract: TEXT_ANNOTATION_CONTRACT,
      action: "upsert",
      ranges: [{ part: 1, start: 0, end: 6, text: "Second" }],
      text: "Review this claim",
    });
    expect((screen.getByRole("button", { name: "Saving…" }) as HTMLButtonElement).disabled).toBe(true);

    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      {
        ...state,
        comments: [],
        comment_operation: {
          id: event.operation_id,
          status: "failed",
          error: "Backend unavailable",
        },
      },
    );
    await tick();
    expect(screen.getByRole("alert").textContent).toBe("Backend unavailable");
    expect((screen.getByRole("textbox", { name: "Comment" }) as HTMLTextAreaElement).value)
      .toBe("Review this claim");

    await fireEvent.click(screen.getByRole("button", { name: "Save comment" }));
    const retryEvent = slotMock.sendExtensionEvent.mock.calls.at(-1)![1];
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      {
        ...state,
        comments: [{
          id: retryEvent.comment_id,
          ranges: [{ part: 1, start: 0, end: 6 }],
          text: "Review this claim",
        }],
        comment_operation: { id: retryEvent.operation_id, status: "completed" },
      },
    );
    await tick();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  // The extension frame is userland code: it can be disabled mid-edit or
  // simply never answer. A pending write must therefore never be able to trap
  // the user in the dialog.
  async function openCommentEditorForSave() {
    const { container } = render(ChatMessage, { message: message({ text: THREE_PARTS }) });
    const state = {
      ...marksPayload([{ part: 1, start: 0, end: 6 }]),
      comments: [{ id: "comment-1", ranges: [{ part: 1, start: 0, end: 6 }], text: "Review this" }],
      comment_labels: { add: "Add comment", edit: "Edit comment" },
    };
    slotMock.props!.onExtensionState("app/surface", "org.example.reading", "Reading Insights", state);
    await tick();
    await fireEvent.click(container.querySelector<HTMLElement>("mark[data-chat-text-comment]")!);
    await fireEvent.input(screen.getByRole("textbox", { name: "Comment" }), {
      target: { value: "Review this claim" },
    });
    return container;
  }

  it("keeps Cancel usable while a comment write is pending", async () => {
    await openCommentEditorForSave();
    await fireEvent.click(screen.getByRole("button", { name: "Save comment" }));

    // Save is busy, but the way out must stay open.
    expect((screen.getByRole("button", { name: "Saving…" }) as HTMLButtonElement).disabled).toBe(true);
    const cancel = screen.getByRole("button", { name: "Cancel" }) as HTMLButtonElement;
    expect(cancel.disabled).toBe(false);

    await fireEvent.click(cancel);
    await tick();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("closes the comment editor on Escape even while a write is pending", async () => {
    await openCommentEditorForSave();
    await fireEvent.click(screen.getByRole("button", { name: "Save comment" }));

    await fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    await tick();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("releases a pending comment write when the extension frame is gone", async () => {
    await openCommentEditorForSave();
    slotMock.sendExtensionEvent.mockReturnValueOnce(false);

    await fireEvent.click(screen.getByRole("button", { name: "Save comment" }));
    await tick();

    // No reply is coming, so the editor must not sit on "Saving…".
    expect(screen.queryByRole("button", { name: "Saving…" })).toBeNull();
    expect(screen.getByRole("button", { name: "Save comment" })).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("isn’t available right now");
    // The typed text survives so the write can be retried.
    expect((screen.getByRole("textbox", { name: "Comment" }) as HTMLTextAreaElement).value)
      .toBe("Review this claim");
  });

  it("releases a pending comment write when the extension is removed mid-save", async () => {
    await openCommentEditorForSave();
    await fireEvent.click(screen.getByRole("button", { name: "Save comment" }));
    expect(screen.getByRole("button", { name: "Saving…" })).toBeTruthy();

    slotMock.props!.onExtensionRemoved("app/surface");
    await tick();

    expect(screen.queryByRole("button", { name: "Saving…" })).toBeNull();
    expect(screen.getByRole("alert").textContent).toContain("no longer available");
  });

  it("renders a single logical comment target across inline markdown fragments", async () => {
    const text = "Alpha `beta` gamma";
    const plainLength = splitMessageParts(text)[0].plainText.length;
    const { container } = render(ChatMessage, {
      message: message({ text }),
    });
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      {
        ...marksPayload([{ part: 0, start: 0, end: plainLength }]),
        comments: [{ id: "comment-1", ranges: [{ part: 0, start: 0, end: plainLength }], text: "Note" }],
        comment_labels: { add: "Add comment", edit: "Edit comment" },
      },
    );
    await tick();

    const commentMarks = container.querySelectorAll<HTMLElement>("mark[data-chat-text-comment]");
    expect(commentMarks.length).toBeGreaterThan(0);
    expect(container.querySelectorAll<HTMLElement>("mark[tabindex='0']")).toHaveLength(1);
    expect(container.querySelectorAll<HTMLElement>("mark[tabindex='-1']").length).toBeGreaterThan(0);

    await fireEvent.click(commentMarks[1]);
    const dialog = screen.getByRole("dialog", { name: "Edit reading comment" });
    expect((screen.getByRole("textbox", { name: "Comment" }) as HTMLTextAreaElement).value)
      .toBe("Note");
    expect(dialog.textContent).not.toContain("Alpha beta gamma");
  });

  it("warns that unmarking removes an attached comment", async () => {
    const { container } = render(ChatMessage, { message: message({ text: THREE_PARTS }) });
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      {
        ...marksPayload([{ part: 1, start: 0, end: 6 }]),
        comments: [{ id: "comment-1", ranges: [{ part: 1, start: 0, end: 6 }], text: "Remember" }],
      },
    );
    await tick();
    const text = container.querySelector("mark[data-chat-text-mark]")!.firstChild!;
    const range = document.createRange();
    range.setStart(text, 0);
    range.setEnd(text, 6);
    window.getSelection()!.removeAllRanges();
    window.getSelection()!.addRange(range);
    await fireEvent(document, new Event("selectionchange"));

    expect(screen.getByRole("button", {
      name: "Mark as unread and delete attached comment",
    })).toBeTruthy();
  });

  it("clips a drag that starts in the response and ends outside it", async () => {
    const { container } = render(ChatMessage, { message: message({ text: THREE_PARTS }) });
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      marksPayload(),
    );
    await tick();

    const part = container.querySelectorAll<HTMLElement>(".markdown p")[2];
    const text = part.firstChild!;
    const outside = document.createElement("span");
    outside.textContent = "Outside";
    container.append(outside);
    await fireEvent.pointerDown(part);
    const range = document.createRange();
    range.setStart(text, 0);
    range.setEnd(outside.firstChild!, 3);
    window.getSelection()!.removeAllRanges();
    window.getSelection()!.addRange(range);
    await fireEvent.pointerUp(outside);

    expect(screen.getByRole("button", { name: "Mark as read" })).toBeTruthy();
  });

  it("keeps adjacent marked list items in compact list rows", async () => {
    const { container } = render(ChatMessage, {
      message: message({ text: "- One\n- Two\n- Three" }),
    });
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      marksPayload([{ part: 0, start: 0, end: 3 }]),
    );
    await tick();

    expect(container.querySelectorAll(".markdown ul")).toHaveLength(1);
    expect(container.querySelectorAll(".markdown ul > li")).toHaveLength(3);
  });

  it("ignores malformed or out-of-range mark payloads", async () => {
    const { container } = render(ChatMessage, { message: message({ text: THREE_PARTS }) });
    slotMock.props!.onExtensionState("app/surface", "org.example.reading", "Reading Insights", {
      kind: TEXT_MARKS_KIND,
      contract: TEXT_ANNOTATION_CONTRACT,
      ranges: [{ part: 7, start: 0, end: 1 }],
    });
    slotMock.props!.onExtensionState("app/surface", "org.example.reading", "Reading Insights", { kind: "other" });
    await tick();
    expect(container.querySelectorAll("mark[data-chat-text-mark]")).toHaveLength(0);
  });

  it("keeps extension state out of the visible conversation", async () => {
    const { container } = render(ChatMessage, {
      message: message({ text: "Read this." }),
      assistantMessageNumber: 5,
    });
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      marksPayload([{ part: 0, start: 0, end: 4 }]),
    );
    await tick();

    expect(container.textContent).not.toContain("explicit-read");
  });

  it("clears prior marks when an app publishes unreadable state", async () => {
    const { container } = render(ChatMessage, {
      message: message({ text: THREE_PARTS }),
    });
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      marksPayload([{ part: 0, start: 0, end: 5 }]),
    );
    await tick();
    expect(container.querySelectorAll("mark[data-chat-text-mark]").length).toBeGreaterThan(0);

    // The owner replaces valid state with a payload this contract cannot read.
    // Keeping the old marks would show annotations the app no longer claims.
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      { ...marksPayload([{ part: 0, start: 0, end: 5 }]), state_revision: -1 },
    );
    await tick();

    expect(container.querySelectorAll("mark[data-chat-text-mark]")).toHaveLength(0);
  });

  it("forwards bounded observation aggregates only to apps that asked", async () => {
    const reports: boolean[] = [];
    let deliver: ((report: ReadingOpportunityReport) => void) | null = null;
    render(ChatMessage, {
      message: message({ text: THREE_PARTS }),
      readingObservation: {
        register: (_messageId, _element, receive) => {
          deliver = receive;
        },
        unregister: () => {},
        setRequested: (_messageId, requested) => reports.push(requested),
      },
    });
    await tick();
    // Nothing has asked for observation, so the controller is told not to watch.
    expect(reports).toEqual([false]);

    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      { ...marksPayload(), observe_reading_opportunity: true },
    );
    await tick();
    expect(reports.at(-1)).toBe(true);

    deliver!({
      messageId: "msg-1",
      sessionId: "session-1",
      qualifiedVisibleMs: 18_000,
      exposedMask: 0xffffff,
      firstQualifiedAt: "2026-07-01T10:00:00.000Z",
      lastQualifiedAt: "2026-07-01T10:00:18.000Z",
      final: false,
    });

    expect(slotMock.sendExtensionEvent).toHaveBeenCalledWith("app/surface", {
      kind: "message-reading-opportunity",
      contract: TEXT_ANNOTATION_CONTRACT,
      session_id: "session-1",
      qualified_visible_ms: 18_000,
      exposed_mask: 0xffffff,
      first_qualified_at: "2026-07-01T10:00:00.000Z",
      last_qualified_at: "2026-07-01T10:00:18.000Z",
      final: false,
    });
  });

  it("clears marks when the contributing app disappears", async () => {
    const { container } = render(ChatMessage, {
      message: message({ text: "Read this." }),
    });
    slotMock.props!.onExtensionState(
      "app/surface",
      "org.example.reading",
      "Reading Insights",
      {
        ...marksPayload([{ part: 0, start: 0, end: 4 }]),
        model_context: {
          root_tag: "reading-state",
          marked_tag: "read",
          unmarked_tag: "unread",
        },
      },
    );
    await tick();
    expect(container.querySelector("mark[data-chat-text-mark]")).toBeTruthy();

    slotMock.props!.onExtensionRemoved("app/surface");
    await tick();

    expect(container.querySelector("mark[data-chat-text-mark]")).toBeNull();
  });
});
