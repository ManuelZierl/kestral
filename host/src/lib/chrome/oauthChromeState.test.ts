import { describe, expect, it } from "vitest";

import type { LlmOAuthEvent } from "$lib/api";
import {
  applyOAuthEvent,
  registerOAuthSession,
  waitingOAuthSession,
} from "$lib/chrome/oauthChromeState";

describe("oauthChromeState", () => {
  it("handles every event variant while preserving recovery details", () => {
    const events: LlmOAuthEvent[] = [
      { kind: "auth-url", session_id: "one", url: "https://login.example", instructions: "Use your account" },
      {
        kind: "device-code",
        session_id: "one",
        user_code: "ABCD-EFGH",
        verification_uri: "https://device.example",
        interval_seconds: 5,
        expires_in_seconds: 600,
      },
      { kind: "progress", session_id: "one", message: "Waiting for browser sign-in" },
      {
        kind: "prompt",
        session_id: "one",
        prompt_id: "prompt-1",
        prompt: { type: "text", message: "Account", placeholder: null },
      },
      { kind: "completed", session_id: "one" },
    ];
    const completed = events.reduce(applyOAuthEvent, []);

    expect(completed[0]).toMatchObject({
      status: "completed",
      authUrl: { url: "https://login.example" },
      deviceCode: { userCode: "ABCD-EFGH" },
      prompt: null,
    });

    const failed = applyOAuthEvent(
      [waitingOAuthSession("failed")],
      { kind: "failed", session_id: "failed", message: "Provider unavailable" },
    );
    expect(failed[0]).toMatchObject({ status: "failed", failure: "Provider unavailable" });
  });

  it("queues concurrent sessions and does not duplicate a session registered after its first event", () => {
    let sessions = applyOAuthEvent([], {
      kind: "progress",
      session_id: "event-first",
      message: "Starting",
    });
    sessions = registerOAuthSession(sessions, "event-first");
    sessions = registerOAuthSession(sessions, "second");

    expect(sessions.map((session) => session.sessionId)).toEqual(["event-first", "second"]);
  });
});
