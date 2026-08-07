import { writable } from "svelte/store";

export const pendingChromeRequests = writable(0);
export const startedOAuthSessions = writable<string[]>([]);

export interface OAuthSessionResult {
  sessionId: string;
  status: "completed" | "failed";
  message: string | null;
}

export const oauthSessionResults = writable<OAuthSessionResult[]>([]);

export function setPendingChromeRequests(count: number) {
  pendingChromeRequests.set(Math.max(0, count));
}

export function registerStartedOAuthSession(sessionId: string) {
  startedOAuthSessions.update((sessions) =>
    sessions.includes(sessionId) ? sessions : [...sessions, sessionId],
  );
}

export function forgetStartedOAuthSession(sessionId: string) {
  startedOAuthSessions.update((sessions) => sessions.filter((candidate) => candidate !== sessionId));
}

export function recordOAuthSessionResult(result: OAuthSessionResult) {
  oauthSessionResults.update((results) => [
    ...results.filter((candidate) => candidate.sessionId !== result.sessionId),
    result,
  ].slice(-20));
}

export function forgetOAuthSessionResult(sessionId: string) {
  oauthSessionResults.update((results) => results.filter((candidate) => candidate.sessionId !== sessionId));
}
