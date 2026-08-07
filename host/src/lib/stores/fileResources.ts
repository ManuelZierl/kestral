import { writable } from "svelte/store";
import {
  grantFileResourceAccess,
  listTrustedFileResources,
  registerFileResource,
  removeFileResource,
  type FileResourceGrantOperation,
  type TrustedFileResourceView,
} from "$lib/api";

export const fileResources = writable<TrustedFileResourceView[]>([]);
export const fileResourcesLoaded = writable(false);

export async function refreshFileResources() {
  fileResources.set(await listTrustedFileResources());
  fileResourcesLoaded.set(true);
}

export async function registerFileResourceAndRefresh(path: string) {
  await registerFileResource(path);
  await refreshFileResources();
}

export async function removeFileResourceAndRefresh(resourceId: string) {
  await removeFileResource(resourceId);
  await refreshFileResources();
}

export async function grantFileResourceAccessAndRefresh(
  holder: string,
  resourceId: string,
  operations: FileResourceGrantOperation[],
) {
  await grantFileResourceAccess(holder, resourceId, operations);
}
