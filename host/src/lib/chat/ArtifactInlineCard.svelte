<script lang="ts">
  import type { SessionArtifactCard } from "$lib/provenance/sessionReferences";

  interface Props {
    artifact: SessionArtifactCard;
    openArtifacts: () => void;
  }

  let { artifact, openArtifacts }: Props = $props();
</script>

<section class="artifact" class:stale={!artifact.available}>
  <div class="artifact-head">
    <strong>{artifact.title}</strong>
    <span>{artifact.type}</span>
  </div>
  <p>{artifact.preview}</p>
  {#if artifact.available}
    <button type="button" class="artifact-link" onclick={openArtifacts}>Open in Artifacts</button>
  {:else}
    <span class="artifact-stale-label">Unavailable reference</span>
  {/if}
</section>

<style>
  .artifact {
    border: 1px solid var(--color-border-subtle);
    border-radius: 12px;
    background: var(--color-surface-raised);
    padding: 0.75rem 0.85rem;
    display: grid;
    gap: 0.4rem;
  }
  .artifact.stale {
    background: var(--color-warning-soft);
    border-color: var(--color-warning-border);
  }
  .artifact-head {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
  }
  strong {
    color: var(--color-text);
    font-size: 0.9rem;
  }
  span {
    color: var(--color-text-faint);
    font-size: 0.75rem;
  }
  p {
    margin: 0;
    color: var(--color-text-soft);
    font-size: 0.86rem;
    line-height: 1.45;
    white-space: pre-line;
  }
  .artifact-link {
    width: fit-content;
    /* 24 CSS px minimum touch target (WCAG 2.2 SC 2.5.8); centering the text
       in a taller flex box keeps the link's position visually unchanged. */
    min-height: 1.5rem;
    display: inline-flex;
    align-items: center;
    border: none;
    background: transparent;
    color: var(--color-accent);
    padding: 0;
    font-size: 0.82rem;
    font-weight: 500;
    cursor: pointer;
  }
  .artifact-link:hover {
    text-decoration: underline;
  }
  .artifact-stale-label {
    color: var(--color-warning-text);
    font-weight: 600;
  }
</style>
