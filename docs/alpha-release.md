---
title: v0.1.0-alpha.1 release candidate
layout: default
parent: Contributing
nav_order: 1
---

# v0.1.0-alpha.1 release candidate

Kestral is a personal-first, open-source AI workspace and lean local host for
user-chosen apps. Chat is the default starting app, not the canonical interface
for all AI work. Focused apps use shared model, permission, Run, artifact, and
composition mechanisms. `v0.1.0-alpha.1` is the planned first public testing
release; it has not been published.

## Audience and status

The planned prerelease is for developers, early technical testers, potential
contributors, and people willing to report installation, compatibility,
security, persistence, and usability problems. It is not production-ready or a
finished general-user release. The release tests the personal workspace and app
host with technical users; it does not claim a mature ecosystem or prove the
broader general-user direction.

## Release scope

The release candidate contains exactly these end-user artifacts:

| Artifact | Supported use |
|---|---|
| `kestral-0.1.0-alpha.1-windows-x86_64-nsis.exe` | Current-user Windows desktop installer. |
| `kestral-0.1.0-alpha.1-windows-x86_64-portable.zip` | Portable Windows desktop and backend binaries. |
| `kestral-0.1.0-alpha.1-linux-x86_64.AppImage` | Portable Linux desktop application. |
| `kestral-0.1.0-alpha.1-linux-x86_64.deb` | System-installed Linux desktop application. |
| `kestral-0.1.0-alpha.1-linux-x86_64-server.tar.gz` | Backend-only Ubuntu split deployment. |
| `kestral-browser-client-0.1.0-alpha.1.zip` | Static trusted owner console for split deployment. |

`THIRD-PARTY-NOTICES.txt`, `BUILD-PROVENANCE.txt`, `PROMOTED-APPS.json`,
`RELEASE-EVIDENCE.md`, and `SHA256SUMS.txt` accompany that matrix. External apps
remain independently built and distributed; they are not Kestral release
artifacts.

The alpha compatibility-evidence set contains Daily Notes, Chat Export,
Whiteboard, Model Profiles, Kestral Pi, and Reading Insights. The promoted-app
record pins each independent repository commit, package digest, exact tested
host version, and extension contracts. Publication is blocked until every entry
also pins an immutable evidence document covering package inspection, permission
denial, activation, one representative action, restart, update with data
preservation, disable/enable, and both keep-data and purge-data uninstall.

Promotion is narrower than curation: it records a tested compatibility claim
for exact bytes. It does not bundle the app, grant special authority, or endorse
the app for general use. There are no curated apps in this alpha.

This alpha makes no claim of:

- production or general-user readiness;
- a mature app ecosystem or marketplace;
- OS-level filesystem, network, or process isolation for native backends;
- measured or proven leanness; or
- macOS, MSI, signed native, multi-user, or tenant-hosting support.

Custom app UI does run in constrained opaque-origin frames. That browser boundary
does not sandbox a native backend, which runs with the backend operating-system
account's authority after explicit unsafe-backend opt-in. Resource, startup, and
time-to-first-result baselines remain release evidence to collect, not results
that follow from the architecture.

## Unsigned artifacts

All native Windows and Linux artifacts are currently unsigned.

| Artifact | Privilege expectation |
|---|---|
| Windows current-user NSIS `.exe` | No elevation intended. |
| Windows portable `.zip` | No installation or elevation intended. |
| Linux AppImage | No root required; mark the file executable. |
| Linux `.deb` | System installation normally requires root or `sudo`. |
| Linux backend `.tar.gz` | Runs from a user-owned directory; no installation required. |
| Static browser-client `.zip` | Serve as static files behind HTTPS or an encrypted private tunnel. |

Every public file is covered by `SHA256SUMS.txt`. `BUILD-PROVENANCE.txt` records
the exact commit and public GitHub Actions run that assembled the release. It is
a traceability record, not a code signature.

Every archive and native bundle also includes `THIRD-PARTY-NOTICES.txt`, which
lists dependency license metadata and reproduces license or notice files shipped
by those dependencies. The same notice inventory is published beside the
release artifacts.

No MSI is declared for this alpha because the current Tauri/WiX pipeline does
not accept its SemVer prerelease identifier. An MSI or machine-wide installer
can require elevation depending on configuration; unsigned status alone does
not impose that requirement.

On Windows, SmartScreen may show **Unknown publisher**, and managed devices may
block Kestral completely. Corporate policy can block an executable even when
the current-user installer or portable archive needs no administrator rights.
Download only from `ManuelZierl/kestral`, verify `SHA256SUMS.txt`, or build from
source.

On Linux, the AppImage normally runs from a user's home directory after
`chmod +x`; the `.deb` normally needs root or `sudo` for system installation.
Desktop tools can warn about packages outside trusted repositories, and managed
systems can add execution policy. Verify checksums and the release CI provenance.

## Deployment choices

- **Desktop mode:** Tauri shell and local Kestral runtime on Windows or Linux.
- **Split mode:** complete Kestral backend on Ubuntu with the static Svelte UI
  opened from a normal browser on another device.

In split mode, the Ubuntu machine owns all data, files, workers, MCP connections,
grants, Runs, artifacts, and secrets. The browser has no access to the client
machine's filesystem and needs no Tauri executable.

The browser is a paired trusted owner console. SSH bootstraps a WebAuthn passkey,
and its short-lived authenticated session authorizes the complete remote owner
command surface. Use only HTTPS, a VPN, encrypted tunnel, or equivalent secure
transport, and never expose non-loopback plain HTTP merely for convenience.

## Compatibility and data risk

Development data created before `v0.1.0-alpha.1` is disposable and has no
compatibility path. From every public release onward, each later supported
Kestral release must either migrate valid released data forward or refuse
visibly while preserving the original. Downgrade compatibility is not promised,
and app publishers remain responsible for versioning and migrating records
inside their own opaque data.

Persisted-data continuity and app/API compatibility are separate promises.
Versioned package, extension, API, surface, and worker contracts may evolve
during alpha. An incompatible contract must fail visibly rather than be guessed
or silently reinterpreted. Contract evolution does not waive the data promise:
Kestral must preserve or migrate released host-owned state and opaque app-data
bytes, while each app publisher must preserve or migrate the records it owns.
For host-managed contracts v1 and v2, Kestral owns the envelope and validation
behavior; the package owns record schemas, document metadata schemas, opaque
document formats, and domain meaning. A later host must preserve valid data or
visibly refuse the incompatible declaration without rewriting it.

The host now runs one locked, crash-recoverable, idempotent coordinator before
operational stores open and tests it against immutable `alpha.1` fixtures and
non-widening authority rules. Each future release must still add and test its
explicit migration step before publication. Stateful app packages separately
declare a supported host-managed contract or their publisher-owned data format
and migration edges. Kestral migrates
a staged copy, retains at least one source backup, and restores it when target
activation fails. General backup restore UI and undeclared data downgrade remain
unsupported. Uninstall can purge selected app data, while Runs and artifacts
normally remain for provenance. Never use irreplaceable data or production
credentials for pre-publication QA.

Every promoted app owns its package tests, and every stateful app also owns
immutable data fixtures and migration tests in its independent repository. The
release report pins that repository's exact clean source commit, installable
package digest, exact tested host commit and version, external workflow, and
observed lifecycle results. Kestral core validates the pinned evidence record;
it does not vendor, check out, or build the external app source or package.

## Optional agent engine

Kestral Pi is an independently built and distributed optional app. Chat works
without it. The Kestral release neither builds nor silently includes it in the
base desktop or server packages.

## Report results

Use the repository's **Alpha bug report** issue template and include artifact
name and SHA-256, OS, managed-device status, elevation behavior, exact warnings,
reproduction steps, logs with secrets removed, persistence/recovery behavior,
and whether Kestral Pi or remote mode was involved. The
[alpha testing checklist]({% link contributing.md %}#alpha-testing-checklist)
lists concrete areas where reports are needed.
