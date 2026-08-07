<script lang="ts">
  import type { ManagedAppTransitionPlan } from "$lib/api";
  import { managedAppOperationLabel } from "$lib/apps/managedAppLifecycle";
  import UnsandboxedBackendWarning from "$lib/apps/UnsandboxedBackendWarning.svelte";

  interface Props {
    plan: ManagedAppTransitionPlan;
  }

  let { plan }: Props = $props();

  const operationLabel = $derived(managedAppOperationLabel(plan.operation));
</script>

<section class="diff" aria-label="Managed app review">
  <header class="head">
    <div>
      <h4>{operationLabel}</h4>
      <p>
        Current revision {plan.current_revision_id ?? "none"} → target revision {plan.target_revision_id}
      </p>
    </div>
    <span class="tag">v{plan.target_version}</span>
  </header>

  <dl class="facts">
    <div><dt>Version relation</dt><dd>{plan.diff.version_relation}</dd></div>
    <div><dt>Publisher continuity</dt><dd>{plan.diff.publisher_key_continuity}</dd></div>
    <div>
      <dt>Data recovery</dt>
      <dd>
        {plan.data_transition
          ? "Source backup retained"
          : plan.diff.target_data.kind === "host-managed"
            ? "Retained data validated against this contract; a pre-change snapshot is kept before the first changed-contract write"
            : "No app-data migration"}
      </dd>
    </div>
  </dl>

  {#if plan.diff.target_backend_authority_mode === "unsandboxed"}
    <UnsandboxedBackendWarning />
  {/if}

  {#if plan.diff.extension_warnings.length > 0}
    <section class="extension-warning" aria-label="Extension compatibility warning">
      <h5>Extension compatibility</h5>
      <p>
        This change will leave {plan.diff.extension_warnings.length} installed
        contribution{plan.diff.extension_warnings.length === 1 ? "" : "s"} dormant.
      </p>
      <ul>
        {#each plan.diff.extension_warnings as warning}
          <li>
            <strong>{warning.contributor_app_id} / {warning.extension_point}</strong>
            requires contract v{warning.contribution_contract_version}; the target
            {warning.target_contract_version === null
              ? "will no longer provide this extension point"
              : `will provide v${warning.target_contract_version}`}.
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  {#if plan.data_rollback_caveat}
    <p class="caveat">{plan.data_rollback_caveat}</p>
  {/if}

  {#if plan.data_transition}
    <section class:destructive={plan.data_transition.destructive} class="data-transition">
      <h5>{plan.data_transition.reverse_migration ? "Reverse data migration" : "App-data migration"}</h5>
      <p>
        Format {plan.data_transition.source_format_version ?? "none"} → {plan.data_transition.target_format_version}.
        The previous bytes remain in a recoverable backup.
      </p>
      {#if plan.data_transition.destructive}
        <strong>This publisher marks the migration as destructive. Applying this review confirms the data change.</strong>
      {/if}
    </section>
  {/if}

  {#if plan.diff.target_data.kind === "host-managed" && plan.diff.target_data.proposals.length > 0}
    <section class="proposal-disclosure" aria-label="Proposal update disclosure">
      <h5>Reviewable proposals in target</h5>
      <p>
        These operations create reviewable artifacts for the listed targets. They do not change
        managed data; applying an artifact remains a separate compare-and-swap action in the app.
      </p>
      <ul>
        {#each plan.diff.target_data.proposals as proposal}
          <li>
            <strong>{proposal.title}</strong>
            <span>{proposal.target_kind} target: {proposal.collection}</span>
            <span class="muted">up to {proposal.max_payload_bytes} payload bytes</span>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <div class="sections">
    <section class="card-section">
      <h5>Manifest changes</h5>
      <ul>
        {#if plan.diff.display_name_changed}<li>Display name changed</li>{/if}
        {#if plan.diff.description_changed}<li>Description changed</li>{/if}
        {#if plan.diff.backend_kind_changed}<li>Backend kind changed</li>{/if}
        {#if plan.diff.capabilities_added.length > 0}
          <li>Added capabilities: {plan.diff.capabilities_added.join(", ")}</li>
        {/if}
        {#if plan.diff.capabilities_removed.length > 0}
          <li>Removed capabilities: {plan.diff.capabilities_removed.join(", ")}</li>
        {/if}
        {#if plan.diff.surfaces_added.length > 0}
          <li>Added surfaces: {plan.diff.surfaces_added.join(", ")}</li>
        {/if}
        {#if plan.diff.surfaces_removed.length > 0}
          <li>Removed surfaces: {plan.diff.surfaces_removed.join(", ")}</li>
        {/if}
      </ul>
    </section>

    <section class="card-section">
      <h5>Permission diff</h5>
      <div class="diff-list">
        <h6>Added</h6>
        {#if plan.diff.permissions.added.length === 0}
          <p class="muted">None</p>
        {:else}
          <ul>
            {#each plan.diff.permissions.added as item}
              <li>
                <strong>{item.scope_label}</strong>
                <span>{item.condition}</span>
                <span class="muted">· {item.duration_label}</span>
                <p>{item.reason}</p>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
      <div class="diff-list">
        <h6>Removed</h6>
        {#if plan.diff.permissions.removed.length === 0}
          <p class="muted">None</p>
        {:else}
          <ul>
            {#each plan.diff.permissions.removed as item}
              <li>
                <strong>{item.scope_label}</strong>
                <span>{item.condition}</span>
                <span class="muted">· {item.duration_label}</span>
                <p>{item.reason}</p>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
      {#if plan.diff.permissions.unchanged.length > 0}
        <div class="diff-list">
          <h6>Unchanged</h6>
          <ul>
            {#each plan.diff.permissions.unchanged as item}
              <li>
                <strong>{item.scope_label}</strong>
                <span>{item.condition}</span>
                <span class="muted">· {item.duration_label}</span>
                <p>{item.reason}</p>
              </li>
            {/each}
          </ul>
        </div>
      {/if}
      {#if plan.diff.consumer_permissions.added.length || plan.diff.consumer_permissions.removed.length}
        <h6>Consumer permissions</h6>
        <div class="diff-list">
          <h6>Added</h6>
          {#if plan.diff.consumer_permissions.added.length === 0}
            <p class="muted">None</p>
          {:else}
            <ul>
              {#each plan.diff.consumer_permissions.added as item}
                <li>
                  <strong>{item.scope_label}</strong>
                  <span>{item.condition}</span>
                  <span class="muted">· {item.duration_label}</span>
                  <p>{item.reason}</p>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
        <div class="diff-list">
          <h6>Removed</h6>
          {#if plan.diff.consumer_permissions.removed.length === 0}
            <p class="muted">None</p>
          {:else}
            <ul>
              {#each plan.diff.consumer_permissions.removed as item}
                <li>
                  <strong>{item.scope_label}</strong>
                  <span>{item.condition}</span>
                  <span class="muted">· {item.duration_label}</span>
                  <p>{item.reason}</p>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}
    </section>
  </div>
</section>

<style>
  .diff {
    display: grid;
    gap: 0.75rem;
    border: 1px solid var(--color-border);
    border-radius: 14px;
    padding: 0.9rem 1rem;
    background: var(--color-surface-muted);
  }
  .head {
    display: flex;
    justify-content: space-between;
    gap: 0.75rem;
    flex-wrap: wrap;
    align-items: start;
  }
  h4,
  h5,
  h6,
  p {
    margin: 0;
  }
  .head p,
  .caveat,
  .muted {
    color: var(--color-text-muted);
  }
  .tag {
    border-radius: 999px;
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
    padding: 0.15rem 0.55rem;
    font-size: 0.75rem;
    white-space: nowrap;
  }
  .facts {
    display: grid;
    gap: 0.35rem;
  }
  .facts div {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  dt {
    color: var(--color-text-muted);
    min-width: 8rem;
  }
  dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .sections {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 16rem), 1fr));
    gap: 0.75rem;
  }
  .data-transition {
    border: 1px solid var(--color-warning-border);
    border-radius: 12px;
    padding: 0.75rem;
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
    display: grid;
    gap: 0.35rem;
  }
  .extension-warning {
    border: 1px solid var(--color-warning-border);
    border-radius: 12px;
    padding: 0.75rem;
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
    display: grid;
    gap: 0.35rem;
  }
  .proposal-disclosure {
    border: 1px solid var(--color-border-subtle);
    border-radius: 12px;
    padding: 0.75rem;
    background: var(--color-surface);
    display: grid;
    gap: 0.35rem;
  }
  .extension-warning ul {
    margin-top: 0;
  }
  .data-transition.destructive {
    border-color: var(--color-danger-border);
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }
  .card-section {
    border: 1px solid var(--color-border-subtle);
    border-radius: 12px;
    padding: 0.75rem;
    background: var(--color-surface);
    min-width: 0;
  }
  ul {
    margin: 0.45rem 0 0;
    padding-left: 1rem;
    display: grid;
    gap: 0.35rem;
  }
  li p {
    margin-top: 0.15rem;
  }
  .diff-list + .diff-list {
    margin-top: 0.6rem;
  }
  .diff-list ul {
    margin-top: 0.35rem;
  }
</style>
