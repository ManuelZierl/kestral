<section class="authority-warning" aria-label="Unsandboxed native backend warning">
  <strong>Unsandboxed native backend</strong>
  <p>
    This backend runs with the full authority of the backend operating-system account. Kestral
    grants control only actions routed through Kestral; they cannot prevent direct filesystem or
    network access by the backend.
  </p>
</section>

<style>
  .authority-warning {
    display: grid;
    gap: 0.35rem;
    padding: 0.75rem 0.85rem;
    border: 1px solid var(--color-warning-border);
    border-radius: 12px;
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }

  p {
    margin: 0;
    max-width: 70ch;
    line-height: 1.45;
  }
</style>
