import { writable } from "svelte/store";
import { listTrustedNotices, type TrustedNoticeRecord } from "$lib/api";

export const trustedNotices = writable<TrustedNoticeRecord[]>([]);

export async function refreshTrustedNotices() {
  trustedNotices.set(await listTrustedNotices());
}

export function appendTrustedNotice(record: TrustedNoticeRecord) {
  trustedNotices.update((current) => {
    if (current.some((existing) => existing.sequence === record.sequence)) {
      return current;
    }
    return [record, ...current];
  });
}
