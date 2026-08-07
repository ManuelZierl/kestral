---
title: Curated apps
layout: default
parent: Using Kestral
nav_order: 2
---

# Curated apps
{: .no_toc }

1. TOC
{:toc}

This list is an optional discovery aid for focused independent apps, not a
host-controlled marketplace. Curated apps are useful starting points for
growing a personal workspace beyond Chat. They are not bundled with Kestral and
remain separate products maintained and released from their own repositories.

{: .warning }
Curated does not mean sandboxed, security-audited, or endorsed without
reservation. Always review the package identity, publisher, backend authority,
and every requested permission before installing. A native backend can exercise
the full authority of your operating-system account outside Kestral's grants.

## App list

No app is curated for `v0.1.0-alpha.1`. Notes was removed from the candidate
list because its repository does not yet provide the repository license, exact
runtime declaration, current package contract, and immutable lifecycle evidence
required by the criteria below. It can be reconsidered after those publication
requirements are met.

Apps named as release compatibility evidence are not automatically curated.
Compatibility evidence proves only the recorded package and lifecycle checks;
it is not a security endorsement or a general recommendation.

## Install a curated app

1. Obtain an app from its publisher and install its declared runtime
   requirements. Public Git installation also requires `git` on the machine
   running the Kestral host.
2. Open **Apps → Install an app** and select **Public Git URL**.
3. Enter the repository URL supplied by the publisher.
4. Choose **Review app**. Check publisher trust, how the app runs, and its
   requested permissions first. Open the technical details when you need the
   app ID, package contents, or compatibility information.
5. Continue only with the permissions you intend to grant.

Kestral installs the reviewed package bytes from `app.json` or `dist/app.json`;
it does not run a repository's build during installation. See
[Managing apps]({% link managing-apps.md %}) for the complete review, update,
disable, and uninstall workflow.

## Curation criteria

An app proposed for this list should:

- have a public HTTPS Git repository with an installable `app.json` or
  `dist/app.json` and a clear open-source license;
- declare a stable app ID, version, supported Kestral version, maintainer, and
  runtime requirements;
- explain its purpose, data storage, deletion behavior, requested permissions,
  and any unsandboxed native authority;
- request only the authority needed for its documented behavior;
- provide source, build instructions, tests, and committed package output that
  can be compared with a clean build;
- pass Kestral package inspection and work on a currently supported Kestral
  release; and
- have no unresolved critical security issue known to the Kestral maintainers.

Curation is intentionally revocable. A listing may be changed or removed when
an app becomes incompatible, unmaintained, misleading, or unsafe.

## Propose an app

Open a [Kestral issue](https://github.com/ManuelZierl/kestral/issues/new) titled
`Curated app proposal: <app name>` and include:

- the app name, repository URL, app ID, current version, license, and
  maintainer;
- a short description of the user problem it solves;
- the minimum Kestral version and supported operating systems;
- screenshots or a short demonstration of its primary workflow;
- backend kind, authority mode, runtime dependencies, data locations, and
  network behavior;
- every requested permission and why it is necessary;
- exact build and test commands, plus how committed package output is produced;
  and
- known limitations, security considerations, and maintenance expectations.

A maintainer will review the proposal against the criteria above and may ask
for package, documentation, permission, or test changes. Once accepted, add the
app to this page in a pull request. Listing decisions concern this curated page
only; they do not grant an app special host authority or bypass normal package
review.
