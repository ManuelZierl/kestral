<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    /** Standalone app view: the surface owns the workspace edge to edge. */
    flush?: boolean;
    children: Snippet;
  }

  let { flush = false, children }: Props = $props();
</script>

<main class="surface" class:flush>
  {@render children()}
</main>

<style>
  .surface {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 1.2rem 1.35rem 1.35rem;
    background: linear-gradient(180deg, var(--color-bg-gradient-a) 0%, var(--color-bg-gradient-b) 100%);
    display: flex;
    flex-direction: column;
  }
  .surface.flush {
    padding: 0;
  }
</style>
