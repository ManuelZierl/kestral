const CLEAN_START_MARKER_KEY = "kestral.development-clean-start";
const KESTRAL_BROWSER_STATE_KEYS = [
  "host-theme-preference",
  "host-custom-theme-profiles",
  "host-sidebar-layout",
  "kernel.active-chat-thread",
  "kernel.pending-chat-sends",
] as const;

export function applyDevelopmentCleanStart(
  storage: Pick<Storage, "getItem" | "removeItem" | "setItem">,
  cleanStartId: string | undefined,
): boolean {
  if (!cleanStartId || storage.getItem(CLEAN_START_MARKER_KEY) === cleanStartId) return false;

  for (const key of KESTRAL_BROWSER_STATE_KEYS) storage.removeItem(key);
  storage.setItem(CLEAN_START_MARKER_KEY, cleanStartId);
  return true;
}

if (typeof localStorage !== "undefined") {
  applyDevelopmentCleanStart(
    localStorage,
    (import.meta.env.VITE_KESTRAL_CLEAN_START_ID as string | undefined)?.trim() || undefined,
  );
}
