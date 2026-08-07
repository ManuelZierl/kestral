<script lang="ts">
  import { onDestroy, tick } from "svelte";
  import { scrollTargetIntoView } from "$lib/a11y/scroll";
  import {
    artifacts,
    artifactsLoaded,
    grantArtifactAccessAndRefresh,
  } from "$lib/stores/artifacts";
  import { grants, grantsLoaded } from "$lib/stores/grants";
  import { shellError } from "$lib/stores/hostState";
  import { artifactTarget } from "$lib/stores/navigation";
  import ArtifactCard from "$lib/stuff/ArtifactCard.svelte";
  import { artifactPreview } from "$lib/stuff/artifactRenderer";
  import EmptyState from "$lib/shell/EmptyState.svelte";
  import LoadingIndicator from "$lib/shell/LoadingIndicator.svelte";
  import type { ArtifactAccessTarget } from "$lib/api";
  import {
    CHAT_APP_ID,
    chatCanAccessAllArtifacts,
    chatCanAccessArtifact,
  } from "$lib/stuff/artifactAccess";

  let query = $state("");
  let selectedType = $state("");
  let selectedProducer = $state("");
  let grantingTarget = $state<string | null>(null);
  let accessError = $state<string | null>(null);
  let handledArtifactRequest = 0;
  let highlightedArtifactId = $state<string | null>(null);
  let highlightTimer: ReturnType<typeof setTimeout> | null = null;

  const artifactTypes = $derived([...new Set($artifacts.map((artifact) => artifact.artifact_type))].sort());
  const producers = $derived([...new Set($artifacts.map((artifact) => artifact.provenance.produced_by))].sort());
  const filteredArtifacts = $derived.by(() => {
    const needle = query.trim().toLocaleLowerCase();
    return [...$artifacts].reverse().filter((artifact) => {
      if (selectedType && artifact.artifact_type !== selectedType) return false;
      if (selectedProducer && artifact.provenance.produced_by !== selectedProducer) return false;
      if (!needle) return true;
      return `${artifact.title} ${artifact.artifact_type} ${artifact.provenance.produced_by} ${artifactPreview(artifact)}`
        .toLocaleLowerCase()
        .includes(needle);
    });
  });
  const chatHasAllArtifacts = $derived(chatCanAccessAllArtifacts($grants));

  $effect(() => {
    const target = $artifactTarget;
    if (!target || target.request === handledArtifactRequest || !$artifactsLoaded) return;
    handledArtifactRequest = target.request;
    if (!$artifacts.some((artifact) => artifact.artifact_id === target.artifactId)) {
      artifactTarget.update((current) => current?.request === target.request ? null : current);
      return;
    }

    query = "";
    selectedType = "";
    selectedProducer = "";
    highlightedArtifactId = target.artifactId;
    if (highlightTimer) clearTimeout(highlightTimer);
    void tick().then(() => {
      const element = document.getElementById(`artifact-${target.artifactId}`);
      element?.focus({ preventScroll: true });
      scrollTargetIntoView(element);
    });
    highlightTimer = setTimeout(() => {
      highlightedArtifactId = null;
      artifactTarget.update((current) => current?.request === target.request ? null : current);
    }, 3000);
  });

  onDestroy(() => {
    if (highlightTimer) clearTimeout(highlightTimer);
  });

  async function allowChat(target: ArtifactAccessTarget) {
    const key = target.kind === "all-artifacts" ? "all" : target.artifact_id;
    if (grantingTarget !== null) return;
    grantingTarget = key;
    accessError = null;
    try {
      await grantArtifactAccessAndRefresh(CHAT_APP_ID, target);
    } catch (failure) {
      accessError = String(failure);
    } finally {
      grantingTarget = null;
    }
  }
</script>

<section class="stack">
  {#if !$artifactsLoaded && $shellError}
    <EmptyState title="Couldn't load artifacts" message="Something went wrong reaching the host. Retrying automatically…" />
  {:else if !$artifactsLoaded}
    <LoadingIndicator fill label="Loading artifacts…" />
  {:else if $artifacts.length === 0}
    <EmptyState title="Nothing here yet" message="Use Chat or an app to create something." />
  {:else}
    <section class="chat-access" aria-labelledby="chat-access-heading">
      <div>
        <h2 id="chat-access-heading">Chat access</h2>
        <p>
          Choose which artifacts Chat may list and read. Each use remains recorded and follows its approval setting.
        </p>
      </div>
      {#if !$grantsLoaded}
        <span class="access-state" role="status">Checking access…</span>
      {:else if chatHasAllArtifacts}
        <span class="access-state allowed">Chat can use all current and future artifacts</span>
      {:else}
        <button
          type="button"
          disabled={grantingTarget !== null}
          onclick={() => void allowChat({ kind: "all-artifacts" })}
        >{grantingTarget === "all" ? "Requesting…" : "Allow all artifacts"}</button>
      {/if}
      {#if accessError}<p class="access-error" role="alert">{accessError}</p>{/if}
    </section>
    <div class="filters" aria-label="Artifact filters">
      <label class="search">
        <span>Search</span>
        <input bind:value={query} type="search" placeholder="Search artifacts" />
      </label>
      <label>
        <span>Type</span>
        <select bind:value={selectedType}>
          <option value="">All types</option>
          {#each artifactTypes as type}<option value={type}>{type}</option>{/each}
        </select>
      </label>
      <label>
        <span>Made by</span>
        <select bind:value={selectedProducer}>
          <option value="">All apps</option>
          {#each producers as producer}<option value={producer}>{producer}</option>{/each}
        </select>
      </label>
    </div>
    <p class="result-count" aria-live="polite">
      {filteredArtifacts.length} {filteredArtifacts.length === 1 ? "artifact" : "artifacts"}
    </p>
    {#each filteredArtifacts as artifact (artifact.artifact_id)}
      <ArtifactCard
        {artifact}
        highlighted={highlightedArtifactId === artifact.artifact_id}
        chatAccessReady={$grantsLoaded}
        chatCanAccess={chatCanAccessArtifact($grants, artifact.artifact_id)}
        grantingChatAccess={grantingTarget === artifact.artifact_id}
        onGrantChat={() => void allowChat({ kind: "artifact", artifact_id: artifact.artifact_id })}
      />
    {:else}
      <EmptyState title="No matching artifacts" message="Change or clear the filters to see more results." />
    {/each}
  {/if}
</section>

<style>
  .stack {
    min-width: 0;
    margin-top: 1rem;
    display: grid;
    gap: 1rem;
  }
  .filters {
    min-width: 0;
    display: grid;
    grid-template-columns: minmax(min(100%, 16rem), 2fr) repeat(2, minmax(min(100%, 10rem), 1fr));
    gap: 0.7rem;
    padding: 0.85rem;
    border: 1px solid var(--color-border);
    border-radius: 14px;
    background: var(--color-surface);
  }
  .chat-access {
    min-width: 0;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem 1rem;
    padding: 0.85rem 1rem;
    border: 1px solid var(--color-border);
    border-radius: 0.9rem;
    background: var(--color-surface);
  }
  .chat-access > div {
    flex: 1 1 22rem;
    min-width: min(100%, 16rem);
  }
  .chat-access h2,
  .chat-access p {
    margin: 0;
  }
  .chat-access h2 {
    font-size: 0.95rem;
  }
  .chat-access p {
    margin-top: 0.2rem;
    color: var(--color-text-muted);
    font-size: 0.82rem;
    line-height: 1.45;
  }
  .chat-access button {
    min-height: 2.25rem;
    border: 1px solid var(--color-accent-border);
    border-radius: 0.6rem;
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
    padding: 0.5em 0.8em;
    font: inherit;
    font-weight: 650;
  }
  .chat-access button:disabled {
    cursor: wait;
    opacity: 0.7;
  }
  .access-state {
    color: var(--color-text-muted);
    font-size: 0.82rem;
    font-weight: 600;
  }
  .access-state.allowed {
    color: var(--color-success-text);
  }
  .access-error {
    flex-basis: 100%;
    margin: 0;
    color: var(--color-danger-text);
    font-size: 0.84rem;
  }
  label {
    min-width: 0;
    display: grid;
    gap: 0.25rem;
    color: var(--color-text-muted);
    font-size: 0.78rem;
    font-weight: 600;
  }
  input,
  select {
    width: 100%;
    min-width: 0;
    border: 1px solid var(--color-border-strong);
    border-radius: 9px;
    background: var(--color-surface-raised);
    color: var(--color-text);
    padding: 0.55rem 0.65rem;
    font: inherit;
  }
  .result-count {
    margin: -0.35rem 0;
    color: var(--color-text-muted);
    font-size: 0.82rem;
  }
  @media (max-width: 40em) {
    .filters {
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    }
    .search {
      grid-column: 1 / -1;
    }
  }
</style>
