<script lang="ts">
  import type { PackageInspection } from "$lib/api";
  import {
    signatureStatusExplanation,
    signatureStatusLabel,
    signatureTrustNote,
  } from "$lib/apps/managedAppLifecycle";

  interface Props {
    inspection: PackageInspection;
    showPermissions?: boolean;
    trustAction?: {
      label: string;
      disabled?: boolean;
      onClick: () => void;
    } | null;
  }
  let {
    inspection,
    showPermissions = true,
    trustAction = null,
  }: Props = $props();

  const signatureKeyId = $derived(
    inspection.signature.kind === "valid-unknown-key" ||
      inspection.signature.kind === "trusted" ||
      inspection.signature.kind === "revoked"
      ? inspection.signature.key_id
      : null,
  );

  const runtimeSummary = $derived.by(() => {
    if (inspection.backend_authority_mode === "unsandboxed") {
      return {
        label: "Unsandboxed backend",
        detail: "Runs as a native process with full account access. Kestral permissions cannot prevent direct filesystem or network access.",
        tone: "warning",
      } as const;
    }
    if (inspection.backend_authority_mode === "sandboxed") {
      return {
        label: "Sandboxed backend",
        detail: "Its backend runs inside a Kestral-managed sandbox.",
        tone: "safe",
      } as const;
    }
    return {
      label: "No native backend",
      detail: "Uses host-rendered or sandboxed app screens only.",
      tone: "safe",
    } as const;
  });

  const trustTone = $derived.by(() => {
    switch (inspection.signature.kind) {
      case "trusted":
        return "safe";
      case "invalid":
      case "revoked":
        return "danger";
      case "unsigned":
      case "valid-unknown-key":
        return "warning";
    }
  });
</script>

<div class="inspection" data-testid="package-inspection">
  <header>
    <h3>{inspection.display_name}</h3>
    <span class="version">v{inspection.version}</span>
  </header>
  <p class="desc">{inspection.description}</p>

  {#if !inspection.installable}
    <p class="blocker" role="alert">
      Cannot install: {inspection.blocking_error ?? inspection.integrity_error ?? "package is not installable"}
    </p>
  {/if}

  <div class="decision-summary" aria-label="App safety summary">
    <section
      class="decision-card"
      class:safe={trustTone === "safe"}
      class:warning={trustTone === "warning"}
      class:danger={trustTone === "danger"}
    >
      <span class="decision-label">Publisher trust</span>
      <strong>{signatureStatusLabel(inspection.signature)}</strong>
      <span>{inspection.publisher?.name ?? "Publisher not declared"}</span>
      <p>{signatureStatusExplanation(inspection.signature)} {signatureTrustNote()}</p>
    </section>
    <section
      class="decision-card"
      class:warning={runtimeSummary.tone === "warning"}
    >
      <span class="decision-label">How it runs</span>
      <strong>{runtimeSummary.label}</strong>
      <p>{runtimeSummary.detail}</p>
    </section>
  </div>

  {#if trustAction && inspection.signature.kind === "valid-unknown-key"}
    <div class="trust-callout" role="group" aria-label="Trust this publisher key">
      <p>
        Trust this exact app id before install. This only records the key for
        <code>{inspection.id}</code> and does not skip the install permission review.
      </p>
      <button type="button" onclick={trustAction.onClick} disabled={trustAction.disabled}>
        {trustAction.label}
      </button>
    </div>
  {/if}

  {#if showPermissions}
    <section class="permissions" aria-labelledby="permissions-heading">
      <h4 id="permissions-heading">Permissions this app requests ({inspection.grant_requests.length})</h4>
      {#if inspection.grant_requests.length === 0}
        <p class="no-permissions">No permissions requested.</p>
      {:else}
        <ul>
          {#each inspection.grant_requests as grant}
            <li>
              <strong>{grant.scope_label}</strong>
              <span class="cond">{grant.condition}</span>
              <span class="muted">· {grant.duration_label}</span>
              <div class="muted">Data access: {grant.data_scope_label}</div>
              <div class="reason">{grant.reason}</div>
            </li>
          {/each}
        </ul>
        <p class="muted small">You can approve or deny each permission in the trusted install prompt.</p>
      {/if}
    </section>
  {/if}

  {#if inspection.warnings.length > 0}
    <section class="warnings">
      <h4>Warnings</h4>
      <ul>
        {#each inspection.warnings as warning}<li>{warning}</li>{/each}
      </ul>
    </section>
  {/if}

  {#if inspection.data.kind === "host-managed"}
    <details class="disclosure">
      <summary>Data storage and retention</summary>
      <section class="data-handling" aria-labelledby="managed-data-heading">
        <h4 id="managed-data-heading">How this app stores data</h4>
        <p>
          This app stores its declared records in Kestral without a native backend. Its open
          screens can read and change this data directly; access from Chat or other apps still
          requires a permission and creates a Run.
        </p>
        <p>
          Data is kept when the app is disabled or removed unless you explicitly choose to purge
          it. A compatible reinstall with the same app ID can reuse retained data.
        </p>
        {#if inspection.data.contract_version === 2}
          <p>
            This package also declares managed documents. Kestral stores their opaque content as
            immutable hashed blobs; the app supplies only schema-validated metadata and bounded
            chunks, never filesystem paths.
          </p>
        {/if}
        {#if inspection.data.proposals.length > 0}
          <h4>Reviewable proposals ({inspection.data.proposals.length})</h4>
          <p>
            A proposal creates a reviewable artifact for the declared target. It does not change
            managed data until you apply it separately in the app.
          </p>
          <ul>
            {#each inspection.data.proposals as proposal}
              <li>
                <strong>{proposal.title}</strong>
                <span class="muted">{proposal.target_kind} target: {proposal.collection}</span>
                <span class="muted">up to {proposal.max_payload_bytes} payload bytes</span>
                <div class="muted small">{proposal.description}</div>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    </details>
  {/if}

  <details class="technical">
    <summary>Technical package details</summary>
    <dl class="facts">
      <div><dt>App ID</dt><dd><code>{inspection.id}</code></dd></div>
      <div><dt>Runtime</dt><dd>{inspection.backend_kind} — {inspection.backend_detail}</dd></div>
      <div>
        <dt>App data</dt>
        <dd>
          {inspection.data.kind === "versioned"
            ? `Publisher-owned format ${inspection.data.format_version}`
            : inspection.data.kind === "host-managed"
               ? `Host-managed contract v${inspection.data.contract_version}, ${inspection.data.collections.length} record collection(s), ${inspection.data.documents.length} document collection(s), ${inspection.data.total_bytes} bytes maximum`
              : "No app-owned durable data"}
        </dd>
      </div>
      <div>
        <dt>Min host</dt>
        <dd class:bad={!inspection.host_compatible}>
          {inspection.min_host_version} (this host {inspection.host_version})
        </dd>
      </div>
      {#if signatureKeyId}<div><dt>Signed key</dt><dd><code>{signatureKeyId}</code></dd></div>{/if}
      <div><dt>Package digest</dt><dd class="digest">{inspection.package_digest}</dd></div>
      <div>
        <dt>Integrity</dt>
        <dd class:bad={!inspection.integrity_ok} class:ok={inspection.integrity_ok}>
          {inspection.integrity_ok ? "verified (sha256)" : (inspection.integrity_error ?? "failed")}
        </dd>
      </div>
    </dl>

    {#if inspection.data.kind === "host-managed"}
      <section>
        <h4>Host-managed collections ({inspection.data.collections.length})</h4>
        {#if inspection.data.batch_operations !== null}
          <p class="muted small">Staged batches allow up to {inspection.data.batch_operations} record and document operations.</p>
        {/if}
        <ul>
          {#each inspection.data.collections as collection}
            <li>
              <strong>{collection.name}</strong>
              <span class="muted">{collection.operations.join(", ")}</span>
              <div class="muted small">
                Up to {collection.records} records, {collection.record_bytes} bytes each,
                {collection.query_results} results per query
                {#if collection.indexes.length > 0} · indexes: {collection.indexes.join(", ")}{/if}
                {#if collection.unique_indexes.length > 0}
                  · unique: {collection.unique_indexes.join(", ")}
                {/if}
              </div>
              <details class="schema">
                <summary>Stored record fields</summary>
                <pre>{JSON.stringify(collection.schema, null, 2)}</pre>
              </details>
            </li>
          {/each}
        </ul>
      </section>
      {#if inspection.data.documents.length > 0}
        <section>
          <h4>Managed document collections ({inspection.data.documents.length})</h4>
          <ul>
            {#each inspection.data.documents as collection}
              <li>
                <strong>{collection.name}</strong>
                <span class="muted">{collection.operations.join(", ")}</span>
                <div class="muted small">
                  Up to {collection.documents} documents, {collection.metadata_bytes} bytes metadata,
                  {collection.content_bytes} bytes content
                </div>
                <details class="schema">
                  <summary>Document metadata schema</summary>
                  <pre>{JSON.stringify(collection.metadata_schema, null, 2)}</pre>
                </details>
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    {/if}

    <section>
      <h4>Provided actions ({inspection.capabilities.length})</h4>
      {#if inspection.capabilities.length === 0}
        <p class="muted">None declared.</p>
      {:else}
        <ul>
          {#each inspection.capabilities as cap}
            <li><strong>{cap.name}</strong> <span class="effect">{cap.effect}</span> — {cap.description}</li>
          {/each}
        </ul>
      {/if}
    </section>
  </details>

  {#if inspection.extension_contributions.length > 0 || inspection.secrets.length > 0 || inspection.config.length > 0 || inspection.surfaces.length > 0}
    <details class="disclosure">
      <summary>Features and setup</summary>
      {#if inspection.extension_contributions.length > 0}
        <section>
          <h4>App integrations ({inspection.extension_contributions.length})</h4>
          <p class="muted small">These screens can appear inside other installed apps when their integration contract is compatible.</p>
          <ul>
            {#each inspection.extension_contributions as contribution}
              <li>
                <strong>{contribution.target_app} / {contribution.extension_point}</strong>
                <span class="muted">contract v{contribution.contract_version}, screen {contribution.surface}</span>
              </li>
            {/each}
          </ul>
        </section>
      {/if}
      {#if inspection.secrets.length > 0}
        <section>
          <h4>Secrets needed ({inspection.secrets.length})</h4>
          <ul>
            {#each inspection.secrets as secret}
              <li><strong>{secret.name}</strong> <span class="muted">({secret.connector})</span> — {secret.description}</li>
            {/each}
          </ul>
          <p class="muted small">You enter secret values later; they never ship inside the package.</p>
        </section>
      {/if}
      {#if inspection.config.length > 0}
        <section>
          <h4>Configuration</h4>
          <ul>
            {#each inspection.config as config}
              <li><strong>{config.title}</strong> — {config.description}</li>
            {/each}
          </ul>
        </section>
      {/if}
      {#if inspection.surfaces.length > 0}
        <section>
          <h4>Screens ({inspection.surfaces.length})</h4>
          <ul>
            {#each inspection.surfaces as surface}
              <li>
                <strong>{surface.title}</strong>
                <span class="muted">({surface.kind})</span>
                {#if surface.has_custom_ui}<span class="chip">custom UI</span>{/if}
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    </details>
  {/if}

</div>

<style>
  .inspection {
    border: 1px solid var(--color-border);
    border-radius: 14px;
    padding: 1rem 1.1rem;
    background: var(--color-surface);
    display: grid;
    gap: 0.6rem;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  h3 {
    margin: 0;
  }
  h4 {
    margin: 0 0 0.3rem;
    font-size: 0.9rem;
  }
  .version {
    color: var(--color-text-muted);
    font-size: 0.85rem;
  }
  .muted {
    color: var(--color-text-muted);
    font-size: 0.82rem;
  }
  .digest {
    overflow-wrap: anywhere;
    font-family: ui-monospace, monospace;
  }
  .desc {
    margin: 0;
  }
  .decision-summary {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 15rem), 1fr));
    gap: 0.65rem;
  }
  .decision-card {
    border: 1px solid var(--color-border-subtle);
    border-radius: 12px;
    padding: 0.7rem 0.8rem;
    background: var(--color-surface-muted);
    display: grid;
    gap: 0.2rem;
  }
  .decision-card.warning {
    border-color: var(--color-warning-border);
    background: var(--color-warning-soft);
    color: var(--color-warning-text);
  }
  .decision-card.safe {
    border-color: var(--color-success-border);
    background: var(--color-success-soft);
    color: var(--color-success-text);
  }
  .decision-card.danger {
    border-color: var(--color-danger-border);
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }
  .decision-card p {
    margin: 0;
    font-size: 0.82rem;
    line-height: 1.4;
  }
  .decision-label {
    color: var(--color-text-muted);
    font-size: 0.75rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .decision-card.warning .decision-label,
  .decision-card.safe .decision-label,
  .decision-card.danger .decision-label {
    color: inherit;
  }
  .permissions {
    display: grid;
    gap: 0.4rem;
    padding: 0.75rem 0.85rem;
    border: 1px solid var(--color-border-strong);
    border-radius: 12px;
    background: var(--color-surface-raised);
  }
  .permissions h4,
  .permissions p {
    margin: 0;
  }
  .no-permissions {
    color: var(--color-success-text);
    font-weight: 600;
  }
  .trust-callout {
    display: grid;
    gap: 0.5rem;
    padding: 0.7rem 0.8rem;
    border-radius: 12px;
    border: 1px solid var(--color-warning-border);
    background: var(--color-warning-soft);
  }
  .trust-callout p {
    margin: 0;
    color: var(--color-warning-text);
    font-size: 0.85rem;
  }
  .trust-callout button {
    width: fit-content;
    border: 1px solid var(--color-warning-border);
    border-radius: 8px;
    background: var(--color-surface-raised);
    color: var(--color-text);
    padding: 0.45rem 0.75rem;
  }
  .small {
    font-size: 0.78rem;
  }
  .facts {
    display: grid;
    gap: 0.3rem;
    margin: 0;
  }
  .facts div {
    display: flex;
    gap: 0.5rem;
    font-size: 0.85rem;
    min-width: 0;
  }
  dt {
    color: var(--color-text-muted);
    min-width: 6rem;
  }
  dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  dd.bad {
    color: var(--color-danger-text);
    font-weight: 700;
  }
  dd.ok {
    color: var(--color-success-text);
  }
  .technical section,
  .disclosure section,
  .warnings {
    border-top: 1px solid var(--color-border);
    padding-top: 0.5rem;
  }
  .data-handling {
    display: grid;
    gap: 0.35rem;
  }
  .data-handling p {
    margin: 0;
    max-width: 65ch;
    font-size: 0.85rem;
  }
  .schema {
    margin-top: 0.25rem;
  }
  .schema summary {
    cursor: pointer;
  }
  .schema pre {
    max-width: 100%;
    overflow: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-size: 0.75rem;
  }
  .technical,
  .disclosure {
    border-top: 1px solid var(--color-border);
    padding-top: 0.5rem;
  }
  .technical > summary,
  .disclosure > summary {
    cursor: pointer;
    color: var(--color-text-muted);
    font-size: 0.85rem;
    font-weight: 600;
  }
  .technical .facts {
    margin-top: 0.6rem;
  }
  .disclosure .data-handling {
    margin-top: 0.6rem;
  }
  ul {
    margin: 0;
    padding-left: 1.1rem;
    display: grid;
    gap: 0.3rem;
    font-size: 0.85rem;
  }
  .effect,
  .cond {
    border-radius: 999px;
    padding: 0.05rem 0.45rem;
    font-size: 0.72rem;
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
  }
  .reason {
    color: var(--color-text-muted);
    font-size: 0.8rem;
  }
  .chip {
    border-radius: 999px;
    padding: 0.05rem 0.45rem;
    font-size: 0.72rem;
    background: var(--color-success-soft);
    color: var(--color-success-text);
  }
  .warnings {
    color: var(--color-warning-text);
  }
  .blocker {
    margin: 0;
    padding: 0.6rem 0.75rem;
    border-radius: 10px;
    background: var(--color-warning-soft);
    border: 1px solid var(--color-warning-border);
    color: var(--color-warning-text);
    font-weight: 600;
  }
</style>
