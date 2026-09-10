# Daily Review

A focused Kestral authoring example for a recurring personal workflow. This
in-tree example is qualified by core CI but is never bundled or installed at
startup. Independently released apps keep their own repositories and release
qualification.

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

`npm run build` reads only this app's `src/` directory and produces `dist/` with a deterministic asset digest. Text assets use LF and exactly one final newline. The tests rebuild in a temporary directory and compare the result with the tracked distribution, so stale committed package output fails qualification. The host integration suite also inspects the tracked package with Kestral's authoritative package validator.

The app tests execute the actual surface script using a small DOM adapter and a host-data fake. They cover draft preservation, concurrent saves, generation-pinned pagination, hierarchy/archive behavior, proposal races, optional AI, and local-date rollover. These tests are not a substitute for a packaged-host browser or native-webview exercise.

## Install

Install the `dist/` directory through **Apps → Install an app**. The package has no native backend and no runtime dependency after installation. The install flow separately asks whether Chat may read the task collection and whether it may create revision-bound task-title proposals; denying either integration leaves the Daily Review surface usable.

## Drafts, refresh, and conflicts

Task changes and **Refresh data** preserve unsaved notes. A failed save keeps both the draft and the revision it was edited from, so refreshing cannot silently authorize overwriting another window's changes. The saved-note preview shows the current stored version; **Discard draft and load saved note** is the explicit way to abandon a local draft. After an unconfirmed write, inspect refreshed data before retrying.

Tasks are read across all pages at one store generation, and the daily note is looked up through its unique day index. At midnight, a clean editor advances to the new local date. An unsaved note stays attached to its original date until saved or explicitly discarded. Shift+Tab and Tab with an empty task input retain normal keyboard focus navigation.

## Limits of this reference app

This is intentionally a product-validation app, not a new host subsystem. Selective per-app restore, a portable user-facing domain export, externally distributed authoring/validation tooling, and a clean-profile packaged-host exercise remain milestone gates in `docs/recurring-workflow-milestone.md`.
