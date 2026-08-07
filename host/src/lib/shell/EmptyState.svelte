<script lang="ts">
  interface Props {
    title: string;
    message: string;
  }

  let { title, message }: Props = $props();
</script>

<div class="empty">
  <strong>{title}</strong>
  <p>{message}</p>
</div>

<style>
  .empty {
    border: 1px dashed var(--color-border-strong);
    border-radius: 16px;
    padding: 1.25rem;
    background: var(--color-surface);
    color: var(--color-text-muted);
  }
  strong {
    color: var(--color-text);
  }
  p {
    margin: 0.35rem 0 0;
  }
</style>
