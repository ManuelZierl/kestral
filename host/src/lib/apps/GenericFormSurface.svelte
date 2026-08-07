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
  import { collectJsonObject, schemaFields } from "$lib/settings/jsonSchemaFormModel";
  import { grantsRevision } from "$lib/stores/grants";

  interface Props {
    appId: string;
    surface: string;
    capability: CapabilityDeclaration;
    onOutcome: (outcome: SurfaceActionOutcome) => void;
  }

  let { appId, surface, capability, onOutcome }: Props = $props();

  const fields = $derived(schemaFields(capability.input_schema));
  let values = $state<Record<string, string>>({});
  let submitting = $state(false);
  let error = $state<string | null>(null);
  let availableCapabilities = $state<CapabilityUseView[]>([]);
  let capabilityRequestId = 0;

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
    appId;
    capability.name;
    values = {};
    error = null;
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
    if (!access.available) {
      error = missingWarning;
      return;
    }
    error = null;
    let input: JsonObject;
    try {
      input = collectJsonObject(capability.input_schema, values);
    } catch (failure) {
      error = String((failure as Error).message);
      return;
    }
    submitting = true;
    let binding: Awaited<ReturnType<typeof openSurface>> | null = null;
    try {
      binding = await openSurface(appId, surface);
      const outcome = await submitAction(binding, {
        capability: { provider: appId, capability: capability.name },
        input,
        data_scope: { kind: "none" },
        goal: `${capability.name} via ${surface} form`,
      });
      if (outcome.result.kind === "refused") {
        error =
          outcome.result.reason === "approval-denied"
            ? "The request was denied in trusted chrome."
            : missingWarning;
        return;
      }
      values = {};
      onOutcome(outcome);
    } catch (failure) {
      error = String(failure);
    } finally {
      if (binding) {
        try {
          await closeSurface(binding);
        } catch (failure) {
          if (!error) error = String(failure);
        }
      }
      submitting = false;
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
  <button type="submit" disabled={submitting || !access.available}>
    {submitting ? "running..." : capability.name}
  </button>
  {#if error}
    <p class="error" role="alert">{error}</p>
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
  .warning {
    margin: 0;
    color: var(--color-warning-text);
    background: var(--color-warning-soft);
    border: 1px solid var(--color-warning-border);
    border-radius: 10px;
    padding: 0.75rem;
  }
</style>
