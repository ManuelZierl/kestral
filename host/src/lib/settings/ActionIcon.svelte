<script lang="ts">
  interface Props {
    name: "add" | "delete" | "edit" | "export" | "import";
  }

  let { name }: Props = $props();
</script>

<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#if name === "add"}
    <path d="M12 5v14M5 12h14" />
  {:else if name === "edit"}
    <path d="M4 20h4L19 9l-4-4L4 16v4Z" />
    <path d="m13.5 6.5 4 4" />
  {:else if name === "delete"}
    <path d="M4 7h16M9 7V4h6v3M7 7l1 13h8l1-13M10 11v5M14 11v5" />
  {:else if name === "import"}
    <path d="M12 3v12m0 0 4-4m-4 4-4-4M5 19h14" />
  {:else}
    <path d="M12 16V4m0 0 4 4m-4-4L8 8M5 20h14" />
  {/if}
</svg>
