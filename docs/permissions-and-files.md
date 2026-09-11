---
title: Permissions and files
layout: default
parent: Using Kestral
nav_order: 4
---

# Permissions, artifacts, and file resources
{: .no_toc }

1. TOC
{:toc}

## How grants work

A manifest permission request describes a need; it does not grant authority.
The host issues a grant only after a trusted-chrome decision. A grant records:

- the holder app;
- one exact provider capability or all capabilities from one provider;
- no registered resource scope, an exact set of resource IDs, or explicit
  access to all current and future resources governed by the capability;
- an interaction policy: silent, notify, or requires approval;
- a non-expiring or time-limited duration; and
- an audit reason.

Artifact browsing keeps each invocation exact. The bundled artifact browser app
reads only artifact resource IDs authorized for that invocation; it does not
infer neighboring artifacts. Its
`artifacts.query` capability lists only exact-scope metadata plus provenance,
and its `artifacts.read` capability returns exact-id content plus provenance.

Use **Settings → Permissions** to add, change, or revoke grants. Broader
provider-wide scope and silent execution carry more authority than an exact,
approval-required grant. Prefer the narrowest scope that completes the task.
An app manifest may request `all-resources`; this is persistent access to
resources created later as well as resources that exist now. Trusted chrome
marks it as broad and leaves it unchecked by default during install review.

Host-managed collection capabilities use the stable exact resource ID
`app-data:<provider-app-id>:<collection>`. A generated `get` or `list`
capability accepts only that fixed collection and checks that the invocation
scope contains exactly its resource ID. The owning live surface does not need a
grant for direct `window.appHost.data.v1` or `data.v2` access; Chat, agents,
automations, and other apps cannot use that private bridge and must invoke the
exported capability through an approved grant and Run.

Contract-v2 managed-data proposals use additional exact IDs:
`app-data:<provider-app-id>:<collection>:record:<uuid>` and
`app-data:<provider-app-id>:<collection>:document:<uuid>`. The host-generated
proposal schema lets Chat and agents derive these IDs from `targetId`; a broad
all-resources grant never turns into a broad invocation scope. Collection
proposals use the current store generation, while record and document proposals
use the current CAS revision. A successful call creates a provenance-stamped,
reviewable artifact and does not change managed data. The owning surface must
compare and replay that artifact through its own CAS UI; there is no delegated
write path or foreign-artifact read exception.

The bundled **Permissions** app provides three conversational capabilities.
`permissions.list_active` returns the calling app's standing active grants at
snapshot time, including data scopes and interaction conditions. A listed grant
is not necessarily a tool supplied to the model for that turn: a model-profile
allowlist or host contextual rule can narrow the tool set further. The read does
not expose secrets, grant IDs, inactive or revoked history, raw audit records,
requestable permissions, or another app's permissions.

`permissions.list_requestable` returns the bounded, exact catalog of
capabilities declared by installed providers that the calling app does not
currently hold. Each entry includes the provider identity, capability, bounded
capability description, and declared effect so Chat can explain and suggest
available access before asking for it. This is the general permission catalog:
installed app capabilities and connected MCP tools appear under the same model.
An empty list means that no installed capability is currently requestable; it
does not mean the read tool failed. The returned provider metadata is untrusted
descriptive data and confers no authority.

`permissions.propose_grant` can produce a reviewable proposal for the calling
app to use one exact capability from that catalog. When at least one ungranted
capability is eligible, the host supplies this proposal tool with the same exact
choices in its input schema and injects the catalog again when the tool runs.
When none is eligible, the host does not supply the proposal tool even though
Chat's standing grant to the Permissions app remains active. None of these
capabilities can issue, edit, or revoke a grant. The host accepts only a proposal
artifact stamped by the proposal capability, revalidates its fixed shape and
originating app, confirms that the provider and capability remain installed,
and sends the exact request through trusted chrome when the user chooses
**Review and grant**.

Conversational proposals always start as exact, no-resource, non-expiring,
approval-required grants. Resource-bound permissions, provider-wide access,
revocation, and policy or duration changes remain deliberate Settings tasks.
The user may later change the interaction policy in **Settings → Permissions**.
For MCP tools, choosing `notify` or `silent` requires an explicit warning
acknowledgement because future Chat and LLM-driven calls can then proceed
without asking first. Kestral also reports when a broader or less interactive
existing grant already controls the effective policy.

Revocation blocks future calls immediately. In-flight work is revalidated
before finalization, so a late result cannot commit after its authority is
removed.

Chat's `chat.inject_user_context` capability is intentionally powerful. It lets
the holder add bounded text that the model may follow in authorized
conversations. That text can influence Chat to use tools Chat already holds,
although it cannot create a tool or grant. Apps that need this across arbitrary
conversations request `all-resources`; trusted chrome shows the request as broad
and leaves it unchecked by default. Each update is a Run. Chat also revalidates
the update's original grant before every send, so revocation disables stored
text and a replacement grant does not revive it.

Grant interaction policies govern delegated access. Clicking an action in the
provider app's own surface is already your explicit instruction, so Kestral
does not show a `notify` notice for that same-app action. If the policy requires
approval, the click itself is enough only for an action declared `read-only` or
`local-write`. An action declared `unspecified`, `external-write`, or
`destructive` still opens trusted chrome for confirmation. Every action still
follows the normal validated Run path and is auditable. Calls from another app,
Chat or an LLM workflow, an agent, automation, or other programmatic code
continue to use the grant's configured silent, notify, or approval-required
policy.

{: .warning }
This shortcut is not proof that a person clicked. Effect labels come from the
installed provider, and a custom surface can submit its declared intent through
the bridge without a verifiable browser user gesture. Treat custom apps as code
you trust; the alpha does not make this exception a malicious-app sandbox.

## Let Chat use artifacts

Choosing an Artifacts capability and choosing which artifacts it may use are
separate decisions. **All Artifacts capabilities** means query and read; it does
not by itself authorize any artifact content. Kestral labels that incomplete
state **No artifact access** rather than treating it as broad data access.

Use the Artifacts page instead of entering resource IDs:

1. Open **Artifacts**.
2. Choose **Allow Chat** on one artifact, or **Allow all artifacts** for every
   current and future artifact.
3. Review the grouped `artifacts.query` and `artifacts.read` permissions in
   trusted chrome.
4. Ask Chat to find or read the artifact. New artifact permissions ask before
   each use by default; change that behavior under **Settings → Permissions**
   only when you want less interaction.

The same Chat permissions apply when Chat delegates to the optional Agent
Engine. The engine receives no artifact grant of its own. Before each query,
Kestral resolves the currently authorized artifacts to exact resource IDs. A
broad standing permission can therefore cover future artifacts without putting
an `all-resources` wildcard in the invocation or ledger.

Under **Settings → Permissions**, an Artifacts permission can keep its existing
selected artifacts or explicitly change to **All current and future artifacts**.
The **Add permission** form requires that broad artifact choice; use the
Artifacts page for a narrower selection.

## Share a file or folder

Do not grant an app an arbitrary path. Instead:

1. Open **Settings → File resources**.
2. Choose **Add file** or **Add folder** and select the resource in the trusted
   file picker.
3. Select an installed app.
4. Grant only the operation it needs: list, read, create or replace, or delete.
5. Review the resulting entry under **Settings → Permissions**.

The File Broker keeps the canonical path in host-owned settings. Apps receive a
bounded resource view and must call the broker with the resource ID. Different
files and folders can be granted to different apps independently.
One list call accepts at most 1,024 directory entries. Reads and create-or-replace
writes accept at most 1 MiB per call; register a narrower folder or split larger
work instead of sending an unbounded broker request.

Grant access to **Chat** when you want a conversation to use that file or
folder. Chat and the optional Agent Engine pass the selected resource ID through
the normal kernel action path; the Agent Engine does not receive its own file
grant. File operations run with the backend operating-system account and do not
require administrator rights merely because Kestral performs them. A location
that account cannot read or modify stays inaccessible and reports an
operating-system permission error.

Removing a registered resource revokes access tied to it. Historical permission
entries for a removed resource cannot be granted again; register the file or
folder again to create a new resource. Removing access does not delete the
underlying file or folder.

{: .warning }
File-resource grants constrain calls through the File Broker. They cannot
constrain an unsandboxed native backend that reads the filesystem directly.

Audio capture is unrelated to the File Broker. Kestral does not bundle Media or
microphone capture; a future external Media app needs its own explicit device
and data-access design.
