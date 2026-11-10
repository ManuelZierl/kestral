---
title: Secure LAN access
layout: default
parent: Operations
nav_order: 2
---

# Secure access from another device on your LAN

This is a **single-owner** deployment for an Ubuntu `host-server` and trusted
browsers on your home or office LAN. It removes the everyday SSH tunnel, **not**
Kestral's owner-level security boundary. Anyone who obtains an authenticated
browser session can administer the entire host, including secrets, grants and
installed native backends. Do not expose the owner console to the Internet.

The recommended topology is:

~~~text
Trusted browser -- HTTPS (TCP 443) --> LAN-bound reverse proxy
                                        |-- /api/* --> 127.0.0.1:4310 (host-server)
                                        '-- /*     --> static browser client
~~~

The backend must remain on **loopback**. Only the reverse proxy listens on the
LAN address. The proxy must serve the browser and API from the **same HTTPS
origin**, including custom app documents and the event stream.

## 1. Pick the name and certificate strategy first

A hostname alone does not give you browser-trusted HTTPS. Different devices may
resolve names differently, and WebAuthn passkeys need a secure browser origin.

| Approach | When to use it | Tradeoff |
|---|---|---|
| **Local DNS + local CA** (worked example below) | No domain or cloud DNS required | Each client must explicitly trust the local CA; CA key must be protected |
| **Owned domain + publicly trusted certificate via DNS-01** | You own a domain and want normal browser trust on every device | Requires DNS API / certificate automation and internal DNS; keep web traffic private |
| **Private overlay (e.g. WireGuard / Tailscale)** | Devices need access beyond one LAN or router DNS is unsuitable | Adds a VPN dependency; you still need an HTTPS origin trusted by the browser for WebAuthn |

Use a **stable hostname**, such as `kestral.home.arpa` on a private network.
`.home.arpa` is designated for home network naming; it is **not** globally
resolvable by default. Configure a local DNS record or per-device hosts-file
entry pointing to the Ubuntu server's reserved LAN address. Check resolution
**on each client**, not just on Ubuntu. Bare hostnames like `homeserver`,
`.local`/mDNS names and IP literals are not portable assumptions.

For a publicly trusted certificate without publishing your site, use a domain
you control with **ACME DNS-01**, and resolve that domain to a private address
only for your authorized clients. Caddy's DNS-01 provider typically needs an
additional module and narrowly scoped DNS credentials; this is **not** part of
the stock Caddyfile below. An HTTP-01 challenge or a public A record is not
required for DNS-01. Don't open or forward HTTP/HTTPS from the WAN to obtain
the certificate.

A VPN encrypts traffic but does **not** make `http://192.168.x.x` a secure
WebAuthn origin. A self-signed leaf certificate is also not automatically
trusted by browsers. Neither is a substitute for a trusted HTTPS endpoint.

## 2. Set up the local address and firewall

This example uses:

- Ubuntu server's reserved LAN address: `192.168.1.20`
- Browser URL: `https://kestral.home.arpa` (HTTPS default port 443)
- Backend: `127.0.0.1:4310`
- Static client files: `/srv/kestral/browser-client`

**Replace the example address and hostname consistently.** Reserve the server
address in DHCP or configure a stable address. Create the DNS record in your
router/local DNS (or a hosts entry on each client). Test it from the *other*
device:

~~~sh
nslookup kestral.home.arpa
~~~

It must resolve to your intended **private** address. If it does not, fix DNS
before doing anything involving passkeys. Name resolution on the server says
nothing about the browser device.

Allow inbound TCP **443 only from the intended LAN/VLAN or VPN clients**.
Deny WAN port forwarding, UPnP exposure, and untrusted guest-network access.
Leave port 4310 closed from the network. If using UFW, an example restricted
rule is:

~~~sh
sudo ufw allow from 192.168.1.0/24 to 192.168.1.20 port 443 proto tcp
~~~

Adapt this to your real subnet and firewall (and check IPv6 rules). Do not
enable or reset a firewall over SSH without preserving SSH access. A home LAN,
guest Wi-Fi, and VPN peers are not inherently trusted.

## 3. Run the backend on loopback

Extract the **server** release archive using the
[split deployment instructions](deployment-modes.html#run-an-ubuntu-backend).
Start `host-server` as an unprivileged account, with a persistent data directory:

~~~sh
export HOST_REMOTE_BIND="127.0.0.1:4310"
export HOST_REMOTE_ORIGIN="https://kestral.home.arpa"
export HOST_RESOURCE_DIR="$PWD"
export KESTRAL_DATA_DIR="$HOME/.local/share/kestral-alpha"
./host-server
~~~

The exact browser origin (scheme, hostname, **and port**) must be fixed before
the first passkey registration. Do **not** set
`HOST_REMOTE_ALLOW_INSECURE_HTTP` or bind the backend to `0.0.0.0`.
Retain the same `KESTRAL_DATA_DIR` for later pairing/recovery commands.

## 4. Serve the same origin with HTTPS

Install Caddy through your operating system's supported installation path.
Extract the released **browser client** into
`/srv/kestral/browser-client`; the Caddy service account needs read access
to these files. For a standalone Kestral Caddy instance, an example Caddyfile
is:

~~~caddyfile
{
    # Avoid requiring LAN port 80 solely for HTTP -> HTTPS redirects.
    auto_https disable_redirects
}

https://kestral.home.arpa {
    # Use your Ubuntu server's actual LAN address, not 0.0.0.0.
    bind 192.168.1.20
    tls internal

    header {
        X-Content-Type-Options "nosniff"
        Referrer-Policy "no-referrer"
        Permissions-Policy "camera=(), microphone=(), geolocation=(), payment=()"
    }

    handle /api/* {
        reverse_proxy 127.0.0.1:4310
    }

    handle {
        header {
            Content-Security-Policy "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; frame-src 'self'; form-action 'none'; img-src 'self' data:; font-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'"
            X-Frame-Options "DENY"
        }
        root * /srv/kestral/browser-client
        try_files {path} /index.html
        file_server
    }
}
~~~

Validate the configuration and reload the Caddy service according to how you
installed it (for a packaged systemd service, typically
`sudo caddy validate --config /etc/caddy/Caddyfile` then
`sudo systemctl reload caddy`). If this Caddy instance already has a global
options block, merge the `auto_https` option into it.

**Do not** add the static-client `Content-Security-Policy` or
`X-Frame-Options: DENY` to `/api/*`: Kestral custom app frames require
their own host-generated policy. The `/api/events/stream` endpoint also needs
unbuffered streaming. Do not enable proxy response buffering.

The `tls internal` option issues certificates from Caddy's local CA. **Every
browser device must trust that CA**, not merely dismiss a certificate warning.
For an official Ubuntu systemd service, Caddy commonly stores its public root
certificate at:

~~~text
/var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt
~~~

The path depends on the Caddy service's data directory. Copy **only the
public root certificate** to each client through a trusted channel (for
example, SCP over SSH). Verify its SHA-256 fingerprint on the server and
client before explicitly adding it to the client's trusted root store.
Never export or install the CA private key. Some browsers (including some
Firefox configurations) have a separate trust store; mobile devices have their
own certificate-installation and explicit-trust steps. System or managed-device
policy may prevent CA installation.

Trusting a CA gives it certificate-signing authority, not just access to one
Kestral page: protect Caddy's CA key, back it up securely if appropriate, and
remove the client trust entry if the CA key is compromised. With local-CA
deployment, avoid enabling long-lived HSTS until certificate trust and recovery
are working on every device.

## 5. Pair and verify on a second device

From SSH on Ubuntu, under the **same account and data directory**, run:

~~~sh
export KESTRAL_DATA_DIR="$HOME/.local/share/kestral-alpha"
./host-server owner pair
~~~

The pairing code expires in ten minutes and is consumed when registration
starts. SSH is needed for administrative pairing/recovery, **not** for ordinary
browser access afterward.

From a **different device on the intended LAN**:

1. Check `nslookup kestral.home.arpa` returns the server's private address.
2. Open `https://kestral.home.arpa` in a current browser. It must load with
   no TLS warning. A certificate-warning bypass is **not** sufficient.
3. On the login screen, leave **Host URL** set to that same HTTPS origin and
   click **Check connection**. It should report the Kestral API as reachable.
   You can also request `https://kestral.home.arpa/api/auth/status` directly:
   expect JSON containing boolean `paired` and `authenticated` fields.
4. Enter the SSH-generated code, complete passkey registration and open Kestral.
   Sign out, sign back in, and verify the previous session is rejected.
5. Confirm Apps, Settings, Artifacts, an installed custom app action and its
   result/progress all work from this browser.
6. Restart the backend. Existing passkeys should still work, but old sessions
   must not. Verify port `4310` is not remotely reachable and that neither
   the router nor any public DNS/router rule exposes TCP 443 to the WAN.

Record the OS/browser, hostname, DNS result, certificate issuer and trust
method, LAN/firewall topology, exact release artifact hashes, pairing result,
one authenticated app action, sign-out/revocation, and any failures.
**This is a manual acceptance procedure, not a claim that cross-device
qualification has already been completed.** See issue #15 for the distinct
cross-machine release qualification.

## Troubleshooting and recovery

| Symptom | Check |
|---|---|
| Name doesn't resolve on another device | DHCP reservation, local DNS/hosts entry, VPN DNS, guest isolation and IPv6 |
| HTTPS certificate warning | Caddy is serving the expected hostname, root CA verified and installed in that browser's trust store |
| **Check connection** reports network failure | DNS, certificate trust, allowed firewall source, reverse proxy and backend; a browser does not disclose which of these failed |
| **Check connection** says no Kestral API | `/api/*` reverse proxy must reach `127.0.0.1:4310`; do not point Host URL to a static-only server |
| Backend rejects browser origin | `HOST_REMOTE_ORIGIN` must exactly equal the page's scheme, host and port; no trailing path |
| Passkey UI doesn't appear | Trusted HTTPS in a supported browser; no private-IP HTTP or certificate-warning bypass |
| Login works but custom app frames fail | Do not attach shell CSP / X-Frame-Options to `/api/surfaces/*` |
| Live progress/events stop | Ensure reverse proxy does not buffer `text/event-stream` |

**Changing hostname, scheme, port or relying-party ID is a passkey migration, not
just a DNS edit.** Kestral refuses to load an existing owner auth document with
a different configured origin/RP ID. Preserve a backup of the profile, stop the
backend, then run `./host-server owner reset --confirm` using the **same
`KESTRAL_DATA_DIR`**. This revokes *all* owner passkeys (not app data).
Start with the new stable `HOST_REMOTE_ORIGIN`, generate a fresh pairing code,
and re-pair each device. Do not delete the entire profile to fix auth.

This guide deliberately does not automate router changes, firewall rules, CA
trust, or public DNS. Those operations require the owner to inspect and approve
the actual network authority being created.
