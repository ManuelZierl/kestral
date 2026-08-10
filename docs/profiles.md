---
title: Profiles and data
layout: default
parent: Using Kestral
nav_order: 5
---

# Profiles and data
{: .no_toc }

1. TOC
{:toc}

A Kestral profile is a separate host data and runtime root. It contains host
settings, credential references, chat threads, durable kernel state, trusted
notices, registered files, publisher trust, installed apps, and app data.
Profile separation is not an OS sandbox for native app processes. Model-provider
profiles are different: they live inside one Kestral profile.

## Create and use a managed profile

1. Open **Settings → Kestral profiles**.
2. Use the **Create profile** (+) action.
3. Enter a name. Kestral suggests the lowercase short name used by the launch
   command; you can change it before creating the profile.
4. Close Kestral and restart it with the command under **Profile details**:
   `--profile <slug>`.

Creating a profile does not switch the running process. It selects the new
profile for the next launch. The UI distinguishes **Current** from **Next
launch**, and neither profile can be deleted. Deleting any other managed profile
permanently removes its root; the UI requires the profile name as confirmation.

Profile creation and deletion use a persisted transition record around the
registry and directory changes. Startup completes a committed deletion or
removes an uncommitted created root before exposing profiles, so an interrupted
operation cannot leave the registry pointing at deleted data.

## Select a profile at startup

```powershell
Kestral.exe --profile work
Kestral.exe --data-dir D:\Kestral\isolated
```

Environment-variable equivalents are `KESTRAL_PROFILE` and
`KESTRAL_DATA_DIR`. Command-line options take precedence. `--profile` and
`--data-dir` are mutually exclusive, as are their environment-variable forms.

`--data-dir` selects an ad hoc root that is displayed as **Custom data dir**;
it is not added to the managed profile registry. The directory must either be
empty or already contain the current `kestral-profile.json` identity. Kestral
refuses a non-empty unidentified directory instead of adopting its contents as
a profile.

## Backup and recovery

### Portable workspace

Open **Settings → Kestral profiles → Portable workspace** to create or import a
single `.kestral-portable.zip` archive. Export verifies that the profile did not
change while it was read, writes a manifest-first archive through a temporary
file, and reports the final SHA-256 digest.

Portable archives include durable kernel state, configuration, Chat threads,
trusted notices, publisher trust, gateway audit history, and bytes under
`apps/.data`. They deliberately exclude OS-vault secret values, remote-owner
passkeys, app package binaries, external file contents and absolute paths,
locks, temporary trees, and in-progress transitions.

Import first validates every manifested path, size, and SHA-256 digest. You can
then create a fresh managed profile or overwrite the current profile. A fresh
profile is selected for the next launch. Overwrite requires the exact phrase
`RESTORE <current-profile-slug>`, applies before stores open on restart, and
retains the prior profile under `.kestral-profile-backups/<transaction-id>/`.

After import, re-enter listed credentials, reinstall each listed third-party app
from a package with the recorded digest, and re-register external files and
folders. Imported third-party app registrations are dormant; their prior grant
facts remain in history but are revoked, so reinstall uses the normal permission
review instead of activating transferred authority. Passkeys must be paired
again because WebAuthn credentials are origin- and authenticator-bound.

In backend-only mode, the paths entered in this section are paths on the host,
not on the browser device. The backend does not expose profile archives through
the browser transport.

Close Kestral before copying a profile root. Copying the full root preserves
the profile's JSON stores and installed app data, but OS-protected credential
values remain in the operating system's credential store and are not made
portable by the directory copy alone.

Persisted formats are strict and checksummed where appropriate. Corruption or
an unknown version fails visibly rather than silently discarding fields.
Version `0.1.0-alpha.1` is the current pre-publication testing data shape.
Earlier development state is not migrated and should be deleted or opened only
with the build that created it.

On startup Kestral locks the global registry and selected profile before any
operational store opens. A recognized forward migration stages and validates a
complete candidate, retains the original under
`.kestral-profile-backups/<transaction-id>/`, and commits through a restart-safe
journal. Repeated startup is idempotent. Unknown or corrupt versions preserve
the source and stop startup rather than creating empty settings, chats, apps, or
kernel state. Credential values remain in the OS vault and require separate
backup/recovery handling.
