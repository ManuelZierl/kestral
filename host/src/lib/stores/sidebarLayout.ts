import { writable } from "svelte/store";
import "$lib/developmentCleanStart";

export interface SidebarDestination {
  id: string;
  label: string;
  kind: "host" | "app";
}

export interface SidebarLayout {
  collapsed: boolean;
  order: string[];
  hidden: string[];
}

interface StoredSidebarLayout extends SidebarLayout {
  version: 2;
}

export const SIDEBAR_LAYOUT_STORAGE_KEY = "host-sidebar-layout";

const HOST_DESTINATION_IDS = new Set([
  "host:chat",
  "host:apps",
  "host:stuff",
  "host:settings",
  "host:system",
]);
const MAX_DESTINATIONS = 256;
const DEFAULT_HIDDEN_DESTINATIONS = ["app:mcp-kestral-docs"];
const defaultLayout: SidebarLayout = {
  collapsed: false,
  order: [],
  hidden: [...DEFAULT_HIDDEN_DESTINATIONS],
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const expected = [...keys].sort();
  const actual = Object.keys(value).sort();
  return actual.length === expected.length && actual.every((key, index) => key === expected[index]);
}

function isDestinationId(value: unknown): value is string {
  if (typeof value !== "string") return false;
  if (HOST_DESTINATION_IDS.has(value)) return true;
  const appId = value.startsWith("app:") ? value.slice("app:".length) : "";
  return appId.length > 0 && !appId.match(/\s/);
}

function parseDestinationIds(value: unknown): string[] | null {
  if (!Array.isArray(value) || value.length > MAX_DESTINATIONS || !value.every(isDestinationId)) return null;
  return new Set(value).size === value.length ? value : null;
}

export function parseStoredSidebarLayout(source: string): SidebarLayout {
  const value: unknown = JSON.parse(source);
  if (!isRecord(value) || !hasExactKeys(value, ["version", "collapsed", "order", "hidden"])) {
    throw new Error("Sidebar customization uses an unsupported storage format.");
  }
  const order = parseDestinationIds(value.order);
  const hidden = parseDestinationIds(value.hidden);
  const supportedVersion = value.version === 1 || value.version === 2;
  if (!supportedVersion || typeof value.collapsed !== "boolean" || !order || !hidden) {
    throw new Error("Saved sidebar customization is invalid.");
  }
  if (value.version === 1) {
    for (const id of DEFAULT_HIDDEN_DESTINATIONS) {
      if (!hidden.includes(id)) hidden.push(id);
    }
  }
  return { collapsed: value.collapsed, order, hidden };
}

function loadSidebarLayout(): { layout: SidebarLayout; error: string | null } {
  if (typeof localStorage === "undefined") return { layout: defaultLayout, error: null };
  const stored = localStorage.getItem(SIDEBAR_LAYOUT_STORAGE_KEY);
  if (!stored) return { layout: defaultLayout, error: null };
  try {
    return { layout: parseStoredSidebarLayout(stored), error: null };
  } catch (failure) {
    return { layout: defaultLayout, error: (failure as Error).message };
  }
}

const loaded = loadSidebarLayout();
let ready = false;

export const sidebarLayoutStorageError = writable<string | null>(loaded.error);
export const sidebarLayout = writable<SidebarLayout>(loaded.layout);

sidebarLayout.subscribe((layout) => {
  if (!ready) {
    ready = true;
    return;
  }
  const stored: StoredSidebarLayout = { version: 2, ...layout };
  localStorage.setItem(SIDEBAR_LAYOUT_STORAGE_KEY, JSON.stringify(stored));
  sidebarLayoutStorageError.set(null);
});

export function arrangeSidebarDestinations<T extends SidebarDestination>(
  destinations: readonly T[],
  layout: SidebarLayout,
): T[] {
  const byId = new Map(destinations.map((destination) => [destination.id, destination]));
  const ordered = layout.order.flatMap((id) => {
    const destination = byId.get(id);
    if (!destination) return [];
    byId.delete(id);
    return [destination];
  });
  return [...ordered, ...byId.values()];
}

export function setSidebarCollapsed(collapsed: boolean): void {
  sidebarLayout.update((layout) => ({ ...layout, collapsed }));
}

export function setSidebarDestinationHidden(id: string, hidden: boolean): void {
  if (!isDestinationId(id)) throw new Error("Cannot save an unknown sidebar destination.");
  sidebarLayout.update((layout) => ({
    ...layout,
    hidden: hidden
      ? [...layout.hidden.filter((candidate) => candidate !== id), id]
      : layout.hidden.filter((candidate) => candidate !== id),
  }));
}

export function setSidebarDestinationOrder(ids: readonly string[]): void {
  const order = parseDestinationIds([...ids]);
  if (!order) throw new Error("Cannot save an invalid sidebar order.");
  sidebarLayout.update((layout) => ({ ...layout, order }));
}

export function resetSidebarLayout(): void {
  sidebarLayout.set(defaultLayout);
  sidebarLayoutStorageError.set(null);
  localStorage.removeItem(SIDEBAR_LAYOUT_STORAGE_KEY);
}
