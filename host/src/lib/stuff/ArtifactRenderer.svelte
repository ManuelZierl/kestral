<script lang="ts">
  import type { Artifact } from "$lib/api";

  interface Props {
    artifact: Artifact;
  }

  let { artifact }: Props = $props();
  const rawContent = $derived(
    typeof artifact.content === "string"
      ? artifact.content
      : JSON.stringify(artifact.content, null, 2),
  );
  const rawProvenance = $derived(JSON.stringify(artifact.provenance, null, 2));
</script>

<pre>{rawContent}</pre>
<p class="label">Provenance</p>
<pre>{rawProvenance}</pre>

<style>
  pre {
    max-width: 100%;
    background: var(--color-surface-muted);
    border-radius: 10px;
    padding: 0.8rem;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    margin: 0.75rem 0 0;
  }
  .label {
    margin: 0.75rem 0 0;
    color: var(--color-text-muted);
    font-size: 0.8rem;
    font-weight: 600;
  }
</style>
