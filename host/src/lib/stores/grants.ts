import { get, writable } from "svelte/store";
import {
  issueEditorGrant,
  listGrants,
  replaceGrant,
  requestAppGrants,
  requestManifestGrant,
  revokeGrant,
  type GrantEditorRequest,
  type GrantRequest,
  type GrantView,
} from "$lib/api";

export const grants = writable<GrantView[]>([]);
export const grantsRevision = writable(0);
export const grantsLoaded = writable(false);

interface RefreshBatch {
  forceRevision: boolean;
  promise: Promise<void>;
}

let refreshBatch: RefreshBatch | null = null;
const BUSY_REFRESH_ATTEMPTS = 3;
const BUSY_REFRESH_DELAY_MS = 250;

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function listGrantsAfterContention(): Promise<GrantView[]> {
  for (let attempt = 1; ; attempt += 1) {
    try {
      return await listGrants();
    } catch (failure) {
      if (attempt >= BUSY_REFRESH_ATTEMPTS || !String(failure).includes("kernel busy")) {
        throw failure;
      }
      await delay(BUSY_REFRESH_DELAY_MS);
    }
  }
}

function grantsKey(value: GrantView[]): string {
  return JSON.stringify([...value].sort((left, right) => left.grant_id.localeCompare(right.grant_id)));
}

export function refreshGrants(forceRevision = false): Promise<void> {
  if (refreshBatch) {
    refreshBatch.forceRevision ||= forceRevision;
    return refreshBatch.promise;
  }

  const batch: RefreshBatch = { forceRevision, promise: Promise.resolve() };
  batch.promise = (async () => {
    // Defer the API call until after refreshBatch owns this operation, including
    // the unlikely case where a transport throws before returning a promise.
    await Promise.resolve();
    try {
      const next = await listGrantsAfterContention();
      const changed = grantsKey(get(grants)) !== grantsKey(next);
      if (changed) grants.set(next);
      if (changed || batch.forceRevision) grantsRevision.update((value) => value + 1);
      grantsLoaded.set(true);
    } finally {
      if (refreshBatch === batch) refreshBatch = null;
    }
  })();
  refreshBatch = batch;
  return batch.promise;
}

export async function revokeGrantAndRefresh(grantId: string) {
  await revokeGrant(grantId);
  await refreshGrants(true);
}

export async function requestAppGrantsAndRefresh(appId: string) {
  await requestAppGrants(appId);
  await refreshGrants(true);
}

export async function requestManifestGrantAndRefresh(appId: string, request: GrantRequest) {
  await requestManifestGrant(appId, request);
  await refreshGrants(true);
}

export async function issueEditorGrantAndRefresh(request: GrantEditorRequest) {
  await issueEditorGrant(request);
  await refreshGrants(true);
}

export async function replaceGrantAndRefresh(grantId: string, request: GrantEditorRequest) {
  await replaceGrant(grantId, request);
  await refreshGrants(true);
}
