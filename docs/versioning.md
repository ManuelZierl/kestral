---
title: Versioning and recovery
layout: default
parent: Operations
nav_order: 2
---

{% assign internal_link_prefix = "" %}{% assign jekyll_major = jekyll.version | split: "." | first %}{% if jekyll_major == "3" %}{% assign internal_link_prefix = site.baseurl %}{% endif %}

# Versioning and recovery
{: .no_toc }

1. TOC
{:toc}

Version `0.1.0-alpha.1` is the current pre-publication testing baseline. All
earlier development state is disposable test data and has no compatibility
path. Current documents must contain the complete current shape, including
explicit `null` values for nullable persisted fields. Unknown versions, missing
or unknown fields, invalid checksums, and inconsistent identities fail visibly;
Kestral does not guess or silently downgrade development data. The one explicit
pre-publication path migration renames the old single-document
`update-journal.jsonl` to its alpha.1 name, `update-journal.json`, through the
same journaled coordinator used for future migrations.

The host now has the required forward-migration foundation and immutable alpha.1
fixtures. From every public release onward, each later supported Kestral release
must migrate valid released data forward or refuse visibly while preserving the
original. A later release still needs an explicit tested step from each earlier
public format; the coordinator never treats an unknown version as a compatible
empty store. Downgrade compatibility is not promised.

Host-managed data v2 is a host-owned structural migration, not a publisher
migration. When a v2 package opens a valid v1 managed-data store, Kestral reads
the existing record envelope and publishes a v2 envelope only when a v2
mutation commits. Records retain their IDs, revisions, and values. Document
blobs, staged batches, and mutation receipts are new v2-owned state; malformed
or unknown v2 state refuses startup. A published v2 state is not silently read
by a v1 package.

V2 proposal bindings are part of the managed-data contract digest. Changing a
proposal capability, artifact type, target binding, payload schema, or payload
bound therefore participates in the normal managed-data contract compatibility
check and update review. A proposal input carries an exact target generation or
CAS revision; the handler refuses stale or missing targets and creates no data
mutation. The generated artifact envelope is host-derived and its artifact type
schema must match exactly. Existing collection resource IDs remain stable;
record and document targets use the `:record:<uuid>` and `:document:<uuid>`
suffixes. Applying a proposal is a frontend CAS operation, not a migration or a
delegated write.

Persisted-data continuity is distinct from app and API compatibility. Versioned
package, extension, API, surface, and worker contracts may evolve during alpha.
Unknown or incompatible contract versions fail visibly rather than being guessed
or silently reinterpreted. Such evolution does not waive the data promise:
released host-owned state and opaque app-data bytes remain preserved or migrate
forward, while each app publisher owns migration of the records inside its
opaque data.

## App package compatibility

App package format 1 is the public package contract for the complete `0.1`
series. Every later `0.1` host must continue to read every valid format-1
package accepted by an earlier public `0.1` host. Kestral keeps a frozen
format-1 package in the alpha.1 persistence corpus and reads it in the host test
suite as a release gate.

Format 1 may gain an optional field only when omission has an explicit default
and preserves the meaning of every existing package. This is backward
readability: an older host is not required to understand a newer package that
uses the addition. A new required field, changed meaning, removed case, or
other incompatible package change requires a new integer `format_version`;
Kestral does not redefine format 1 or silently translate an unknown format.

## Extension compatibility

Extension contract versions are exact major versions. A contribution mounts
only when its target app is active and declares the same extension-point name
and contract version. Compatible additions preserve the version; a breaking
context, behavior, or contribution change requires a new contract version.
Kestral never guesses compatibility or adapts one version to another.

During alpha, a target may stop declaring an older extension version; Kestral
does not promise that every extension version remains operational throughout
`0.1`. Before an installed target update makes a currently compatible
contribution incompatible, update review names the affected contribution and
the version mismatch. The contribution remains installed with its data intact
and appears as dormant rather than disappearing silently. It becomes mountable
again if an exact target/point/version match is installed later. Publishers
must announce deprecated or removed contract versions in their release notes;
there is no time-based deprecation window in `0.1`.

## Release evidence contracts

`release/promoted-apps.json` format 1 is the core-owned roster for external app
compatibility claims. `schemas/external-app-release-evidence.schema.json` format
1 is the app-owned result contract. Both are release metadata, not user data and
not an app package format.

The roster pins the exact Kestral version and tested core commit, external
repository and commit, package version and host-canonical package digest,
extension contributions, and the URL plus SHA-256 of an immutable evidence
document. The evidence binds those facts to one clean external commit, one exact
clean tested core commit, a retained workflow, tested platforms, and all
required lifecycle observations. Unknown
fields, missing checks, non-exact host or extension versions, dirty-source
evidence, unreachable commits, changed evidence bytes, and any result other than
`passed` fail release validation.

Core CI does not import the external source or package to validate this record.
The app repository owns its build and tests; the evidence producer owns the
claim that the recorded package bytes completed the lifecycle run. Kestral's
release workflow verifies the immutable pins and contract consistency rather
than silently substituting another build.

The tested core commit precedes one metadata-only promotion commit because a
commit cannot contain a hash of evidence that names that same not-yet-created
commit. Release validation proves ancestry and permits only
`release/promoted-apps.json` and
`release/v<version>-evidence.md` to differ between those commits. The tested
core commit is an executable/build source freeze. Any code,
dependency, configuration, schema, workflow, or documentation change requires a
new candidate and new external evidence.

## Persisted stores

| Store | alpha.1 format |
|---|---|
| Host config storage envelope | v3 (config document v2) |
| Secret reference index | v2 |
| Durable kernel state | v1 |
| Installed-app registry | v4 |
| App update journal | v2 |
| App-data active revision and backup index | v1 |
| Profile migration journal | v1 |
| Chat threads | v4 |
| Trusted notices | v1 |
| Publisher trust store | v1 |
| File resources | v1 |
| Profile registry, transition, and profile identity | v1 |
| Remote owner passkeys | v1 |
| Pending system reset | v1 |
| Portable workspace archive and import journal | v1 |
| Portable post-import recovery index | v1 |
| MCP gateway audit event | v1 JSONL record |
| Browser custom color profiles | v2 |
| Browser sidebar layout | v2 |
| Browser pending-send recovery | v1 |
| Private surface state | v2 |
| Host-managed app data | v1 records; v2 records, receipts, staged batches, and content-addressed document blobs |
| App package | format 1 |
| Surface bridge | protocol 3 |
| Provider worker | protocol 2 |
| Agent worker | protocol 1 |

## Migration ownership

One owner parses and evolves each durable representation:

| Durable state | Migration owner | Location |
|---|---|---|
| Profile registry, transition, and profile identity | `profiles` | Default root and profile root |
| Host config | `config` | Profile root |
| Secret-reference index and derived vault account identities | `config` | Profile root and OS credential vault |
| Checksummed kernel projection | `kernel_state` with the kernel durable types | Profile root |
| Chat threads and pending backend send markers | `chat_store` | Profile root |
| Installed-app registry, revision records, payloads, and update journal | `app_manager` | Profile root |
| App-data active revision and backup index | `app_data` | Profile root |
| Opaque records inside an app-data revision | Its app publisher; host copies bytes only | Profile root |
| Host-managed app records and documents | `managed_data` validates the envelope, receipts, blobs, and package schema; the app owns domain meaning | Profile root |
| Private surface-state envelope | `surface_state`; record values remain app-owned | Profile root |
| Trusted notices | `chrome::notices` | Profile root |
| Publisher trust | `publisher_trust` | Profile root |
| File-resource registrations | `file_resources` | Profile root |
| Remote-owner passkeys | `remote_auth` | Profile root |
| Pending system reset | `system_reset` | Profile root |
| Portable archive, staged import, and recovery index | `portable` | User-selected archive path, default root, and profile root |
| MCP gateway audit | `mcp_gateway` | Profile root |
| Custom themes and preference | Frontend Appearance store | Device-local browser storage |
| Sidebar order, visibility, and collapsed state | Frontend Sidebar store | Device-local browser storage |
| Pending-send retry IDs and active-thread selection | Frontend Chat store | Device-local browser storage |

Vault secret values are not copied into a profile candidate or backup. The
coordinator validates the strict owner/name reference index and preserves its
derived profile namespace; the OS credential backend remains the value owner.
Missing values stay missing and are never synthesized. Browser stores are
outside the profile filesystem transaction. Their frontend owners parse exact
versions; pending-send v1 is `{ version: 1, sends: {...} }` and malformed data
is refused rather than converted to an empty retry map.

## Ephemeral state

Runtime authority is not a migration input. Restart recreates invocation and
app workers; disconnects MCP sessions until an explicit reconnect; denies or
abandons pending approvals, pairing/OAuth/WebAuthn ceremonies, and live owner
sessions; rebuilds declared surfaces; discards leases and event inboxes; and
interrupts transient progress and live streams. Kernel recovery closes active
durable Runs once as `interrupted`, but does not restore the process, handler,
approval, lease, stream, or session that had been executing them.

App package format 1 defines an optional `icon` as either an
integrity-covered package image path or a validated name from Kestral's built-in
icon catalog. Packages that omit it use the display name's first letter. It also
defines optional bounded `theme_colors` metadata. These host-only,
app-namespaced presentation defaults do not enter the kernel.

App package format 1 and durable kernel state v1 recognize the grant data scope
`{ "kind": "all-resources" }`. A host that does not know an enum variant must
reject the document rather than narrow or discard authority.

Trusted-notice v1 `grant-use` records require the selected grant ID so the shell
can open the exact permission. Every record also requires an explicit nullable
`acknowledged_at` field. Missing fields invalidate the store.

The host-config storage envelope is v3 and contains a complete config document
whose internal domain version is v2. Its app, connector, MCP, transition,
gateway, and cloud-egress collections are required. Provider profiles require
`default_variant` and `default_text_verbosity`; `null` means to use the
provider's default for that independent control. Streamable HTTP MCP entries
require an explicit authentication shape: `none` or static-header metadata.
Secret header values remain only in the OS credential vault and are not part of
the document. `host.default_llm_profile` is also required and nullable: `null`
means Chat has no selected model provider. A fresh config uses `null` and an
empty connector map; a string must resolve to a configured profile.

Chat thread v4 is the only supported chat-store format. Every thread, message,
profile receipt, and composition receipt must contain the complete v4 field set;
nullable values are represented explicitly as `null`. Untouched empty threads
remain runtime drafts and are omitted from the document; draft contributions
give an otherwise message-free thread meaningful persisted content. Contributions
are keyed by `source_app_id` plus
`item_id`, deduplicated on that pair, and removed only through host-owned
commands so persistence stays authoritative. A composition receipt records the
system-prompt digest, profile, reviewed skill digests, context and attachment
references, available capability refs, provider profile, selected agent engine,
agent-engine features, and creation time. Chat also retains its exact prompt and
layers for the trusted prompt-transparency UI. Thread v4 also stores bounded,
revisioned app-context entries with their source app version/content hash, source
Run, content digest, exact text, and timestamps. These records are not authority:
Chat revalidates the original Run and grant before each use.
Bounded removal tombstones retain the latest removed revision so an out-of-order
older upsert cannot recreate deleted context. Loading v4 rechecks context
identity uniqueness, timestamps, content digests, and every size/count bound;
malformed state fails fast rather than reaching prompt composition.

Composition receipt v3 requires `injected_context`. It is `null` when no
grant-authorized app context was sent. Otherwise it records the effective
message digest and each entry's installed app identity, revision, source Run,
original grant, and content digest. `exact_message` contains the host-final
message only when exact app-context recording was enabled for that send; it is
explicitly `null` otherwise.

The required nullable `model_profile_ref`, `model_profile_receipt`, and
composition-receipt `model_profile` fields are `null` when Chat uses its default.
A selected receipt pins the external source app/version, profile digest,
connector and model, generation parameters, and configured tool references. A
model profile must also contain `prompt`: `null` preserves the complete Chat
composition, while an object selects Chat layer IDs and custom text. The
composition receipt separately records the exact effective prompt and capability
intersection for the sent turn.

Artifact snapshot reads are not persisted as a separate store in 0.1. Their
runtime limits are fixed in code: exact artifact resource IDs, bounded cursor
pages, and bounded serialized content. Oversized snapshots fail closed rather
than truncating.

Selected chat profiles are host-owned state: the frontend can request a change,
but the host validates the live installed manifest and reviewed skill digests,
stores the exact profile receipt on the thread, and falls back to Standard for
future sends if the selected profile or reviewed skill content is no longer
available. Historical receipts remain pinned to the reviewed source that was
accepted at send time.

Private worker protocols are not persisted and ship with their corresponding
host/app versions. Provider and agent protocol changes require coordinated
host/worker releases. A mismatch is a packaging error and fails the invocation.

Custom color profile v2 is device-local browser storage with a strict
`{ version: 2, profiles: [...] }` document. Every profile has a unique ID and
name, an immutable Light or Dark base mode, one valid value for every
`ThemeColors` token, and an `appColors` map keyed by app ID and app-local token
name. Any other version, missing or extra host tokens, invalid colors, and
duplicate identities are reported in Appearance instead of being applied.
The separate theme preference defaults to System and falls back to System when
its selected custom profile no longer exists.

Sidebar layout v2 is device-local browser storage with the strict shape
`{ version: 2, collapsed, order, hidden }`. Order and visibility refer only to
stable `host:<screen>` and `app:<app-id>` destination IDs; labels and icons are
always resolved from the current host and app manifests. Unknown saved app IDs
do not create navigation entries, and newly available destinations append after
the saved order. Invalid versions, fields, IDs, and duplicate entries are
reported in the sidebar editor instead of being applied. Version 1 migrates
forward by adding the potential generated Kestral documentation MCP destination
to `hidden`; all existing collapse, order, and other visibility choices remain
unchanged. Version 2 then preserves an explicit choice to show that destination
after its server is connected.

Pending-send recovery v1 is also device-local. It stores only a thread-keyed
idempotency request ID and exact message so a manual retry can reuse the same
backend request identity. It is not a Run, grant, owner session, or authority to
replay work automatically.

Each MCP gateway audit line is a strict v1 envelope with `format_version`, an
RFC 3339 `at` timestamp, and an `event` object whose required `event` name and
event-specific fields describe the security-relevant action. Unknown envelope
fields, malformed lines, timestamps, and missing event names fail profile
validation.

Appearance import/export uses a separate portable format-1 JSON document. It
contains `format`, `version`, `name`, `base_theme`, `colors`, and `app_colors`,
but no local profile ID. Import validates the complete shape and creates a fresh
ID. Surface bridge protocol 3 carries the resolved Light/Dark mode, bounded host
and app CSS variables, and bounded host-authored `hostContext` on init. Only the
exact current protocol version is accepted.

## Crash behavior

Host-owned JSON stores use atomic write, flush, and rename. Durable kernel state
is one `format_version: 1` document containing `state_sha256` and the complete
state projection. Unknown fields, unsupported versions, checksum errors,
duplicate identities, non-contiguous ledger sequences, invalid Runs, and
artifact/provenance disagreement abort startup. Kestral never replaces corrupt
state with an empty profile.

On Unix, the shared host JSON persistence boundary creates files with mode
`0600` and repairs existing files to that mode before reading them. This keeps
configuration, secret-reference metadata, authentication records, lifecycle
journals, and audit data private to the account running Kestral. Native app
backends run as that same account in the 0.1 series, so file modes do not create
an isolation boundary between Kestral and enabled native backend code.

Profile registry v1 stores `selected_next_launch_profile_id` separately from
the running process's immutable runtime profile identity. Profile creation and
deletion use a v1 transition record so startup can remove an uncommitted
created root or finish cleanup after a committed registry deletion.

Each kernel mutation writes and syncs complete candidate service views before
replacing memory. Invocation completion commits produced artifacts and their
ledger events together. Failure before file replacement leaves the old file and
memory unchanged. A post-replacement failure is indeterminate: the live kernel
refuses further durable transitions and requires restart rather than retrying
from stale memory. Restart loads whichever checksummed transition is durable.

Ledger invocation and completion events store payload SHA-256 digests, not full
input/result bodies. Every invocation, result, refusal, cancellation, and
approval event includes its requested data-scope field. Missing digest or scope
fields are invalid.

Startup first acquires the global profile-registry lock, resolves the selected
identity, and acquires the selected profile lock. The profile migration
coordinator then runs before reset recovery and before config, Chat, app,
notice, trust, file-resource, remote-auth, surface-state, audit, or kernel stores
open operationally. The same profile lock is retained by the kernel-state store
for the host lifetime.

A migration records `planned`, `candidate-staged`, `candidate-validated`,
`backup-retained`, `commit-started`, and `committed` in a strict v1 journal. It
copies the complete profile candidate, including opaque app bytes, rejects
symlinks and unknown/corrupt versions, validates cross-store profile and payload
identity, checks package and kernel digests, and proves candidate grants are no
broader than the source. It retains the byte-identical original under
`.kestral-profile-backups/<transaction-id>/`. A restart after any persisted
phase either continues from verified bytes or refuses without replacing the
original; it never opens default-empty operational stores as recovery.

The host holds an exclusive profile lock for its lifetime. A kernel commit writes
a unique temporary file, syncs it, atomically replaces the primary, and syncs
the final file; Unix also syncs the parent directory. An interrupted app update
is separately recorded in `update-journal.json` and completes or rolls back on
startup.
Journal recovery begins during host bootstrap only after trusted-chrome event
delivery is ready. Recovery preserves the recorded prepared, deactivated,
activated, rolling-back, rolled-back, and committed phases, while backend
startup and approval waits occur outside durable kernel and app-manager locks.
Every resumed commit revalidates the retained revision identity, lifecycle
generation, and package digest.

Remote owner authentication v1 stores the immutable WebAuthn RP ID, exact
origin, owner UUID, and registered public credentials in
`remote-owner-auth-v1.json`. Authentication sessions and WebAuthn ceremony
state are not persisted; restart revokes sessions and abandons incomplete
ceremonies. `remote-owner-pairing-v1.json` is a transient, single-use digest with
a ten-minute expiry, not a durable credential store.

A system reset is a restart-transactional profile transition. The host first
writes `kestral-system-reset.json` v1 with the active profile identity. Early on
the next startup, after resolving profile selection but before opening that
profile's operational stores, it validates the identity, clears every credential
named by the protected secret index, and removes current-profile state. It
removes the reset request only after deletion completes. A crash, locked
credential service, malformed secret index, or file deletion failure therefore
leaves the request for a visible retry instead of starting from a partially
reset profile. The profile identity, profile registry, other managed profile
roots, and any in-progress profile-registry transition are preserved.

Portable workspace format v1 is a ZIP whose first member is
`kestral-portable.json`. The manifest records the source profile and host
version, capture time, and the relative path, byte size, and SHA-256 digest of
every content member. Import rejects unknown manifest fields or versions,
duplicate, unmanifested, or missing entries, unsafe paths, size or digest
mismatches, and malformed current store documents before registration or
overwrite.

The archive carries profile-owned durable stores and `apps/.data`, but not
package payloads, OS-vault values, remote-owner authentication, external file
targets, lock files, temporary or transition trees, or live runtime state.
Secret owner/name references become a re-entry checklist. File resources become
an unmatched re-registration checklist. Third-party app records become
digest-bound dormant recovery records; their imported kernel app registrations
are removed and every grant involving those app IDs is added to the revoked set
without deleting the issued grant fact. This is monotonic narrowing and forces
normal package verification and permission review on reinstall. The imported
default model profile is cleared so an excluded OAuth credential cannot make the
new profile fail startup; connector definitions stay present for
re-authentication.

Fresh import creates a new managed identity under `profiles/<profile-id>/`,
validates its current stores, commits it through the profile creation journal,
and selects it for the next launch. Current-profile overwrite writes a v1 import
journal and validated candidate, then runs before profile migration and before
operational stores open. It retains the original under
`.kestral-profile-backups/<transaction-id>/`; persisted phases make backup,
commit, validation, and cleanup repeatable after interruption.

A Run without `RunEnded` is durably closed once as `interrupted` before the host
exposes recovered state. Pending invocations, leases, surfaces, event inboxes,
and handler process state are not restored. Bundled and managed handlers rebind
only when their recovered manifest bytes and content hash match.

Migration authority is monotonic. A source exact capability cannot become
provider-wide; fixed resources cannot become `all-resources`; `notify` or
`requires-approval` cannot weaken toward `silent`; expiry cannot move later;
issued grant facts cannot disappear; and revoked grant IDs cannot disappear.
Owner sessions are not represented in the candidate at all.

## Protected secrets

The v2 secret store is a status-only owner/name reference index. Values live in
Windows Credential Manager, macOS Keychain, or Linux Secret Service through one
host storage boundary. Secret values are never persisted in profile JSON. A
headless Linux host does not fall back to plaintext when Secret Service is
unavailable or locked; it reports an error.

## App code and data

Managed app updates retain verified code revisions and can revert to a prior
content-addressed payload without downloading it again. App configuration,
secrets, and data are keyed by app ID and survive the host-orchestrated
uninstall/install code transition. Enable, update, downgrade, and revert always
run the complete permission review again. Kestral persists no approved subset,
so a previously denied or revoked request is never silently restored.

Format-1 packages must declare `data.kind` as `none`, `versioned`, or
`host-managed`. Versioned
packages declare a positive publisher-owned format, an integrity-covered
JSON-RPC migration command, and exact supported `{from,to,destructive}` edges.
The host never parses publisher records. It stores the active pointer in
`app-data-state-v1.json` and app bytes in host-indexed
`app-data-revisions/<revision-id>/` directories, separate from the host-owned
surface-state envelope.

An incompatible update stops the old backend, stages a byte copy, checks that
the source stayed unchanged, runs the target package's declared migration, and
starts the target backend against the candidate. The active pointer changes
only after validation. Activation failure restores the source pointer and old
code. The update journal records candidate validation, data commit, and data
rollback phases so every boundary is idempotently recoverable after restart.

The global **Settings → Kestral profiles → App-data backups** count controls
retention, with a minimum and default of one prior revision per app. Old backups
are pruned only after the replacement migration and code transition commit.
There is no general restore UI yet. A lower code version is refused after an
incompatible data migration unless the active newer package declares the exact
reverse edge and tests it; code-only reversion with an unchanged data format
remains available.

Host-managed contract v1 is a separate backend-free representation. The host
interprets bounded JSON-object records in `managed-data-v1.json`, generates
record IDs, timestamps, revisions, and the store generation, and validates
records against the active package declaration on every open. Atomic mutations
replace the complete document. Equality queries are limited to declared
top-level indexes; indexes may require unique non-null values. Transactions are
limited to the fixed create/replace/delete union. Generated indexed reads may
bind their equality value from trusted current-Chat context rather than model
input. The package declaration supplies lower quotas under host ceilings.

The persisted document binds a digest of the contract version, collections,
schemas, indexes, limits, and exports. A package update or downgrade may use a
changed declaration only when all retained records and quotas remain valid.
Before the first mutation under the changed declaration, Kestral atomically
preserves the prior document as `managed-data-contract-backup-v1.json`. An
incompatible declaration refuses activation without changing either file.
Kestral does not infer transformations, run package migration code, silently
drop records, or convert opaque `versioned` data or private surface state into
host-managed data. There is no general restore UI yet.

Bundled apps have a separate declaration-only upgrade path. An installed Chat,
LLM Provider, Artifacts, Permissions, or File Broker declaration may update its
version, display name, or top-level description in place. Capability and data
schemas, effects, surfaces, grants, connectors, config, artifacts, extensions,
subscriptions, and every other behavioral declaration must remain identical.
The atomic transition preserves grants, revocations, Runs, ledger records,
artifacts, and app/config data; an incompatible declaration is refused without
replacing the installed manifest.

The active revision and two newest retained revisions are preserved; older
inactive payloads are pruned. Reversion uses the exact previously verified
bytes and warns when selecting a lower SemVer version.

Uninstall can purge app configuration, credentials, and app data by explicit
choice. Runs and artifacts remain to preserve provenance.

Sandboxed custom-surface state uses `surface-state-v2.json` under each managed
app's data directory. The strict document begins with format `version: 2` and a
host-written `generation`, followed by surface-name and app-chosen key maps with
monotonically increasing revisions and JSON-object values or tombstones. Every
successful write increments the generation in the same atomic document
replacement. This lets a native backend reuse a parsed snapshot only while the
generation from the same opened file snapshot is unchanged. Unknown document
versions or shapes fail visibly; no best-effort downgrade or truncation is
performed.

The record *inside* that envelope is opaque to Kestral and versioned by its
owning app. Its format rules belong to that app's repository and release
documentation. The host preserves valid JSON values and CAS revisions without
interpreting app-owned records.

Chat's `message-actions` extension contract is version 6, documented in
[Writing apps]({{ internal_link_prefix }}{% link writing-apps.md %}). No other contract version is
accepted.

The composition-receipt v3 shape is an embedded substructure of the Chat v4
store, not an independently versioned persisted envelope.

Chat's `model-profile-editor` extension contract is version 1. A provider must
declare both the matching contribution and a `model-profiles` config section;
the host does not recognize any external app identity as a special case.
Per-thread receipts include the source app ID. Selection commands use the full
`app-id/profile-id` reference so profiles from multiple contributing apps cannot
collide. Before the app's first schema-validated config write, the absent config
entry represents an empty profile list; saved profile objects and receipts still
require their complete current field set.

System reset is the explicit exception: it removes the current profile's Runs
and artifacts together with all other current-profile state. This destroys
local provenance by design and is guarded by a profile-specific typed phrase.

## Recovery guidance

1. Back up the complete data root while Kestral is closed before recovery tests.
2. Do not edit profile JSON by hand to bypass a validation error.
3. Preserve the reported file before attempting recovery.
4. Restore a full profile backup made while Kestral was closed, or start a clean
   profile and reinstall apps.
5. Restore required credentials separately because directory backups do not
   copy values from the operating system's credential store.

## Schema evolution rules

Before the first publication, persisted and wire formats evolve by replacing
the development shape. Kestral carries one parser per current format or protocol
version and no development-data migrations, aliases, or tolerant readers. Use
this no-compatibility window to simplify formats and remove stale concepts, not
to preserve speculative development shapes.

1. Any shape change updates the format table and strict boundary tests in the
   same change.
2. Host-owned stores continue to use atomic write, flush, and replacement.
3. Unknown fields, missing required fields, and non-current versions fail.
4. Private worker and surface protocol changes require coordinated rebuilds of
   the host and apps.
5. The first public baseline freezes immutable fixtures for every durable store.
6. Later supported Kestral releases migrate valid released data forward before
   operational stores open, or refuse without modifying the source data.
7. Migrations are idempotent, crash-recoverable, and validated across the whole
   profile. They never widen grants, revive revoked authority, extend expiry, or
   persist session-only authority.
8. App-owned data remains opaque to the host. The host stages, indexes, and
   backs up bytes; each publisher owns format versions, fixtures, forward
   migration, destructive-transition disclosure, and any tested reverse edge.
9. Package format 1 follows the `0.1` backward-readability promise above.
   Versioned API, extension, surface, and worker contracts may evolve during
   alpha, but incompatible contracts fail visibly and do not permit released
   data to be discarded or silently reinterpreted.

Update the format table on this page in the same change as any persisted or
wire-format version. Private worker protocols require coordinated host/worker
releases even though they are not persisted.
