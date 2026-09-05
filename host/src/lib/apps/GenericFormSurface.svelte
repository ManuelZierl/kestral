<script lang="ts">
  import {
    availableCapabilitiesFor,
    closeSurface,
    openSurface,
    submitAction,
    type CapabilityDeclaration,
    type CapabilityUseView,
    type JsonObject,
    type SurfaceActionOutcome,
  } from "$lib/api";
  import {
    capabilityAccessBadge,
    capabilityAccessState,
    missingCapabilityWarning,
  } from "$lib/apps/capabilityAccess";
  import {
    collectJsonObject,
    parseJsonObjectInput,
    schemaFields,
    supportsJsonSchemaForm,
  } from "$lib/settings/jsonSchemaFormModel";
  import { grantsRevision } from "$lib/stores/grants";

  interface Props {
    appId: string;
    surface: string;
    capability: CapabilityDeclaration;
    onOutcome: (outcome: SurfaceActionOutcome) => void;
  }

  let { appId, surface, capability, onOutcome }: Props = $props();

  const scalarFormSupported = $derived(supportsJsonSchemaForm(capability.input_schema));
  const fields = $derived(scalarFormSupported ? schemaFields(capability.input_schema) : []);
  const inputSchema = $derived(JSON.stringify(capability.input_schema, null, 2));
  const inputSchemaFingerprint = $derived(JSON.stringify(capability.input_schema));
  const formIdentity = $derived(
    `${appId}\u0000${surface}\u0000${capability.name}\u0000${inputSchemaFingerprint}`,
  );
  const jsonGuidanceId = $derived(`${appId}-${surface}-${capability.name}-json-guidance`);
  const jsonSchemaId = $derived(`${appId}-${surface}-${capability.name}-json-schema`);
  let values = $state<Record<string, string>>({});
  let rawInput = $state("{}");
  let submitting = $state(false);
  let error = $state<string | null>(null);
  let lastOutcome = $state<SurfaceActionOutcome | null>(null);
  let availableCapabilities = $state<CapabilityUseView[]>([]);
  let capabilityRequestId = 0;
  let submissionRequestId = 0;

  const access = $derived(capabilityAccessState(availableCapabilities, appId, capability.name));
  const accessBadge = $derived(capabilityAccessBadge(access.grantCondition));
  const missingWarning = $derived(missingCapabilityWarning(appId, capability.name));

  $effect(() => {
    appId;
    capability.name;
    $grantsRevision;
    void loadAvailableCapabilities();
  });

  // Reset transient form state when the surface switches to a different app or
  // capability. Form instances are reused across capabilities that share a
  // surface name, so without this a stale value (or error) could be submitted
  // against a different capability's schema. Deliberately not tracking
  // `$grantsRevision`, so a grant change never wipes in-progress input.
  $effect(() => {
    formIdentity;
    submissionRequestId += 1;
    values = {};
    rawInput = "{}";
    submitting = false;
    error = null;
    lastOutcome = null;
    return () => {
      submissionRequestId += 1;
    };
  });

  async function loadAvailableCapabilities() {
    const requestId = ++capabilityRequestId;
    try {
      const next = await availableCapabilitiesFor(appId);
      if (requestId !== capabilityRequestId) return;
      availableCapabilities = next;
    } catch (failure) {
      if (requestId !== capabilityRequestId) return;
      if (!String(failure).includes("kernel busy")) {
        error = String(failure);
      }
    }
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (submitting) return;
    if (!access.available) {
      error = missingWarning;
      return;
    }
    const submittedFormIdentity = formIdentity;
    const submittedAppId = appId;
    const submittedSurface = surface;
    const submittedCapabilityName = capability.name;
    const submittedSchema = JSON.parse(inputSchemaFingerprint) as JsonObject;
    const submittedScalarFormSupported = scalarFormSupported;
    const submittedValues = { ...values };
    const submittedRawInput = rawInput;
    const submittedMissingWarning = missingWarning;
    const submittedOnOutcome = onOutcome;
    const requestId = ++submissionRequestId;
    const isCurrent = () => requestId === submissionRequestId && formIdentity === submittedFormIdentity;
    error = null;
    lastOutcome = null;
    let input: JsonObject;
    try {
      input = submittedScalarFormSupported
        ? collectJsonObject(submittedSchema, submittedValues)
        : parseJsonObjectInput(submittedRawInput);
    } catch (failure) {
      error = String((failure as Error).message);
      return;
    }
    submitting = true;
    let binding: Awaited<ReturnType<typeof openSurface>> | null = null;
    try {
      binding = await openSurface(submittedAppId, submittedSurface);
      if (!isCurrent()) return;
      const outcome = await submitAction(binding, {
        capability: { provider: submittedAppId, capability: submittedCapabilityName },
        input,
        data_scope: { kind: "none" },
        goal: `${submittedCapabilityName} via ${submittedSurface} form`,
      });
      if (!isCurrent()) return;
      lastOutcome = outcome;
      if (outcome.result.kind === "refused") {
        error =
          outcome.result.reason === "approval-denied"
            ? "The request was denied in trusted chrome."
            : submittedMissingWarning;
        return;
      }
      if (outcome.result.kind === "failed") {
        error = outcome.result.error;
        submittedOnOutcome(outcome);
        return;
      }
      submittedOnOutcome(outcome);
    } catch (failure) {
      if (isCurrent()) error = String(failure);
    } finally {
      if (binding) {
        try {
          await closeSurface(binding);
        } catch (failure) {
          if (isCurrent() && !error) error = String(failure);
        }
      }
      if (isCurrent()) submitting = false;
    }
  }
</script>

<form onsubmit={submit}>
  <div class="meta">
    {#if accessBadge}
      <span class="badge">{accessBadge}</span>
    {/if}
    {#if !access.available}
      <p class="warning">{missingWarning}</p>
    {/if}
  </div>
  {#if scalarFormSupported}
    {#each fields as field}
      <label>
        {field.title}{#if field.required}<span class="required" aria-hidden="true"> *</span>{/if}
        <input
          bind:value={values[field.name]}
          placeholder={field.type}
          disabled={!access.available || submitting}
          aria-required={field.required}
        />
      </label>
    {/each}
  {:else}
    <div class="structured-input">
      <p id={jsonGuidanceId}>
        This capability uses structured input that the simple field editor cannot represent.
        Enter a JSON object matching its input schema.
      </p>
      <label>
        Structured JSON input
        <textarea
          bind:value={rawInput}
          disabled={!access.available || submitting}
          aria-describedby={jsonGuidanceId}
          aria-details={jsonSchemaId}
          spellcheck={false}
        ></textarea>
      </label>
      <details id={jsonSchemaId}>
        <summary>View input schema</summary>
        <pre>{inputSchema}</pre>
      </details>
    </div>
  {/if}
  <button type="submit" disabled={submitting || !access.available}>
    {submitting ? "running..." : capability.name}
  </button>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  {#if lastOutcome}
    <section class="outcome" aria-label="Latest action outcome">
      <div class="outcome-heading" role="status">
        <strong>
          {lastOutcome.result.kind === "completed"
            ? "Action completed"
            : lastOutcome.result.kind === "failed"
              ? "Action failed"
              : "Action not run"}
        </strong>
        <span>Run <code>{lastOutcome.run_id}</code></span>
      </div>
      {#if lastOutcome.result.kind === "completed"}
        <details open>
          <summary>Returned result</summary>
          <pre>{JSON.stringify(lastOutcome.result.result, null, 2)}</pre>
        </details>
        {#if lastOutcome.result.artifacts.length > 0}
          <div class="artifacts">
            <strong>Artifacts</strong>
            <ul>
              {#each lastOutcome.result.artifacts as artifact (artifact.artifact_id)}
                <li>
                  <span>{artifact.title} <small>({artifact.artifact_type})</small></span>
                  <code>{artifact.artifact_id}</code>
                </li>
              {/each}
            </ul>
          </div>
        {/if}
      {/if}
    </section>
  {/if}
</form>

<style>
  form {
    display: flex;
    flex-wrap: wrap;
    gap: 0.6rem;
    align-items: flex-end;
  }
  .meta,
  .warning,
  .error {
    width: 100%;
  }
  .meta {
    display: grid;
    gap: 0.5rem;
  }
  .badge {
    width: fit-content;
    border-radius: 999px;
    padding: 0.2rem 0.55rem;
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
    font-size: 0.78rem;
    font-weight: 700;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.82rem;
  }
  .required {
    color: var(--color-danger-text);
  }
  input {
    border: 1px solid var(--color-border-strong);
    border-radius: 8px;
    padding: 0.45rem 0.55rem;
  }
  .structured-input {
    display: grid;
    gap: 0.6rem;
    width: 100%;
  }
  .structured-input p {
    margin: 0;
    color: var(--color-text-muted);
  }
  .structured-input label {
    width: 100%;
  }
  textarea,
  pre {
    width: 100%;
    margin: 0;
    border: 1px solid var(--color-border-strong);
    border-radius: 8px;
    padding: 0.65rem;
    background: var(--color-surface);
    color: var(--color-text);
    font-family: ui-monospace, SFMono-Regular, Consolas, "Liberation Mono", monospace;
    font-size: 0.82rem;
    line-height: 1.45;
  }
  textarea {
    min-height: 9rem;
    resize: vertical;
  }
  pre {
    margin-top: 0.5rem;
    overflow: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  summary {
    width: fit-content;
    cursor: pointer;
    color: var(--color-text-muted);
  }
  button {
    border: none;
    border-radius: 8px;
    background: var(--color-accent);
    color: var(--color-accent-contrast);
    padding: 0.55rem 0.9rem;
  }
  .error {
    color: var(--color-danger-text);
    margin: 0;
  }
  .outcome {
    display: grid;
    gap: 0.65rem;
    width: 100%;
    padding: 0.75rem;
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    background: var(--color-surface-raised);
  }
  .outcome-heading,
  .outcome li {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: 0.4rem 0.8rem;
  }
  .outcome-heading span,
  .outcome small {
    color: var(--color-text-muted);
  }
  .outcome ul {
    display: grid;
    gap: 0.35rem;
    margin: 0.35rem 0 0;
    padding-left: 1.25rem;
  }
  .warning {
    margin: 0;
    color: var(--color-warning-text);
    background: var(--color-warning-soft);
    border: 1px solid var(--color-warning-border);
    border-radius: 10px;
    padding: 0.75rem;
  }
</style>
