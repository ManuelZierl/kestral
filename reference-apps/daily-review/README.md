# Daily Review

A focused Kestral reference app for a recurring personal workflow.

## What it proves

- backend-free installation through the normal package path;
- local host-managed notes and hierarchical tasks;
- Markdown-like `- [ ]` task entry, Enter to create, and Tab to create a child under the selected task;
- a root task and its descendants move to Archive only when the root is completed;
- daily notes remain useful without a model;
- AI is optional, explicitly invoked, permission-mediated, and staged for review before it changes data;
- Chat can read tasks only through an approved, bounded read capability and can create only a reviewable proposal rather than directly writing app data;
- Chat proposals bind the target store generation and task revision, and Daily Review disables stale proposals before applying through its own CAS mutation path; and
- the app has its own dependency-free Node 22 build/test path and does not need host or kernel source to produce its installable `dist/` package.

## Build and test

From this directory, with Node.js 22 or newer:

```bash
npm run build
npm test
```

`npm run build` reads only this app's `src/` directory and produces `dist/` with a deterministic asset digest. The tests rebuild in a temporary directory and compare the result with the tracked distribution, so stale committed package output fails qualification. The host integration suite also inspects the tracked package with Kestral's authoritative package validator.

## Install

Install the `dist/` directory through **Apps → Install an app**. The package has no native backend and no runtime dependency after installation. The install flow separately asks whether Chat may read the task collection and whether it may create revision-bound task-title proposals; denying either integration leaves the Daily Review surface usable.

## Limits of this reference app

This is intentionally a product-validation app, not a new host subsystem. Selective per-app restore, a portable user-facing domain export, externally distributed authoring/validation tooling, and a clean-profile packaged-host exercise remain milestone gates in `docs/recurring-workflow-milestone.md`.
