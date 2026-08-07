---
title: Operations
layout: default
nav_order: 6
has_children: true
---

{% assign internal_link_prefix = "" %}{% assign jekyll_major = jekyll.version | split: "." | first %}{% if jekyll_major == "3" %}{% assign internal_link_prefix = site.baseurl %}{% endif %}

# Operations

The planned Kestral `0.1.0-alpha.1` candidate supports native desktop mode and a
single-owner split deployment. This section covers both modes, the
persisted-state contract, and the limitations relevant to operating an early
testing build.

| Page | Purpose |
|---|---|
| [Deployment modes]({{ internal_link_prefix }}{% link deployment-modes.md %}) | Run the all-in-one desktop or the advanced backend/client split. |
| [Versioning and recovery]({{ internal_link_prefix }}{% link versioning.md %}) | Understand strict data formats, updates, backups, and failure behavior. |
| [Alpha limitations]({{ internal_link_prefix }}{% link honest-gaps.md %}) | Know what is intentionally outside the supported boundary. |

For profile selection and backup basics, see
[Profiles and data]({{ internal_link_prefix }}{% link profiles.md %}).
