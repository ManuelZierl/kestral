import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { openUrl as tauriOpenUrl } from "@tauri-apps/plugin-opener";
import { createPasskey, getPasskey } from "$lib/remotePasskeys";
import { writable } from "svelte/store";

type Unlisten = () => void;
type EventHandler<T> = (payload: T) => void;

interface RemoteEvent {
  sequence: number;
  event: string;
  payload: unknown;
}

interface RemoteEventBatch {
  instance_id: string;
  oldest_sequence: number;
  next_sequence: number;
  events: RemoteEvent[];
}

interface RemoteApprovalBatch {
  instance_id: string;
  requests: Array<{ request_id?: number }>;
}

interface RemoteEventGap {
  requested_sequence: number;
  oldest_sequence: number;
  next_sequence: number;
}

const REMOTE_URL_KEY = "host.remote.url";
const configuredUrl = (import.meta.env.VITE_HOST_API_URL as string | undefined)?.trim();
const listeners = new Map<string, Set<EventHandler<unknown>>>();
const REMOTE_EVENT_GAP = "host-remote:event-gap";
const MAX_DELIVERED_APPROVAL_IDS = 1_024;
const APPROVAL_REFRESH_RETRY_INITIAL_MS = 250;
const APPROVAL_REFRESH_RETRY_MAX_MS = 4_000;
let eventCursor = 0;
let eventInstanceId: string | null = null;
let eventSource: EventSource | null = null;
let eventGeneration = 0;
let eventRequest: { generation: number; promise: Promise<void> } | null = null;
const deliveredApprovalIds = new Set<number>();
const deliveredApprovalOrder: number[] = [];
let approvalRefreshPending = false;
let approvalRefreshRetry: ReturnType<typeof setTimeout> | null = null;
let approvalRefreshRetryDelay = APPROVAL_REFRESH_RETRY_INITIAL_MS;
let remoteAuthenticated = false;
export const remoteConnectionAuthenticated = writable(false);

function setRemoteAuthenticated(authenticated: boolean): void {
  remoteAuthenticated = authenticated;
  remoteConnectionAuthenticated.set(authenticated);
}

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function remoteUrl(): string | null {
  if (typeof sessionStorage === "undefined") return configuredUrl || null;
  return sessionStorage.getItem(REMOTE_URL_KEY) || configuredUrl || null;
}

export function isRemoteTransport(): boolean {
  return remoteUrl() !== null;
}

export function resolveHostResourceUrl(path: string): string {
  if (!path.startsWith("/")) return path;
  const baseUrl = remoteUrl();
  if (!baseUrl) throw new Error("Relative host resource URL requires a remote host connection");
  return `${normalizedUrl(baseUrl)}${path}`;
}

function normalizedUrl(value: string): string {
  return value.trim().replace(/\/+$/, "");
}

async function remoteFetch(path: string, init?: RequestInit): Promise<Response> {
  const baseUrl = remoteUrl();
  if (!baseUrl) throw new Error("Remote host connection is not configured");
  const response = await fetch(`${baseUrl}${path}`, {
    ...init,
    credentials: "include",
    headers: {
      ...(init?.body ? { "Content-Type": "application/json" } : {}),
      ...init?.headers,
    },
  });
  if (response.status === 401) setRemoteAuthenticated(false);
  return response;
}

async function authFetch(baseUrl: string, path: string, init?: RequestInit): Promise<Response> {
  return fetch(`${baseUrl}${path}`, {
    ...init,
    credentials: "include",
    headers: {
      ...(init?.body ? { "Content-Type": "application/json" } : {}),
      ...init?.headers,
    },
  });
}

async function responseValue<T>(response: Response): Promise<T> {
  const body = (await response.json()) as T | { error?: string };
  if (!response.ok) {
    const message = typeof body === "object" && body !== null && "error" in body
      ? String(body.error)
      : `Remote host request failed (${response.status})`;
    throw new Error(message);
  }
  return body as T;
}

export async function invokeHost<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!isRemoteTransport()) {
    return tauriInvoke<T>(command, args);
  }
  const response = await remoteFetch(`/api/invoke/${encodeURIComponent(command)}`, {
    method: "POST",
    body: JSON.stringify(args),
  });
  return responseValue<T>(response);
}

// Full approval prompts come only from the authoritative approvals endpoint.
// Deliver each request id exactly once so reconnect and overlapping refreshes
// never show a duplicate dialog.
function deliverApproval(request: { request_id?: unknown } | null): void {
  const requestId = request?.request_id;
  if (typeof requestId !== "number" || deliveredApprovalIds.has(requestId)) return;
  deliveredApprovalIds.add(requestId);
  deliveredApprovalOrder.push(requestId);
  if (deliveredApprovalOrder.length > MAX_DELIVERED_APPROVAL_IDS) {
    const expired = deliveredApprovalOrder.shift();
    if (expired !== undefined) deliveredApprovalIds.delete(expired);
  }
  for (const listener of listeners.get("trusted-chrome:request") ?? []) listener(request);
}

function adoptRemoteInstance(instanceId: string): boolean {
  if (eventInstanceId === null) {
    eventInstanceId = instanceId;
    return false;
  }
  if (eventInstanceId === instanceId) return false;
  eventInstanceId = instanceId;
  eventCursor = 0;
  deliveredApprovalIds.clear();
  deliveredApprovalOrder.length = 0;
  return true;
}

function resetRemoteReplayState(): void {
  eventGeneration += 1;
  eventCursor = 0;
  eventInstanceId = null;
  deliveredApprovalIds.clear();
  deliveredApprovalOrder.length = 0;
  approvalRefreshPending = false;
  approvalRefreshRetryDelay = APPROVAL_REFRESH_RETRY_INITIAL_MS;
  if (approvalRefreshRetry !== null) clearTimeout(approvalRefreshRetry);
  approvalRefreshRetry = null;
}

function reportEventGap(detail: RemoteEventGap): void {
  console.warn("Remote host events were dropped; refresh authoritative state", detail);
  for (const listener of listeners.get(REMOTE_EVENT_GAP) ?? []) listener(detail);
}

function recoverAfterCurrentRequest(): void {
  const current = eventRequest?.promise;
  if (current) {
    void current.then(() => pollEvents());
  } else {
    void pollEvents();
  }
}

function requestApprovalRefresh(): void {
  approvalRefreshPending = true;
  recoverAfterCurrentRequest();
}

function completeApprovalRefresh(): void {
  approvalRefreshPending = false;
  approvalRefreshRetryDelay = APPROVAL_REFRESH_RETRY_INITIAL_MS;
  if (approvalRefreshRetry !== null) clearTimeout(approvalRefreshRetry);
  approvalRefreshRetry = null;
}

function scheduleApprovalRefreshRetry(): void {
  if (!approvalRefreshPending || approvalRefreshRetry !== null || !isRemoteConnectionReady()) {
    return;
  }
  const generation = eventGeneration;
  approvalRefreshRetry = setTimeout(() => {
    approvalRefreshRetry = null;
    if (generation !== eventGeneration || !approvalRefreshPending) return;
    recoverAfterCurrentRequest();
  }, approvalRefreshRetryDelay);
  approvalRefreshRetryDelay = Math.min(
    approvalRefreshRetryDelay * 2,
    APPROVAL_REFRESH_RETRY_MAX_MS,
  );
}

function processRemoteEvents(
  batch: RemoteEventBatch,
  pending: RemoteApprovalBatch | null,
  generation: number,
): void {
  if (generation !== eventGeneration) return;
  const requestedSequence = eventCursor;
  const serverRestarted = adoptRemoteInstance(batch.instance_id);
  if (pending && pending.instance_id !== batch.instance_id) {
    reportEventGap({
      requested_sequence: requestedSequence,
      oldest_sequence: batch.oldest_sequence,
      next_sequence: batch.next_sequence,
    });
    return;
  }
  if (serverRestarted && requestedSequence !== 0) {
    reportEventGap({
      requested_sequence: requestedSequence,
      oldest_sequence: batch.oldest_sequence,
      next_sequence: batch.next_sequence,
    });
    recoverAfterCurrentRequest();
    return;
  }
  for (const request of pending?.requests ?? []) deliverApproval(request);
  let expectedSequence = eventCursor;
  let gap = serverRestarted || batch.oldest_sequence > expectedSequence;
  let approvalRefreshNeeded = false;
  for (const event of batch.events) {
    if (event.sequence < eventCursor) continue;
    if (event.sequence > expectedSequence) gap = true;
    expectedSequence = event.sequence + 1;
    if (event.event === "trusted-chrome:request") {
      // Remote request events intentionally contain only a wake-up id. Fetch
      // the current pending set before displaying anything: replayed signals
      // may refer to prompts that were already resolved or expired.
      approvalRefreshNeeded = true;
      continue;
    }
    for (const listener of listeners.get(event.event) ?? []) listener(event.payload);
  }
  if (expectedSequence < batch.next_sequence) gap = true;
  eventCursor = Math.max(eventCursor, batch.next_sequence);
  if (gap) {
    reportEventGap({
      requested_sequence: requestedSequence,
      oldest_sequence: batch.oldest_sequence,
      next_sequence: batch.next_sequence,
    });
  }
  if (approvalRefreshNeeded) requestApprovalRefresh();
}

async function requestRemoteEvents(generation: number): Promise<void> {
  let pending: RemoteApprovalBatch | null = null;
  try {
    try {
      const pendingResponse = await remoteFetch("/api/approvals");
      pending = await responseValue<RemoteApprovalBatch>(pendingResponse);
      if (generation !== eventGeneration) return;
      adoptRemoteInstance(pending.instance_id);
      completeApprovalRefresh();
    } catch (error) {
      if (generation !== eventGeneration) return;
      console.error("Remote host approval recovery failed", error);
      scheduleApprovalRefreshRetry();
    }
    const requestedSequence = eventCursor;
    const response = await remoteFetch(`/api/events?after=${requestedSequence}`);
    const batch = await responseValue<RemoteEventBatch>(response);
    processRemoteEvents(batch, pending, generation);
  } catch (error) {
    if (generation !== eventGeneration) return;
    console.error("Remote host event polling failed", error);
  }
}

function pollEvents(): Promise<void> {
  if (listeners.size === 0 || !isRemoteConnectionReady()) return Promise.resolve();
  const generation = eventGeneration;
  if (eventRequest?.generation === generation) return eventRequest.promise;
  const request = requestRemoteEvents(generation).finally(() => {
    if (eventRequest?.promise === request) eventRequest = null;
  });
  eventRequest = { generation, promise: request };
  return request;
}

async function flushRemoteEvents(): Promise<void> {
  // Join an in-flight poll, then perform one poll started after the command's
  // final event. This prevents a quick command from finishing and removing its
  // listener before the bounded replay feed has been drained.
  if (eventRequest?.generation === eventGeneration) await eventRequest.promise;
  await pollEvents();
}

function startEventStream(): void {
  if (eventSource !== null || !isRemoteConnectionReady()) return;
  const baseUrl = remoteUrl();
  if (!baseUrl) return;
  if (typeof EventSource === "undefined") {
    throw new Error("Remote host events require EventSource support");
  }
  const generation = eventGeneration;
  const source = new EventSource(
    `${baseUrl}/api/events/stream?after=${eventCursor}`,
    { withCredentials: true },
  );
  eventSource = source;
  source.addEventListener("open", () => {
    if (generation !== eventGeneration || source !== eventSource) return;
    void pollEvents();
  });
  source.addEventListener("remote-events", (event) => {
    if (generation !== eventGeneration || source !== eventSource) return;
    try {
      processRemoteEvents(JSON.parse((event as MessageEvent<string>).data) as RemoteEventBatch, null, generation);
    } catch (error) {
      console.error("Remote host event stream returned invalid data", error);
      void pollEvents();
    }
  });
  source.addEventListener("transport-error", (event) => {
    if (generation !== eventGeneration || source !== eventSource) return;
    console.error("Remote host event stream failed", (event as MessageEvent<string>).data);
  });
}

function stopEventStream(): void {
  eventSource?.close();
  eventSource = null;
}

function stopEventStreamIfIdle(): void {
  if (listeners.size !== 0) return;
  stopEventStream();
}

export async function listenHostEvent<T>(event: string, handler: EventHandler<T>): Promise<Unlisten> {
  if (!isRemoteTransport()) {
    return tauriListen<T>(event, ({ payload }) => handler(payload));
  }
  const handlers = listeners.get(event) ?? new Set<EventHandler<unknown>>();
  handlers.add(handler as EventHandler<unknown>);
  listeners.set(event, handlers);
  startEventStream();
  return () => {
    handlers.delete(handler as EventHandler<unknown>);
    if (handlers.size === 0) listeners.delete(event);
    stopEventStreamIfIdle();
  };
}

export function listenHostStateScope(scope: string, handler: () => void): Promise<Unlisten> {
  if (!isRemoteTransport()) return Promise.resolve(() => {});
  return listenHostEvent<{ scopes?: unknown }>("host-state:changed", (payload) => {
    if (Array.isArray(payload?.scopes) && payload.scopes.includes(scope)) handler();
  });
}

export async function invokeChatWithProgress<T, TEvent>(
  requestId: string,
  args: Record<string, unknown>,
  onStream: (event: TEvent) => void,
): Promise<T> {
  if (!isRemoteTransport()) {
    const onEvent = new Channel<TEvent>();
    onEvent.onmessage = onStream;
    return tauriInvoke<T>("send_chat_message", { ...args, requestId, onEvent });
  }
  const eventName = `chat-stream:${requestId}`;
  const unlisten = await listenHostEvent<TEvent>(eventName, onStream);
  try {
    return await invokeHost<T>("send_chat_message", { ...args, requestId });
  } finally {
    try {
      await flushRemoteEvents();
    } finally {
      unlisten();
    }
  }
}

export async function invokeHostWithProgress<T, TEvent>(
  command: string,
  args: Record<string, unknown>,
  onProgress: ((event: TEvent) => void) | undefined,
): Promise<T> {
  if (!onProgress) return invokeHost<T>(command, args);
  if (isRemoteTransport()) {
    const requestId = crypto.randomUUID();
    const unlisten = await listenHostEvent<TEvent>(`host-progress:${command}:${requestId}`, onProgress);
    try {
      return await invokeHost<T>(command, { ...args, requestId });
    } finally {
      try {
        await flushRemoteEvents();
      } finally {
        unlisten();
      }
    }
  }
  const onEvent = new Channel<TEvent>();
  onEvent.onmessage = onProgress;
  return tauriInvoke<T>(command, { ...args, onEvent });
}

export function needsRemoteConnection(): boolean {
  return !isTauriRuntime() && !isRemoteConnectionReady();
}

export function isRemoteConnectionReady(): boolean {
  return remoteUrl() !== null && remoteAuthenticated;
}

export function remoteUrlDefault(): string {
  const configured = remoteUrl();
  if (configured) return configured;
  if (typeof location !== "undefined" && /^https?:$/.test(location.protocol)) {
    return location.origin;
  }
  return "http://127.0.0.1:4310";
}

export async function openExternalUrl(url: string): Promise<void> {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    throw new Error("Sign-in URL is invalid");
  }
  if (parsed.protocol !== "https:" && parsed.protocol !== "http:") {
    throw new Error("Sign-in URL must use HTTP or HTTPS");
  }
  if (!isRemoteTransport()) {
    await tauriOpenUrl(parsed.href);
    return;
  }
  const opened = window.open(parsed.href, "_blank", "noopener,noreferrer");
  if (!opened) throw new Error("The browser blocked the sign-in window");
}

export function clearRemoteConnection(): void {
  sessionStorage.removeItem(REMOTE_URL_KEY);
  setRemoteAuthenticated(false);
  stopEventStream();
  resetRemoteReplayState();
}

export async function restoreRemoteConnection(): Promise<boolean> {
  if (isTauriRuntime()) return true;
  const baseUrl = remoteUrl();
  if (!baseUrl) return false;
  setRemoteAuthenticated(false);
  resetRemoteReplayState();
  try {
    const status = await responseValue<{ authenticated: boolean }>(
      await authFetch(baseUrl, "/api/auth/status"),
    );
    setRemoteAuthenticated(status.authenticated);
    if (remoteAuthenticated && listeners.size > 0) startEventStream();
    return status.authenticated;
  } catch {
    setRemoteAuthenticated(false);
    return false;
  }
}

export async function pairRemoteConnection(url: string, pairingCode: string): Promise<void> {
  const nextUrl = normalizedUrl(url);
  if (!/^https?:\/\//.test(nextUrl)) throw new Error("Host URL must start with http:// or https://");
  if (!pairingCode.trim()) throw new Error("Enter the pairing code shown by the host");
  setRemoteAuthenticated(false);
  resetRemoteReplayState();
  sessionStorage.setItem(REMOTE_URL_KEY, nextUrl);
  try {
    const start = await responseValue<{ ceremony_id: string; options: unknown }>(
      await authFetch(nextUrl, "/api/auth/register/start", {
        method: "POST",
        body: JSON.stringify({ pairing_code: pairingCode.trim() }),
      }),
    );
    const credential = await createPasskey(start.options);
    await responseValue(
      await authFetch(nextUrl, "/api/auth/register/finish", {
        method: "POST",
        body: JSON.stringify({ ceremony_id: start.ceremony_id, credential }),
      }),
    );
    setRemoteAuthenticated(true);
    if (listeners.size > 0) startEventStream();
  } catch (error) {
    clearRemoteConnection();
    throw error;
  }
}

export async function signInRemoteConnection(url: string): Promise<void> {
  const nextUrl = normalizedUrl(url);
  if (!/^https?:\/\//.test(nextUrl)) throw new Error("Host URL must start with http:// or https://");
  setRemoteAuthenticated(false);
  resetRemoteReplayState();
  sessionStorage.setItem(REMOTE_URL_KEY, nextUrl);
  try {
    const start = await responseValue<{ ceremony_id: string; options: unknown }>(
      await authFetch(nextUrl, "/api/auth/login/start", { method: "POST", body: "{}" }),
    );
    const credential = await getPasskey(start.options);
    await responseValue(
      await authFetch(nextUrl, "/api/auth/login/finish", {
        method: "POST",
        body: JSON.stringify({ ceremony_id: start.ceremony_id, credential }),
      }),
    );
    setRemoteAuthenticated(true);
    if (listeners.size > 0) startEventStream();
  } catch (error) {
    clearRemoteConnection();
    throw error;
  }
}

export async function signOutRemoteConnection(): Promise<void> {
  const baseUrl = remoteUrl();
  setRemoteAuthenticated(false);
  resetRemoteReplayState();
  try {
    if (baseUrl) {
      await responseValue(
        await authFetch(baseUrl, "/api/auth/logout", { method: "POST", body: "{}" }),
      );
    }
  } finally {
    clearRemoteConnection();
  }
}
