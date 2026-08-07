<script lang="ts">
  import "$lib/stores/theme";
  import { pairRemoteConnection, remoteUrlDefault, signInRemoteConnection } from "$lib/hostTransport";
  import { passkeysAvailable } from "$lib/remotePasskeys";

  interface Props {
    onConnected: () => void;
  }

  let { onConnected }: Props = $props();
  let url = $state(remoteUrlDefault());
  let pairingCode = $state("");
  let connecting = $state(false);
  let error = $state<string | null>(null);

  async function signIn() {
    await runConnection(() => signInRemoteConnection(url));
  }

  async function pair(event: SubmitEvent) {
    event.preventDefault();
    await runConnection(() => pairRemoteConnection(url, pairingCode));
  }

  async function runConnection(connect: () => Promise<void>) {
    connecting = true;
    error = null;
    try {
      await connect();
      onConnected();
    } catch (failure) {
      error = connectionErrorMessage(failure);
    } finally {
      connecting = false;
    }
  }

  // A failed `fetch` to an unreachable host throws a bare "TypeError: Failed to
  // fetch", which tells the user nothing actionable. Validation and auth
  // failures already carry human-readable messages, so pass those through.
  function connectionErrorMessage(failure: unknown): string {
    const message = failure instanceof Error ? failure.message : String(failure);
    if (/failed to fetch|networkerror|load failed/i.test(message)) {
      return `Could not reach a host at ${url}. Check the URL and that the backend is running.`;
    }
    return message;
  }
</script>

<main class="connection-page">
  <section class="connection-card">
    <p class="eyebrow">Kestral</p>
    <h1>Connect to your host</h1>
    <p class="description">
      Sign in with a passkey to use this browser as your trusted owner console. Kestral files, configuration, and provider secrets stay on the host.
    </p>
    <label>
      Host URL
      <input bind:value={url} type="url" autocomplete="url" required />
    </label>
    {#if !passkeysAvailable()}
      <p class="error" role="alert">This browser does not support passkeys. Use a current browser over HTTPS.</p>
    {/if}
    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}
    <button type="button" onclick={signIn} disabled={connecting || !passkeysAvailable()}>
      {connecting ? "Waiting for passkey..." : "Sign in with passkey"}
    </button>
    <div class="divider" aria-hidden="true"><span>Pair a new browser</span></div>
    <form class="pairing" onsubmit={pair}>
      <p class="pairing-help">Run <code>host-server owner pair</code> through SSH, then enter the one-time code shown there.</p>
      <label>
        Pairing code
        <input bind:value={pairingCode} type="password" autocomplete="one-time-code" required />
      </label>
      <button type="submit" disabled={connecting || !passkeysAvailable()}>
        {connecting ? "Waiting for passkey..." : "Pair this browser"}
      </button>
    </form>
  </section>
</main>

<style>
  :global(*) {
    box-sizing: border-box;
  }
  :global(body) {
    margin: 0;
    font-family: Inter, "Segoe UI", system-ui, sans-serif;
    color: var(--color-text);
    background: var(--color-bg-gradient-b);
  }
  .connection-page {
    min-height: 100vh;
    min-height: 100dvh;
    display: grid;
    place-items: center;
    padding: clamp(1rem, 4vw, 3rem);
  }
  .connection-card {
    width: min(100%, 28rem);
    display: grid;
    gap: 1rem;
    padding: clamp(1.25rem, 5vw, 2.5rem);
    border: 1px solid var(--color-border);
    border-radius: 1rem;
    background: var(--color-surface);
    box-shadow: 0 1rem 3rem var(--color-shadow-strong);
  }
  .eyebrow {
    margin: 0;
    color: var(--color-accent-text);
    font-size: 0.75rem;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }
  h1 {
    margin: 0;
    font-size: clamp(1.75rem, 1.5rem + 1vw, 2.25rem);
  }
  .description {
    margin: 0 0 0.5rem;
    color: var(--color-text-muted);
    line-height: 1.5;
  }
  .pairing {
    display: grid;
    gap: 1rem;
  }
  .pairing-help {
    margin: 0;
    color: var(--color-text-muted);
    line-height: 1.5;
  }
  code {
    font-family: "Cascadia Code", "SFMono-Regular", Consolas, monospace;
  }
  .divider {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    color: var(--color-text-muted);
    font-size: 0.8rem;
    font-weight: 700;
    text-transform: uppercase;
  }
  .divider::before,
  .divider::after {
    content: "";
    height: 1px;
    flex: 1;
    background: var(--color-border);
  }
  label {
    display: grid;
    gap: 0.4em;
    font-weight: 600;
  }
  input {
    min-height: 2.75rem;
    width: 100%;
    padding: 0.65em 0.75em;
    border: 1px solid var(--color-border);
    border-radius: 0.5rem;
    color: var(--color-text);
    background: var(--color-surface-raised);
    font: inherit;
  }
  input:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  button {
    min-height: 2.75rem;
    margin-top: 0.5rem;
    border: 0;
    border-radius: 0.5rem;
    color: var(--color-accent-contrast);
    background: var(--color-accent);
    font: inherit;
    font-weight: 700;
    cursor: pointer;
  }
  button:disabled {
    cursor: wait;
    opacity: 0.65;
  }
  .error {
    margin: 0;
    color: var(--color-danger-text);
  }
</style>
