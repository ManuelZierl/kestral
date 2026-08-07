<script lang="ts">
  import type { Artifact } from "$lib/api";
  import ArtifactRenderer from "$lib/stuff/ArtifactRenderer.svelte";
  import { artifactPreview } from "$lib/stuff/artifactRenderer";
  import ProvenanceLine from "$lib/stuff/ProvenanceLine.svelte";

  interface Props {
    artifact: Artifact;
    chatAccessReady: boolean;
    chatCanAccess: boolean;
    grantingChatAccess: boolean;
    onGrantChat: () => void;
    highlighted?: boolean;
  }

  let {
    artifact,
    chatAccessReady,
    chatCanAccess,
    grantingChatAccess,
    onGrantChat,
    highlighted = false,
  }: Props = $props();

  let copied = $state(false);
  let resetTimer: ReturnType<typeof setTimeout> | undefined;

  async function copyId() {
    try {
      await navigator.clipboard.writeText(artifact.artifact_id);
      copied = true;
      clearTimeout(resetTimer);
      resetTimer = setTimeout(() => {
        copied = false;
      }, 1500);
    } catch {
      // clipboard unavailable in this context; leave copied false
    }
  }

  $effect(() => () => clearTimeout(resetTimer));
</script>

<article
  id={`artifact-${artifact.artifact_id}`}
  class="card"
  class:highlighted
  tabindex="-1"
>
  <div class="heading">
    <div class="title">
      <h2>{artifact.title}</h2>
      <span>{artifact.artifact_type}</span>
    </div>
    <div class="heading-actions">
      <button
        type="button"
        class="copy-id"
        class:copied
        aria-label={copied ? "Copied artifact ID" : "Copy artifact ID"}
        title={copied ? "Copied" : `Copy artifact ID: ${artifact.artifact_id}`}
        onclick={copyId}
      >
        {#if copied}
          <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
            <path
              d="M20 6 9 17l-5-5"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
        {:else}
          <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
            <rect
              x="9"
              y="9"
              width="11"
              height="11"
              rx="2"
              ry="2"
              fill="none"
              stroke="currentColor"
              stroke-width="1.7"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
            <path
              d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"
              fill="none"
              stroke="currentColor"
              stroke-width="1.7"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
        {/if}
      </button>
      {#if chatCanAccess}
        <span class="access-status">Available to Chat</span>
      {:else}
        <button type="button" disabled={!chatAccessReady || grantingChatAccess} onclick={onGrantChat}>
          {!chatAccessReady ? "Checking…" : grantingChatAccess ? "Requesting…" : "Allow Chat"}
        </button>
      {/if}
    </div>
  </div>
  <p class="preview">{artifactPreview(artifact)}</p>
  <ProvenanceLine {artifact} />
  <details>
    <summary>View raw data</summary>
    <ArtifactRenderer {artifact} />
  </details>
</article>

<style>
  .card {
    min-width: 0;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 18px;
    padding: 1rem 1.1rem;
    transition: border-color 160ms ease, background-color 160ms ease, box-shadow 160ms ease;
  }
  .card.highlighted {
    border-color: var(--color-accent-border);
    background: var(--color-accent-soft);
    box-shadow: 0 0 0 2px var(--color-accent-border);
  }
  .card:focus {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  h2 {
    margin: 0;
    min-width: 0;
    font-size: 1rem;
    overflow-wrap: anywhere;
  }
  .heading {
    min-width: 0;
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: center;
    gap: 0.75rem;
  }
  .heading-actions {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .title {
    min-width: 0;
    display: flex;
    flex: 1 1 16rem;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.35rem 0.75rem;
  }
  .title > span {
    flex-shrink: 0;
    color: var(--color-text-muted);
    font-size: 0.9rem;
    font-weight: 400;
  }
  .copy-id {
    min-width: 1.5rem;
    min-height: 1.5rem;
    display: inline-grid;
    place-items: center;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--color-text-muted);
    border-radius: 0.4rem;
    cursor: pointer;
  }
  .copy-id:hover {
    background: var(--color-surface-hover);
    color: var(--color-text);
  }
  .copy-id:focus-visible {
    outline: 2px solid var(--color-focus-ring);
    outline-offset: 1px;
  }
  .copy-id.copied {
    color: var(--color-success-text);
  }
  .access-status {
    color: var(--color-success-text);
    font-size: 0.82rem;
    font-weight: 600;
  }
  button {
    min-height: 2rem;
    border: 1px solid var(--color-accent-border);
    border-radius: 0.55rem;
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
    padding: 0.4em 0.7em;
    font: inherit;
    font-size: 0.82rem;
    font-weight: 650;
  }
  button:disabled {
    cursor: wait;
    opacity: 0.7;
  }
  .preview {
    margin: 0.65rem 0 0;
    color: var(--color-text-soft);
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    line-clamp: 3;
    -webkit-line-clamp: 3;
    overflow: hidden;
  }
  details {
    margin-top: 0.65rem;
  }
  summary {
    width: fit-content;
    color: var(--color-accent-text);
    cursor: pointer;
    font-size: 0.82rem;
    font-weight: 600;
  }
</style>
