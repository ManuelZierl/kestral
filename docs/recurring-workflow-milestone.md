---
title: Recurring workflow proof
layout: default
parent: Development
---

# Recurring workflow proof

This milestone tests two claims together: Kestral can become useful for a recurring job, and that job can remain an ordinary independently authored app rather than host product logic.

The reference workflow is **Daily Review** under `reference-apps/daily-review`. It deliberately uses only the public package/surface/data/model path. Task hierarchy, daily-note behavior, archive semantics, and proposal presentation stay app-owned. No Daily Review concept belongs in the kernel.

## Acceptance gates

1. **Truthful entry and readiness.** A fresh profile is guided by the presence of a usable focused app, not by a literally empty registry. Discovery must not imply that curated apps exist when none qualify.
2. **Independent everyday workflow.** Daily Review installs as an ordinary backend-free package and remains useful with model access absent or denied.
3. **Controlled assistance and recovery.** Optional assistance is visibly staged before application. The next slice must prove an app-to-app proposal handoff with source revision checks, denial/cancellation safety, selective per-app restore, and a useful domain export.
4. **Authoring parity.** The workflow must be reproducible outside the host checkout using a versioned package contract and authoritative host inspection. The repository scaffold is evidence, not the final distribution mechanism.
5. **Qualification.** Run frontend checks/tests on Node 22, Rust workspace tests, package inspection, and a clean-profile packaged-host install/restart exercise. Record failures rather than weakening the gates.

## Product boundary

The host supplies identity, grants, runs, artifacts, trusted chrome, sandboxed surfaces, and host-managed data. Daily Review owns the meaning of a task, parent/child relationships, daily notes, archive behavior, and review UX. This preserves the five-primitive architecture and third-party parity.

## Validation exercise

Use Daily Review for ten working days. Record: days opened, tasks captured, unfinished tasks resumed, notes reopened, AI reviews requested/accepted/rejected, permission prompts encountered, recovery events, and any host-specific friction. A successful milestone requires recurring value without making Chat or Daily Review privileged