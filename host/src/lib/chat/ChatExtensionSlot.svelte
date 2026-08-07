<script lang="ts">
  import {
    getSurfaceUi,
    validateExtensionContext,
    type JsonObject,
    type SurfaceUiBundle,
  } from "$lib/api";
  import AppSurfaceFrame from "$lib/surfaces/AppSurfaceFrame.svelte";
  import { apps } from "$lib/stores/apps";
  import { resolveChatExtensions } from "$lib/chat/chatExtensions";
  import LoadingIndicator from "$lib/shell/LoadingIndicator.svelte";

  interface Props {
    pointName: string;
    context: JsonObject;
    /// An extension surface published slot state (untrusted, already
    /// bridge-validated as an object). Keyed by the extension's slot key so
    /// the owner can hold state per contributing app.
    onExtensionState?: (
      extensionKey: string,
      appId: string,
      appName: string,
      payload: JsonObject,
    ) => void;
    onExtensionRemoved?: (extensionKey: string) => void;
  }

  interface ExtensionFrameHandle {
    sendExtensionEvent: (payload: JsonObject) => void;
  }

  let { pointName, context, onExtensionState, onExtensionRemoved }: Props = $props();
  const extensions = $derived(resolveChatExtensions($apps, pointName));
  let bundles = $state<Record<string, SurfaceUiBundle>>({});
  let failures = $state<Record<string, string>>({});
  let frames = $state<Record<string, ExtensionFrameHandle | undefined>>({});
  let hidden = $state<Record<string, boolean>>({});
  let previousExtensionKeys = new Set<string>();

  /// Route a slot-contract message to one contributing extension's frame.
  /// Returns whether a live frame was there to receive it: an extension that
  /// was disabled, uninstalled, or failed to load has no handle, and callers
  /// waiting for a reply must be able to tell that none is coming.
  export function sendExtensionEvent(extensionKey: string, payload: JsonObject): boolean {
    const frame = frames[extensionKey];
    if (!frame) return false;
    frame.sendExtensionEvent(payload);
    return true;
  }

  function handleExtensionState(
    key: string,
    appId: string,
    appName: string,
    payload: JsonObject,
  ): void {
    hidden = { ...hidden, [key]: payload.surface_visible === false };
    onExtensionState?.(key, appId, appName, payload);
  }

  const LOAD_ATTEMPTS = 3;
  const RETRY_DELAY_MS = 250;

  $effect(() => {
    const currentKeys = new Set(
      extensions.map(({ app, surface }) => `${app.manifest.app_id}/${surface.name}`),
    );
    for (const key of previousExtensionKeys) {
      if (!currentKeys.has(key)) onExtensionRemoved?.(key);
    }
    previousExtensionKeys = currentKeys;
  });

  function delay(milliseconds: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, milliseconds));
  }

  $effect(() => {
    const current = extensions;
    let cancelled = false;

    void (async () => {
      let validationFailure = "Extension context unavailable.";
      for (let attempt = 1; attempt <= LOAD_ATTEMPTS && !cancelled; attempt += 1) {
        try {
          await validateExtensionContext("chat", pointName, context);
          validationFailure = "";
          break;
        } catch (error) {
          validationFailure = String(error);
          if (attempt < LOAD_ATTEMPTS) await delay(RETRY_DELAY_MS);
        }
      }
      if (cancelled) return;
      const resolved = validationFailure
        ? current.map(({ app, surface }) => ({
            key: `${app.manifest.app_id}/${surface.name}`,
            bundle: null,
            failure: validationFailure,
          }))
        : await Promise.all(current.map(async ({ app, surface }) => {
            const key = `${app.manifest.app_id}/${surface.name}`;
            let failure = "Extension surface unavailable.";
            for (let attempt = 1; attempt <= LOAD_ATTEMPTS && !cancelled; attempt += 1) {
              try {
                const bundle = await getSurfaceUi(app.manifest.app_id, surface.name);
                if (!bundle) throw new Error("The app did not register its surface UI.");
                return { key, bundle, failure: null };
              } catch (error) {
                failure = String(error);
                if (attempt < LOAD_ATTEMPTS) await delay(RETRY_DELAY_MS);
              }
            }
            return { key, bundle: null, failure };
          }));
      if (cancelled) return;
      bundles = Object.fromEntries(
        resolved
          .filter((entry): entry is typeof entry & { bundle: SurfaceUiBundle } => entry.bundle !== null)
          .map((entry) => [entry.key, entry.bundle]),
      );
      failures = Object.fromEntries(
        resolved
          .filter((entry): entry is typeof entry & { failure: string } => entry.failure !== null)
          .map((entry) => [entry.key, entry.failure]),
      );
    })();
    return () => {
      cancelled = true;
    };
  });
</script>

{#each extensions as extension (`${extension.app.manifest.app_id}/${extension.surface.name}`)}
  {@const key = `${extension.app.manifest.app_id}/${extension.surface.name}`}
  {#if bundles[key]}
    <div class="extension-slot" class:hidden={hidden[key] === true}>
      <AppSurfaceFrame
        bind:this={frames[key]}
        app={extension.app}
        surface={extension.surface}
        bundle={bundles[key]}
        eager
        extensionContext={context}
        onExtensionState={(payload) => handleExtensionState(
          key,
          extension.app.manifest.app_id,
          extension.app.manifest.display_name,
          payload,
        )}
      />
    </div>
  {:else if failures[key]}
    <p class="extension-error" role="status">
      {extension.app.manifest.display_name} could not load: {failures[key]}
    </p>
  {:else}
    <div class="extension-loading">
      <LoadingIndicator size={1.25} label={`Loading ${extension.app.manifest.display_name}…`} />
    </div>
  {/if}
{/each}

<style>
  .extension-slot {
    margin-top: 0.55rem;
  }
  .extension-slot.hidden {
    display: none;
  }
  :global(.extension-slot .surface-frame) {
    margin-top: 0;
    gap: 0.35rem;
  }
  /* A chat extension is a small inline action, not a full-page app surface.
     The app's identity strip already labels it, so drop the framed box that
     otherwise strands the control in empty space. */
  :global(.extension-slot iframe) {
    border: none;
    border-radius: 0;
    background: transparent;
  }
  .extension-error {
    margin: 0.55rem 0 0;
    color: var(--color-danger-text);
    font-size: 0.82rem;
    overflow-wrap: anywhere;
  }
  .extension-loading {
    margin-top: 0.55rem;
    min-height: 2rem;
  }
</style>
