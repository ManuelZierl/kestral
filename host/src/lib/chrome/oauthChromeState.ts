import type { LlmOAuthEvent, LlmOAuthPrompt } from "$lib/api";

export type OAuthSessionStatus = "waiting" | "active" | "completed" | "failed";

export interface OAuthSessionState {
  sessionId: string;
  status: OAuthSessionStatus;
  authUrl: { url: string; instructions: string | null } | null;
  deviceCode: {
    userCode: string;
    verificationUri: string;
    intervalSeconds: number | null;
    expiresInSeconds: number | null;
  } | null;
  progress: string | null;
  prompt: { promptId: string; value: LlmOAuthPrompt } | null;
  failure: string | null;
}

export function waitingOAuthSession(sessionId: string): OAuthSessionState {
  return {
    sessionId,
    status: "waiting",
    authUrl: null,
    deviceCode: null,
    progress: "Waiting for the sign-in provider…",
    prompt: null,
    failure: null,
  };
}

export function applyOAuthEvent(
  sessions: OAuthSessionState[],
  event: LlmOAuthEvent,
): OAuthSessionState[] {
  const index = sessions.findIndex((session) => session.sessionId === event.session_id);
  const current = index === -1 ? waitingOAuthSession(event.session_id) : sessions[index];
  const next = reduceOAuthSession(current, event);
  if (index === -1) {
    return [...sessions, next];
  }
  return sessions.map((session, sessionIndex) => (sessionIndex === index ? next : session));
}

export function registerOAuthSession(
  sessions: OAuthSessionState[],
  sessionId: string,
): OAuthSessionState[] {
  return sessions.some((session) => session.sessionId === sessionId)
    ? sessions
    : [...sessions, waitingOAuthSession(sessionId)];
}

function reduceOAuthSession(current: OAuthSessionState, event: LlmOAuthEvent): OAuthSessionState {
  switch (event.kind) {
    case "auth-url":
      return {
        ...current,
        status: "active",
        authUrl: { url: event.url, instructions: event.instructions },
        progress: event.instructions ?? current.progress,
        failure: null,
      };
    case "device-code":
      return {
        ...current,
        status: "active",
        deviceCode: {
          userCode: event.user_code,
          verificationUri: event.verification_uri,
          intervalSeconds: event.interval_seconds,
          expiresInSeconds: event.expires_in_seconds,
        },
        failure: null,
      };
    case "progress":
      return { ...current, status: "active", progress: event.message, failure: null };
    case "prompt":
      return {
        ...current,
        status: "active",
        prompt: { promptId: event.prompt_id, value: event.prompt },
        failure: null,
      };
    case "completed":
      return { ...current, status: "completed", progress: null, prompt: null, failure: null };
    case "failed":
      return { ...current, status: "failed", progress: null, prompt: null, failure: event.message };
  }
}
