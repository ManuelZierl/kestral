---
title: Product completion gates
layout: default
parent: Contributing
nav_order: 5
---

# Product completion gates

The next product milestone is not another kernel subsystem. It is proving that an ordinary Kestral app can be obtained, used repeatedly, recovered, and independently authored without privileged host behavior.

## Implemented foundation

### Navigation does not discard app drafts

The Apps workspace remains mounted while the owner visits Chat, Settings, Artifacts, or System. Inactive app UI is removed from layout and the accessibility tree, but the sandboxed surface is not destroyed merely because the owner navigated elsewhere. This protects unsaved in-frame work such as a Daily Review note draft. It does not silently make drafts durable: app-owned persistence still uses the ordinary host data/state contracts.

### Creator tooling is independently packageable

`packages/create-kestral-app` is a dependency-free npm package containing the focused-app template it needs at runtime. It no longer depends on a Kestral source checkout to scaffold an app. Its tests pack and install the actual tarball outside the repository, invoke its public npm binary, rebuild the generated project, and execute that project's tests. Registry publication remains a release step.

The repository helper under `scripts/create-app.mjs` delegates to that same package and template for contributor convenience; the package is the distributable authoring path.

## Remaining product gates

1. **Daily Review everyday completeness.** Previous-day navigation, ordinary edit/delete interactions, explicit proposal outcomes, and a readable domain export still belong in the app rather than the host.
2. **Selective recovery.** The owner must be able to inspect and restore one app-data backup without rewinding unrelated apps, restoring revoked grants, or making stale proposals valid again.
3. **Obtainable app distribution.** At least one independently released app package must be installable without a development checkout. Curation should point to a real installable artifact before the host restores a prominent curated-app entry point.
4. **Packaged-host qualification.** A clean supported machine must exercise obtain → install → deny optional permissions → useful result → restart → resume → update/recovery → uninstall/retain. Source-level and DOM-adapter tests remain necessary but are not a substitute for this journey.
5. **Measured leanness.** Record cold/warm startup, idle CPU/RAM, app-open latency, first-useful-result latency, and durable-write latency on fresh and aged profiles before setting regression budgets.
6. **Authorization cleanup.** Replace the custom-surface direct-provider approval shortcut with one authoritative kernel policy before removing the conservative host gesture guard. Do not let iframe messages or frontend grant snapshots become authorization evidence.

## Qualification rule

A product gate is complete only when the path works through public app/host contracts, has a regression test or reproducible exercise, and preserves the five-primitive architecture. Daily Review is evidence for the platform; it must not become a privileged host concept.
