<script lang="ts">
  import type { JsonObject } from "$lib/api";
  import {
    collectJsonObject,
    schemaFields,
    toInputValue,
  } from "$lib/settings/jsonSchemaFormModel";

  interface Props {
    schema: JsonObject;
    initialValue?: JsonObject;
    submitLabel?: string;
    onSubmit: (value: JsonObject) => Promise<void> | void;
  }

  let { schema, initialValue = {}, submitLabel = "Save", onSubmit }: Props = $props();

  let values = $state<Record<string, string>>({});
  let error = $state<string | null>(null);
  let saving = $state(false);
  let saved = $state(false);
  let loadedValue = "";

  $effect(() => {
    const nextValue = JSON.stringify([schema, initialValue]);
    if (nextValue !== loadedValue) {
      loadedValue = nextValue;
      values = Object.fromEntries(
        schemaFields(schema).map((field) => [field.name, toInputValue(initialValue[field.name])]),
      );
    }
  });

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    error = null;
    saved = false;
    let patch: JsonObject;
    try {
      patch = collectJsonObject(schema, values);
    } catch (failure) {
      error = String((failure as Error).message);
      return;
    }
    saving = true;
    try {
      await onSubmit(patch);
      saved = true;
    } catch (failure) {
      error = String(failure);
    } finally {
      saving = false;
    }
  }

  function updateValue(name: string, value: string) {
    values[name] = value;
    saved = false;
  }
</script>

<form onsubmit={submit}>
  {#each schemaFields(schema) as field}
    <label class:boolean={field.type === "boolean"}>
      {#if field.type === "boolean"}
        <input
          type="checkbox"
          checked={values[field.name] === "true"}
          onchange={(event) => updateValue(field.name, event.currentTarget.checked ? "true" : "false")}
        />
        <span>{field.title}</span>
      {:else}
        <span>{field.title}</span>
        {#if field.type === "string" && field.input === "multiline"}
          <textarea
            maxlength={field.maxLength}
            value={values[field.name]}
            oninput={(event) => updateValue(field.name, event.currentTarget.value)}
            required={field.required}
          ></textarea>
        {:else}
          <input
            type={field.type === "integer" || field.type === "number" ? "number" : "text"}
            step={field.type === "integer" ? "1" : field.type === "number" ? "any" : undefined}
            min={field.minimum}
            max={field.maximum}
            maxlength={field.maxLength}
            value={values[field.name]}
            oninput={(event) => updateValue(field.name, event.currentTarget.value)}
            required={field.required}
          />
        {/if}
      {/if}
      {#if field.description}
        <small>{field.description}</small>
      {/if}
    </label>
  {/each}
  <div class="actions">
    <button type="submit" disabled={saving}>{saving ? "Saving..." : submitLabel}</button>
    {#if saved}
      <span class="success" role="status">Settings saved.</span>
    {/if}
  </div>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</form>

<style>
  form {
    display: grid;
    gap: 0.8rem;
  }
  label {
    display: grid;
    gap: 0.25rem;
  }
  label.boolean {
    grid-template-columns: auto minmax(0, 1fr);
    align-items: center;
    /* Keep the clickable row at/above the 24px WCAG 2.5.8 target floor
       without enlarging the visual checkbox. */
    padding: 0.4rem 0;
  }
  label.boolean small {
    grid-column: 2;
  }
  label.boolean input {
    width: 1.1rem;
    height: 1.1rem;
  }
  input,
  textarea {
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    padding: 0.6rem 0.7rem;
    background: var(--color-surface);
    color: var(--color-text);
  }
  textarea {
    min-height: 8rem;
    resize: vertical;
  }
  small {
    color: var(--color-text-muted);
  }
  button {
    width: fit-content;
    border: none;
    border-radius: 10px;
    background: var(--color-accent);
    color: var(--color-accent-contrast);
    padding: 0.65rem 0.95rem;
  }
  .actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.65rem;
  }
  .success {
    color: var(--color-success-text);
    font-size: 0.9rem;
    font-weight: 600;
  }
  .error {
    color: var(--color-danger-text);
    margin: 0;
  }
</style>
