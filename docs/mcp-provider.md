---
title: Serving capabilities over MCP
layout: default
parent: Extending Kestral
nav_order: 3
---

# Serving selected capabilities over MCP
{: .no_toc }

1. TOC
{:toc}

## Model

Outbound access is disabled by default. An MCP export profile names exact
capabilities and materializes as a zero-capability `mcp-export/<profile-id>`
virtual app with ordinary exact grants. `tools/list` is derived from that
principal's currently live grants. Each `tools/call` creates an attributable
Run under that principal before using the normal kernel action path. Remote
clients never choose app IDs, grant IDs, or capability references.

## Create an export

1. Open **Settings → Advanced → MCP exports**.
2. Choose **Create export profile** and enter an ID and display name.
3. Select exact installed capabilities. Provider-wide export is not available.
4. Choose the call policy. Keep **Require local approval** unless unattended
   access is intentional.
5. Leave full results and artifact references disabled unless the client needs
   them; either may disclose local data.
6. Save the profile, generate a credential, and store the displayed bearer
   token immediately. It is shown once.
7. Enable the profile, then start the gateway.

The gateway serves Streamable HTTP at `/mcp`, bound to `127.0.0.1:8137` by
default. Enabled profiles with a credential become independent virtual
principals. Rotate or revoke a credential without changing other profiles.

## Security and audit

The gateway validates origins, limits request bodies, sessions, rates, and
invocation time, and writes `mcp-gateway-audit.jsonl`. An unavailable audit sink
prevents startup or tool execution. The UI shows recent in-memory activity for
the current session; the JSONL file is the durable audit source.

`requires-approval` exports prompt through local trusted chrome for every call.
MCP read/write annotations remain advisory; grants are authoritative. OAuth 2.1
protected-resource metadata is disabled in the 0.1 series because resource and audience
validation are not implemented. Bearer tokens authenticate an owner but do not
provide transport encryption.

## Remote transport

A Cloudflare Tunnel may forward a public hostname to the loopback listener, but
it is transport only, not authentication:

```yaml
ingress:
  - hostname: mcp.example.com
    service: http://127.0.0.1:8137
  - service: http_status:404
```

Use a TLS-protected private tunnel or reverse proxy, retain bearer
authentication, and restrict the exact client origin where applicable. Do not
expose the loopback gateway directly by changing its bind address.
