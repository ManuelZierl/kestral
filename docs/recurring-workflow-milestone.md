---
title: Recurring workflow proof
layout: default
parent: Development
---

# Recurring workflow proof

This milestone tests two claims together: Kestral can become useful for a recurring job, and that job can remain an ordinary independently authored app rather than host product logic.

The reference workflow is **Daily Review** under `reference-apps/daily-review`. It deliberately uses only the public package/surface/data/model path. Task hierarchy, daily-note behavior, archive semantics, and proposal presentation stay app-owned. No Daily Review concept belongs in the kernel.

## Acceptance gates

1. **Truthful entry and readiness — implemented in this branch.** Fresh-profile guidance now depends on whether the workspace has an active independently installed app with a custom screen, not on an empty registry. Bundled startup apps therefore no longer suppress first-app guidance. The Apps page also no longer presents an empty curated catalog as a primary action.
2. **Independent everyday workflow — implemented and under CI qualification.** Daily Review installs as an ordinary backend-free package and remains useful with model access absent or denied. It has its own dependency-free Node 22 source/build/test path; the tracked installable package must reproduce from that source.
3. **Controlled assistance and recovery — partially implemented.** Optional model assistance is visibly staged before application and failure does not mutate workflow data. Still required: an app-to-app proposal handoff with source revision checks, denial/cancellation safety, selective per-app restore, and a useful domain export.
4. **Authoring parity — partially implemented.** Daily Review can be rebuilt from its own directory without host or kernel source and targets the versioned public package contract. Still required before calling the broader authoring story complete: independently distributed authoring/validation tooling rather than requiring a Kestral source checkout for scaffolding or authoritative inspection.
5. **Qualification — automated portion added.** A PR-specific Linux workflow uses Node 22 for the Daily Review package tests and frontend checks/tests, then runs Rust formatting and the workspace test matrix. Still required: authoritative package inspection plus a real clean-profile packaged-host install/restart exercise.

## Product boundary

The host supplies identity, grants, runs, artifacts, trusted chrome, sandboxed surfaces, and host-managed data. Daily Review owns the meaning of a task, parent/child relationships, daily notes, archive behavior, and review UX. This preserves the five-primitive architecture and third-party parity.

## Validation exercise

Use Daily Review for ten working days. Record: days opened, tasks captured, unfinished tasks resumed, notes reopened, AI reviews requested/accepted/rejected, permission prompts encountered, recovery events, and any host-specific friction. A successful milestone requires recurring value without making Chat or Daily Review privileged.
