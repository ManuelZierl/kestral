---
title: Managing apps
layout: default
parent: Using Kestral
nav_order: 1
---

# Managing apps
{: .no_toc }

1. TOC
{:toc}

Apps are how a Kestral workspace grows around its owner. A package can add a
focused standalone surface, contextual integration, capabilities for granted
callers, or a combination of them without adding task-specific behavior to the
host. Bundled and third-party apps use the same capability authority model.

## Supported install sources

Kestral 0.1 installs app packages from:

- a local package directory; or
- a public HTTPS Git repository.

The selected folder or repository may contain `app.json` at its root or under
`dist/`. `.ahpkg` archives and private Git authentication are not supported in
the 0.1 series.

## Review and install

1. Open **Apps → Install an app**.
2. Select **Local folder** or **Public Git URL** and enter the source.
3. Choose **Review app**. Kestral parses declarations, verifies checksums and an
   optional publisher signature, and stages immutable bytes. It does not load
   app UI or run backend code.
4. Start with the decision summary: publisher trust, how the app runs, and every
   requested permission. Data retention, features, setup, schemas, and package
   internals remain available in collapsed disclosures.
5. If a valid signature uses an unknown key, compare the key through a trusted
   channel before choosing **Trust key**. Trust is scoped to that app identity.
6. Choose **Install app** to continue. A fresh install does not repeat the same
   declarations in a second diff. Trusted chrome groups the app's full permission
   request into one checklist before the kernel commits the app.

Unsigned packages are visibly marked but may be installed. Invalid or revoked
signatures are rejected.

The reviewed bytes, not the source path, are installed. Kestral copies the
staged package into its content-addressed app store and retains the package
digest alongside the separately sealed kernel manifest. Backend code is not run
during review. A denied permission produces no grant; installation may continue
with the approved subset.

The install record retains app/version identity, package and manifest digests,
install time, host-version floor, active state, and verified code revisions. It
does not persist the approved permission subset separately. Declared secret
names appear during review, but values are entered later through the app's
host-owned settings and secret UI; secret values are never part of a package or
install record.

For `data.kind = "host-managed"`, review explains direct owning-screen access,
delegated grants and Runs, retention, reinstall, and explicit purge behavior.
Technical details list every collection and its record schema, enabled operation,
index and uniqueness rule, record/query quota, and total byte ceiling. These apps
have no backend process. Any exported read action is a fixed host operation, not
publisher code, and still requires grants from delegated callers.

{: .warning }
An `mcp-stdio`, executable, or agent-worker backend marked **unsandboxed** runs
as the backend operating-system account. Kernel grants control calls routed
through Kestral; they do not prevent that process from directly reading files or
using the network. Kestral repeats this warning prominently, outside collapsed
technical details, in every install, update, downgrade, and revert review whose
target package is unsandboxed. The existing operation confirmation remains the
per-operation decision; Kestral does not add a habituating second checkbox.
Install native backends only from publishers you trust.
Release builds require an explicit unsafe-native-backend opt-in before such a
package can activate.
The per-user, non-administrator procedure and its broad security effect are
documented under [Agent Engine opt-in]({% link getting-started.md %}#agent-engine-opt-in).

## Updates, downgrades, and revisions

Inspecting another package with the same app ID starts a managed transition.
Kestral shows capability, permission, surface, backend, and version changes
before applying it. The review also shows any app-data format change and marks a
publisher-declared destructive migration. Higher versions are updates. Lower
versions require explicit acknowledgement and are refused when current data
would require an undeclared reverse migration. A same-version package with
different bytes is rejected as a version conflict.

Prior code revisions are retained for an explicit revert. A transition is
journaled so startup can complete or roll it back after interruption. For a
declared data-format change, Kestral migrates a staged copy, validates it with
the target backend, retains the source as a recoverable backup, and then changes
the active pointer. Failed activation restores the old data and code. Configure
the retained backup count under **Settings → Kestral profiles**; it can never be
lower than one.

Host-managed updates do not run publisher migration code. Transition planning
validates all retained records and quotas against the target declaration before
the approval can be applied, including while the app is disabled. Compatible
changes are accepted and preserve a pre-change contract snapshot before the
first later write; incompatible changes refuse without replacing data or code.

The kernel registry has no in-place upgrade operation. The host therefore
performs update, downgrade, and revert as one serialized uninstall/install
transition. Uninstall tears down the old identity and authority before the new
sealed manifest is installed. The current candidate asks for the target
revision's full requested grant set again; only grants approved in that new
review are issued. Kestral stores no separate approved subset. Prior denials and
revocations therefore cannot become silent grants, while unchanged requests are
still shown again. Removed capabilities lose their grants.

## Disable, enable, and uninstall

- **Disable** stops the backend, removes the app from the active kernel, closes
  its surfaces, and revokes related grants while retaining its installed bytes
  and host-owned configuration.
- **Enable** reinstalls the retained, verified revision and asks the owner to
  review its requested grants again.
- **Uninstall** removes retained app code. The confirmation lets you purge app
  settings, credentials, and app data explicitly.

Host-managed records follow the same keep/purge choice. Reinstalling the same
app ID may reuse retained records only when the active package declaration can
validate them; app identity alone never reinterprets or deletes retained data.

Runs and artifacts remain after uninstall because they are trusted audit and
provenance records. Bundled components are read-only and cannot be managed from
this lifecycle UI.
