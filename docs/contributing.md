---
title: Contributing
layout: default
nav_order: 8
has_children: true
---

{% assign internal_link_prefix = "" %}{% assign jekyll_major = jekyll.version | split: "." | first %}{% if jekyll_major == "3" %}{% assign internal_link_prefix = site.baseurl %}{% endif %}

# Contributing
{: .no_toc }

1. TOC
{:toc}

## Prerequisites

- stable Rust toolchain;
- Node.js 22 and npm;
- platform dependencies required by Tauri 2;
- Node on `PATH` for MCP stdio conformance and packaged-app lifecycle tests.

Read [Architecture]({{ internal_link_prefix }}{% link architecture.md %}),
[Trust model]({{ internal_link_prefix }}{% link trust-model.md %}), and the repository `AGENTS.md` before
changing architecture. The five primitives are a closed set, MCP belongs in
the adapter/host boundary, and Chat, LLM Provider, and agent behavior remain
userland.

Product decisions follow the same priority order as the architecture: personal
workspace value first, an ecosystem-ready developer platform second, and
broader general-user access third. Before adding host behavior, establish why it
cannot remain app-owned, which recurring work it improves, and what resource,
interaction, compatibility, and support burden it adds. Tauri and a small kernel
do not by themselves prove that the complete product is lean.

## Run locally

```sh
cd host
npm install
npm run tauri dev
```

## Verification

The host crate bundles `THIRD-PARTY-NOTICES.txt` as a Tauri resource, so
generate it once (and after dependency changes) before building or testing the
host crate:

```sh
node scripts/generate-third-party-notices.mjs
```

```sh
cargo test -p app-host-kernel
cargo test -p mcp-adapter
cargo check -p host
cargo test -p host

cd host
npm run check
npm test
```

Build the documentation locally from `docs/` with Ruby and Bundler:

```sh
cd docs
bundle install
bundle exec jekyll build --strict_front_matter
cd ..
node scripts/check-doc-links.mjs docs/_site /kestral
```

Without local Ruby, run the same build in a disposable container from the
repository root:

```powershell
docker run --rm -v "${PWD}/docs:/site" -w /site ruby:3.3 `
  sh -lc "bundle install && bundle exec jekyll build --strict_front_matter"
node scripts/check-doc-links.mjs docs/_site /kestral
```

Before a release-bound change, also run workspace formatting, Clippy with
warnings denied, all features, frontend build, dependency audits, and the core
isolation gate used by `.github/workflows/ci.yml`. External app repositories
run their own package builds, tests, audits, and reproducibility checks.

Architecture-boundary changes must preserve focused evidence for these failure
and recovery cases:

| Contract | Required evidence |
|---|---|
| Package inspection and lifecycle | Inspection executes no package code; checksums/signatures and manifest consistency fail closed; permission prompts occur; update, revert, disable, and uninstall drive normal kernel install/teardown paths. |
| Durable kernel transaction | Injected failure before commit leaves memory unchanged; durable-write-before-memory-swap recovers the candidate; grants, revocations, ledger, and artifacts survive restart. |
| Crash recovery | An active Run becomes `interrupted` exactly once; corrupt state aborts startup; a second host cannot acquire the state lock. |
| Multi-file workspace transaction | Prepared work is discarded, committed work completes idempotently, and rename/delete batches recover together. |
| Provider worker | Strict protocol, conversion, OAuth, credential rotation, cancellation, fake-provider behavior, packaged runtime version, and no ambient authentication. |
| Agent worker | Strict protocol and bounds, mediated child model/tool Runs, recursion exclusion, cancellation, transcript provenance, Chat fallback, and real package startup. |
| Trust presentation | Trusted-chrome approval/OAuth tests and custom-surface progress/cancellation tests preserve the UI boundary. |

## Branch and release model

`main` is the integration and release branch. Pull requests to `main` and pushes
to it run Linux and Windows tests, native Windows credential integration, and
package builds. A `v*` tag contained in `main` runs the release workflow,
verifies the tag against every product version, reruns the release gates on
Linux and Windows, builds both platform artifact sets, requires the complete
matrix, writes and verifies checksums, and marks versions with `-alpha.N` or
`-beta.N` as GitHub prereleases.

Before creating a tag, run the **Release** workflow manually from the intended
`main` commit and enter the manifest version without a `v` prefix. This
`workflow_dispatch` path executes validation, both platform builds, artifact
download, matrix checks, provenance, and checksums, then retains the assembled
release as a workflow artifact without creating a GitHub Release. Only a `v*`
tag push enables the publish job and its repository write permission. The tag
workflow rebuilds and retests the tagged commit; it publishes the assembled
artifact downloaded within that same run, not the earlier manual-run bytes.

### Promoted external app gate

`release/promoted-apps.json` is the release roster for external compatibility
claims. Each entry names an HTTPS repository, full source commit, package
version, host-canonical package SHA-256, exact host version, extension
contributions, and a SHA-256-pinned external evidence URL. The release workflow
verifies that each source commit exists in its declared GitHub repository,
downloads only the evidence document, and checks it against
`schemas/external-app-release-evidence.schema.json`. It never checks out or
builds external app source and never downloads an external package.

Run the local contract tests with:

```sh
node --test scripts/check-alpha-release-evidence.test.mjs
node scripts/check-alpha-release-evidence.mjs
node --test scripts/check-promoted-app-contracts.test.mjs
node scripts/check-promoted-app-contracts.mjs
```

The release-only form also requires a complete report, immutable evidence, and
remote commits:

```sh
node scripts/check-alpha-release-evidence.mjs \
  --require-complete \
  --release-commit <full-release-commit>
node scripts/check-promoted-app-contracts.mjs \
  --require-evidence \
  --verify-remotes \
  --release-commit <full-release-commit>
```

Evidence cannot name the same commit that first records its own hash. Use one
clean tested core commit for the candidate binaries and lifecycle runs, then one
metadata-only commit that fills `tested_core_commit`, `evidence_url`, and
`evidence_sha256` in `release/promoted-apps.json` and completes
`release/v<version>-evidence.md`. Release validation requires the tested
commit to be an ancestor and refuses any intervening change outside those two
release metadata files. The tested core commit is an executable/build source
freeze; changing any source or build input requires a new candidate.

Create one evidence document per promoted app from the exact clean external
commit and package digest. Retain the workflow log and record a concrete
observation for every required result:

1. Inspect the real package and confirm no package code executes during review.
2. Deny an applicable permission or cancel permission issuance and confirm no
   denied grant appears. For an app requesting no grants, cancel installation at
   the host confirmation boundary and confirm no app state is created.
3. Activate the package and confirm its expected surfaces or headless provider.
4. Complete one representative user action through Kestral's normal action path.
5. Restart the host and verify activation plus retained state.
6. Update from the preceding published package and verify app-owned and
   host-owned state is preserved or explicitly migrated.
7. Disable and re-enable the app, confirming authority is absent while disabled
   and restored only through normal activation.
8. Uninstall once with data retained, reinstall, and confirm the retained data.
9. Uninstall a separate test installation with data purged and confirm package,
   app data, app config, and app secrets are absent while Runs and artifacts
   retain their normal provenance.

For the first alpha evidence pass, use these clean predecessor packages for the
update step. They are external package inputs to the app-owned evidence run, not
dependencies fetched by Kestral core CI.

| App | Predecessor version | Source commit | Package digest |
|---|---|---|---|
| Daily Notes | `0.1.0` | `543332db5162ee63e8686cb4ad08c60e76791b6a` | `sha256-8fc172678cb11c51907bb3bb15d97a0ee4e0acf9ef27b16e143e2c07c84f7c08` |
| Chat Export | `0.1.1` | `d83a64b336ebc4ee8f6c75d710ed7689c6571a19` | `sha256-9fa54047ed2046d7d4efd3248762893a5270e826d96444c46ee20990076d37e4` |
| Whiteboard | `0.1.1` | `fcb5c6d453f7de4db8c3b0a0f19746a3b1a36891` | `sha256-e03b8097f932e13fdc542b7526881a0d812b1d90777c00988e9fa7cc5bef3c94` |
| Model Profiles | `0.1.0` | `f31fd0da4c07eb1ffff746d2b8861b20ed679dd7` | `sha256-6e870e024a62b562a49b7a3a52f1b9d954647fa55f5be94bb5ef2d3d4889c82e` |
| Kestral Pi | `0.1.1` | `d1d63bfaee1730fa8361810d70aae8e83b99323e` | `sha256-6b1dbe0fb9d5a22c90e7602a653d8bfc063af0fb1f7ded2fe18a9ff1ff72532f` |
| Reading Insights | `0.3.1` | `e92051b2f91350a0684d6de4d24b639178714d96` | `sha256-ac6c85f561ac8ee60a05d0af99ab801ed15c6a14e3b1cd42235c3873f16cc255` |

The evidence document must identify the exact host commit, workflow URL,
timestamp, and tested platforms. `source.clean` must be `true`; every lifecycle
status must be `passed`; and every observation must identify what the retained
run actually proved. A dirty core checkout, dirty app checkout, missing evidence,
unreachable source commit, changed evidence bytes, host mismatch, or extension
contract mismatch blocks release publication.

## Non-administrator release verification

The supported installer must work from a standard Windows account. Do not run
this acceptance pass from an elevated terminal.

1. Build the exact Windows installer with
   `cd host && npm run tauri build -- --bundles nsis`. MSI is not declared for
   the alpha because Tauri/WiX rejects its SemVer prerelease identifier.
2. Use a clean Windows VM snapshot or a separate standard local user with no
   prior Kestral profile. Ensure `node` is absent from `PATH` if possible.
3. Start `kestral-0.1.0-alpha.1-windows-x86_64-nsis.exe` normally. A UAC credential or consent
   prompt is a release failure. Confirm the destination is under the user's
   local application data, not `Program Files`.
4. Launch Kestral from the Start menu. Confirm Chat, LLM Provider, and File
   Broker appear; navigate through Apps, Artifacts, Settings, and System; close
   and reopen the app; and confirm startup remains clean.
5. Add and remove a disposable provider credential and confirm Windows
   Credential Manager is used without elevation. Do not use a real production
   credential for release QA.
6. If testing an independently distributed Kestral Pi package, use the
   documented user-level unsafe-backend opt-in, send one agent message, then
   remove the app and clear the opt-in.
7. Uninstall Kestral from the current user's Installed apps page. Uninstallation
   must not request elevation. Confirm the application files are removed; keep
   or delete the profile intentionally according to the test case.

Record the Windows version, account type, installer SHA-256, install path,
whether UAC appeared, first-launch result, restart result, credential result,
and uninstall result in the release notes or release checklist. Code signing
changes SmartScreen reputation but does not change this per-user privilege
model.

## Alpha testing checklist

Reports should identify the exact artifact, version, commit when source-built,
and SHA-256. Exercise the applicable cases rather than only confirming that a
window opened:

- Windows version and edition, and whether the device is managed or unmanaged;
- NSIS or portable ZIP, whether elevation was requested, and the exact
  SmartScreen or corporate-policy message;
- clean-machine startup and installed WebView2 availability;
- Ubuntu or other Linux distribution and desktop environment;
- AppImage executable-bit/startup behavior and `.deb` install/remove behavior;
- startup time and idle memory after initial bootstrap;
- provider configuration with disposable credentials and secrets removed from
  reports;
- Chat without Kestral Pi, then optional Kestral Pi installation and one agent
  interaction;
- profile, Chat, app, and artifact persistence after restart;
- third-party app inspection, installation, disable/enable, removal, and data
  purge choice;
- permission prompts, denials, revocations, and resulting capability behavior;
- Ubuntu backend/static browser-client connection, exact origin, HTTPS or
  encrypted tunnel, disconnect/session clearing, server-side file access, and
  one successful action from an independently built test app's custom surface;
- forced process termination, restart, interrupted Run recovery, and visible
  corrupt-state failure where safely testable;
- Windows uninstall without elevation, Linux package removal, and retained or
  purged data according to the selected test case.

Use the repository's **Alpha bug report** issue template. Include exact steps,
expected and actual behavior, relevant logs or screenshots with tokens and
secrets removed, and whether the problem reproduces after restart.

Rust unit tests live in child modules (`src/foo/tests.rs`), not inline test
blocks in production files. Integration tests live under each crate's `tests/`
directory. Frontend tests use Vitest and jsdom.
