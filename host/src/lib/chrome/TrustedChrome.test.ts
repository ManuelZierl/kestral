import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DataScope, LlmOAuthEvent } from "$lib/api";
import {
  oauthSessionResults,
  pendingChromeRequests,
  startedOAuthSessions,
} from "$lib/stores/chromeState";
import { currentTab } from "$lib/stores/hostState";
import { activityTarget, permissionTarget } from "$lib/stores/navigation";
import { get } from "svelte/store";
import TrustedChrome from "$lib/chrome/TrustedChrome.svelte";

const mocks = vi.hoisted(() => ({
  handlers: new Map<string, (payload: unknown) => void>(),
  openUrl: vi.fn(),
  resolvePrompt: vi.fn(),
  cancelOAuth: vi.fn(),
  resolveApproval: vi.fn(),
  resolveInstallApproval: vi.fn(),
}));

vi.mock("$lib/hostTransport", () => ({
  listenHostEvent: vi.fn(async (event: string, handler: (payload: unknown) => void) => {
    mocks.handlers.set(event, handler);
    return () => mocks.handlers.delete(event);
  }),
  openExternalUrl: mocks.openUrl,
}));

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    resolveApproval: mocks.resolveApproval,
    resolveInstallApproval: mocks.resolveInstallApproval,
    resolveLlmOAuthPrompt: mocks.resolvePrompt,
    cancelLlmOAuth: mocks.cancelOAuth,
  };
});

function emit(event: LlmOAuthEvent) {
  mocks.handlers.get("trusted-chrome:oauth")?.(event);
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.handlers.clear();
  mocks.openUrl.mockResolvedValue(undefined);
  mocks.resolvePrompt.mockResolvedValue(undefined);
  mocks.cancelOAuth.mockResolvedValue(undefined);
  mocks.resolveApproval.mockResolvedValue(undefined);
  mocks.resolveInstallApproval.mockResolvedValue(undefined);
  startedOAuthSessions.set([]);
  oauthSessionResults.set([]);
  currentTab.set("chat");
  activityTarget.set(null);
  permissionTarget.set(null);
});

describe("TrustedChrome notices", () => {
  it("opens the matching activity and exact permission from a grant-use notice", async () => {
    render(TrustedChrome, { onReady: vi.fn() });
    await waitFor(() => expect(mocks.handlers.has("trusted-chrome:notice")).toBe(true));
    mocks.handlers.get("trusted-chrome:notice")?.({
      sequence: 8,
      recorded_at: "2026-07-25T12:00:00Z",
      acknowledged_at: null,
      notice: {
        kind: "grant-use",
        app_id: "chat",
        capability: { provider: "notes", capability: "create" },
        grant_id: "grant-exact",
        run_id: "run-8",
      },
    });

    await fireEvent.click(await screen.findByRole("button", { name: /View activity: chat used notes\/create/ }));
    expect(get(currentTab)).toBe("system");
    expect(get(activityTarget)).toMatchObject({ runId: "run-8", grantId: "grant-exact" });

    await fireEvent.click(screen.getByRole("button", { name: "Open the permission used by chat" }));
    expect(get(currentTab)).toBe("settings");
    expect(get(permissionTarget)).toMatchObject({ grantId: "grant-exact" });
  });
});

describe("TrustedChrome install checklist", () => {
  const grant = (capability: string, dataScope: DataScope = { kind: "none" }) => ({
    app_id: "reader",
    app_display_name: "Reader",
    scope: { kind: "exact-capability" as const, provider: "files", capability },
    data_scope: dataScope,
    condition: "silent" as const,
    duration: { kind: "non-expiring" as const },
    reason: `Use ${capability}`,
  });

  function emitInstall() {
    mocks.handlers.get("trusted-chrome:request")?.({
      kind: "install-approval",
      request_id: 7,
      prompt: {
        app_id: "reader",
        app_display_name: "Reader",
        event: null,
        grants: [grant("read"), grant("write")],
      },
    });
  }

  it("grants every checked permission of one app in a single decision", async () => {
    render(TrustedChrome, { onReady: vi.fn() });
    await waitFor(() => expect(mocks.handlers.has("trusted-chrome:request")).toBe(true));
    emitInstall();

    expect(await screen.findByRole("heading", { name: "Review requested permissions" })).toBeTruthy();
    expect(screen.getAllByText("For Reader")).toHaveLength(2);
    // Both grants start checked; uncheck the second, then grant.
    const checkboxes = await screen.findAllByRole("checkbox");
    expect(checkboxes).toHaveLength(2);
    await fireEvent.click(checkboxes[1]);
    await fireEvent.click(screen.getByRole("button", { name: "Grant selected" }));

    expect(mocks.resolveInstallApproval).toHaveBeenCalledWith(7, null, [true, false]);
  });

  it("denies the whole app request with one action", async () => {
    render(TrustedChrome, { onReady: vi.fn() });
    await waitFor(() => expect(mocks.handlers.has("trusted-chrome:request")).toBe(true));
    emitInstall();

    await fireEvent.click(await screen.findByRole("button", { name: "Don't allow" }));
    expect(mocks.resolveInstallApproval).toHaveBeenCalledWith(7, null, [false, false]);
  });

  it("requires an explicit selection for all current and future resources", async () => {
    render(TrustedChrome, { onReady: vi.fn() });
    await waitFor(() => expect(mocks.handlers.has("trusted-chrome:request")).toBe(true));
    mocks.handlers.get("trusted-chrome:request")?.({
      kind: "install-approval",
      request_id: 8,
      prompt: {
        app_id: "reader",
        app_display_name: "Reader",
        event: null,
        grants: [grant("read", { kind: "all-resources" as const })],
      },
    });

    expect(await screen.findByText("Data scope: All current and future resources")).toBeTruthy();
    expect(screen.getByText("Broad data access, including resources added later.")).toBeTruthy();
    expect((await screen.findByRole("checkbox") as HTMLInputElement).checked).toBe(false);
  });

  it("removes an approval when the backend timeout denies it", async () => {
    render(TrustedChrome, { onReady: vi.fn() });
    await waitFor(() => expect(mocks.handlers.has("trusted-chrome:request-expired")).toBe(true));
    emitInstall();
    expect(await screen.findByRole("heading", { name: "Review requested permissions" })).toBeTruthy();
    expect(get(pendingChromeRequests)).toBe(1);

    mocks.handlers.get("trusted-chrome:request-expired")?.(7);

    await waitFor(() => expect(screen.queryByRole("heading", { name: "Review requested permissions" })).toBeNull());
    expect(get(pendingChromeRequests)).toBe(0);
  });
});

describe("TrustedChrome OAuth", () => {
  it("preserves the safe approval decision path", async () => {
    render(TrustedChrome, { onReady: vi.fn() });
    await waitFor(() => expect(mocks.handlers.has("trusted-chrome:request")).toBe(true));
    mocks.handlers.get("trusted-chrome:request")?.({
      kind: "capability-approval",
      request_id: 42,
      prompt: {
        app_id: "notes",
        app_display_name: "Notes",
        capability: { provider: "files", capability: "write" },
        data_scope: { kind: "none" },
        grant_id: "grant-1",
        run_id: "run-1",
        goal: "Save a note",
      },
    });

    await fireEvent.click(await screen.findByRole("button", { name: "Don't allow" }));
    expect(mocks.resolveApproval).toHaveBeenCalledWith(42, false);
  });

  it("does not auto-open an auth URL and exposes explicit manual recovery", async () => {
    const onReady = vi.fn();
    render(TrustedChrome, { onReady });
    await waitFor(() => expect(onReady).toHaveBeenCalledOnce());

    emit({
      kind: "auth-url",
      session_id: "session-1",
      url: "https://login.example/oauth",
      instructions: null,
    });

    expect(await screen.findByText("https://login.example/oauth")).toBeTruthy();
    expect(mocks.openUrl).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole("button", { name: "Open sign-in page" }));
    expect(mocks.openUrl).toHaveBeenCalledWith("https://login.example/oauth");

    emit({
      kind: "device-code",
      session_id: "session-1",
      user_code: "ABCD-EFGH",
      verification_uri: "https://device.example",
      interval_seconds: 5,
      expires_in_seconds: 600,
    });
    expect(await screen.findByText("ABCD-EFGH")).toBeTruthy();
    expect(screen.getByText("https://device.example")).toBeTruthy();
  });

  it("correlates prompt submit and safe cancel without echoing a secret", async () => {
    render(TrustedChrome, { onReady: vi.fn() });
    await waitFor(() => expect(mocks.handlers.has("trusted-chrome:oauth")).toBe(true));

    emit({
      kind: "prompt",
      session_id: "session-secret",
      prompt_id: "secret-prompt",
      prompt: { type: "secret", message: "Account secret", placeholder: null },
    });
    const secret = await screen.findByLabelText("Account secret");
    expect(secret.getAttribute("type")).toBe("password");
    await fireEvent.input(secret, { target: { value: "not-for-display" } });
    await fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    expect(mocks.resolvePrompt).toHaveBeenCalledWith(
      "session-secret",
      "secret-prompt",
      "not-for-display",
      false,
    );
    expect(screen.queryByDisplayValue("not-for-display")).toBeNull();

    emit({
      kind: "prompt",
      session_id: "session-secret",
      prompt_id: "manual-prompt",
      prompt: { type: "manual_code", message: "Paste the code", placeholder: "Code" },
    });
    await screen.findByLabelText("Paste the code");
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(mocks.resolvePrompt).toHaveBeenLastCalledWith(
      "session-secret",
      "manual-prompt",
      null,
      true,
    );

    emit({
      kind: "prompt",
      session_id: "session-secret",
      prompt_id: "select-prompt",
      prompt: {
        type: "select",
        message: "Choose an account",
        options: [{ id: "work", label: "Work", description: "Company account" }],
      },
    });
    await fireEvent.click(await screen.findByRole("radio", { name: /Work/ }));
    await fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(mocks.resolvePrompt).toHaveBeenLastCalledWith(
      "session-secret",
      "select-prompt",
      "work",
      false,
    );
  });

  it("keeps a failed prompt response visible and allows retry", async () => {
    mocks.resolvePrompt.mockRejectedValueOnce(new Error("temporarily busy"));
    render(TrustedChrome, { onReady: vi.fn() });
    await waitFor(() => expect(mocks.handlers.has("trusted-chrome:oauth")).toBe(true));
    emit({
      kind: "prompt",
      session_id: "retry-session",
      prompt_id: "retry-prompt",
      prompt: { type: "text", message: "Email", placeholder: null },
    });

    const input = await screen.findByLabelText("Email");
    await fireEvent.input(input, { target: { value: "person@example.com" } });
    await fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect((await screen.findByRole("alert")).textContent).toContain("temporarily busy");
    expect(screen.getByLabelText("Email")).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(mocks.resolvePrompt).toHaveBeenCalledTimes(2);
  });

  it("Escape cancels an active OAuth session", async () => {
    render(TrustedChrome, { onReady: vi.fn() });
    await waitFor(() => expect(mocks.handlers.has("trusted-chrome:oauth")).toBe(true));
    emit({ kind: "progress", session_id: "escape-session", message: "Waiting" });

    await fireEvent.keyDown(await screen.findByRole("alertdialog"), { key: "Escape" });
    expect(mocks.cancelOAuth).toHaveBeenCalledWith("escape-session");
  });

  it("announces completion and keeps terminal failures closable", async () => {
    render(TrustedChrome, { onReady: vi.fn() });
    await waitFor(() => expect(mocks.handlers.has("trusted-chrome:oauth")).toBe(true));
    emit({ kind: "completed", session_id: "complete-session" });

    const completion = await screen.findByText(/model account is connected/);
    expect(completion.getAttribute("aria-live")).toBe("assertive");
    await fireEvent.click(screen.getByRole("button", { name: "Close" }));

    emit({ kind: "failed", session_id: "failed-session", message: "Provider rejected sign-in" });
    expect((await screen.findByRole("alert")).textContent).toContain("Provider rejected sign-in");
    expect(screen.getByRole("button", { name: "Close" })).toBeTruthy();
  });
});
