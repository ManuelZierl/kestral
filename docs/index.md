---
title: Home
layout: default
nav_order: 1
---

{% assign internal_link_prefix = "" %}{% assign jekyll_major = jekyll.version | split: "." | first %}{% if jekyll_major == "3" %}{% assign internal_link_prefix = site.baseurl %}{% endif %}

# Kestral documentation

Kestral is a personal-first, open-source AI workspace and lean local host for
user-chosen apps. Chat is the default starting app, not the canonical interface
for all AI work. The workspace becomes more capable as you install or build
focused apps for work better served by a document, canvas, form, dashboard, or
other dedicated interface.

Version `0.1.0-alpha.1` is the planned first public testing release and has not
been published. The candidate supports a native Windows or Linux desktop and a
split deployment with an Ubuntu backend and a static browser-based trusted owner
console. It is intended for technical testers and contributors, not production
use. Read the
[release notice]({{ internal_link_prefix }}{% link alpha-release.md %}) before installing it.

{: .important }
Installing an app is not blanket trust. Read the package inspection and each
permission request. Native app backends run as the backend OS user in the 0.1
series and are not isolated from that account's files or network by the kernel.

## Start here

| Page | What it covers |
|------|----------------|
| [Getting started]({{ internal_link_prefix }}{% link getting-started.md %}) | Build or install Kestral, configure a model, start with Chat, and add a focused app |
| [Alpha release notice]({{ internal_link_prefix }}{% link alpha-release.md %}) | Understand the audience, artifacts, unsigned status, and testing risks |
| [Using Kestral]({{ internal_link_prefix }}{% link user-guide.md %}) | Work across Chat, focused apps, artifacts, settings, and activity history |
| [Managing apps]({{ internal_link_prefix }}{% link managing-apps.md %}) | Inspect, install, update, disable, and remove apps |
| [Curated apps]({{ internal_link_prefix }}{% link curated-apps.md %}) | Discover reviewed independent apps and propose a listing |
| [Tool servers]({{ internal_link_prefix }}{% link tool-servers.md %}) | Connect local or remote MCP servers explicitly |
| [Permissions and files]({{ internal_link_prefix }}{% link permissions-and-files.md %}) | Understand grants and share only selected files or folders |
| [Extending Kestral]({{ internal_link_prefix }}{% link extending-kestral.md %}) | Build app packages or expose capabilities over MCP |
| [Architecture]({{ internal_link_prefix }}{% link architecture.md %}) | Understand the kernel, adapters, host, and action path |
| [Operations]({{ internal_link_prefix }}{% link operations.md %}) | Deployment, state, versioning, and release limitations |
| [Roadmap]({{ internal_link_prefix }}{% link roadmap.md %}) | Review explicitly non-shipped product direction and constraints |

## Product boundary

Kestral's product bet is that Chat is a useful entrance to AI, but often not the
best interface for repeated, structured, visual, or stateful work. Conversational
apps, notes, development tools, canvases, automations, and new interaction
patterns should coexist as apps in one personal workspace. The owner chooses the
models and apps instead of adopting one suite's complete product assumptions.

The host owns only shared mechanisms: app identity and lifecycle, sandboxed
surfaces, trusted chrome, provider and credential mediation, grants, capability
Runs, artifacts and provenance, and cross-app composition. Apps own the actual
product experiences. Default-installed Chat uses the same public primitives and
grants as an external app; its place on first launch is an onboarding choice, not
a privileged authority class.

The project is designed to stay lean in resource use and conceptual burden, but
that is a measured product goal rather than a consequence of using Tauri. The
first release must record a reproducible baseline for startup, footprint, idle
resources, workers, and time to first useful result.

Profile state stays on the machine running the backend. Explicit provider calls,
surface network access, remote MCP connections, and unsandboxed native backends
can send or access data outside that profile according to their own authority.
In split mode, the backend machine is normally an Ubuntu server rather than the
browser client.

Product priorities are personal usefulness first, an ecosystem-ready developer
platform second, and broader general-user accessibility third. Kestral is not
being built as a commercial product, although its MIT license permits commercial
use and forks.

## What Kestral is not

- It is not an LLM provider. Providers are configured through an ordinary
  userland LLM Provider app.
- It is not an agent engine. The optional `kestral-pi` engine is an installable,
  headless app.
- It is not a workflow engine. Every app capability action becomes a Run and
  crosses the same permission and provenance boundary.
- It is not a multi-user server in the 0.1 series. Remote mode is for one owner using
  multiple clients.
- It is not an IDE, a VS Code clone, a hosted cloud platform, a model runtime,
  or a Zapier/Shortcuts-style workflow builder. Those experiences can be apps
  without becoming privileged host features.
