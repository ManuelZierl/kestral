<script lang="ts">
  import { trustedNotices } from "$lib/stores/trustedNotices";
  import type { TrustedNoticeRecord } from "$lib/api";
  import { openActivity, openPermission } from "$lib/stores/navigation";

  function summary(record: TrustedNoticeRecord) {
    if (record.notice.kind === "grant-use") {
      return `${record.notice.app_id} used ${record.notice.capability.provider}/${record.notice.capability.capability}`;
    }
    return `Lease conflict on ${record.notice.resource}`;
  }

  function openNoticeActivity(record: TrustedNoticeRecord) {
    if (record.notice.kind !== "grant-use") return;
    openActivity(record.notice.run_id, record.notice.grant_id);
  }

  function openNoticePermission(record: TrustedNoticeRecord) {
    if (record.notice.kind !== "grant-use") return;
    openPermission(record.notice.grant_id);
  }
</script>

{#if $trustedNotices.length === 0}
  <p class="empty">No trusted notices yet in this session.</p>
{:else}
  <div class="list">
    {#each $trustedNotices as record}
      <article class="notice">
        {#if record.notice.kind === "grant-use"}
          <div class="notice-main">
            <button
              type="button"
              class="summary"
              onclick={() => openNoticeActivity(record)}
            >{summary(record)}</button>
              <button
                type="button"
                class="settings"
                onclick={() => openNoticePermission(record)}
                aria-label={`Open the permission used by ${record.notice.app_id}`}
                title="Open permission settings"
              >
                <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
                  <path d="M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Z" />
                  <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.86 2.86-.06-.06A1.7 1.7 0 0 0 15 19.4a1.7 1.7 0 0 0-1 .6 1.7 1.7 0 0 0-.4 1.1V21H9.5v-.1A1.7 1.7 0 0 0 8.4 19.4a1.7 1.7 0 0 0-1.88.34l-.06.06-2.86-2.86.06-.06A1.7 1.7 0 0 0 4 15a1.7 1.7 0 0 0-.6-1 1.7 1.7 0 0 0-1.1-.4H2V9.5h.3A1.7 1.7 0 0 0 4 8.4a1.7 1.7 0 0 0-.34-1.88l-.06-.06L6.46 3.6l.06.06A1.7 1.7 0 0 0 8.4 4a1.7 1.7 0 0 0 1-.6A1.7 1.7 0 0 0 9.8 2H14v.3A1.7 1.7 0 0 0 15 4a1.7 1.7 0 0 0 1.88-.34l.06-.06 2.86 2.86-.06.06A1.7 1.7 0 0 0 19.4 8.4a1.7 1.7 0 0 0 .6 1 1.7 1.7 0 0 0 1.1.4h.1V14h-.1a1.7 1.7 0 0 0-1.7 1Z" />
                </svg>
              </button>
          </div>
        {:else}
          <div class="summary-text">{summary(record)}</div>
        {/if}
        <div class="meta">
          <time datetime={record.recorded_at}>
            {new Date(record.recorded_at).toLocaleString()}
          </time>
          <span>#{record.sequence}</span>
        </div>
      </article>
    {/each}
  </div>
{/if}

<style>
  .empty {
    color: var(--color-text-muted);
    margin: 0;
  }
  .list {
    display: grid;
    gap: 0.75rem;
  }
  .notice {
    border: 1px solid var(--color-border);
    border-radius: 14px;
    padding: 0.8rem 0.9rem;
    background: var(--color-surface-muted);
  }
  .notice-main {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .summary,
  .settings {
    border: 0;
    background: transparent;
    color: var(--color-text);
    font: inherit;
  }
  .summary {
    flex: 1 1 auto;
    min-width: 0;
    padding: 0;
    text-align: left;
  }
  .summary,
  .summary-text {
    font-weight: 600;
    color: var(--color-text);
  }
  .settings {
    flex: 0 0 2rem;
    min-height: 2rem;
    padding: 0.25rem;
    border-radius: 8px;
    display: grid;
    place-items: center;
  }
  .settings svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .summary:focus-visible,
  .settings:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  .meta {
    margin-top: 0.35rem;
    display: flex;
    justify-content: space-between;
    gap: 0.75rem;
    color: var(--color-text-muted);
    font-size: 0.9rem;
  }
</style>
