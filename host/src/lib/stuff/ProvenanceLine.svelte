<script lang="ts">
  import type { Artifact } from "$lib/api";
  import { capabilityLabel } from "$lib/system/capabilityLabel";

  interface Props {
    artifact: Artifact;
  }

  let { artifact }: Props = $props();

  // Short form for the inline line; the title attribute carries the full id
  // for anyone who needs to correlate it with the run ledger.
  const shortRunId = $derived(artifact.provenance.run_id.slice(0, 8));
</script>

<p class="provenance">
  Made by <strong>{artifact.provenance.produced_by}</strong>
  · {new Date(artifact.provenance.recorded_at).toLocaleDateString()}
  · run <span title={artifact.provenance.run_id}>{shortRunId}</span>
  · {capabilityLabel(artifact.provenance.capability)}
</p>

<style>
  .provenance {
    color: var(--color-text-muted);
    margin: 0.75rem 0 0;
    font-size: 0.85rem;
  }
</style>
