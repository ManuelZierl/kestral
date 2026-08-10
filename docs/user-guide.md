---
title: Using Kestral
layout: default
nav_order: 3
has_children: true
---

{% assign internal_link_prefix = "" %}{% assign jekyll_major = jekyll.version | split: "." | first %}{% if jekyll_major == "3" %}{% assign internal_link_prefix = site.baseurl %}{% endif %}

# Using Kestral
{: .no_toc }

1. TOC
{:toc}

Kestral is a workspace of app-owned experiences, not only a conversation
screen. Chat is installed as the starting app because it offers a low-friction
way to connect a model, ask open-ended questions, and coordinate granted tools.
For repeated, structured, visual, or stateful work, installed apps can provide a
more appropriate document, canvas, form, dashboard, or other focused surface.

## Main screens

| Screen | Purpose |
|---|---|
| **Chat** | Manage threads and talk to the configured model and granted app tools. |
| **Apps** | Inspect, install, update, enable, disable, and uninstall apps. |
| **Artifacts** | Browse durable output produced by app runs, including provenance. |
| **Settings** | Configure models, profiles, tool servers, file resources, package trust, app settings, permissions, appearance, and advanced features. |
| **System** | Inspect Run activity, trusted notices, local storage status, host information, and the current-profile reset. |

Installed apps with standalone panel or dashboard surfaces also appear in the
sidebar. A dashboard remains available there when it also integrates with
another app. The fresh profile's Kestral documentation MCP app is hidden there
by default to keep the workspace navigation focused; it remains available in
**Customize navigation**. Use the cog button on an app's **Apps** card to open
its settings.
Chat opens **Settings → Chat**, LLM Provider opens **Settings → Model
providers**, and File Broker opens **Settings → File resources**. Other active
apps open **Settings → App settings**. Inactive apps show what must be resolved
first. The host-owned top bar on Chat and standalone app screens has the same
settings shortcut plus a shield button that opens **Settings → Permissions**,
then scrolls to and briefly highlights that app's permission group when it has
permissions to show.

Use **Customize navigation** at the bottom of the sidebar to show or hide any
host screen or installed-app destination and move it up or down. The editor is
always available even when every destination is hidden, and **Reset to default**
restores the standard order and visibility. Sidebar order, visibility, and
collapsed state are stored only in the current browser or desktop webview; newly
installed apps are appended automatically, except for the default-hidden Kestral
documentation MCP destination. A current-profile system reset also clears this
browser-local layout.

## Work in installed apps

Open an installed app from the sidebar to work in its own surface. The app owns
that task-specific interface; Kestral supplies the sandbox, theme, lifecycle,
validated intents, and permission path. Direct actions in the app remain
attributable capability Runs without an unnecessary second approval prompt.

Apps can also expose capabilities to other granted callers, contribute a
contextual surface to another app, or produce artifacts that retain provenance.
This allows Chat to coordinate work without forcing the complete workflow into
a transcript. Review apps under [Managing apps]({{ internal_link_prefix }}{% link managing-apps.md %}) and
use [Curated apps]({{ internal_link_prefix }}{% link curated-apps.md %}) for independent starting points.

## Appearance

Open **Settings → Appearance** to use the device's System theme, the built-in
Light or Dark theme, or a custom color profile. System follows the operating
system and is the default. Built-in profiles cannot be edited or deleted.

Use the **Create profile** (+) action, name the profile, and start from Light or Dark. Color controls
are grouped by their role; each supports a picker and a precise HEX, `rgb()`, or
`rgba()` value. Saving a new profile selects it immediately. Existing profiles
can be selected, edited, or deleted, and deleting the active profile returns
Kestral to System. Custom profiles are stored only in the current browser or
desktop webview and are removed by a current-profile system reset.

Use a profile's **Export** icon to save a portable JSON copy, or the **Import**
icon to validate, create, and select a profile from a file. An imported name
must not duplicate an existing profile. Installed apps can contribute clearly
named app-specific colors; these appear as additional groups while editing a
custom profile. Apps otherwise use Kestral's existing semantic colors, so one
profile applies to both the shell and sandboxed app surfaces.

## Model providers

Provider profiles live under **Settings → Model providers**. Local and API-key
profiles use **Add another provider**. **Add ChatGPT account** creates a Codex
subscription profile with the ChatGPT backend and a model from the pinned
pi-ai catalog. **Default for Chat** selects the saved profile and configured
model Chat will use, so provider setup and default selection stay on one page.
While creating a profile, select **Use as Chat default** to apply it immediately
after saving. Cloud profiles still require the visible data-sharing confirmation.

Fresh Kestral profiles have no provider profile and no Chat default. Kestral
does not assume Ollama or another local service is running, probe common local
ports, or add profiles without your choice. **No provider selected** clears the
Chat default while keeping saved profiles. When no default is selected, Chat
shows **Configure model provider**; messages receive local setup guidance and
do not start a model-provider worker or network request.

A ChatGPT profile has an explicit **Connected** or **Not connected** account
status. **Connect ChatGPT account** starts host-mediated OpenAI OAuth;
**Reconnect ChatGPT account** replaces an expired or revoked session; and
**Disconnect account** removes the credential from protected host storage.
The UI never receives the access or refresh token. ChatGPT subscription access
requires an eligible Plus or Pro account and consumes provider-managed Codex
quota rather than OpenAI API-key billing.

Use browser login when the Kestral host and browser are on the same desktop.
Use device-code login in split mode, where the provider worker runs on the
backend machine. Built-in OAuth profiles can list their pinned model catalog
before login; the provider still decides which models and quota the connected
account can use.

Use **Discover models** while editing a provider profile to populate the model
picker. When the provider catalog declares thinking variants for the selected
model, Kestral also shows a **Model variant** picker. **Provider default** leaves
the effort unspecified; selecting Minimal, Low, Medium, High, Extra high, or
Maximum saves that effort as the profile default. An app can still request an
explicit reasoning effort for an individual `llm.generate` call, which takes
precedence over the profile default.

When the selected model advertises provider-supported output length control,
Kestral also shows **Text verbosity** with Low, Medium, High, and **Provider
default**. This setting is independent from reasoning effort. Kestral does not
show or send the control for adapters that cannot enforce it. Changing provider
kind clears the old model, reasoning variant, and text-verbosity selection so
discovery cannot present a model from the previous provider as current.

## Chat

Create, rename, and delete threads from the thread list. An untouched new chat
remains a draft and is not saved; choosing **New chat** again reuses that draft.
Enter sends a message; use Shift+Enter for a line break. A running send can be cancelled, although
blocking network work may finish before cancellation takes effect. Late results
from a cancelled Run are not committed.

Chat serializes sends within each thread and records a durable request identity
and lifecycle state before model work begins. If Kestral stops while a request
is pending, the request remains visibly interrupted and is not replayed on
restart; ambiguous work is never reported as completed. Retrying creates a new
request rather than risking duplicate tool side effects.

When configured, the selected provider profile is pinned for the duration of a send. Kestral
rejects provider-profile and credential changes while Chat is using them, so a
single request cannot drift between models or credentials. Secret values are
not stored in Chat history.

The optional external **Model Profiles** app lets you save a provider profile,
model, model variant, temperature, maximum output tokens, system-prompt
composition, and exact tool list as one reusable setup. The stable ID is
generated from the profile name and remains editable before saving. Provider
profiles, discovered models and variants, prompt layers from **Settings →
Chat**, and Chat's currently granted tools appear as choices rather than fields
whose internal IDs must be copied. You can append up to eight profile-specific
prompt text blocks. The immutable Kestral protocol prompt always remains.
Clearing every optional prompt layer and custom text deliberately leaves the
protocol layer as that profile's complete system prompt.

Open the app to create or edit profiles, then choose one from **Model profile**
beside Chat's composer. **Chat default** keeps the default provider, the saved
Chat prompt composition, and all tools currently granted to Chat. A changed or
removed selected profile is not applied silently: Chat falls back to its default
until you review and select the updated profile. A selected prompt layer that is
no longer available makes that model profile unavailable until edited.

Credential-free local provider profiles can be selected directly. A cloud
profile with an API key or OAuth credential must first be chosen as **Default
for Chat** under **Settings → Model providers**. This preserves the
invocation-scoped credential boundary; Model Profiles cannot select another
saved cloud credential behind the default.

A profile's tool list is an allowlist, never a permission request. For each
plain-LLM or delegated-agent turn, Chat intersects that list with its current
grants. A configured tool that Chat does not hold appears as excluded and is not
sent to the model. An empty profile tool list means no tools. Revoking a Chat
grant takes effect on the next turn even when the selected profile still names
that tool.

Per thread, Chat also stores the selected assistant profile and optional agent
engine. The engine selector lists only installed providers that exactly match
the supported `agent.run` contract and have an active grant to Chat; **Plain
LLM** remains an explicit choice even when one engine is available. If a
selected engine later becomes incompatible, unavailable, or ungranted, Chat
uses Plain LLM and shows the fallback reason. If the chosen profile or reviewed
skill content is no longer available on a later send, Chat falls back to
Standard for that send and records a receipt pinned to the reviewed source that
was used at the time.

Open **Settings → Chat** to choose the assistant behavior and save it. Choose
**Kestral default** or **Custom** explicitly; the custom editor appears only for
**Custom**, and its text is sent only after the settings are saved. Less common
conversation details, model context, app guidance, and the exact candidate
prompt stay collapsed until opened. The candidate prompt uses the full settings
width so long instructions wrap instead of being clipped. The Chat model
context inspector shows the current saved prompt layers and runtime identity
defaults plus app context currently stored for the thread. Stored app text is
revalidated against its original Run and grant when you send; appearing in the
inspector does not make stale or revoked text effective. Chat shows the first historical receipt as **System prompt used** and
adds **System prompt changed** only when a later request uses a different exact
prompt. These receipts do not change when settings change. Prompt transparency
is read-only: skills do not grant authority, and changed, missing, or oversized
skills are held for review until explicitly re-enabled.

Under **Conversation details**, **Record app context sent to the model** is off
by default. Enabling it stores the exact host-final app-context message with
each future request and shows it in the collapsed per-send receipt. With it off,
Chat retains only app, Run, grant, revision, and digest metadata historically.
Turning it off stops future exact recording but does not delete records already
stored with the thread.

Assistant replies use Chat's built-in, escape-first Markdown presentation for
headings, emphasis, links, lists, quotes, code, and pipe tables. Raw HTML does
not run, and wide tables scroll inside the reply rather than widening the page.
This presentation belongs only to Chat: external app surfaces keep their own
sandboxed HTML and do not inherit Markdown rendering unless the app implements
it itself.

The **Tools** disclosure shows capabilities active for the current model
profile. Which tools can appear depends on installed apps and active grants, not
merely on what an app advertises. Chat can use those tools with its plain model
path or through an installed Agent Engine. Tool status, MCP result cards, detailed
run metadata, and provider thinking are hidden by default. **Settings → Chat →
Conversation details → Show activity details** enables tool status, run details,
and compact MCP result cards; each result card keeps a short preview and an
**Open in Artifacts** action that focuses the complete durable result. Thinking
can be enabled independently and stays inside a collapsed section below the
reply.

If the optional Agent Engine is absent or the `agent.run` grant is missing,
Chat uses the plain grant-aware model/tool loop instead. It excludes the engine
itself and the LLM Provider from model-visible tools. The UI shows a missing
permission hint instead of silently switching engines.

Successful tool activity is also hidden from the conversation by default, but
Chat stores a compact host-authored capability marker with the turn. Later
model turns receive that marker so they can explain which tool supported an
earlier answer. Raw tool results and secrets are not copied into cross-turn
history. Enable Chat's metadata setting to see the corresponding tool activity
in the conversation.

Installed Chat message extensions can make response text directly selectable
for app-owned marks. Extension UI state does not enter the model request. An app
that offers model integration must separately hold the
`chat.inject_user_context` permission and invoke it for the exact conversation.
Review this permission carefully: authorized text can influence Chat's response
and may lead Chat to use tools Chat already has. A silent all-conversation grant
lets the app update that text without asking again until you revoke it.

Chat stores the app's bounded, revisioned entries separately from visible
history and places currently authorized entries immediately before your next
visible message. Your visible message wins conflicts. Revoking the permission,
letting it expire, uninstalling the app, or replacing its code stops previously
stored text from reaching future model calls. Granting permission again does not
resurrect old entries; the app must publish them again. Updating app context
never calls the model by itself.

An extension may provide an app skill explaining how to interpret its state and
when its capabilities are appropriate. Skills are disabled until you review and
enable them; a skill cannot grant context injection or a tool. Consult the
external app's own documentation for the meaning, persistence, and limitations
of its state.

Extensions can optionally ask Chat to observe **reading opportunity** for a
response. Chat reports only bounded focused-visible time and a 32-band exposure
bitset to the asking extension. It does not report scroll positions, window
sizes, pointer activity, or content outside that response. The observation says
only what viewing made possible: it cannot establish attention, comprehension,
or that any text was read, and it never creates or weakens an explicit mark.

Permissions are grouped in collapsed sections by the app that receives access.
Open an app to see each allowed action, its approval behavior, and expiry. The
action itself may be provided by another app. Revoked grants cannot authorize
Runs. Kestral keeps revoked and obsolete grant facts under **Audit history**.

In Chat, asking which permissions are available uses a separate read-only
requestable-permission catalog. It lists exact ungranted capabilities from all
installed providers; connected MCP tools are ordinary entries in the same list.
Chat can then create a review card for one listed capability. You still choose
**Review and grant** in trusted chrome, and use of the resulting capability asks
for approval by default. Conversational proposals carry no file or other
resource scope. Chat cannot inspect revoked history, change or revoke existing
grants, or issue a grant by itself.

The optional **Agent Engine (pi)** is the external headless app
`com.ma-zierl.kestral-pi`. It contributes `agent.run`, requests no permissions
for itself, and separately asks whether Chat may call it. During an agent run,
every model and tool operation is a child Run checked against Chat's grants;
the engine cannot use its app identity to gain authority. Tool results are
bounded and treated as untrusted model input, and the completed transcript is a
provenance-stamped artifact.

If Agent Engine is absent or Chat lacks its `agent.run` grant, Chat uses its
plain grant-aware model/tool loop instead. Conversation remains available, and
the UI identifies a missing permission when restoring the delegated engine is
possible.

## Artifacts and provenance

Apps can produce durable artifacts such as result cards or snapshots. The
kernel validates their declared schema and stamps the producing app,
capability, Run, grant, and time. Use **Artifacts** to search and filter by type
or producer. Use **System** to inspect the complete Run that produced one.

To let Chat use an artifact, choose **Allow Chat** on its card. Choose **Allow
all artifacts** in the Chat access panel only when Chat should use every current
and future artifact. Kestral asks you to review the list and read permissions
together; each use asks again by default. Approval behavior can be changed later
under **Settings → Permissions**.

The bundled artifact browser exposes exact-scope query pages with provenance
and exact-id reads with bounded content plus provenance. You do not need to find
or enter those internal IDs: the host derives them from the artifacts you
allowed. Granting Artifacts capabilities without allowing selected or all
artifacts gives no artifact access.

Uninstalling an app does not erase existing artifacts or Run history. Keeping
that history preserves attribution.

## Trusted chrome

Approval dialogs and trusted notices are host-owned. Apps cannot draw them.
Before approving, verify:

- the app requesting authority;
- the provider and exact capability;
- any selected file or folder resource;
- whether calls are silent, notified, or require approval;
- the grant duration and stated reason.

Closing or timing out a request denies it. Permissions can be changed later
under **Settings → Permissions**.

Selecting a notified-action notice opens its matching Run under **System →
Activity**. Its cog button opens **Settings → Permissions**, scrolls to the
exact permission used for that action, and highlights it briefly.

## System reset

**System → System reset** returns the currently running profile to fresh-install
state. It permanently removes that profile's conversations, installed apps and
host-managed app data, configuration, protected credentials, grants, artifacts,
Run history, trusted notices, publisher trust decisions, tool and file-resource
registrations, remote-owner credentials, and Kestral audit and update logs. The
desktop host restarts automatically to apply the reset before opening any store.
In backend-only mode, stop and restart the backend after scheduling the reset;
the remote console must pair again.

The action is deliberately difficult to trigger by accident. Open the review,
read the scope, and type the exact profile-specific phrase shown by Kestral
before the destructive action becomes available. The backend independently
checks the same phrase.

The reset preserves the current profile's identity and the profile registry, so
other profiles and profile selection remain intact. It unregisters external
files and folders but does not delete them. It also cannot erase cloud-provider
data, operating-system logs, or files an unsandboxed backend wrote outside the
Kestral profile root. Those boundaries are shown in the reset review rather
than implied to be erased.

## Backup and transfer

Use **Settings → Kestral profiles → Portable workspace** to export the current
profile or validate and import a `.kestral-portable.zip`. Import shows the app,
credential, and external-file recovery work before target selection. Creating a
fresh profile is non-destructive; overwriting the current profile requires the
displayed `RESTORE <slug>` phrase and a restart. See {% link profiles.md %} for
the complete inclusion and exclusion rules.
