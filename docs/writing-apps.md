---
title: App packages
layout: default
parent: Extending Kestral
nav_order: 1
---

{% assign internal_link_prefix = "" %}{% assign jekyll_major = jekyll.version | split: "." | first %}{% if jekyll_major == "3" %}{% assign internal_link_prefix = site.baseurl %}{% endif %}

# Building an app package
{: .no_toc }

1. TOC
{:toc}

An installable app is a directory with a declarative `app.json` and optional
static UI and backend payload. The format is language-neutral. TypeScript with
Svelte and an MCP stdio backend is the reference path, but the host cares about
the package format and adapter protocol, not the implementation language.

An app is the unit of task-specific product behavior in Kestral. Prefer a
focused document, canvas, form, dashboard, or other purpose-built surface when
it serves the work better than conversation. Use capabilities and artifacts for
grant-mediated composition, and extension contributions for contextual UI.
Apps never call each other directly.

The normative machine-readable format is `schemas/app.schema.json` in the
repository. Package format version `1` targets the Kestral 0.1 release series.

## Package layout

```text
dist/
|-- app.json
|-- app.signature.json       # optional detached Ed25519 signature
|-- ui/
|   |-- icon.svg             # optional app icon
|   `-- index.html           # optional self-contained surface
`-- backend/
    `-- server.mjs           # optional backend payload
```

Every file under `ui/` and `backend/` must appear in `integrity.assets` with
its SHA-256 digest. Extra, missing, traversing, symlinked, non-regular,
duplicate-normalized, case-colliding, or checksum-mismatched payload entries are
rejected. Inspection copies declared files into randomized host-owned staging,
binds approval to that staged digest, and runs no package code. Installation
never returns to the mutable source directory.

The package digest is SHA-256 over a deterministic stream containing every
normalized path, byte length, and exact staged file body, sorted by path. It
covers `app.json` and all declared payload. This differs from the kernel
manifest seal, which covers only the translated kernel declaration.

## Minimal manifest

This package contributes metadata only and therefore uses `backend.kind =
"none"`:

```json
{
  "format_version": 1,
  "id": "com.example.guide",
  "version": "0.1.0",
  "display_name": "Guide",
  "description": "Adds instructions to Kestral.",
  "min_host_version": "0.1.0-alpha.1",
  "manifest": {
    "skills": [
      {
        "name": "explain",
        "description": "Explains the example domain.",
        "instructions": "Answer from the installed guide only."
      }
    ]
  },
  "backend": { "kind": "none" },
  "data": { "kind": "none" },
  "integrity": { "algorithm": "sha256", "assets": {} }
}
```

App IDs are stable reverse-DNS identifiers. They must contain a dot and cannot
start with `mcp-`; dotless IDs, the `mcp-*` namespace, and the bundled IDs
`com.ma-zierl.kestral-artifacts`, `com.ma-zierl.host.file-broker`, and
`com.ma-zierl.host.permissions` are reserved. Versions and `min_host_version`
use SemVer. The
integer `format_version` selects the package schema generation, while
`min_host_version` states the oldest host with the required behavior. An unknown
format or a newer host floor is rejected. Unknown fields are rejected.
`app.json` is limited to 1 MiB. An optional `app.signature.json` is limited to
64 KiB; both files must be UTF-8 JSON.

Optional `publisher`, SPDX `license`, and `icon` fields are host metadata, not
authority. Publisher identity becomes meaningful only when a detached signature
verifies against the local trust store.

## App-owned data

Every format-1 package declares whether it owns durable bytes. Host-owned app
configuration, protected credentials, artifacts, and private surface-state
envelopes are outside this declaration:

```json
"data": { "kind": "none" }
```

A backend-free app that needs durable JSON-object records can instead declare
the host-managed data contract:

```json
"data": {
  "kind": "host-managed",
  "contract_version": 1,
  "collections": {
    "read-marks": {
      "schema": {
        "type": "object",
        "additionalProperties": false,
        "required": ["thread-id", "message-id", "marked"],
        "properties": {
          "thread-id": { "type": "string" },
          "message-id": { "type": "string" },
          "marked": { "type": "boolean" }
        }
      },
      "indexes": [{
        "name": "thread-id",
        "field": "thread-id",
        "value_schema": { "type": "string" },
        "unique": false
      }],
      "operations": ["get", "list", "create", "replace", "delete", "transaction"],
      "limits": {
        "records": 10000,
        "record_bytes": 65536,
        "query_results": 100
      }
    }
  },
  "limits": {
    "total_bytes": 67108864,
    "transaction_operations": 32
  },
  "exports": [],
  "proposals": [{
    "capability": "propose-item",
    "artifact_type": "item-proposal",
    "title": "Propose item change",
    "description": "Create a reviewable change for one item.",
    "target": { "kind": "record", "collection": "read-marks" },
    "payload_schema": {
      "type": "object",
      "additionalProperties": false,
      "required": ["marked"],
      "properties": { "marked": { "type": "boolean" } }
    },
    "max_payload_bytes": 4096
  }]
}
```

The proposal capability and artifact type must also be declared in `manifest`.
Those declarations must use the exact host-derived schemas, not author-supplied
variants. For app ID `com.example.readmarks`, the record proposal above derives
these schemas:

```json
"capabilities": [{
  "name": "propose-item",
  "description": "Create a reviewable change for one item.",
  "effect": "local-write",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["targetId", "targetRevision", "payload"],
    "properties": {
      "targetId": {
        "type": "string",
        "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
        "x-kestral-managed-data-scope": {"kind": "record", "collection": "read-marks"}
      },
      "targetRevision": {"type": "integer", "minimum": 1},
      "payload": {
        "type": "object",
        "additionalProperties": false,
        "required": ["marked"],
        "properties": {"marked": {"type": "boolean"}}
      }
    },
    "x-kestral-managed-data-proposal": true
  },
  "output_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["targetAppId", "targetKind", "collection", "resourceId", "targetGeneration", "targetRevision", "payload"],
    "properties": {
      "targetAppId": {"const": "com.example.readmarks"},
      "targetKind": {"const": "record"},
      "collection": {"const": "read-marks"},
      "resourceId": {"type": "string", "minLength": 1, "maxLength": 256},
      "targetGeneration": {"type": "integer", "minimum": 0},
      "targetRevision": {"type": ["integer", "null"], "minimum": 1},
      "payload": {
        "type": "object",
        "additionalProperties": false,
        "required": ["marked"],
        "properties": {"marked": {"type": "boolean"}}
      }
    }
  }
}],
"artifact_types": [{
  "name": "item-proposal",
  "description": "A reviewable item change.",
  "json_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["targetAppId", "targetKind", "collection", "resourceId", "targetGeneration", "targetRevision", "payload"],
    "properties": {
      "targetAppId": {"const": "com.example.readmarks"},
      "targetKind": {"const": "record"},
      "collection": {"const": "read-marks"},
      "resourceId": {"type": "string", "minLength": 1, "maxLength": 256},
      "targetGeneration": {"type": "integer", "minimum": 0},
      "targetRevision": {"type": ["integer", "null"], "minimum": 1},
      "payload": {
        "type": "object",
        "additionalProperties": false,
        "required": ["marked"],
        "properties": {"marked": {"type": "boolean"}}
      }
    }
  }
}]
```

The artifact type's `json_schema` must repeat the complete `output_schema` object
as JSON data with the same exact shape.
Backend-free packages may declare only capabilities bound to `exports` or
`proposals`. Proposal payload schemas must be strict JSON objects with
`additionalProperties: false`; `max_payload_bytes` is between 1 byte and 1 MiB.
Inspection and install/update review show each proposal's title, target kind and
collection, description, and maximum payload size. Proposal capabilities use
`local-write` because they create a durable artifact, even though they never
mutate managed data.

Contract v1 requires `backend.kind = "none"`, 1-64 lowercase-named
collections, strict object schemas with `additionalProperties: false`, and
top-level equality indexes whose `value_schema` exactly matches the indexed
property schema. Set an index's optional `unique` flag to make every non-null
indexed value unique within that collection; create, replace, and transaction
requests that would duplicate it fail atomically. Package limits cannot exceed 100,000 records per collection,
1 MiB per record, 1,000 results per query, 64 operations per transaction, or
64 MiB total. The host enforces both the package limits and these ceilings.
Unknown fields, contract versions, operations, schemas, indexes, and excessive
limits refuse inspection.
Set `min_host_version` to at least `0.1.0-alpha.1` when using this contract.

### Host-managed data contract v2

Contract v2 supports record collections, document collections, or both. Record-only
and document-only v2 packages are valid; a v2 package must declare at least one
record or document collection. A
document has host-generated UUID identity, a CAS revision, RFC 3339 timestamps,
schema-validated JSON metadata, and opaque content addressed by SHA-256. The
content has no app-visible path. For a document-centric app, metadata can carry
any app-defined title, summary, trash state, or other listing fields permitted by
the declared metadata schema.

```json
"data": {
  "kind": "host-managed",
  "contract_version": 2,
  "collections": {
    "records": {
      "schema": {
        "type": "object",
        "additionalProperties": false,
        "properties": { "value": { "type": "string" } }
      },
      "indexes": [],
      "operations": ["get", "list"],
      "limits": { "records": 100, "record_bytes": 4096, "query_results": 100 }
    }
  },
  "documents": {
    "scenes": {
      "metadata_schema": {
        "type": "object",
        "additionalProperties": false,
        "required": ["title"],
        "properties": { "title": { "type": "string" } }
      },
      "operations": ["get", "list", "create", "replace", "update-metadata", "delete"],
      "limits": {
        "documents": 100,
        "metadata_bytes": 4096,
        "content_bytes": 8388608
      }
    }
  },
  "limits": {
    "total_bytes": 67108864,
    "transaction_operations": 32,
    "batch_operations": 2048
  },
  "exports": []
}
```

The host accepts document content in immutable base64 chunks of at most 384
KiB raw (about 512 KiB on the bridge). A collection may hold documents up to
8 MiB each, which covers the demonstrated 7 MiB scene size while the store
remains bounded by the 64 MiB total. Metadata listings never include content.

Every v2 read returns `generation`. Prefer one `read-snapshot` request for
related reads; its `results` array is in request order and every result comes
from exactly one loaded generation. Clients may send `expectedGeneration` on a
snapshot; a changed generation returns a visible conflict instead of mixing
snapshots. Every mutation requires a caller-chosen `mutationId` and the
expected store generation. Kestral durably records request digests and results:
repeating the same ID and request returns the original result, while reusing an
ID for different input is refused. Receipts are bounded; once the receipt quota
is full, Kestral refuses new mutations rather than dropping old receipts.

The canonical multi-read wire shape is:

```json
{
  "contractVersion": 2,
  "request": {
    "kind": "read-snapshot",
    "expectedGeneration": 4,
    "reads": [
      { "kind": "record-list", "collection": "records" },
      { "kind": "document-content", "collection": "scenes", "id": "00000000-0000-4000-8000-000000000001", "offset": 0, "length": 393216 }
    ]
  }
}
```

It returns `{ "generation": 4, "results": [...] }`; each result carries its
read kind and results remain in request order.

Records can be changed directly with the v2 CAS operations. `transaction_operations`
continues to cap ordinary transactions at 64. Contract v2 also declares
`batch_operations`, the cumulative limit for one staged batch; the host bounds it
to 1-2048. Document creates and replacements use a staged batch so large content
is never sent in one bridge message. Metadata-only document updates use the same batch but carry only
the document ID, `expectedRevision`, and schema-validated metadata. They require
no `stageId` or content chunks, preserve the immutable content hash and bytes,
increment the document revision once, and update its timestamp. Begin the batch
with record and document operations, append record operations and required
content chunks in order, then commit or abort. The initial record-operation
chunk and each appended record-operation chunk contain at most 64 operations:

```json
{
  "contractVersion": 2,
  "request": {
    "kind": "begin-batch",
    "mutationId": "batch-01",
    "expectedGeneration": 4,
    "operations": [],
    "documents": [{
      "kind": "create",
      "stageId": "scene",
      "collection": "scenes",
      "metadata": { "title": "Board" },
      "contentLength": 5,
      "contentSha256": "sha256-2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    }]
  }
}
```

The begin response is `{ "batchId": string, "generation": number,
"documents": [{ "stageId": string, "documentId": string }] }`. Append record
chunks with `append-batch-operations` and a new `mutationId`; retrying the same
mutation ID and payload is idempotent, while changing the payload is refused.
Append document content chunks as needed, then send `commit-batch` with its
`mutationId`.
Incomplete batches are invisible, expire, and are cleaned. Commit validates the
complete candidate, writes immutable blobs first, and atomically publishes one
state envelope; a failed commit leaves the previous state authoritative.
Contract v2 can structurally read retained v1 record stores without publisher
code. A v1 package cannot reactivate after v2 state has been published.

An optional `exports` entry binds one manifest capability to a fixed host read:

### Managed-data proposals

Contract v2 proposals are fixed host operations for backend-free packages. A
`collection` target carries `targetGeneration`; a `record` or `document` target
carries `targetId` and `targetRevision`. The host derives the provider app,
target kind, collection, exact resource ID, current generation/revision, and the
artifact envelope. Public input fields are camelCase and contain only the target
version plus the typed `payload`; callers cannot supply an app ID or a broader
scope.

Stable resource IDs are:

```text
app-data:<app-id>:<collection>                         collection (existing ID)
app-data:<app-id>:<collection>:record:<uuid>            record
app-data:<app-id>:<collection>:document:<uuid>          document
```

The proposal handler revalidates the installed package and contract, requires
the target to exist at the supplied generation/revision, checks the exact
derived `DataScope`, validates the payload and byte bound, and returns an
ordinary `ArtifactDraft`. It never calls a managed-data mutation. Kernel
finalization performs the normal artifact-schema validation, provenance stamp,
and atomic Run/artifact ledger commit. A `consumer_grant_requests` entry may
request the capability for Chat or another app and is approved through the
normal install flow. Declare `requires-approval` for that request by default;
there is no proposal-specific grant bypass.

The target surface sees its own proposal artifacts through its ordinary
`listArtifacts` surface helper because the artifact producer is the target app.
Other app surfaces receive no foreign-artifact read exception. Applying or
replaying a proposal is frontend responsibility: the surface must perform the
managed-data CAS mutation itself after comparing the artifact's target version.
Delegated managed-data writes remain unsupported.

```json
"exports": [{
  "capability": "list_read_marks",
  "operation": "list",
  "collection": "read-marks",
  "index": "thread-id",
  "equals_host_input": "current-chat-thread-id"
}]
```

The matching capability for that example is:

```json
{
  "name": "list_read_marks",
  "description": "List read marks for one thread",
  "effect": "read-only",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["equals"],
    "properties": {
      "equals": {
        "type": "string",
        "x-kestral-host-input": "current-chat-thread-id"
      },
      "after": {
        "type": ["string", "null"],
        "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
      },
      "limit": { "type": "integer", "minimum": 1, "maximum": 100 }
    }
  },
  "output_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["records", "next_after"],
    "properties": {
      "records": {
        "type": "array",
        "maxItems": 100,
        "items": {
          "type": "object",
          "additionalProperties": false,
          "required": ["id", "revision", "created_at", "updated_at", "value"],
          "properties": {
            "id": {
              "type": "string",
              "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
            },
            "revision": { "type": "integer", "minimum": 1 },
            "created_at": { "type": "string", "format": "date-time" },
            "updated_at": { "type": "string", "format": "date-time" },
            "value": {
              "type": "object",
              "additionalProperties": false,
              "required": ["thread-id", "message-id", "marked"],
              "properties": {
                "thread-id": { "type": "string" },
                "message-id": { "type": "string" },
                "marked": { "type": "boolean" }
              }
            }
          }
        }
      },
      "next_after": { "type": ["string", "null"] }
    }
  }
}
```

Only `get` and `list` exports exist in contract v1. Every backend-free
capability must have exactly one export and declare `effect: "read-only"`.
The host also requires the capability schemas to equal the fixed operation:
`get` accepts exactly one canonical UUID `id`; indexed `list` requires `equals` and
optionally accepts `after` and `limit`; an unindexed list accepts only `after`
and `limit`. Results are either one record or
`{ "records": [...], "next_after": string | null }`. A record contains exactly
`id`, `revision`, `created_at`, `updated_at`, and the declared `value`. This
exact-schema check prevents a package from describing behavior different from
the host operation it receives.

An indexed `list` export may set `equals_host_input` to
`current-chat-thread-id`. The generated capability then requires the matching
`equals` property with `x-kestral-host-input`; Chat removes it from the model's
tool schema and injects the live thread identity immediately before invocation.
The binding is valid only for a string-valued indexed list. Other callers still
invoke the capability contract normally with an explicit equality value.

Delegated callers invoke that capability through a grant and Run using the
exact collection resource ID `app-data:<provider-app-id>:<collection>`. A grant
may cover that resource or use `all-resources`, but each invocation must request
the exact collection resource. Contract v1 does not generate delegated writes:
the current handler path cannot safely commit a side effect after finalization
revalidates cancellation and authority.

A backend that persists publisher-defined files declares a positive data format
and a dedicated migration command:

```json
"data": {
  "kind": "versioned",
  "format_version": 2,
  "migration": {
    "protocol_version": 1,
    "command": "node",
    "entry": "backend/migrate.mjs",
    "args": ["backend/migrate.mjs"],
    "transitions": [
      { "from": 1, "to": 2, "destructive": false }
    ]
  }
}
```

`entry` must be a safe, integrity-covered package path and the command or one of
its arguments must execute that exact entry. Versioned data currently requires
a local `mcp-stdio` or packaged `executable` backend. The migration process
inherits that backend's authority mode and receives no secret or kernel
capability. Kestral gives it only the verified payload path and a staged
candidate as `APP_HOST_DATA_DIR`.

The command speaks one newline-delimited JSON-RPC method. For the example above,
Kestral sends:

```json
{"jsonrpc":"2.0","id":1,"method":"kestral/app-data/migrate","params":{"protocol_version":1,"from_format_version":1,"to_format_version":2}}
```

Success returns exactly the target version:

```json
{"jsonrpc":"2.0","id":1,"result":{"protocol_version":1,"format_version":2}}
```

The target package owns forward transitions. A newer active package may also
declare and test a reverse transition for a supported code downgrade. Without
an exact edge, Kestral refuses the transition without changing the source data.
`destructive: true` is shown in the explicit update review; it is a disclosure,
not permission to bypass staging or backups.

Kestral stops the active backend, copies the active data revision, verifies that
the source did not change, runs the migration against the copy, and starts the
target backend against that candidate as validation. Only then does it replace
the host-owned active pointer. Failed activation restores the previous pointer
and code. Candidate preparation and pointer commits are idempotent so startup
can resume every journal phase.

The publisher must keep immutable fixtures and migration tests for every public
data format. Migrations must support every public source version they declare,
be idempotent, preserve unknown or corrupt source bytes behind a visible
refusal, and never modify the source directory. The host retains the configured
number of pre-migration revisions, with a minimum and default of one.

### App icons

Omit `icon` to use the uppercase first letter of `display_name`. To ship a
custom icon, set `icon` to a package path:

```json
"icon": "ui/icon.svg"
```

The asset must be listed in `integrity.assets`, cannot exceed 256 KiB, and must
be a valid SVG, PNG, WebP, or JPEG image. SVG icons cannot contain scripts,
foreign objects, external references, or imported styles. Kestral displays the
verified bytes as a passive image resource; it never injects package SVG markup
into trusted chrome. An SVG that uses `currentColor` is treated as a monochrome
mask and inherits the shell icon color. Other image assets retain their authored
colors.

Apps can alternatively select an icon from Kestral's built-in current-color
catalog instead of shipping an asset:

```json
"icon": { "kind": "kestral", "name": "check-square" }
```

Kestral does not currently depend on Bootstrap Icons. The supported catalog is
closed and validated at inspection: `activity`, `app-grid`, `artifact-box`,
`book-open`, `chat-bubble`, `check-square`, `pencil-ruler`, and `settings`.
Unknown names are rejected rather than rendered as an empty icon.

## Declaring product behavior

The `manifest` can declare:

| Field | Purpose |
|---|---|
| `capabilities` | Named actions through which this app, Chat, or another granted caller can do work. Each has an input and optional output JSON Schema plus an advisory effect. |
| `surfaces` | Focused panels, cards, forms, pickers, or dashboards that emit declared intents. |
| `agents`, `skills`, `assistant_profiles`, `automations` | Data contributions interpreted by capable userland apps; the kernel does not become an agent or scheduler. Skills are reviewed, enabled, and digested by Chat but never grant authority. Assistant profiles reference locally declared skills and only suggest capabilities or an engine contract. |
| `connectors` | Names and descriptions of required secrets, never secret values. |
| `artifact_types` | Schemas for durable outputs that the kernel stamps with provenance and other granted apps can compose through artifact capabilities. |
| `artifacts.query` / `artifacts.read` | Read-only artifact access exposed by the bundled artifact browser app. Query returns metadata and provenance only; read returns bounded content for an exact authorized artifact ID. |
| `extension_points` | Versioned contextual presentation seams exposed to other apps. |
| `extension_contributions` | A declared surface contributed to another app's extension point without creating direct app RPC. |
| `config_declarations` | One host-stored config section in the 0.1 series. The generic host editor supports scalar fields; apps with structured values edit them through a standalone dashboard. |
| `grant_requests` | Permissions the app needs; requests confer no authority. |
| `event_subscriptions` | Limited minimized host event topics, not cross-app RPC. |

For a config schema string that needs line breaks, set
`"x-kestral-input": "multiline"` on the property. The host renders that field
as a multiline text area; unannotated strings remain single-line inputs.
Array, object, and union-valued properties are not reduced to raw text inputs.
If an app declares them, provide a standalone dashboard that edits the same
host-stored config through the surface bridge.

Top-level `consumer_grant_requests` asks the host to grant another installed app
access to capabilities provided by this package. This is useful when installing
a provider should make a tool available to Chat or another known consumer. Each request still goes
through trusted chrome and may be denied independently. Denial leaves the
provider active but that integration unavailable. Ordinary denied requests also
do not have to abort installation; the app activates with the approved subset
and must degrade honestly when optional authority is absent.

## Chat integration contracts

The following public contracts integrate focused apps with the default-installed
Chat app. They are examples of app-to-app composition, not privileged host APIs
or the only way to build a Kestral app.

### Current-conversation tool input

A Chat tool that must be scoped to the active conversation can declare a
required top-level string property with the host annotation:

```json
{
  "thread_id": {
    "type": "string",
    "minLength": 1,
    "x-kestral-host-input": "current-chat-thread-id"
  }
}
```

The property must also appear in the capability schema's `required` array.
Chat removes it from the model-visible tool schema and injects the active
thread ID immediately before kernel input validation. A model-supplied value
cannot override it. Delegated agent runs receive the same binding; callers
without a current Chat conversation are not offered a capability that requires
this input. The annotation is context binding, not authority: Chat still needs
an approved grant for the capability.

Extension contributions may be installed before their target app. They remain
dormant while the target is absent or exposes an incompatible contract, and
become mountable automatically when an exact app/point/contract-version match
exists. Package review lists these integrations because contributing a screen
inside another app is user-visible even when it requests no capability grant.
A breaking extension change uses a new contract version; it never redefines an
existing version. Target updates warn when they will make an installed,
currently compatible contribution dormant. The contribution remains installed
with its data intact, and the Apps page shows whether it is compatible, waiting
for its target, missing its extension point, or version-mismatched. There is no
automatic version adaptation and no guarantee that an old extension version
remains operational for the complete `0.1` series.
A contributed `dashboard` remains a standalone app destination as well. Other
contributed surface kinds are treated as inline-only unless the app declares a
separate, non-contributed standalone surface.

Chat's `thread-actions` v1 extension point is for small actions on the visible
conversation. Its context is `{ thread_id, resource_id, revision }`. The host
remounts contributed surfaces when the active resource or observed revision
changes, so the init context cannot silently refer to a previous conversation
snapshot. A thread resource ID identifies the scope to request; it does not
grant access.

### Chat thread reads

Chat provides `chat.list_threads` and `chat.read_thread` as ordinary read-only
capabilities. Each invocation must request exact Chat thread resource IDs. A
grant may name those exact resources or use `{ "kind": "all-resources" }` to
cover every current and future conversation after explicit install approval.
Use the broad form only when the app's core workflow genuinely spans arbitrary
conversations; trusted chrome labels it as broad and leaves it unchecked by
default.
Listing returns metadata only for resources in the invocation's authorized data
scope and never reveals private messages or unauthorized threads. Reading
returns the observed thread revision and a canonical sequence-cursor page of
public user and assistant messages.

The public message view contains visible text, artifact and Run references,
lifecycle status, sequence, and timestamps. It excludes provider reasoning,
system messages, host tool-status records, grant-authorized app context, and retry
state. External apps must use these capabilities rather than the trusted
shell's private Chat commands.

Thread list pagination is cursor-by-thread-resource-id and limit-bounded. Thread
read pagination is cursor-by-public-message-sequence and limit-bounded. Both use
stable ordering and return a `next_cursor` when more authorized items remain.

`chat.propose_draft` is the compose-time contribution capability. The input must
carry the exact `resource_id` of an authorized Chat thread and a non-empty list
of contributions. Supported contribution kinds are `text-snapshot`,
`artifact-ref`, `resource-ref`, and `draft-proposal`. Each contribution has a
unique `(kind, item_id)` identity per source app, a revision, completeness,
title, content body, and lifecycle. The host stores contributions durably on the
thread, caps each call at 32 contributions, caps each thread at 128
contributions, and caps each contribution body at 16 KiB serialized JSON. A
proposal fails if the resource is not authorized, the thread is unknown, the
kind is invalid, the identity is duplicated, or the limits are exceeded.
Accepted contributions persist until explicitly removed or consumed by a
successful request that records a prompt receipt. Removal and consumption are
host-owned lifecycle transitions, not model-visible mutations.

### Artifact reads

The bundled artifact browser app exposes `artifacts.query` and `artifacts.read`
as read-only capabilities. Each invocation must name exact artifact resource
IDs even when a broader grant covers them. Query returns only bounded metadata
plus provenance for authorized artifacts, ordered stably with a cursor. Read requires an exact authorized
artifact ID and returns bounded typed content plus provenance. The host rejects
malformed cursors, oversized content, and unscoped access instead of truncating
or guessing.

For Chat and delegated agent tools, the host derives those invocation scopes.
It expands a live query grant to its authorized current artifact IDs and derives
read scope from the validated `artifact_id` input. A grant with no artifact
resources does not expose either tool. App backends outside this host tool path
must still submit their own exact invocation scope through the ordinary action
path.

`artifacts.query` returns `artifact_id`, `artifact_type`, `title`, and
provenance. `artifacts.read` returns the same plus the artifact content. Query
is capped at 50 items per page with a default of 20 and a cursor string of at
most 64 characters. Read is exact-id only. The content snapshot is bounded by
the kernel's artifact-content cap, and the title is capped at 256 characters.

### Assistant profiles

An app may declare `assistant_profiles`. Each declaration has a unique
`profile_name`, title, description, local `instruction_skill_refs`, suggested
capability references, an optional suggested agent-engine contract, and bounded
starter prompts. Referenced skills must be declared by the same app. The host
derives contributor identity, app version, and content digest from the live
installed manifest. Profile text and suggestions grant no authority.

Chat discovers assistant profiles from installed manifests, lets the user select
one per thread, and stores a receipt containing the app id, profile name,
version, digest, reviewed skill digests, suggested capability refs, optional
engine contract, and availability status. If a selected profile or reviewed
skill content disappears later, Chat falls back to Standard for future sends.
The stored receipt remains pinned to the exact reviewed source that was chosen
for that send.

### Model profile providers

Assistant profiles describe instructions, reviewed skills, and capability
suggestions. They do not choose a provider model or generation parameters. The
An external app opts into that separate workflow by declaring a host-stored
`model-profiles` config section and contributing its editor surface to Chat's
`model-profile-editor` extension point at contract version 1:

```json
{
  "target_app": "chat",
  "extension_point": "model-profile-editor",
  "contract_version": 1,
  "surface": "model-profiles"
}
```

The Kestral Model Profiles reference app uses a static dashboard,
`backend.kind = "none"`, no capabilities, and no grants. Other apps can
implement the same declared contract without adopting its identity.

Each saved entry contains a stable lowercase ID, title, description, configured
LLM connector profile ID, model ID, optional reasoning/temperature/output-token
settings, at most 64 unique `provider/capability` tool references, and an
explicit `prompt` field. `null` uses the complete Chat composition; an override
object selects at most 64 live Chat prompt layer IDs and appends at most eight
bounded custom text blocks. It cannot select or replace the mandatory `protocol`
layer. Chat validates this strict shape independently of the app's JSON Schema
and stores a digest-bound per-thread receipt, including the source app identity,
when the user selects it. Omitting `prompt` is invalid.

The contributed editor receives bounded host context containing configured
connector IDs and model catalogs, Chat's current prompt layers, and Chat's
grant-filtered tool catalog. It receives no connector base URLs, credentials, or
new authority. An ordinary surface without this exact contribution and config
declaration receives no such context.

Tool references are narrowing policy, not authority. Chat computes the
intersection with its live grant-aware capability catalog immediately before
building plain-LLM tools or the delegated `agent.run` request. Apps must not
describe a model profile as granting, approving, or restoring a permission.
Changing the app version or selected profile content makes the saved selection
stale until the person explicitly accepts the updated profile.

Credential-free connector profiles can be selected directly. A profile backed
by an API key or OAuth credential is available only while it is the host's
**Default for Chat**. The LLM Provider exposes only that active profile's
synthetic broker alias to an invocation; the companion app and Chat do not read
another profile's stored credential to make a selection work.

### Chat message text marks

Chat's `message-actions` extension point uses contract version 6. Its context
identifies the thread, exact thread resource, and completed assistant message
and includes Chat's canonical reading parts. Each part has an `index`, a short
`excerpt`, and the complete rendered-readable `plain_text`. Text offsets below
are zero-based, end-exclusive offsets into that `plain_text`. The resource ID is
context for exact scoped invocations; it is not authority by itself.

The context also carries host-stamped `created_at` and `completed_at` times.
`completed_at` is the earliest moment the full response text was available, so
an extension can bound how long a response could have been read without
trusting its own clock and without counting generation time as reading time.
Only completed assistant responses receive the extension, and a message with no
host timestamp receives none rather than a guessed one.

A contributed surface publishes current marks over `extension-state`:

```json
{
  "kind": "message-text-marks",
  "contract": 6,
  "state_revision": 7,
  "groups": [{
    "id": "selection-1",
    "ranges": [{ "part": 0, "start": 4, "end": 18 }]
  }],
  "labels": {
    "mark": "Mark selected text as read",
    "unmark": "Mark selected text as unread"
  }
}
```

`state_revision` is required: it is the app's own revision for this state, and
it lets the host tell a material change from a republish of the same snapshot.
If a payload does not parse under v6, Chat removes that extension's previous
marks instead of leaving stale state on screen.

Each group is one user selection and may contain ranges from multiple message
parts. Ranges inside a group must be continuous in rendered reading order.
Strictly overlapping groups belong to one group; exactly adjacent groups remain
separate because they represent separate user actions. Chat validates the
groups, then highlights them on the canonical message text without splitting a
Markdown list into separately rendered items. Native text selection remains
passive so users can copy normally.
After a pointer or keyboard selection, Chat shows the app-provided mark or
unmark label in a viewport-fixed action tray, so the action remains reachable
on long responses without replacing native copy behavior. Choosing it sends the
contributing surface an `extension-event` with kind `message-text-selection`, contract `6`, a
`marked` target, and one or more `{ part, start, end, text }` ranges. The app
owns durable state and should publish optimistic and confirmed state around a
surface-state write. Provide keyboard-accessible bulk actions in the
contributed surface as an alternative to selecting text.

An app may make marked spans commentable by publishing a bounded `comments`
array alongside `groups`. Each item has `{ id, ranges, text }`; its continuous
ranges may cross message parts, must stay entirely inside marked text, and
cannot overlap another comment. Chat
makes those spans keyboard-focusable and sends add, edit, and delete requests
back as `message-text-comment` extension events. The app still owns persistence
and reports the matching operation status in extension state, allowing Chat to
preserve an unsaved editor after failure. Comments are limited to 500 Unicode
characters. Removing any part of a comment's marked anchor removes that
comment; the unmark action states this consequence before applying it.

#### Reading-opportunity observation

An extension may add `"observe_reading_opportunity": true` to ask Chat to
measure whether reading a response was *possible*. Chat observes nothing until
an extension asks, and an app should only ask while the user has explicitly
enabled it.

Chat runs one observer for the whole conversation log and sends the asking
surface `message-reading-opportunity` extension events:

```json
{
  "kind": "message-reading-opportunity",
  "contract": 6,
  "session_id": "session-1",
  "qualified_visible_ms": 18000,
  "exposed_mask": 4294967040,
  "first_qualified_at": "2026-07-31T10:20:00.000Z",
  "last_qualified_at": "2026-07-31T10:20:18.000Z",
  "final": false
}
```

Two independent signals are reported and must stay independent:

- `qualified_visible_ms` accrues only while Chat is the active destination, the
  document is visible, the window is focused, and this response is the single
  *primary reading region* — the one holding keyboard focus, or else the one
  filling the middle of the viewport. A shared interval is therefore credited to
  one message, so the same 30 seconds never becomes 30 seconds for every visible
  response.
- `exposed_mask` is a 32-band bitset of which vertical parts of the response
  reached the viewport. Geometry is converted at the boundary and discarded.

Values are **cumulative per session**, so a repeated or retried report merges to
the maximum rather than adding. A response that leaves the viewport ends its
session with `final: true`; returning to it later opens a new session, so time
spent elsewhere in between is never counted. Reports are bucketed (roughly every
15 seconds, plus on focus or visibility loss and on session end) rather than
emitted per scroll event.

No scroll offset, viewport size, ratio, focus-event log, or intersection sample
is ever persisted or sent. The aggregates bound what was *possible*: fast
scrolling yields broad exposure with negligible time, and a stationary view
yields time only for the part on screen. They are never a read mark, never
override or weaken an explicit mark, and say nothing about attention or
comprehension. Deriving a bound from them is the app's job; if it puts one in
model context, it publishes bounded integers and a closed exposure word:

```json
{
  "reading_opportunity": {
    "possible_words_upper_bound": 120,
    "total_words": 184,
    "text_exposure": "most"
  }
}
```

`text_exposure` is one of `none`, `some`, `about-half`, `most`, `all`, and the
bound may not exceed `total_words`. This remains extension UI state unless the
app includes a bounded interpretation in text accepted through
`chat.inject_user_context`; raw geometry is never available to inject.

### Chat contributions

Chat persists draft contributions as canonical host-owned records. Each record
is identified by `source_app_id` plus `item_id`, is hydrated back into the
composer when a thread is reopened, and can only be removed through the host's
authoritative removal command. Draft contributions are not exposed to the
composer extension by default. The `composer-context` v1 payload contains only
the current thread ID, a bounded current selection (empty when Chat has no
explicit selection), and a request ID for render-time decisions.

An extension that needs to keep publishing state without displaying its inline
surface may set `"surface_visible": false`. Chat keeps the sandboxed frame
mounted for state and events but hides both its frame and host identity strip.
This is suitable for an app setting that hides a repetitive summary without
disabling the extension's message interactions.

Extension state never becomes model input by itself. An app that needs dynamic,
actionable context must declare Chat's ordinary capability in the contributing
surface and request a grant:

```json
{
  "surfaces": [{
    "name": "message-reading-mark",
    "kind": "card",
    "title": "Reading insights",
    "description": "Annotates assistant text",
    "intents": [
      { "provider": "chat", "capability": "chat.inject_user_context" }
    ],
    "ui": { "entry": "ui/index.html" }
  }],
  "grant_requests": [{
    "scope": {
      "kind": "exact-capability",
      "provider": "chat",
      "capability": "chat.inject_user_context"
    },
    "data_scope": { "kind": "all-resources" },
    "condition": "silent",
    "reason": "Add current annotations as model context in Chat. This may influence Chat to use tools Chat already has.",
    "duration": { "kind": "non-expiring" }
  }]
}
```

`all-resources` is appropriate only when the app genuinely works across every
current and future conversation. Trusted chrome marks it broad and leaves it
unchecked by default. The `silent` condition means that, after issuance, each
context update runs without another prompt. Use `notify` or
`requires-approval` when that repeated interaction is appropriate.

Invoke the capability with the exact `resource_id` supplied by the v6 extension
context and the same exact resource in the invocation data scope:

```json
{
  "resource_id": "chat-thread-…",
  "operations": [{
    "kind": "upsert",
    "item_id": "assistant-message-7",
    "revision": 8,
    "content": "User comment on the marked claim: Review this source."
  }]
}
```

An invocation accepts 1–32 operations. An upsert requires non-empty text of at
most 16,384 Unicode characters. A remove uses `{ "kind": "remove", "item_id":
"…", "revision": 9 }`. Item IDs are scoped by source app and thread. Revisions
must not move backwards. Chat caps stored entries and aggregate context at the
provider boundary; exceeding a bound fails the update instead of truncating or
silently omitting authority-bearing text.

The source app ID, version, content hash, source Run, and content digest are
host-derived. Before each send, Chat requires that source Run to have completed
`chat.inject_user_context` under its original still-active grant and exact
thread scope. Revocation, expiry, uninstall, or replacement makes the entry
inert. A later grant does not resurrect it. The app must publish again.

Authorized entries form one attributed late user message immediately before
the visible user message. Their text is supplemental user-level input and may
contain requests; the visible message wins conflicts. Injected text cannot
override the host protocol, grant tools, make a tool available, or prove a side
effect. It can influence Chat to exercise tools Chat already holds, which is why
the permission reason must explain that consequence.

Chat's host-authored system prompt is a bounded composition: visible
immutable protocol layer, assistant instructions, enabled manifest skills, and
optional runtime context. The prompt receipt records the exact composed prompt,
its digest, and the active layer set for the sent turn. Capability tools are
independent and are available only when the host explicitly supplies them.

The app-visible Chat settings surface includes assistant-instruction editing and
reset. Conversation details, skill review and enablement, runtime context, and
the exact authoritative prompt preview use collapsed disclosures so the primary
choice remains clear. Skill instructions are capped at 8192 Unicode characters
in `schemas/app.schema.json`. Changed, missing, or oversized skills require
review and do not contribute until explicitly re-enabled.

Runtime identity defaults on in Chat's prompt metadata: host version,
delegated-agent or plain-LLM mode, model, and connector kind. App inventory and
connector/profile identifiers default off. Secrets, base URLs, filesystem paths,
tool outputs, history, and grant-authorized app context remain separate from the
system prompt. The Model context inspector shows currently stored entries.
When **Record app context sent to the model** is enabled, each future composition
receipt retains the exact host-final context message. When disabled, the receipt
keeps source app, Run, original grant, revision, and digests but not an exact
historical text copy.

Every grant request must include `scope`, `data_scope`, `condition`, `reason`,
and `duration`. Use `{ "kind": "none" }` for a grant that is not tied to a
registered data resource. Use `{ "kind": "resources", "resource_ids": [...] }`
for fixed resources. `{ "kind": "all-resources" }` grants access to all current
and future resources under the requested capability; it is valid only for a
grant, never as an invocation data scope. State that breadth in `reason`.
Prefer exact capabilities and `requires-approval` for external writes or
destructive effects.

Grant interaction conditions are delegation policy, not extra confirmation for
the provider app's own UI. A declared capability invoked by a person from that
provider's live surface remains grant-checked and audited, but the host does not
emit a `notify` notice or request per-use approval. The same capability keeps
its configured condition when another app, an LLM or agent flow, automation, or
other programmatic initiator invokes it. Apps should still provide an
in-surface confirmation for an irreversible direct action when recovery is not
available.

## Backend kinds

| Kind | Behavior |
|---|---|
| `none` | No process. The app cannot declare capabilities of its own. |
| `mcp-stdio` | Starts a command already available on the host and speaks MCP over stdio. |
| `mcp-streamable-http` | Connects to an MCP Streamable HTTP endpoint. |
| `executable` | Selects a checksummed packaged executable for the host platform; it speaks MCP over stdio. Supported keys are `windows-x86_64`, `windows-aarch64`, `macos-x86_64`, `macos-aarch64`, `linux-x86_64`, and `linux-aarch64`. |
| `agent-worker` | Runs the version 1 callback protocol used by a headless agent engine. It must declare exactly `agent.run` and an `agent-transcript` artifact type. |

See [Agent workers]({{ internal_link_prefix }}{% link agent-workers.md %}) for the complete version 1
message contract, limits, callback mediation, progress, and cancellation rules.

Native backend kinds declare `authority_mode`. In the 0.1 series,
OS sandboxing is not proven and production packages using unsandboxed native
backends require the host's explicit unsafe-backend opt-in. Do not describe a
backend as sandboxed unless the target host reports that support.

At activation, MCP-backed packages must advertise tools matching their static
capability declarations. A mismatch blocks activation.

The general executable contract is deliberately MCP stdio rather than a second
backend RPC protocol. The language-neutral host therefore only selects and runs
a verified file, then uses the existing MCP adapter. `agent-worker` is a narrow
exception because an agent must request model and tool callbacks from the host;
those callbacks are mediated as child Runs and never expose a kernel handle or
credential to the worker.

## Host translation

The kernel never parses a package. The host validates `app.json`, injects its
identity fields into the exhaustive manifest, strips host-only surface `ui`
bindings, checks cross-reference consistency, and seals the resulting generic
manifest. Backend adapters independently create ordinary capability handlers,
and the surface registry binds static UI to the kernel's intent-only surface
declarations. No package, path, process, or MCP type enters the kernel.

This translation is intentionally mechanical but not translation-free. A raw
serialized kernel manifest would couple the public format to internal serde
layout and could not cleanly carry host-only package, backend, publisher, and UI
metadata.

## Custom surfaces

A surface can bind a package-local HTML entry:

```json
{
  "name": "guide-panel",
  "kind": "panel",
  "title": "Guide",
  "description": "Example app surface.",
  "intents": [
    { "provider": "com.example.guide", "capability": "search" }
  ],
  "ui": { "entry": "ui/index.html" }
}
```

The host loads custom UI in an iframe with
`allow-scripts allow-forms allow-downloads`, an opaque origin, and a
deny-by-default app CSP. `allow-forms` lets JavaScript receive and cancel normal
`submit` events; the host-enforced `form-action 'none'` directive still blocks
form navigation, so configuration writes must use the surface bridge.
`allow-downloads` permits a surface to offer a browser-managed file download,
but does not let it choose a path, inspect an existing file, or read the
filesystem. The frame has no Tauri, kernel, direct filesystem, credential, or
trusted-chrome access. It communicates through the versioned surface bridge and
can request only declared intents, bounded state in its own app-and-surface
namespace, or declared host-managed domain data. Keep the entry self-contained
and do not depend on host component
internals. The HTML entry must be UTF-8 and cannot exceed 32 MiB.

From package-bundle lookup through surface opening, host-context loading, iframe
load, and the `ready` handshake, Kestral keeps its shared animated loading mark
visible and the partial frame concealed. The frame must call
`window.appHost.ready()` only after it can accept the init contract. A missing
handshake becomes a visible bounded failure with retry rather than an empty
workspace.

The init payload also includes `window.appHost.hostContext`, a bounded,
host-authored object. It is empty for ordinary surfaces. Kestral may use it for
a documented host-integrated companion workflow, such as giving Model Profiles
sanitized provider/model choices, Chat prompt layers, and Chat's grant-aware
tool catalog. A frame cannot request another app's context or use this
presentation data as authority.

Surface capability calls use `window.appHost.invoke(capability, input, goal)`
when no data scope is required. For an intent that requires exact resources,
use `window.appHost.invokeScoped(capability, input, dataScope, goal)`, where a
resource scope has the shape
`{ kind: "resources", resource_ids: ["resource-id"] }`. Both methods are
limited to the surface's declared intents and run through the complete kernel
grant and action path. Supplying a resource ID never creates or widens a grant.

### Colors and Appearance

The bridge injects Kestral's resolved workspace and status palette into the frame
as CSS variables such as `--color-text`, `--color-surface-raised`,
`--color-border`, `--color-accent`, `--color-warning-text`, and
`--color-focus-ring`. These values follow System, Light, Dark, and custom color
profiles. App CSS should consume them directly and should not duplicate host
Light/Dark literals:

```css
body {
  color: var(--color-text);
  background: var(--color-surface);
}

button:focus-visible {
  outline: 0.2rem solid var(--color-focus-ring);
}
```

Only declare a new color when its app-domain meaning is not represented by a
host semantic token. Top-level `theme_colors` provides a Light and Dark default:

```json
"theme_colors": [{
  "name": "storm-track",
  "title": "Storm track",
  "description": "Forecast path on the map.",
  "light": "#315ea8",
  "dark": "#8db1ff"
}]
```

The frame receives it as `var(--app-color-storm-track)`. Names are app-local;
the host binds them to the installed app identity, refuses duplicate or invalid
declarations, and never lets them replace `--color-*` or trusted chrome. Each
declared color appears under the app's name while a person edits a custom color
profile in **Settings → Appearance**. Built-in and System themes use the
package defaults. Imported/exported profiles preserve namespaced overrides,
including overrides for an app that is temporarily uninstalled.

Theme variables are presentation inputs, not authority and not durable app
content. Do not recolor stored user data when the host theme changes; a drawing
stroke, document highlight, or user-selected brand color belongs to that data's
own schema.

Trusted-chrome `--color-chrome-*` tokens are deliberately not sent to app
frames. Approvals, identity, secret entry, and permission warnings remain
host-owned surfaces rather than an app styling API.

### Host-managed domain data

An open owning surface for a `host-managed` package receives the closed
`window.appHost.data.v1` API:

```text
get(collection, id)
list(collection, { index?, equals?, after?, limit? })
create(collection, value)
replace(collection, id, expectedRevision, value)
delete(collection, id, expectedRevision)
transaction([{ kind, collection, ... }])
```

The host derives app identity from the live `SurfaceBinding`; none of these
methods accepts an app ID, profile path, storage path, SQL, JSONPath, script, or
generic host command. Canonical UUIDs and RFC 3339 timestamps are host-generated. Creates
start at revision 1. Replace and delete require exact positive CAS revisions.
Lists are ordered by record ID and use the returned `next_after` as the next
request's `after`. Equality queries must supply both a declared `index` and an
`equals` value, or neither. Transactions contain only create, replace, and
delete operations and commit through one atomic document replacement; any
invalid operation rejects the entire transaction.

Owning-surface calls are private storage operations, not delegated capability
actions, and therefore create no grant, prompt, or Run. Disable and ordinary
uninstall retain the store. **Purge app data** removes it. On activation the
host validates every retained record against the current declaration; an
incompatible package or downgrade refuses without rewriting or deleting the
store. A compatible contract change writes one preserved pre-change snapshot
before its first mutation. There is not yet an export/restore UI or general
change-notification subscription.

### Private surface state

`window.appHost.getState(key)` returns `{ revision, value }` for the open
surface's own namespace. `window.appHost.putState(key, expectedRevision, value)`
atomically replaces that entry and rejects a stale revision; pass `null` to
leave a revisioned tombstone. Keys are 1-200 ASCII identifier characters,
values are JSON objects up to 1 MiB, and each app store is bounded to 2,000
entries and 64 MiB. The host validates the live surface binding, writes with
atomic replacement under the app's host-state directory, and removes the
store when the user uninstalls with **Purge app data**.

The native-backend file is `surface-state-v2.json`. Its fixed top-level prefix
contains `version: 2` followed by a host-written integer `generation`; each
successful state write increments that generation in the same atomic file
replacement. A persistent backend may cache parsed state by generation, but it
must read the generation and any replacement content through the same opened
file snapshot. Do not pair independent path reads or filesystem timestamps:
either can observe different replacements and return stale state.

This state path is for private presentation state caused directly by the user,
not cross-app authority. It does not create a Run or exercise a grant because
it is not a capability invocation. Calls to another app, model-visible tools,
external effects, and other declared capabilities still use surface intents
and the complete grant-checked action path. An unsandboxed native backend can
still locate host-owned files through its operating-system authority, so
surface state is not a secret boundary from that backend. It is never copied
into an app-data migration candidate.

### Network access from a surface

The policy is always written by the host. A package cannot supply a raw
`ui.csp` — doing so refuses the install — and widens network access only
through `ui.connect_src`:

```json
"ui": {
  "entry": "ui/index.html",
  "connect_src": ["https://api.example.com"]
}
```

Each entry must be a bare source expression such as `https://api.example.com`,
`https://api.example.com:8443`, `wss://stream.example.com`, or `https:`. An
entry containing whitespace, quotes, `;`, or `,` is refused at install with a
located error, because those characters would end the `connect-src` directive
and let a package append directives of its own. Omit `connect_src` (or leave it
empty) and the surface gets no network access at all.

## Build and test

An app repository owns its compiler/runtime dependencies, lockfiles, tests,
reproducible package build, notices, and release artifacts. It may consume a
versioned Kestral crate, schema, or SDK through a normal package-manager or Git
dependency. Do not resolve tools or source files from a Kestral checkout: the
host and app repositories must be buildable and testable independently.

The build output is an installable directory containing `app.json` and every
integrity-listed asset. Install it through **Apps → Install an app**. Kestral
does not install or start source-built examples in a normal profile.

Before distributing a package:

1. Validate `app.json` against `schemas/app.schema.json`.
2. Rebuild twice and confirm deterministic payload checksums.
3. Install from `dist`, deny at least one permission, and verify graceful
   degradation.
4. For stateful apps, freeze byte fixtures and test successful, failed, repeated,
   and interrupted migrations from every public format.
5. Test disable/enable, update, supported downgrade or refusal, restart, and
   uninstall with both keep-data and purge-data choices.
6. Verify the custom surface at 320 CSS pixels and keyboard-only operation.
7. Inspect Runs and artifacts for correct attribution.
8. Run these checks in the app repository's own CI from a clean immutable
   commit. If the app is promoted for a Kestral release, publish a format-1
   evidence document matching
   `schemas/external-app-release-evidence.schema.json`. Pin the exact source
   commit, Kestral package digest, host commit/version, workflow, platforms, and
   concrete observation for every lifecycle check.

Kestral's release record stores only the external repository, commit, package
digest, evidence URL, and evidence SHA-256. Core CI validates those records and
exact extension versions without resolving the app repository or package. A
normal app test may consume a versioned Kestral schema or SDK through a pinned
Git or package-manager dependency; it must not depend on a developer's local
Kestral checkout.

## Current parity limit

Packaged MCP backends do not yet receive broker-mediated own secrets, initiate
child Runs, consume minimized host events, or propose multiple independently
typed artifacts through the child protocol. Build those behaviors through
host-mediated capabilities rather than bypassing the boundary.

Kestral 0.1 accepts package directories and public HTTPS Git repositories.
The `.ahpkg` archive form is a reserved transport design, not an implemented
install source. Runtime restart policies and full native process sandboxing are
also outside the current package guarantee.
