<script lang="ts">
  import { submitPermissionProposal, type PermissionProposalSubmission } from "$lib/api";
  import { dataScopeCovers, scopeCovers } from "$lib/apps/appMetadata";
  import type { ChatPermissionProposal } from "$lib/provenance/sessionReferences";
  import { grants, grantsLoaded, refreshGrants } from "$lib/stores/grants";

  interface Props {
    proposal: ChatPermissionProposal;
  }

  let { proposal }: Props = $props();
  let busy = $state(false);
  let result = $state<PermissionProposalSubmission | null>(null);
  let error = $state("");

  const activeCondition = $derived.by(() => {
    if (!$grantsLoaded) return null;
    const conditions = $grants
      .filter((grant) =>
        grant.status === "active"
        && grant.holder === proposal.holder
        && scopeCovers({
          kind: "exact-capability",
          provider: proposal.provider,
          capability: proposal.capability,
        }, grant.scope)
        && dataScopeCovers({ kind: "none" }, grant.data_scope)
      )
      .map((grant) => grant.condition);
    return conditions.includes("silent")
      ? "silent"
      : conditions.includes("notify")
        ? "notify"
        : conditions.includes("requires-approval")
          ? "requires-approval"
          : null;
  });
  const effectiveCondition = $derived(
    result && result.status !== "refused" ? result.effective_condition : activeCondition,
  );
  const lessInteractive = $derived(
    effectiveCondition !== null && effectiveCondition !== "requires-approval",
  );

  async function submit(): Promise<void> {
    busy = true;
    error = "";
    try {
      result = await submitPermissionProposal(proposal.artifactId);
      if (result.status !== "refused") await refreshGrants(true);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      busy = false;
    }
  }
</script>

<section class="proposal" aria-label="Permission proposal">
  <div class="heading">
    <strong>Permission requested</strong>
    <span>Capability</span>
  </div>
  <p>
    <strong>{proposal.holder}</strong> requests access to
    <code>{proposal.provider}/{proposal.capability}</code>.
  </p>
  <p class="reason">{proposal.reason}</p>
  <p class="policy">By default, Kestral will ask before every use of this capability.</p>

  {#if lessInteractive}
    <p class="warning" role="alert">
      Another active permission already allows this capability with
      {effectiveCondition === "silent" ? "no approval or notice" : "a notice but no approval"}.
      Review it under Settings → Permissions if that is not what you want.
    </p>
  {:else if result?.status === "issued"}
    <p class="success" role="status">Permission granted. Each use will ask for approval.</p>
  {:else if result?.status === "already-active" || (result === null && activeCondition !== null)}
    <p class="success" role="status">This approval-required permission is already active.</p>
  {:else if result?.status === "refused"}
    <p class="muted" role="status">Permission not granted.</p>
  {/if}

  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if $grantsLoaded && activeCondition === null && (result === null || result.status === "refused")}
    <button type="button" disabled={busy} onclick={() => void submit()}>
      {busy ? "Waiting for your decision…" : "Review and grant"}
    </button>
  {:else if !$grantsLoaded}
    <p class="muted" role="status">Checking current permissions…</p>
  {/if}
</section>

<style>
  .proposal {
    border: 1px solid var(--color-warning-border);
    border-radius: 12px;
    background: var(--color-warning-soft);
    padding: 0.8rem 0.9rem;
    display: grid;
    gap: 0.5rem;
    min-width: 0;
  }
  .heading {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.3rem 0.75rem;
  }
  .heading strong {
    color: var(--color-text);
  }
  .heading span,
  .muted {
    color: var(--color-text-muted);
    font-size: 0.8rem;
  }
  p {
    margin: 0;
    color: var(--color-text-soft);
    line-height: 1.45;
    overflow-wrap: anywhere;
  }
  code {
    color: var(--color-text);
    overflow-wrap: anywhere;
  }
  .reason {
    font-size: 0.88rem;
  }
  .policy {
    color: var(--color-warning-text);
    font-size: 0.84rem;
    font-weight: 600;
  }
  .warning,
  .error {
    color: var(--color-danger-text);
    font-size: 0.84rem;
  }
  .success {
    color: var(--color-success-text);
    font-size: 0.84rem;
    font-weight: 600;
  }
  button {
    width: fit-content;
    min-height: 2rem;
    border: 1px solid var(--color-warning-border);
    border-radius: 8px;
    background: var(--color-surface-raised);
    color: var(--color-text);
    padding: 0.35em 0.75em;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }
  button:disabled {
    cursor: wait;
    opacity: 0.7;
  }
</style>
