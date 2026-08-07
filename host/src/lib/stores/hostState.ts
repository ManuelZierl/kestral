import { get, writable } from "svelte/store";
import { bootstrapStartupApps, ledgerRecords, type LedgerRecord } from "$lib/api";
import { listenHostEvent } from "$lib/hostTransport";
import { ensureChatThread, refreshChatThreads } from "$lib/stores/chatThreads";
import { refreshApps } from "$lib/stores/apps";
import { refreshArtifacts } from "$lib/stores/artifacts";
import { refreshConfig } from "$lib/stores/config";
import { refreshGrants } from "$lib/stores/grants";

export type Tab = "chat" | "apps" | "stuff" | "settings" | "system";

export const currentTab = writable<Tab>("chat");
export const activeAppId = writable<string | null>(null);
export const shellError = writable<string | null>(null);
export const records = writable<LedgerRecord[]>([]);
export const recordsLoaded = writable(false);
export const hostInitialized = writable(false);

let initialized = false;
let poller: ReturnType<typeof setInterval> | null = null;
let stateUnlisten: Promise<() => void> | null = null;
let refreshInFlight: Promise<void> | null = null;
let refreshIncludesRecords = false;
let recordsRefreshInFlight: Promise<void> | null = null;
const SAFETY_REFRESH_MS = 30_000;
const STATE_SCOPES = ["apps", "artifacts", "chat", "config", "grants", "records"] as const;
type StateScope = typeof STATE_SCOPES[number];
const pendingScopes = new Set<StateScope>();
let invalidationInFlight: Promise<void> | null = null;
// A bootstrap failure is a persistent condition, not a transient blip. Latch it
// so the routine refresh poll below cannot silently clear the banner (which
// previously made startup failures flash for one frame and vanish).
let bootstrapError: string | null = null;
// True while startup bootstrap has failed and a retry is the way forward.
// The shell renders a retry action against this, so a transient failure does
// not force the user to quit and relaunch the app.
export const bootstrapFailed = writable(false);

// Discards out-of-order poll responses so stale ledger data can't overwrite
// fresh, mainly under the HTTP transport.
let recordsSequence = 0;

export function refreshRecords(): Promise<void> {
  if (recordsRefreshInFlight) return recordsRefreshInFlight;
  const sequence = ++recordsSequence;
  const refresh = ledgerRecords().then((next) => {
    if (sequence !== recordsSequence) return;
    records.set(next);
    recordsLoaded.set(true);
  }).finally(() => {
    if (recordsRefreshInFlight === refresh) recordsRefreshInFlight = null;
  });
  recordsRefreshInFlight = refresh;
  return refresh;
}

async function refreshHostOnce(includeRecords: boolean) {
  try {
    // Kernel reads use a non-blocking mutex. Sequence them so one poll cannot
    // manufacture its own `kernel busy` failures under the split transport.
    await refreshApps();
    await refreshArtifacts();
    await refreshGrants();
    if (includeRecords) await refreshRecords();
    await refreshConfig();
    await refreshChatThreads();
    // Keep a latched bootstrap failure visible; only a clean state clears it.
    shellError.set(bootstrapError);
  } catch (error) {
    const message = String(error);
    if (!message.includes("kernel busy")) {
      shellError.set(message);
    }
  }
}

export function refreshHost(options: { includeRecords?: boolean } = {}): Promise<void> {
  const includeRecords = options.includeRecords ?? true;
  if (refreshInFlight) {
    return includeRecords && !refreshIncludesRecords
      ? refreshInFlight.then(() => refreshRecords())
      : refreshInFlight;
  }
  refreshIncludesRecords = includeRecords;
  const refresh = refreshHostOnce(includeRecords).finally(() => {
    if (refreshInFlight === refresh) {
      refreshInFlight = null;
      refreshIncludesRecords = false;
    }
  });
  refreshInFlight = refresh;
  return refresh;
}

async function refreshChangedState(scopes: ReadonlySet<StateScope>): Promise<void> {
  if (refreshInFlight) await refreshInFlight;
  try {
    if (scopes.has("apps")) await refreshApps();
    if (scopes.has("artifacts")) await refreshArtifacts();
    if (scopes.has("grants")) await refreshGrants();
    if (scopes.has("records") && get(currentTab) === "system") await refreshRecords();
    if (scopes.has("config")) await refreshConfig();
    if (scopes.has("chat")) await refreshChatThreads();
    shellError.set(bootstrapError);
  } catch (error) {
    const message = String(error);
    if (!message.includes("kernel busy")) shellError.set(message);
  }
}

function queueStateRefresh(scopes: Iterable<StateScope>): void {
  for (const scope of scopes) pendingScopes.add(scope);
  if (invalidationInFlight) return;
  const refresh = (async () => {
    while (pendingScopes.size > 0) {
      const next = new Set(pendingScopes);
      pendingScopes.clear();
      await refreshChangedState(next);
    }
  })().finally(() => {
    if (invalidationInFlight === refresh) invalidationInFlight = null;
  });
  invalidationInFlight = refresh;
}

function stateChanged(payload: { scopes?: unknown }): void {
  if (!Array.isArray(payload?.scopes)) {
    console.error("Remote host state notification is malformed", payload);
    queueStateRefresh(STATE_SCOPES);
    return;
  }
  const scopes = payload.scopes.filter(
    (scope): scope is StateScope => typeof scope === "string" && STATE_SCOPES.includes(scope as StateScope),
  );
  queueStateRefresh(scopes);
}

function refreshVisibleHost(): void {
  if (typeof document !== "undefined" && document.visibilityState !== "visible") return;
  void refreshHost({ includeRecords: get(currentTab) === "system" });
}

export async function initializeHost() {
  if (initialized) return;
  initialized = true;
  hostInitialized.set(false);
  bootstrapError = null;
  bootstrapFailed.set(false);
  shellError.set(null);
  try {
    await refreshConfig();
    await bootstrapStartupApps();
    await refreshHost();
    await ensureChatThread();
    hostInitialized.set(true);
  } catch (error) {
    const message = `Bootstrap failed: ${String(error)}`;
    // Log too: the banner may be replaced by later UI, but the console record
    // survives for diagnosis.
    console.error(message, error);
    bootstrapError = message;
    shellError.set(message);
    bootstrapFailed.set(true);
    hostInitialized.set(false);
    // Release the latch so the shell's retry action can run bootstrap again.
    initialized = false;
  }
}

export function startPolling() {
  if (poller || stateUnlisten) return;
  stateUnlisten = Promise.all([
    listenHostEvent<{ scopes?: unknown }>("host-state:changed", stateChanged),
    listenHostEvent("host-remote:event-gap", () => queueStateRefresh(STATE_SCOPES)),
  ]).then(([unlistenState, unlistenGap]) => () => {
    unlistenState();
    unlistenGap();
  });
  poller = setInterval(refreshVisibleHost, SAFETY_REFRESH_MS);
  document.addEventListener("visibilitychange", refreshVisibleHost);
}

export function stopPolling() {
  if (poller) clearInterval(poller);
  poller = null;
  document.removeEventListener("visibilitychange", refreshVisibleHost);
  const unlisten = stateUnlisten;
  stateUnlisten = null;
  void unlisten?.then((stop) => stop());
  pendingScopes.clear();
}
