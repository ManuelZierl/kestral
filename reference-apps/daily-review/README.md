# Daily Review

A focused Kestral reference app for a recurring personal workflow.

## What it proves

- backend-free installation through the normal package path;
- local host-managed notes and hierarchical tasks;
- Markdown-like `- [ ]` task entry, Enter to create, and Tab to create a child under the selected task;
- a root task and its descendants move to Archive only when the root is completed;
- daily notes remain useful without a model;
- AI is optional, explicitly invoked, permission-mediated, and staged for review before it changes data; and
- the app has its own dependency-free Node 22 build/test path and does not need host or kernel source to produce its installable `dist/` package.

## Build and test

From this directory, with Node.js 22 or newer:

```bash
npm run build
npm test
```

`npm run build` reads only this app's `src/` directory and produces `dist/` with a deterministic asset digest. The tests rebuild in a temporary directory and compare the result with the tracked distribution, so stale committed package output fails qualification.

## Install

Install the `dist/` directory through **Apps → Install an app**. The package has no native backend and no runtime dependency after installation.

## Limits of this reference app

This is intentionally a product-validation app, not a new host subsystem. It does not yet prove the full cross-app proposal handoff, selective per-app restore, or useful domain export; those remain explicit milestone gates in `docs/recurring-workflow-milestone.md`.
