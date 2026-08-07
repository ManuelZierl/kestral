---
title: Roadmap
layout: default
nav_order: 7
---

# Roadmap
{: .no_toc }

1. TOC
{:toc}

This page records direction beyond the current Kestral release candidate. It is
a planning document, not a release promise. Scope, sequencing, and version
targets may change as personal use, external app work, and security review
produce evidence.

For implemented behavior, use the [0.1 documentation]({% link index.md %}). For
known release limits, use [Alpha limitations]({% link honest-gaps.md %}). Public
Chat and app contracts implemented for the current 0.1 candidate are documented under
[Building an app package]({% link writing-apps.md %}), not repeated here as
future work.

{: .warning }
Nothing described as **Exploring** or **Deferred** is available in the 0.1
series. Do not build an app that depends on it until the corresponding contract
appears in the versioned app documentation.

## Product order

Roadmap priorities remain ordered:

1. Make Kestral useful and dependable as a personal AI workspace.
2. Make the same model practical for independent app developers and similar
   technical users.
3. Improve broader general-user access without turning the host into an
   opinionated all-in-one suite.

Kestral itself is not being built as a commercial product. Revenue, growth,
marketplace control, and enterprise breadth are not roadmap goals. The MIT
license still permits commercial use and forks.

Before adding shared host behavior, establish that it solves a demonstrated
need across apps or cannot function correctly as an ordinary app. Chat, model
providers, agent engines, notes, retrieval, and automation remain userland
behavior. The fact that Chat is installed by default does not make it the
canonical interface or product ontology.

## First public baseline

The immediate priority is a dependable `v0.1.0-alpha.1`, not more platform
breadth. Before publication:

- the planned release status, persisted formats, package contracts, worker
  protocols, and promoted external apps must agree exactly;
- from every public release onward, each later supported Kestral release must
  migrate valid released data forward, or refuse without modifying the original;
- migrations must be idempotent, crash-recoverable, whole-profile validated,
  and unable to widen or revive authority;
- clean-machine installation, restart, credential, app lifecycle, and recovery
  evidence must exist for each published artifact; and
- the project must record a reproducible baseline for startup, installed size,
  idle resources, worker cost, and time to first useful result.

Development data created before that public baseline remains disposable. See
[Versioning and recovery]({% link versioning.md %}) for the exact current and
post-publication rules.

## Personal workspace exploration

### Focused app journey

**Exploring:** make the path from default Chat to a focused app understandable
without architecture knowledge. A user should be able to discover, inspect,
install, open, configure, and remove an app while understanding its native
backend authority and mediated permissions.

Reference workflows should test that documents, canvases, forms, dashboards,
and other task-specific surfaces outperform a Chat-only interaction where their
structure fits the work. Chat remains available for open-ended requests and
cross-app coordination rather than absorbing every workflow into its transcript.

### Measured leanness

**Exploring:** turn "lean" from an architectural intention into a release
quality. Establish repeatable measurements and conservative regression ceilings
for startup, footprint, idle resources, worker processes, app opening, and first
use. Compare complete workflows rather than treating Tauri or kernel size as a
proxy for product cost.

### Data continuity and exit

**Exploring:** make backup history, manual restore, storage pressure, and
eventual exit understandable from the UI. The foundation now preserves and
migrates host-owned state, stages opaque app-data migrations, retains a
configurable minimum-one backup, and refuses undeclared reverse transitions.
App publishers remain responsible for the internal evolution and immutable
fixtures of their data.

## Developer platform exploration

### App authoring path

**Exploring:** reduce the repeated package, schema, protocol, integrity, and
test work exposed by the reference apps. Prefer small versioned SDKs,
validators, scaffolding, conformance fixtures, and reproducible package tools
over new host-specific abstractions.

The practical test is that independent developers can build and update useful
apps from public documentation without host source changes or private coupling.

### Portable app standards

**Exploring:** track current MCP revisions, resources, prompts, and MCP Apps UI.
Adapt portable standards into generic Kestral surfaces and capabilities where
the trust model can be preserved. MCP remains an adapter protocol, not the
kernel ontology. Kestral-specific contracts must provide value that the shared
standard cannot express.

### Live app surfaces

**Exploring:** add host-to-frame change notifications where an open custom
surface currently needs polling or a manual refresh. Event delivery must retain
the kernel's minimized, bounded-loss feed and must not become direct cross-app
RPC. Request-scoped progress remains separate from general out-of-band events.

### Managed app data

**Foundation shipped:** backend-free apps can declare host-managed data contract
v1 and use bounded owning-surface CRUD, equality queries, CAS, and atomic fixed
transactions. Equality indexes may enforce uniqueness. Fixed delegated
`get`/`list` capabilities use exact collection resources and ordinary Runs, and
an indexed read can bind its equality value from trusted current-Chat context.
Next work is owner-facing export/restore, bounded
change notification, and a deferred-commit action-path contract before any
delegated mutation can be safe.

### Artifact composition

**Exploring:** validate artifacts as a real medium of cross-app composition, not
only an audit result. A third-party producer and consumer must exchange a
bounded artifact under exact grants while preserving provenance and ledger
attribution.

### Native backend isolation

**Exploring:** replace the current host-wide unsafe-native opt-in with stronger
per-package trust decisions and, where practical, enforceable filesystem,
network, and process isolation. Until then, native backends remain fully trusted
code for the backend operating-system account, outside Kestral's direct-action
controls.

## General-user direction

**Exploring:** provide a guided starter workspace and curated discovery without
turning curation into a controlled marketplace. General-user readiness requires
simple model setup, prominent native-authority explanations, recoverable
permissions, low-friction focused apps, accessible surfaces, and dependable
updates. It is not established by hiding technical failures or preinstalling
more product behavior in the host.

## Success evidence

The direction is working only when evidence shows:

1. Owners voluntarily use Kestral for recurring work, not only architecture
   experiments or first messages.
2. Focused apps improve representative structured or visual tasks compared with
   a Chat-only workflow.
3. Users can predict native authority, grant scope, direct-action behavior, and
   revocation consequences.
4. Independent developers ship and update apps through public contracts without
   host changes.
5. Published user data migrates forward without loss or authority widening.
6. Complete-workspace resource and maintenance costs remain within declared
   project budgets.
7. The work adds neither a kernel primitive nor service without passing the
   strict kernel-membership test.

## Deferred direction

The following remain outside current exploration unless a new decision changes
scope:

- multi-user or tenant hosting;
- a commercial marketplace, growth system, or enterprise control plane;
- a privileged host workflow engine;
- broad grant vocabulary without a concrete app need;
- a general unbounded event or streaming channel;
- replacing focused app interfaces with a universal Chat interaction; and
- features whose ongoing release and support cost exceeds the available
  maintainer capacity.
