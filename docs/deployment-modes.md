---
title: Deployment modes
layout: default
parent: Operations
nav_order: 1
---

# Deployment modes
{: .no_toc }

1. TOC
{:toc}

Kestral supports two deployment shapes. Both use the same kernel, apps, grants,
Runs, artifacts, and action path.

## Desktop mode

```text
Tauri shell + local Kestral runtime
```

The native desktop application keeps the Svelte UI, host, data, app backends,
provider worker, MCP connections, and filesystem access on one Windows or Linux
machine. The UI calls the in-process host through Tauri IPC. Desktop mode opens
no host HTTP API.

## Split mode

```text
Ubuntu Kestral backend + browser-based trusted owner client
```

The backend-only `host-server` owns the kernel, apps, data, filesystem access,
model-provider and agent workers, MCP connections, grants, Runs, artifacts, and
secrets. The separately served static Svelte frontend is presentation and
trusted approval UI. It cannot access files on the browser's machine; File
Broker resources and app filesystem work refer to the Ubuntu backend.

Split mode is intended for **one owner on multiple devices**, not multiple
users or tenant isolation. A browser connected this way is a paired trusted
owner console. Its approval UI participates in host trusted chrome and must be
controlled by the same owner as the backend.

{: .warning }
An authenticated browser session authorizes the complete remote owner command
surface, including app installation, grants, publisher trust, profile deletion,
secret writes, approvals, and gateway administration. The passkey login is an
owner/root login, not app or integration authority. Expose selected capabilities
through an MCP export profile instead.

## Run an Ubuntu backend

The release server archive contains `host-server`, `LICENSE`,
`THIRD-PARTY-NOTICES.txt`, and the complete `provider-worker/` resource tree
with its pinned Node runtime. Extract it under an unprivileged account. The
server binary is not fully static: Ubuntu must provide its normal glibc,
OpenSSL, and D-Bus runtime libraries. Linux provider credentials additionally
require an available, unlocked Secret Service. GTK/WebKit development packages
are required to build the shared host crate from source, but the backend-only
release binary does not link them at runtime. CI checks that no dynamic library
is missing on the Ubuntu 22.04 build runner. The archive includes this page as
`DEPLOYMENT.md`.

```sh
tar -xzf kestral-0.1.0-alpha.1-linux-x86_64-server.tar.gz
cd Kestral-0.1.0-alpha.1-linux-x86_64-server
export HOST_REMOTE_BIND="127.0.0.1:4310"
export HOST_REMOTE_ORIGIN="https://kestral.example"
export HOST_RESOURCE_DIR="$PWD"
export KESTRAL_DATA_DIR="$HOME/.local/share/kestral-alpha"
./host-server
```

`HOST_RESOURCE_DIR` lets the backend locate the packaged Node runtime and
provider worker. When `KESTRAL_WORKER_RESOURCE_DIR` is unset, the backend uses
the same resource root for provider and external agent workers.
`HOST_REMOTE_ORIGIN` is both the exact allowed
browser origin and the WebAuthn origin, so changing its hostname after pairing
invalidates the configured passkey boundary.

In an SSH session under the same account and with the same profile/data-dir
selection, create a one-time pairing code:

```sh
./host-server owner pair
```

The code is valid for ten minutes, stored on disk only as a SHA-256 digest, and
consumed when one registration ceremony starts. Run the command again to pair
another browser or recover with a new passkey. It can run while `host-server` is
serving.

If a passkey is lost or compromised, stop `host-server` and revoke every owner
passkey from an SSH session before pairing again:

```sh
./host-server owner reset --confirm
```

The command refuses to run while the selected profile is locked by a backend.
It deliberately resets all passkeys rather than pretending to identify a lost
authenticator from its friendly browser label.

Source checkouts can run the same backend with:

```sh
export HOST_REMOTE_ORIGIN="http://localhost:1420"
export KESTRAL_DATA_DIR="$PWD/.host-data"
cargo run -p host --bin host-server
```

Create a source-build pairing code with the same environment in another shell:

```sh
cargo run -p host --bin host-server -- owner pair
```

## Serve the browser client

Extract `kestral-browser-client-0.1.0-alpha.1.zip` to a static web root. The
recommended deployment serves the client and proxies `/api/` from one HTTPS
origin. For example, an equivalent Caddy configuration is:

```text
kestral.example {
  header {
    Content-Security-Policy "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'none'; img-src 'self' data:; font-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'"
    Strict-Transport-Security "max-age=31536000"
    X-Content-Type-Options "nosniff"
    Referrer-Policy "no-referrer"
    Permissions-Policy "camera=(), microphone=(), geolocation=(), payment=()"
    X-Frame-Options "DENY"
  }
  handle /api/* {
    reverse_proxy 127.0.0.1:4310
  }
  handle {
    root * /home/kestral/browser-client
    try_files {path} /index.html
    file_server
  }
}
```

Open `https://kestral.example` and use that same URL as **Host URL**. Pair the
first browser with the SSH-generated code, then use **Sign in with passkey** on
later visits. `HOST_REMOTE_ORIGIN` must exactly match the browser origin,
including scheme and port. The Host URL is kept in `sessionStorage`; no owner
credential is readable by frontend JavaScript. Authentication uses an opaque
`HttpOnly`, `SameSite=Strict` cookie, with `Secure` required for HTTPS. Sessions
expire after 30 minutes idle or 12 hours absolute, are revoked by **Sign out**,
and do not survive a backend restart.

The example policy deliberately permits API connections only to the same
origin. If the browser client and backend use different origins, add only the
exact HTTPS backend origin to `connect-src` and test Custom Surfaces before
deployment. Do not replace it with a wildcard. HSTS is appropriate only when
the hostname is served exclusively over HTTPS.

## Single-tunnel split development

Development uses one browser-facing origin while retaining Vite hot-module
reload. One supervised command builds and starts `host-server` on Ubuntu
loopback port `4310`, serves the Svelte frontend and HMR on loopback port
`1420`, and proxies `/api` internally. The browser never connects to `4310`,
and the backend remains the sole owner of files, configuration, secrets, apps,
workers, MCP connections, grants, and Runs.

Install frontend dependencies once, then start both development processes:

```sh
cd host
npm install
npm run dev:split
```

The launcher uses `$HOME/.local/share/kestral-split-dev` as its data directory
and stops both processes when either fails or when you press `Ctrl+C`. On the
first launch, request a pairing code as part of startup:

```sh
npm run dev:split -- --pair
```

Pre-publication builds read only their current development-data shape. To start
with a completely empty split-development profile, stop the running launcher
and use:

```sh
npm run dev:split -- --clean
```

`--clean` irreversibly deletes the selected data directory, including apps,
configuration, secrets, chats, artifacts, Runs, and owner credentials. It also
clears Kestral's browser-local theme, custom color profiles, sidebar layout,
active-thread selection, and pending-send recovery state for that development
origin. It then prints a new owner pairing code before startup. This option
exists only on the split-development launcher; it is not available on production host binaries.
When combined with `--data-dir`, only that resolved custom directory is
deleted. The launcher refuses filesystem roots, the home or repository root,
and linked directory targets.

From the client machine, forward only the Vite/gateway port:

```sh
ssh -N -L 1420:127.0.0.1:1420 user@ubuntu-host
```

Open `http://localhost:1420`. The **Host URL** defaults to that same origin;
do not enter port `4310`. WebAuthn requires the `localhost` hostname rather
than its numeric `127.0.0.1` address. If Zed initially opens the forwarded port
as `http://127.0.0.1:1420`, the development gateway redirects the browser to
the canonical `localhost` origin before loading Kestral. A remote Zed project
can provide the tunnel without a separate SSH command:

```json
"port_forwards": [
  {
    "local_port": 1420,
    "remote_port": 1420
  }
]
```

The Vite HMR WebSocket also uses forwarded port `1420`, so frontend edits reload
without a second tunnel. If the browser runs directly on the Ubuntu machine,
omit SSH and open the same URL.

Use launcher options only when the defaults conflict with another project:

```sh
npm run dev:split -- --frontend-port 8000 --backend-port 4311
npm run dev:split -- --data-dir "$HOME/.local/share/another-kestral-dev"
npm run dev:split -- --data-dir "$HOME/.local/share/another-kestral-dev" --clean
npm run dev:split -- --help
```

The frontend port must match the forwarded local and remote port. The origin
must be the exact plain-HTTP loopback origin opened by the browser. Equivalent
persistent overrides are available through `KESTRAL_DEV_FRONTEND_PORT`,
`KESTRAL_DEV_BACKEND_PORT`, `HOST_REMOTE_ORIGIN`, and `KESTRAL_DATA_DIR`.

Native and split development call the same host command functions and enforce
the same kernel action path. Their adapters differ: desktop uses Tauri IPC and
channels; split mode uses authenticated HTTP commands plus one credentialed
Server-Sent Events (SSE) stream over the same origin and tunnel. The stream
carries the bounded event replay feed; it does not open another port.
Command-list parity is enforced by a Rust test. Browser-host OAuth links open in
the browser, custom-surface progress is bridged by request ID, package paths and
File Broker paths refer to the Ubuntu host, and browser-local file dialogs are
intentionally not treated as host filesystem access.

For ChatGPT Codex sign-in in split mode, choose **Device code login**. Browser
login starts a loopback callback listener beside the provider worker on the
Ubuntu backend, so a callback to the client device's `localhost` cannot reach
it. Device-code login avoids that callback topology.

`npm run build` creates the static `host/build` directory for deployment.
`VITE_HOST_API_URL` can provide a build-time default Host URL when the static
client and API deliberately use separate origins. It never contains an owner
credential.

## Minimal Windows-to-Ubuntu release test

An SSH tunnel is the smallest safe test before configuring DNS and HTTPS. It
keeps both plain-HTTP listeners on Ubuntu loopback and carries their traffic
inside SSH. No `HOST_REMOTE_ALLOW_INSECURE_HTTP` exception is needed.

On Ubuntu, extract both release archives. Start the backend from the extracted
server directory:

```sh
export HOST_REMOTE_BIND="127.0.0.1:4310"
export HOST_REMOTE_ORIGIN="http://localhost:1420"
export HOST_RESOURCE_DIR="$PWD"
export KESTRAL_DATA_DIR="$HOME/.local/share/kestral-alpha-test"
./host-server
```

In a second Ubuntu shell, serve the extracted browser-client directory:

```sh
python3 -m http.server 1420 --bind 127.0.0.1 --directory /path/to/browser-client
```

In Windows PowerShell, keep this tunnel running and replace the SSH destination
with the same account and host used by the remote project:

```powershell
ssh.exe -N -L 1420:127.0.0.1:1420 -L 4310:127.0.0.1:4310 user@ubuntu-host
```

In another Ubuntu SSH shell with the same `KESTRAL_DATA_DIR`, run
`./host-server owner pair`. Open `http://localhost:1420` on Windows, enter
`http://localhost:4310` as **Host URL**, and pair with that code. Use `localhost`
for both browser and Host URL; the configured browser origin must match exactly.
Loopback HTTP is accepted for local WebAuthn testing, but public deployments
require HTTPS.

The minimum acceptance result is:

1. The connection screen completes passkey pairing and the Kestral shell loads.
2. Apps, Settings, Artifacts, and System views load without remote transport errors.
3. **Sign out** revokes the server session and returns to the connection screen.
4. Reconnecting and restarting `host-server` preserves the test profile.
5. Install a compatible test app package from its own checkout or release,
   approve its grants, and invoke one action from its custom surface. The final
   result and request-correlated progress must reach the browser.

From a source checkout, the compiled backend's authentication, exact-origin,
preflight, health, and command dispatch can also be checked without a browser:

```sh
cargo build -p host --release --bin host-server
node scripts/smoke-host-server.mjs target/release/host-server
```

## Environment

| Variable | Purpose |
|---|---|
| `HOST_REMOTE_BIND` | Backend bind address; defaults to `127.0.0.1:4310`. |
| `HOST_REMOTE_ORIGIN` | Required exact browser/WebAuthn origin. Public origins must use HTTPS; mismatches receive HTTP 403. |
| `HOST_REMOTE_RP_ID` | Optional WebAuthn relying-party ID. Defaults to the origin host and becomes immutable once a passkey is registered. |
| `HOST_REMOTE_ALLOW_INSECURE_HTTP` | Explicitly permits non-loopback plain HTTP. Do not use it for an Internet-facing deployment. |
| `KESTRAL_DATA_DIR` | Selects the backend's server-side Kestral data root. |
| `HOST_RESOURCE_DIR` | Packaged root containing `provider-worker/`; also seeds the shared worker resource root. |
| `KESTRAL_WORKER_RESOURCE_DIR` | Optional explicit resource root for the provider runtime and external agent workers. |
| `KESTRAL_PROVIDER_NODE` | Optional explicit Node executable for the bundled provider worker; must be set with `KESTRAL_PROVIDER_WORKER`. |
| `KESTRAL_PROVIDER_WORKER` | Optional explicit bundled provider worker script; must be set with `KESTRAL_PROVIDER_NODE`. |
| `KESTRAL_AGENT_NODE` | Optional explicit Node executable for external `agent-worker` packages. |
| `KESTRAL_DEV_FRONTEND_PORT` | Optional browser gateway and HMR port for `npm run dev:split`; defaults to `1420`. |
| `KESTRAL_DEV_BACKEND_PORT` | Optional host-local backend port for `npm run dev:split`; defaults to `4310`. |
| `KESTRAL_HOST_API_PROXY_TARGET` | Development-only Vite `/api` proxy target; defaults to `http://127.0.0.1:4310`. |
| `VITE_HOST_API_URL` | Optional browser-client build-time default Host URL; never contains an owner credential. |

## Network security

The backend refuses direct non-loopback plain HTTP unless
`HOST_REMOTE_ALLOW_INSECURE_HTTP=true`. Passkey authentication proves owner
identity but does not encrypt prompts, model traffic, or secret-entry requests.
For every non-loopback deployment, keep `host-server` on loopback and use an HTTPS reverse proxy, VPN,
encrypted tunnel, or equivalent secure transport. WebAuthn rejects public HTTP
origins even if the backend's unsafe HTTP bind exception is enabled. Do not add
a plain-HTTP exception merely to simplify deployment.

Passkey public credentials are stored atomically in
`remote-owner-auth-v1.json` under the selected profile. WebAuthn registration
and authentication challenges remain server-side and expire after five
minutes. The browser receives only public challenge options and its opaque
session cookie; provider credentials and Kestral secrets never cross this auth
boundary.

Remote events are a bounded feed. A client detects an evicted, selectively
dropped, or regressed sequence and refreshes authoritative state. Pending
approvals and missed events are recovered when SSE connects or reconnects.
State-changing commands emit scoped refresh notifications; a visible browser
also performs an infrequent authoritative reconciliation as a safety net. The
Ubuntu backend remains the durable authority even when the browser disconnects.

The SSE stream uses the same HttpOnly owner-session cookie as command requests.
The backend revalidates that session every 15 seconds and closes the stream when
the session is revoked or expires. Reverse proxies must pass `text/event-stream`
responses without buffering; Kestral also sends heartbeats and
`X-Accel-Buffering: no`.

Remote custom-surface actions return final results and request-correlated
transient progress. The replay feed is bounded: sustained pressure can evict
transient records, in which case the browser reports a sequence gap and
refreshes authoritative state. Transient progress itself cannot be reconstructed.
