<script lang="ts">
  import type { AppIcon, KestralIconName } from "$lib/api";

  interface Props {
    icon?: AppIcon | null;
    fallback: string;
  }

  let { icon = null, fallback }: Props = $props();

  const paths: Record<KestralIconName, string> = {
    activity: "M3 12h4l3-8 4 16 3-8h4",
    "app-grid": "M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h6v6h-6z",
    "artifact-box": "M21 8l-9-5-9 5 9 5 9-5ZM3 8v8l9 5 9-5V8M12 13v8",
    "book-open": "M3 5.5A2.5 2.5 0 0 1 5.5 3H11v16H5.5A2.5 2.5 0 0 0 3 21.5Zm18 0A2.5 2.5 0 0 0 18.5 3H13v16h5.5a2.5 2.5 0 0 1 2.5 2.5Z",
    "chat-bubble": "M21 11.5a8.4 8.4 0 0 1-8.5 8.3 8.9 8.9 0 0 1-3.1-.6L4 21l1.9-4.6a8 8 0 0 1-1.4-4.9A8.4 8.4 0 0 1 13 3.2a8.4 8.4 0 0 1 8 8.3Z",
    "check-square": "M9 11l3 3L22 4M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11",
    "pencil-ruler": "m14 6 4 4M4 20l3.5-.8L19 7.7a2.1 2.1 0 0 0-3-3L4.5 16.2ZM12 20h9M16 16l4 4",
    settings: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm7.5-3a7.5 7.5 0 0 0-.1-1.2l2-1.6-2-3.4-2.4 1a7.6 7.6 0 0 0-2-1.2L14.5 3h-5l-.5 2.6a7.6 7.6 0 0 0-2 1.2l-2.4-1-2 3.4 2 1.6a7.6 7.6 0 0 0 0 2.4l-2 1.6 2 3.4 2.4-1a7.6 7.6 0 0 0 2 1.2l.5 2.6h5l.5-2.6a7.6 7.6 0 0 0 2-1.2l2.4 1 2-3.4-2-1.6c.1-.4.1-.8.1-1.2Z",
  };

  function assetSource(value: Extract<AppIcon, { kind: "asset" }>): string {
    return `data:${value.media_type};base64,${value.data_base64}`;
  }

  function currentColorMask(value: Extract<AppIcon, { kind: "asset" }>): string | null {
    if (!value.media_type.startsWith("image/svg+xml")) return null;
    try {
      return /currentcolor/i.test(atob(value.data_base64)) ? assetSource(value) : null;
    } catch {
      return null;
    }
  }
</script>

{#if icon?.kind === "asset"}
  {@const mask = currentColorMask(icon)}
  {#if mask}
    <span class="monochrome-icon" style:--app-icon-mask={`url("${mask}")`}></span>
  {:else}
    <img src={assetSource(icon)} alt="" />
  {/if}
{:else if icon?.kind === "kestral"}
  <svg viewBox="0 0 24 24" width="18" height="18" data-icon-name={icon.name}>
    <path
      d={paths[icon.name]}
      fill="none"
      stroke="currentColor"
      stroke-width="1.7"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
  </svg>
{:else}
  {fallback.slice(0, 1).toUpperCase()}
{/if}

<style>
  img {
    display: block;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: 0.4rem;
    object-fit: contain;
  }
  .monochrome-icon {
    display: block;
    width: 1.5rem;
    height: 1.5rem;
    background: currentColor;
    -webkit-mask: var(--app-icon-mask) center / contain no-repeat;
    mask: var(--app-icon-mask) center / contain no-repeat;
  }
</style>
