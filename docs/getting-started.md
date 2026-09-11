---
title: Getting started
layout: default
nav_order: 2
---

{% assign internal_link_prefix = "" %}{% assign jekyll_major = jekyll.version | split: "." | first %}{% if jekyll_major == "3" %}{% assign internal_link_prefix = site.baseurl %}{% endif %}

# Getting started
{: .no_toc }

1. TOC
{:toc}

Kestral `0.1.0-alpha.1` is the planned first public testing release. It has not
been published and is not a production-ready general-user release. Until it is
published, build from source. The artifact names and installation steps below
describe the intended release candidate.

## First startup

A newly created profile includes a removable **Kestral documentation** tool
server pointing to the public Kestral repository through unauthenticated GitMCP.
This is an inert saved shortcut: Kestral does not contact the endpoint, discover
tools, install an app, or request permissions during startup. To opt in, open
**Settings → Tool servers**, review the saved URL, and choose **Connect**. You
can edit or remove the entry without contacting it. Connected tools and any
later Chat access use the same trusted-chrome review as every other tool server.
See [Tool servers]({{ internal_link_prefix }}{% link tool-servers.md %}) for the
complete flow and security boundary.

If you already have a `.kestral-portable.zip`, start once with a fresh profile,
then open **Settings → Kestral profiles → Portable workspace**. Validate the
archive and import it as a new profile; restart with the command Kestral shows.
Credential values, passkeys, app binaries, and external file paths do not
transfer. See [Profiles and data]({{ internal_link_prefix }}{% link profiles.md %}).

## Choice 1: Native desktop

When the release is published, use the native package on a Windows or Linux
device where local application policy permits it. Download only from the
official GitHub release and compare the artifact with `SHA256SUMS.txt`. Confirm
that `BUILD-PROVENANCE.txt` points to the expected repository commit and
successful GitHub Actions release run.

### Windows

- `kestral-0.1.0-alpha.1-windows-x86_64-nsis.exe` is the primary current-user
  installer. It installs under the current user's local application data and
  should not request elevation.
- `kestral-0.1.0-alpha.1-windows-x86_64-portable.zip` requires no installation.
  Extract the complete archive and run `kestral.exe` without moving it away from
  the bundled `provider-worker/` resources.

This alpha does not publish an MSI: the current Tauri/WiX pipeline rejects a
SemVer prerelease identifier such as `alpha.1`. In general, an MSI or
machine-wide installer may require administrator rights depending on its
configuration; that is separate from whether its files are signed.

Windows SmartScreen may report **Unknown publisher** because these alpha
artifacts are unsigned. Managed company devices may block them completely even
when the user can install software without elevation. A portable build or
current-user installer does not bypass corporate application control. Signing
and administrator privileges are separate concerns: unsigned software does not
inherently require administrator rights.

Verify a download in PowerShell:

```powershell
(Get-FileHash -Algorithm SHA256 .\kestral-0.1.0-alpha.1-windows-x86_64-nsis.exe).Hash.ToLowerInvariant()
```

### Linux

- `kestral-0.1.0-alpha.1-linux-x86_64.AppImage` normally runs from your home
  directory without root. Mark it executable with
  `chmod +x kestral-0.1.0-alpha.1-linux-x86_64.AppImage` first.
- `kestral-0.1.0-alpha.1-linux-x86_64.deb` installs system-wide and normally
  requires `sudo apt install ./kestral-0.1.0-alpha.1-linux-x86_64.deb`.

Linux does not have one universal Authenticode or SmartScreen system, but these
are still unsigned prerelease packages. Desktop environments and package tools
may warn about artifacts outside trusted repositories, and managed Linux
devices may enforce additional policy. Verify checksums and the public CI run.

```sh
sha256sum -c SHA256SUMS.txt
```

Linux credentials require an available, unlocked Secret Service. Headless
Linux fails closed rather than storing provider secrets in plaintext.

## Choice 2: Remote browser client

Use split mode when the complete Kestral backend should run on an Ubuntu
machine while another device uses a normal browser. The Windows client needs no
Kestral executable. Download the Linux server archive and static browser-client
ZIP, then follow [Deployment modes]({{ internal_link_prefix }}{% link deployment-modes.md %}) for the
exact loopback backend, HTTPS proxy, allowed origin, passkey pairing, resource, and
server-side data-directory configuration.

The browser is a paired trusted owner console. Sign-in uses a WebAuthn passkey
registered with a ten-minute, single-use code created through SSH. Its
short-lived server session authorizes the complete remote owner command surface.
The browser cannot browse or directly access its machine's local filesystem;
normal browser downloads remain local to that device. Registered files, app
data, provider workers, Kestral Pi, and MCP connections all live on the Ubuntu
backend. Use **Sign out** to revoke the owner session.

## Build from source

Technical users can install stable Rust, Node.js 22, Tauri 2 prerequisites, and
the documented platform libraries, then run:

```sh
cd host
npm install
npm run tauri dev
```

For browser-host development, `cd host && npm run dev:split` builds and starts
both the loopback backend and Vite gateway. Forward only port `1420`; Vite
proxies `/api` to the host-local backend on port `4310` and carries HMR over the
same tunnel. Use `npm run dev:split -- --pair` when you need an owner pairing
code. During pre-release development, use `npm run dev:split -- --clean` to
irreversibly remove incompatible split-development data and Kestral's
browser-local state, then create a fresh owner pairing code. See [Deployment
modes]({{ internal_link_prefix }}{% link deployment-modes.md %}) for
Zed setup and optional port, origin, and data-directory overrides. For a static
client build, use `cd host && npm run build`.

## First launch

The host activates default-installed **Chat** plus the bundled userland support
apps **LLM Provider**, **Artifacts**, **Permissions**, and **File Broker** in a
new profile. Chat provides an immediate starting point, but it is one ordinary
app rather than the expected interface for every task. Focused apps with their
own document, canvas, form, or dashboard surfaces appear beside it in the
workspace.

A new profile has no model provider profile and makes no assumption that a
local model service is running. Chat shows **Configure model provider** and, if
you send a message first, returns local setup guidance without starting a
provider worker or making a network request.

Sample apps and the optional agent engine are not installed automatically. Notes
and other sample apps are ordinary external packages with no special authority.

Kestral may show a dark amber trusted-chrome approval. Check the requesting
app, capability, reason, interaction policy, data scope, and duration before
approving.

## Configure a model

1. Open **Settings → Model providers**.
2. Choose **Add ChatGPT account** for a ChatGPT Plus/Pro Codex subscription, or
   **Add another provider** for API-key and local providers.
3. Enter the endpoint, model, and required credential; use **Discover models**
   and **Test** where available.
4. Save the profile, then select it under **Default for Chat** on the same page.

Cloud profiles require explicit data-egress acknowledgement. Credential values
stay in the backend operating system's credential store; the frontend retains
status and references only. Kestral does not install or manage Ollama, scan
local provider ports, or add a discovered provider automatically.

### Use a ChatGPT Codex subscription

This path uses the Codex quota included with an eligible ChatGPT Plus or Pro
account. It does not use an OpenAI API key or OpenAI API billing.

1. Under **Settings → Model providers**, choose **Add ChatGPT account**.
2. Review the preselected Codex model and choose **Save**. **Discover models**
   can list the worker's bundled Codex catalog before sign-in.
3. Choose **Connect ChatGPT account**.
4. In the verified host dialog, choose **Browser login** for a desktop host. In
   split mode, choose **Device code login** because the provider worker and its
   callback listener run on the Ubuntu backend.
5. Complete the OpenAI sign-in page. Return to Kestral and confirm that the
   account status says **Connected**.
6. Select the profile under **Default for Chat** on the same page and accept the
   cloud data sharing notice.

OpenAI controls account eligibility, available models, and quota. Kestral can
report provider failures but does not display remaining subscription quota.
Use **Reconnect ChatGPT account** if OpenAI revokes the session, or
**Disconnect account** to remove the stored OAuth credential.

## Add a focused app

After the first message, either install a task-specific app or shape one around
your own recurring work. From a Kestral source checkout, the shortest custom-app
path is:

```bash
node scripts/create-app.mjs ../my-focus-app \
  --id com.example.my-focus-app \
  --name "My Focus App"
cd ../my-focus-app
npm test
```

The generated dependency-free project contains a working custom dashboard,
durable host-managed data, and an optional approval-gated model suggestion.
Install its `dist/` directory through **Apps → Install an app**. See
[Writing app packages]({{ internal_link_prefix }}{% link writing-apps.md %}) to
replace the starter's item model and interface with your use case.

You can instead review another package under **Apps → Install an app**, or
browse [Curated apps]({{ internal_link_prefix }}{% link curated-apps.md %}) for
an independently maintained starting point. A focused app can provide its own
workspace surface and may separately request permission for Chat or another app
to use its capabilities.

## Chat and optional Kestral Pi

Chat works without Kestral Pi through its built-in grant-aware model/tool path.
An independently distributed compatible Kestral Pi package adds multi-turn
agent behavior and holds zero grants itself. Obtain it from that app's publisher;
the Kestral core release does not build or bundle external app artifacts.

### Agent Engine opt-in

Kestral Pi is an unsandboxed native backend in this alpha. Release builds refuse
to activate unsandboxed native apps until the backend owner sets
`KESTRAL_ALLOW_UNSAFE_NATIVE_BACKENDS=true`. On Windows this can be a user-level
environment variable; on Ubuntu set it only in the backend service environment.
The opt-in applies host-wide, so enable it only for inspected packages you
trust. Install the extracted package through **Apps → Install an app → Local
folder**.

## Next steps

- Read the [alpha release notice]({{ internal_link_prefix }}{% link alpha-release.md %}).
- Learn the screens in [Using Kestral]({{ internal_link_prefix }}{% link user-guide.md %}).
- Review [Managing apps]({{ internal_link_prefix }}{% link managing-apps.md %}) before installing a
  package.
- Review [Alpha limitations]({{ internal_link_prefix }}{% link honest-gaps.md %}) and
  [Versioning and recovery]({{ internal_link_prefix }}{% link versioning.md %}).
