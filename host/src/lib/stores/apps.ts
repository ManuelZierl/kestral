import { writable } from "svelte/store";
import { listApps, type InstalledApp } from "$lib/api";

export const apps = writable<InstalledApp[]>([]);
export const appsLoaded = writable(false);

// Apply successful responses in request order. Starting a newer request must
// not invalidate an older success when that newer request later fails because
// the kernel is temporarily busy.
let nextAppsSequence = 0;
let appliedAppsSequence = 0;

export async function refreshApps() {
  const sequence = ++nextAppsSequence;
  const next = await listApps();
  if (sequence < appliedAppsSequence) return;
  appliedAppsSequence = sequence;
  apps.set(next);
  appsLoaded.set(true);
}
