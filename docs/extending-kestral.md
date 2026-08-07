---
title: Extending Kestral
layout: default
nav_order: 4
has_children: true
---

# Extending Kestral

Kestral's developer platform exists to support the personal workspace: a user
should be able to turn a recurring need into a focused app without forking the
host or rebuilding model, credential, permission, and lifecycle infrastructure.
Standalone surfaces provide task-specific interaction, extension contributions
add contextual UI, and capabilities plus artifacts provide grant-mediated
composition.

Kestral supports two extension paths in the 0.1 series:

| Path | Best for | What the host derives or loads |
|---|---|---|
| [App package]({% link writing-apps.md %}) | A named, versioned product with declared permissions, optional custom UI, configuration, artifacts, and managed lifecycle | `app.json`, checksummed UI/backend payload, kernel manifest, handlers, and surfaces |
| [Bare MCP server]({% link tool-servers.md %}) | Making existing tools usable quickly, without a Kestral package | Generic app identity, form surfaces, result artifacts, and approval-required grants from MCP tool schemas |

Kestral can also act as an MCP provider. See
[Serving capabilities over MCP]({% link mcp-provider.md %}) when a remote MCP
client should call a narrow set of capabilities already installed in Kestral.

All paths preserve the same mediated boundary: an extension declares behavior,
but the host controls installation, grants, trusted chrome, capability
invocation, and provenance. Unsandboxed native code can still act directly with
the backend operating-system account's authority. MCP remains an adapter
protocol and does not enter the kernel's domain model.
