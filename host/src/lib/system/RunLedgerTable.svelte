<script lang="ts">
  // The run ledger presented as runs — the unit people reason about — with
  // every raw event kept behind a per-run disclosure. Read-only inspector.
  import { onDestroy, tick } from "svelte";
  import { records, recordsLoaded, shellError } from "$lib/stores/hostState";
  import { activityTarget } from "$lib/stores/navigation";
  import { apps } from "$lib/stores/apps";
  import { groupRuns, runStateLabel, type RunGroup } from "$lib/system/runGroups";
  import { ledgerEventSummary, ledgerEventDetail } from "$lib/system/LedgerEventSummary";
  import LoadingIndicator from "$lib/shell/LoadingIndicator.svelte";
  import { scrollTargetIntoView } from "$lib/a11y/scroll";

  const runs = $derived(groupRuns($records));
  let handledRequest = 0;
  let highlightedRunId = $state<string | null>(null);
  let highlightedEventSequence = $state<number | null>(null);
  let highlightTimer: ReturnType<typeof setTimeout> | null = null;

  function appName(appId: string): string {
    const app = $apps.find((candidate) => candidate.manifest.app_id === appId);
    return app?.manifest.display_name ?? appId;
  }

  function whoLabel(run: RunGroup): string {
    return run.initiator ? appName(run.initiator.app_id) : "Unknown app";
  }

  function stateClass(run: RunGroup): string {
    return `state-${runStateLabel(run)}`;
  }

  $effect(() => {
    const target = $activityTarget;
    if (!target || target.request === handledRequest) return;
    const run = runs.find((candidate) => candidate.runId === target.runId);
    if (!run) return;
    const event = target.grantId
      ? run.events.find(
          (record) =>
            record.event.kind === "capability-invoked" &&
            record.event.grant_id === target.grantId,
        )
      : null;
    if (target.grantId && !event) return;

    handledRequest = target.request;
    highlightedRunId = run.runId;
    highlightedEventSequence = event?.sequence ?? null;
    if (highlightTimer) clearTimeout(highlightTimer);
    void tick().then(() => {
      const runElement = document.getElementById(`activity-run-${run.runId}`);
      const targetElement = event
        ? document.getElementById(`activity-event-${event.sequence}`)
        : runElement;
      const details = runElement?.querySelector("details");
      if (details) details.open = true;
      targetElement?.focus({ preventScroll: true });
      scrollTargetIntoView(targetElement);
    });
    highlightTimer = setTimeout(() => {
      highlightedRunId = null;
      highlightedEventSequence = null;
      activityTarget.update((current) => current?.request === target.request ? null : current);
    }, 3000);
  });

  onDestroy(() => {
    if (highlightTimer) clearTimeout(highlightTimer);
  });
</script>

{#if !$recordsLoaded && $shellError}
  <p class="empty">Couldn't load activity — something went wrong reaching the host. Retrying automatically…</p>
{:else if !$recordsLoaded}
  <LoadingIndicator fill label="Loading activity…" />
{:else if runs.length === 0}
  <p class="empty">No app activity recorded yet this session.</p>
{:else}
  <ol class="runs">
    {#each runs as run (run.runId)}
      <li
        id={`activity-run-${run.runId}`}
        class="run"
        class:highlighted={highlightedRunId === run.runId}
        tabindex="-1"
      >
        <div class="run-head">
          <div class="who-what">
            <strong>{whoLabel(run)}</strong>
            {#if run.goal}<span class="goal">{run.goal}</span>{/if}
          </div>
          <div class="meta">
            <span class={`state ${stateClass(run)}`}>{runStateLabel(run)}</span>
            <time datetime={run.startedAt}>{new Date(run.startedAt).toLocaleTimeString()}</time>
          </div>
        </div>
        <details>
          <summary>{run.events.length} {run.events.length === 1 ? "event" : "events"}</summary>
          <ol class="events">
            {#each run.events as event (event.sequence)}
              <li
                id={`activity-event-${event.sequence}`}
                class:highlighted-event={highlightedEventSequence === event.sequence}
                tabindex="-1"
              >
                <time datetime={event.recorded_at}>{new Date(event.recorded_at).toLocaleTimeString()}</time>
                <span title={ledgerEventDetail(event) ?? undefined}>{ledgerEventSummary(event)}</span>
              </li>
            {/each}
          </ol>
          <p class="run-id">Run <code>{run.runId}</code></p>
        </details>
      </li>
    {/each}
  </ol>
{/if}

<style>
  .empty {
    color: var(--color-text-muted);
    margin: 0;
  }
  .runs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.5rem;
  }
  .run {
    border: 1px solid var(--color-border-subtle);
    border-radius: 12px;
    padding: 0.65rem 0.8rem;
    display: grid;
    gap: 0.4rem;
    transition: border-color 160ms ease, background-color 160ms ease, box-shadow 160ms ease;
  }
  .run.highlighted {
    border-color: var(--color-accent-border);
    background: var(--color-accent-soft);
    box-shadow: 0 0 0 2px var(--color-accent-border);
  }
  .run:focus,
  .events li:focus {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  .run-head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.35rem 0.75rem;
  }
  .who-what {
    flex: 1 1 14rem;
    min-width: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem 0.5rem;
    align-items: baseline;
  }
  .goal {
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }
  .meta {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--color-text-muted);
    font-size: 0.85rem;
  }
  .state {
    display: inline-flex;
    border-radius: 999px;
    padding: 0.15rem 0.55rem;
    font-size: 0.74rem;
    font-weight: 700;
    text-transform: uppercase;
    white-space: nowrap;
  }
  .state-completed {
    background: var(--color-success-soft);
    color: var(--color-success-text);
  }
  .state-failed {
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }
  .state-cancelled,
  .state-interrupted {
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .state-running {
    background: var(--color-surface-muted);
    color: var(--color-text-muted);
  }
  summary {
    cursor: pointer;
    font-size: 0.85rem;
    color: var(--color-text-muted);
  }
  .events {
    list-style: none;
    margin: 0.5rem 0 0;
    padding: 0;
    display: grid;
    gap: 0.3rem;
    font-size: 0.85rem;
  }
  .events li {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem 0.6rem;
    border-radius: 6px;
    padding: 0.2rem 0.3rem;
  }
  .events li.highlighted-event {
    background: var(--color-surface-raised);
    box-shadow: 0 0 0 2px var(--color-accent-border);
  }
  .events time {
    color: var(--color-text-muted);
    white-space: nowrap;
  }
  .events span {
    overflow-wrap: anywhere;
  }
  .run-id {
    margin: 0.5rem 0 0;
    font-size: 0.8rem;
    color: var(--color-text-muted);
  }
  .run-id code {
    overflow-wrap: anywhere;
  }
</style>
