<script lang="ts">
  import { onMount, tick } from "svelte";
  import { listenHostEvent, openExternalUrl } from "$lib/hostTransport";
  import {
    cancelLlmOAuth,
    type LlmOAuthEvent,
    type TrustedNoticeRecord,
    resolveApproval,
    resolveInstallApproval,
    resolveLlmOAuthPrompt,
    type ChromeRequest,
  } from "$lib/api";
  import {
    approveLabel,
    dataScopeIsBroad,
    dataScopeSummary,
    conditionSummary,
    denyLabel,
    durationSummary,
    scopeSummary,
  } from "$lib/chrome/approvalLanguage";
  import {
    forgetStartedOAuthSession,
    recordOAuthSessionResult,
    setPendingChromeRequests,
    startedOAuthSessions,
  } from "$lib/stores/chromeState";
  import { appendTrustedNotice } from "$lib/stores/trustedNotices";
  import { openActivity, openPermission } from "$lib/stores/navigation";
  import {
    applyOAuthEvent,
    registerOAuthSession,
    type OAuthSessionState,
  } from "$lib/chrome/oauthChromeState";

  interface Props {
    onReady: () => void;
  }

  type HostDownloadEvent =
    | { kind: "requested"; file_name: string; directory: string }
    | { kind: "finished"; file_name: string; directory: string; success: boolean }
    | { kind: "failed"; file_name: string; error: string };

  interface DownloadNotice {
    id: number;
    fileName: string;
    message: string;
    status: "pending" | "success" | "failure";
  }

  let { onReady }: Props = $props();

  let queue = $state<ChromeRequest[]>([]);
  let notices = $state<TrustedNoticeRecord[]>([]);
  let downloadNotices = $state<DownloadNotice[]>([]);
  let nextDownloadNoticeId = 0;
  let deciding = $state(false);
  let decisionError = $state<string | null>(null);
  let oauthQueue = $state<OAuthSessionState[]>([]);
  let oauthValue = $state("");
  let oauthBusy = $state(false);
  let oauthError = $state<string | null>(null);
  let activeInteraction = $state<"approval" | "oauth" | null>(null);
  let dialog = $state<HTMLElement | null>(null);
  let denyButton = $state<HTMLButtonElement | null>(null);
  let oauthCancelButton = $state<HTMLButtonElement | null>(null);
  let oauthPrompt = $state<HTMLFormElement | null>(null);

  // Per-grant checkbox state for a batched install checklist, one boolean per
  // grant in the current install prompt. Reset whenever the prompt changes.
  let installSelections = $state<boolean[]>([]);

  const current = $derived(activeInteraction === "approval" ? queue[0] : undefined);
  const currentOAuth = $derived(activeInteraction === "oauth" ? oauthQueue[0] : undefined);
  const scope = $derived(
    current?.kind === "grant-issuance" ? scopeSummary(current.prompt.scope) : null,
  );
  const dataScope = $derived(
    current?.kind === "grant-issuance" ? dataScopeSummary(current.prompt.data_scope) : null,
  );

  onMount(() => {
    const unlistenRequest = listenHostEvent<ChromeRequest>("trusted-chrome:request", (request) => {
      queue = [...queue, request];
      activeInteraction ??= "approval";
    });
    const unlistenNotice = listenHostEvent<TrustedNoticeRecord>("trusted-chrome:notice", (notice) => {
      appendTrustedNotice(notice);
      notices = [...notices, notice].slice(-5);
      setTimeout(() => {
        notices = notices.filter((entry) => entry.sequence !== notice.sequence);
      }, 6000);
    });
    const unlistenExpired = listenHostEvent<number>("trusted-chrome:request-expired", (requestId) => {
      queue = queue.filter((request) => request.request_id !== requestId);
      activeInteraction = queue.length > 0 ? "approval" : oauthQueue.length > 0 ? "oauth" : null;
    });
    const unlistenOAuth = listenHostEvent<LlmOAuthEvent>("trusted-chrome:oauth", (event) => {
      oauthQueue = applyOAuthEvent(oauthQueue, event);
      if (event.kind === "completed" || event.kind === "failed") {
        recordOAuthSessionResult({
          sessionId: event.session_id,
          status: event.kind,
          message: event.kind === "failed" ? event.message : null,
        });
      }
      activeInteraction ??= "oauth";
    });
    const unlistenDownload = listenHostEvent<HostDownloadEvent>("host-download:event", (event) => {
      const id = ++nextDownloadNoticeId;
      const notice: DownloadNotice = event.kind === "requested"
        ? {
            id,
            fileName: event.file_name,
            message: `Saving ${event.file_name} to ${event.directory}`,
            status: "pending",
          }
        : event.kind === "finished" && event.success
          ? {
              id,
              fileName: event.file_name,
              message: `Saved ${event.file_name} to ${event.directory}`,
              status: "success",
            }
          : event.kind === "finished"
            ? {
                id,
                fileName: event.file_name,
                message: `Could not save ${event.file_name}`,
                status: "failure",
              }
            : {
                id,
                fileName: event.file_name,
                message: `Could not save ${event.file_name}: ${event.error}`,
                status: "failure",
              };
      const retained = event.kind === "finished"
        ? downloadNotices.filter(
            (entry) => entry.status !== "pending" || entry.fileName !== event.file_name,
          )
        : downloadNotices;
      downloadNotices = [...retained, notice].slice(-5);
      setTimeout(() => {
        downloadNotices = downloadNotices.filter((entry) => entry.id !== id);
      }, 6000);
    });
    const unsubscribeStarted = startedOAuthSessions.subscribe((sessions) => {
      for (const sessionId of sessions) {
        oauthQueue = registerOAuthSession(oauthQueue, sessionId);
        activeInteraction ??= "oauth";
      }
    });
    Promise.all([
      unlistenRequest,
      unlistenNotice,
      unlistenExpired,
      unlistenOAuth,
      unlistenDownload,
    ]).then(() => onReady());
    return () => {
      unlistenRequest.then((unlisten) => unlisten());
      unlistenNotice.then((unlisten) => unlisten());
      unlistenExpired.then((unlisten) => unlisten());
      unlistenOAuth.then((unlisten) => unlisten());
      unlistenDownload.then((unlisten) => unlisten());
      unsubscribeStarted();
    };
  });

  async function decide(approved: boolean) {
    if (!current || deciding) return;
    deciding = true;
    decisionError = null;
    try {
      await resolveApproval(current.request_id, approved);
      // Only advance once the decision actually landed. A dropped Deny on a
      // trust boundary must never look like it succeeded.
      queue = queue.slice(1);
      activeInteraction = queue.length > 0 ? "approval" : oauthQueue.length > 0 ? "oauth" : null;
    } catch (failure) {
      decisionError = String(failure);
    } finally {
      deciding = false;
    }
  }

  // Answer a batched install checklist in one decision. Approving issues the
  // grants the user left checked (the app installs with whatever it is given);
  // denying refuses everything, including the app's event feed.
  async function decideInstall(approve: boolean) {
    if (!current || current.kind !== "install-approval" || deciding) return;
    deciding = true;
    decisionError = null;
    try {
      const grantApprovals = approve
        ? current.prompt.grants.map((_, index) => installSelections[index] ?? false)
        : current.prompt.grants.map(() => false);
      const eventApproved = current.prompt.event ? approve : null;
      await resolveInstallApproval(current.request_id, eventApproved, grantApprovals);
      queue = queue.slice(1);
      activeInteraction = queue.length > 0 ? "approval" : oauthQueue.length > 0 ? "oauth" : null;
    } catch (failure) {
      decisionError = String(failure);
    } finally {
      deciding = false;
    }
  }

  // A fresh request clears any stale error and moves focus to Deny (the safe
  // default), so keyboard users land inside the dialog on the non-committing
  // choice.
  $effect(() => {
    if (!current) return;
    current.request_id;
    decisionError = null;
    if (current.kind === "install-approval") {
      // Narrow grants start checked (the app needs them to be useful), but
      // broad wildcard grants start unchecked so the widest access is only ever
      // granted by a deliberate click, never by inertia.
      installSelections = current.prompt.grants.map(
        (grant) => !scopeSummary(grant.scope).wildcard && !dataScopeIsBroad(grant.data_scope),
      );
    }
    void tick().then(() => denyButton?.focus());
  });

  $effect(() => {
    setPendingChromeRequests(queue.length + oauthQueue.length);
  });

  $effect(() => {
    if (!currentOAuth) return;
    currentOAuth.sessionId;
    currentOAuth.prompt?.promptId;
    oauthValue = "";
    oauthError = null;
    void tick().then(() => {
      const promptControl = oauthPrompt?.querySelector<HTMLInputElement>("input");
      (currentOAuth.prompt ? promptControl : oauthCancelButton)?.focus();
    });
  });

  async function openSignInPage(url: string) {
    oauthError = null;
    try {
      await openExternalUrl(url);
    } catch (failure) {
      oauthError = `Could not open the sign-in page. ${String(failure)}`;
    }
  }

  async function submitOAuthPrompt() {
    if (!currentOAuth?.prompt || oauthBusy || oauthValue.trim() === "") return;
    oauthBusy = true;
    oauthError = null;
    try {
      await resolveLlmOAuthPrompt(
        currentOAuth.sessionId,
        currentOAuth.prompt.promptId,
        oauthValue,
        false,
      );
      oauthQueue = oauthQueue.map((session) => session.sessionId === currentOAuth.sessionId
        ? { ...session, prompt: null, progress: "Waiting for the sign-in provider…" }
        : session);
      oauthValue = "";
    } catch (failure) {
      oauthError = `Your response didn't go through. ${String(failure)}`;
    } finally {
      oauthBusy = false;
    }
  }

  async function cancelOAuth() {
    if (!currentOAuth || oauthBusy) return;
    oauthBusy = true;
    oauthError = null;
    try {
      await cancelLlmOAuth(currentOAuth.sessionId);
      closeOAuth(currentOAuth.sessionId);
    } catch (failure) {
      oauthError = `Cancellation didn't go through. ${String(failure)}`;
    } finally {
      oauthBusy = false;
    }
  }

  async function cancelOAuthPrompt() {
    if (!currentOAuth?.prompt || oauthBusy) return;
    oauthBusy = true;
    oauthError = null;
    try {
      await resolveLlmOAuthPrompt(
        currentOAuth.sessionId,
        currentOAuth.prompt.promptId,
        null,
        true,
      );
      oauthQueue = oauthQueue.map((session) => session.sessionId === currentOAuth.sessionId
        ? { ...session, prompt: null, progress: "Cancelling sign-in…" }
        : session);
      oauthValue = "";
    } catch (failure) {
      oauthError = `Cancellation didn't go through. ${String(failure)}`;
    } finally {
      oauthBusy = false;
    }
  }

  function closeOAuth(sessionId: string) {
    oauthQueue = oauthQueue.filter((session) => session.sessionId !== sessionId);
    activeInteraction = oauthQueue.length > 0 ? "oauth" : queue.length > 0 ? "approval" : null;
    forgetStartedOAuthSession(sessionId);
    oauthValue = "";
    oauthError = null;
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      // Escape is an explicit, safe dismissal: it denies rather than leaving
      // the request unresolved.
      event.preventDefault();
      if (current) {
        // Deny through the same path the buttons use: an install checklist
        // answers on its own channel, so a plain decide(false) would hit the
        // wrong backend receiver and leave the request wedged.
        void (current.kind === "install-approval" ? decideInstall(false) : decide(false));
      } else if (currentOAuth?.status === "completed" || currentOAuth?.status === "failed") {
        closeOAuth(currentOAuth.sessionId);
      } else {
        void cancelOAuth();
      }
      return;
    }
    if (event.key !== "Tab" || !dialog) return;
    const focusable = dialog.querySelectorAll<HTMLElement>(
      "button:not(:disabled), input:not(:disabled), select:not(:disabled), [href], [tabindex]:not([tabindex='-1'])",
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  function openNoticeActivity(notice: TrustedNoticeRecord) {
    if (notice.notice.kind !== "grant-use") return;
    openActivity(notice.notice.run_id, notice.notice.grant_id);
  }

  function openNoticePermission(notice: TrustedNoticeRecord) {
    if (notice.notice.kind !== "grant-use") return;
    openPermission(notice.notice.grant_id);
  }
</script>

{#if current}
  <div class="chrome-backdrop">
    <div
      class="chrome-dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="chrome-title"
      aria-describedby="chrome-body"
      bind:this={dialog}
      onkeydown={handleKeydown}
      tabindex="-1"
    >
      <div class="chrome-top">
        <div class="chrome-badge">Requested through Kestral</div>
        {#if queue.length > 1}
          <span class="queue-count">Request 1 of {queue.length}</span>
        {/if}
      </div>
      <div id="chrome-body">
        {#if current.kind === "grant-issuance" && scope}
          <h2 id="chrome-title">Permission request</h2>
          <p>
            <strong>{current.prompt.app_display_name}</strong> is asking for a standing
            permission.
          </p>
          <p class="grant-scope" class:wildcard={scope.wildcard || dataScopeIsBroad(current.prompt.data_scope)}>
            {scope.text}
            <code>{scope.code}</code>
          </p>
          <p class="detail">Data scope: {dataScope}</p>
          {#if scope.wildcard || dataScopeIsBroad(current.prompt.data_scope)}
            <p class="scope-warning">
              {scope.wildcard
                ? "This is broad access, not a single action."
                : "Broad data access, including resources added later."}
            </p>
          {/if}
          <p class="consequence">{conditionSummary(current.prompt.condition)}</p>
          <p class="consequence">{durationSummary(current.prompt.duration)}</p>
          <p class="detail">Why: {current.prompt.reason}</p>
        {:else if current.kind === "event-subscription"}
          <h2 id="chrome-title">Event subscription request</h2>
          <p>
            <strong>{current.prompt.app_display_name}</strong> wants to be notified about
            these host events:
          </p>
          <ul class="topic-list">
            {#each current.prompt.topics as topic}
              <li><code>{topic}</code></li>
            {/each}
          </ul>
        {:else if current.kind === "capability-approval"}
          <h2 id="chrome-title">Allow this action once?</h2>
          <p>
            <strong>{current.prompt.app_display_name}</strong> wants to run
            <code>{current.prompt.capability.provider}/{current.prompt.capability.capability}</code>
            one time.
          </p>
          <p class="detail">Data scope: {dataScopeSummary(current.prompt.data_scope)}</p>
          <p class="detail"><span class="attribution">The app says:</span> {current.prompt.goal}</p>
        {:else if current.kind === "install-approval"}
          <h2 id="chrome-title">Review requested permissions</h2>
          <p>
            Review the full request together. Uncheck anything you'd rather withhold.
          </p>
          <ul class="grant-checklist">
            {#each current.prompt.grants as grant, index (index)}
              {@const grantScope = scopeSummary(grant.scope)}
              <li class="grant-item" class:wildcard={grantScope.wildcard || dataScopeIsBroad(grant.data_scope)}>
                <label>
                  <input type="checkbox" bind:checked={installSelections[index]} disabled={deciding} />
                  <span class="grant-body">
                    <span class="grant-holder">For {grant.app_display_name}</span>
                    <span class="grant-headline">
                      {grantScope.text}
                      <code>{grantScope.code}</code>
                    </span>
                    <span class="grant-datascope">Data scope: {dataScopeSummary(grant.data_scope)}</span>
                    {#if grantScope.wildcard || dataScopeIsBroad(grant.data_scope)}
                      <span class="scope-warning">
                        {grantScope.wildcard
                          ? "Broad access, not a single action."
                          : "Broad data access, including resources added later."}
                      </span>
                    {/if}
                    <span class="grant-consequence">{conditionSummary(grant.condition)} · {durationSummary(grant.duration)}</span>
                    <span class="detail">Why: {grant.reason}</span>
                  </span>
                </label>
              </li>
            {/each}
          </ul>
          {#if current.prompt.event}
            <p class="detail event-note">
              Installing also subscribes it to these host events (required to install):
            </p>
            <ul class="topic-list">
              {#each current.prompt.event.topics as topic}
                <li><code>{topic}</code></li>
              {/each}
            </ul>
          {/if}
        {/if}
      </div>

      {#if decisionError}
        <p class="decision-error" role="alert">
          Your response didn't go through. {decisionError}
        </p>
      {/if}

      <p class="timeout-note">Left unanswered, this is denied automatically after 5 minutes.</p>

      <div class="chrome-actions">
        <button
          class="deny"
          bind:this={denyButton}
          onclick={() => (current?.kind === "install-approval" ? decideInstall(false) : decide(false))}
          disabled={deciding}
        >
          {denyLabel()}
        </button>
        <button
          class="approve"
          onclick={() => (current?.kind === "install-approval" ? decideInstall(true) : decide(true))}
          disabled={deciding}
        >
          {deciding ? "Working…" : approveLabel(current.kind)}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if currentOAuth}
  <div class="chrome-backdrop">
    <div
      class="chrome-dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="oauth-title"
      aria-describedby="oauth-body"
      bind:this={dialog}
      onkeydown={handleKeydown}
      tabindex="-1"
    >
      <div class="chrome-top">
        <div class="chrome-badge">Requested through Kestral</div>
        {#if oauthQueue.length + queue.length > 1}
          <span class="queue-count">Interaction 1 of {oauthQueue.length + queue.length}</span>
        {/if}
      </div>
      <div id="oauth-body">
        <h2 id="oauth-title">
          {currentOAuth.status === "completed" ? "Sign-in complete" : "Model account sign-in"}
        </h2>
        <p>
          Browser sign-in grants model-account access to this local host. Continue only if you
          recognize and trust the provider page.
        </p>

        {#if currentOAuth.authUrl}
          {#if currentOAuth.authUrl.instructions}
            <p class="detail">{currentOAuth.authUrl.instructions}</p>
          {/if}
          <div class="recovery-block">
            <span>Sign-in URL</span>
            <code>{currentOAuth.authUrl.url}</code>
            <button type="button" class="approve" onclick={() => openSignInPage(currentOAuth.authUrl!.url)}>
              Open sign-in page
            </button>
          </div>
        {/if}

        {#if currentOAuth.deviceCode}
          <div class="recovery-block">
            <span>Device code</span>
            <strong class="device-code">{currentOAuth.deviceCode.userCode}</strong>
            <span>Verification URL</span>
            <code>{currentOAuth.deviceCode.verificationUri}</code>
            <button type="button" class="approve" onclick={() => openSignInPage(currentOAuth.deviceCode!.verificationUri)}>
              Open sign-in page
            </button>
          </div>
        {/if}

        {#if currentOAuth.prompt}
          <form bind:this={oauthPrompt} class="oauth-prompt" onsubmit={(event) => { event.preventDefault(); void submitOAuthPrompt(); }}>
            {#if currentOAuth.prompt.value.type === "select"}
              <fieldset>
                <legend>{currentOAuth.prompt.value.message}</legend>
                {#each currentOAuth.prompt.value.options as option (option.id)}
                  <label class="radio-option">
                    <input
                      type="radio"
                      name="oauth-option"
                      value={option.id}
                      bind:group={oauthValue}
                      disabled={oauthBusy}
                    />
                    <span><strong>{option.label}</strong>{#if option.description}<small>{option.description}</small>{/if}</span>
                  </label>
                {/each}
              </fieldset>
            {:else}
              <label>
                {currentOAuth.prompt.value.message}
                <input
                  type={currentOAuth.prompt.value.type === "secret" ? "password" : "text"}
                  placeholder={currentOAuth.prompt.value.placeholder ?? undefined}
                  bind:value={oauthValue}
                  autocomplete="off"
                  disabled={oauthBusy}
                />
              </label>
            {/if}
            <div class="chrome-actions">
              <button type="button" class="deny" bind:this={oauthCancelButton} onclick={cancelOAuthPrompt} disabled={oauthBusy}>Cancel</button>
              <button type="submit" class="approve" disabled={oauthBusy || oauthValue.trim() === ""}>Continue</button>
            </div>
          </form>
        {:else if currentOAuth.status === "completed"}
          <p class="completion" role="status" aria-live="assertive">
            Your model account is connected. The credential is stored by the host and is not shown here.
          </p>
        {:else if currentOAuth.status === "failed"}
          <p class="decision-error" role="alert">Sign-in failed. {currentOAuth.failure}</p>
        {:else if currentOAuth.progress}
          <p class="oauth-progress" role="status">{currentOAuth.progress}</p>
        {/if}
      </div>

      {#if oauthError}
        <p class="decision-error" role="alert">{oauthError}</p>
      {/if}

      {#if !currentOAuth.prompt}
        <div class="chrome-actions">
          {#if currentOAuth.status === "completed" || currentOAuth.status === "failed"}
            <button type="button" class="deny" bind:this={oauthCancelButton} onclick={() => closeOAuth(currentOAuth.sessionId)}>Close</button>
          {:else}
            <button type="button" class="deny" bind:this={oauthCancelButton} onclick={cancelOAuth} disabled={oauthBusy}>Cancel</button>
          {/if}
        </div>
      {/if}
    </div>
  </div>
{/if}

<div class="notices" role="status" aria-live="polite">
  {#each downloadNotices as notice (notice.id)}
    <div class="download-notice {notice.status}">{notice.message}</div>
  {/each}
  {#each notices as notice (notice.sequence)}
    <div class="notice">
      {#if notice.notice.kind === "grant-use"}
        <button
          type="button"
          class="notice-link"
          onclick={() => openNoticeActivity(notice)}
          aria-label={`View activity: ${notice.notice.app_id} used ${notice.notice.capability.provider}/${notice.notice.capability.capability}`}
        >
          {notice.notice.app_id} used {notice.notice.capability.provider}/{notice.notice.capability.capability}
        </button>
          <button
            type="button"
            class="notice-settings"
            onclick={() => openNoticePermission(notice)}
            aria-label={`Open the permission used by ${notice.notice.app_id}`}
            title="Open permission settings"
          >
            <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
              <path d="M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Z" />
              <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.86 2.86-.06-.06A1.7 1.7 0 0 0 15 19.4a1.7 1.7 0 0 0-1 .6 1.7 1.7 0 0 0-.4 1.1V21H9.5v-.1A1.7 1.7 0 0 0 8.4 19.4a1.7 1.7 0 0 0-1.88.34l-.06.06-2.86-2.86.06-.06A1.7 1.7 0 0 0 4 15a1.7 1.7 0 0 0-.6-1 1.7 1.7 0 0 0-1.1-.4H2V9.5h.3A1.7 1.7 0 0 0 4 8.4a1.7 1.7 0 0 0-.34-1.88l-.06-.06L6.46 3.6l.06.06A1.7 1.7 0 0 0 8.4 4a1.7 1.7 0 0 0 1-.6A1.7 1.7 0 0 0 9.8 2H14v.3A1.7 1.7 0 0 0 15 4a1.7 1.7 0 0 0 1.88-.34l.06-.06 2.86 2.86-.06.06A1.7 1.7 0 0 0 19.4 8.4a1.7 1.7 0 0 0 .6 1 1.7 1.7 0 0 0 1.1.4h.1V14h-.1a1.7 1.7 0 0 0-1.7 1Z" />
            </svg>
          </button>
      {:else}
        Lease conflict on {notice.notice.resource}
      {/if}
    </div>
  {/each}
</div>

<style>
  .chrome-backdrop {
    position: fixed;
    inset: 0;
    background: var(--color-scrim);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1rem;
    box-sizing: border-box;
    z-index: 1000;
  }
  .chrome-dialog {
    background: var(--color-chrome-bg);
    color: var(--color-chrome-text);
    border: 3px solid var(--color-chrome-accent);
    border-radius: 14px;
    padding: 1.25rem 1.5rem;
    max-width: 30rem;
    max-height: 100%;
    overflow-y: auto;
    box-sizing: border-box;
    box-shadow: 0 12px 48px var(--color-shadow-strong);
  }
  .chrome-dialog:focus {
    outline: none;
  }
  .chrome-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    margin-bottom: 0.75rem;
  }
  .chrome-badge {
    display: inline-block;
    background: var(--color-chrome-accent);
    color: var(--color-chrome-accent-contrast);
    font-weight: 700;
    font-size: 0.75rem;
    padding: 0.25rem 0.55rem;
    border-radius: 6px;
  }
  .queue-count {
    font-size: 0.75rem;
    color: var(--color-chrome-text-muted);
  }
  h2 {
    margin: 0 0 0.5rem;
    font-size: 1.15rem;
  }
  p {
    margin: 0 0 0.5rem;
    line-height: 1.45;
  }
  .grant-scope {
    display: grid;
    gap: 0.25rem;
    padding: 0.6rem 0.7rem;
    border-radius: 8px;
    color: var(--color-chrome-text);
    background: var(--color-chrome-panel-bg);
    border: 1px solid var(--color-chrome-panel-border);
  }
  .grant-scope.wildcard {
    font-weight: 700;
  }
  .grant-scope code {
    font-size: 0.8rem;
    color: var(--color-chrome-text-muted);
    font-weight: 400;
  }
  .scope-warning {
    color: var(--color-chrome-accent);
    font-weight: 700;
    font-size: 0.85rem;
  }
  .consequence {
    font-weight: 600;
  }
  .detail {
    color: var(--color-chrome-text-muted);
  }
  .attribution {
    font-style: italic;
  }
  .decision-error {
    padding: 0.5rem 0.65rem;
    border-radius: 8px;
    border: 1px solid var(--color-chrome-accent);
    color: var(--color-chrome-text);
    font-size: 0.85rem;
  }
  .grant-checklist {
    list-style: none;
    margin: 0.5rem 0;
    padding: 0;
    display: grid;
    gap: 0.6rem;
  }
  .grant-item {
    color: var(--color-chrome-text);
    border: 1px solid var(--color-chrome-panel-border);
    border-radius: 8px;
    background: var(--color-chrome-panel-bg);
  }
  .grant-item.wildcard {
    border-color: var(--color-chrome-accent);
  }
  .grant-item label {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 0.6rem;
    align-items: start;
    padding: 0.6rem 0.7rem;
    cursor: pointer;
  }
  .grant-item input[type="checkbox"] {
    margin-top: 0.15rem;
    width: 1.1rem;
    height: 1.1rem;
    cursor: pointer;
  }
  .grant-body {
    display: grid;
    gap: 0.25rem;
    min-width: 0;
  }
  .grant-holder {
    color: var(--color-chrome-text-muted);
    font-size: 0.78rem;
  }
  .grant-headline {
    font-weight: 600;
  }
  .grant-headline code {
    display: inline-block;
    margin-left: 0.35rem;
    font-size: 0.8rem;
    font-weight: 400;
    color: var(--color-chrome-text-muted);
  }
  .grant-consequence {
    font-size: 0.85rem;
    font-weight: 600;
  }
  .grant-body .detail {
    font-size: 0.85rem;
  }
  .event-note {
    margin-top: 0.75rem;
  }
  .topic-list {
    margin: 0.25rem 0 0.5rem 0;
    padding-left: 1.25rem;
    list-style: disc;
  }
  .topic-list li {
    margin-bottom: 0.2rem;
  }
  code {
    overflow-wrap: anywhere;
  }
  .timeout-note {
    margin: 0.75rem 0 0;
    color: var(--color-chrome-text-muted);
    font-size: 0.78rem;
  }
  .chrome-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 0.75rem;
    margin-top: 1rem;
  }
  button {
    padding: 0.45rem 1rem;
    border-radius: 8px;
    border: none;
    cursor: pointer;
    font-weight: 600;
    min-height: 2rem;
  }
  .approve {
    background: var(--color-chrome-approve);
    color: var(--color-chrome-approve-text);
  }
  .deny {
    background: var(--color-chrome-deny);
    color: var(--color-chrome-text);
  }
  button:focus-visible {
    outline: 2px solid var(--color-chrome-text);
    outline-offset: 2px;
  }
  .notices {
    position: fixed;
    right: 1rem;
    bottom: 1rem;
    max-width: calc(100vw - 2rem);
    max-height: calc(100vh - 2rem);
    max-height: calc(100dvh - 2rem);
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    overflow-y: auto;
    overscroll-behavior: contain;
    z-index: 900;
  }
  .recovery-block,
  .oauth-prompt {
    display: grid;
    gap: 0.6rem;
    margin-top: 0.85rem;
    padding: 0.75rem;
    color: var(--color-chrome-text);
    border: 1px solid var(--color-chrome-panel-border);
    border-radius: 8px;
    background: var(--color-chrome-panel-bg);
    overflow-wrap: anywhere;
  }
  .recovery-block .approve {
    width: fit-content;
  }
  .device-code {
    font-size: 1.25rem;
    letter-spacing: 0.12em;
    overflow-wrap: anywhere;
  }
  .oauth-prompt label,
  .oauth-prompt fieldset {
    display: grid;
    gap: 0.45rem;
  }
  .oauth-prompt fieldset {
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }
  .oauth-prompt input[type="text"],
  .oauth-prompt input[type="password"] {
    min-width: 0;
    padding: 0.6rem 0.7rem;
    border: 1px solid var(--color-chrome-panel-border);
    border-radius: 8px;
  }
  .radio-option {
    grid-template-columns: auto minmax(0, 1fr);
    align-items: start;
    min-height: 1.5rem;
  }
  .radio-option small {
    display: block;
    color: var(--color-chrome-text-muted);
  }
  .completion,
  .oauth-progress {
    margin-top: 0.75rem;
    font-weight: 600;
  }
  .notice {
    background: var(--color-chrome-bg);
    color: var(--color-chrome-text);
    border: 2px solid var(--color-chrome-accent);
    border-radius: 10px;
    max-width: 22.5rem;
    box-sizing: border-box;
    display: flex;
    align-items: stretch;
    overflow: hidden;
  }
  .download-notice {
    max-width: min(22.5rem, 100%);
    padding: 0.65rem 0.8rem;
    border: 1px solid var(--color-accent-border);
    border-radius: 10px;
    background: var(--color-surface-raised);
    color: var(--color-text);
    box-shadow: 0 8px 24px var(--color-shadow-strong);
    overflow-wrap: anywhere;
  }
  .download-notice.success {
    border-color: var(--color-success-border);
    background: var(--color-success-soft);
    color: var(--color-success-text);
  }
  .download-notice.failure {
    border-color: var(--color-danger-border);
    background: var(--color-danger-soft);
    color: var(--color-danger-text);
  }
  .notice-link,
  .notice-settings {
    min-width: 0;
    min-height: 2.75rem;
    border-radius: 0;
    background: transparent;
    color: inherit;
  }
  .notice-link {
    flex: 1 1 auto;
    padding: 0.5rem 0.75rem;
    text-align: left;
  }
  .notice-settings {
    flex: 0 0 2.75rem;
    padding: 0.5rem;
    border-left: 1px solid var(--color-chrome-panel-border);
    display: grid;
    place-items: center;
  }
  .notice-settings svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
