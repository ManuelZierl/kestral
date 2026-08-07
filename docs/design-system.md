---
title: Frontend design system
layout: default
parent: Contributing
nav_order: 2
---

# Frontend design system
{: .no_toc }

1. TOC
{:toc}

## Color and themes

`host/src/lib/design/colors.ts` is the single typed source of truth for frontend
colors. Components use semantic CSS variables such as `var(--color-text)` and
must not contain color literals. Add a `ThemeColors` field and define it for
every registered theme when no semantic token fits.

Users select System, Light, Dark, or a device-local custom color profile under
**Settings → Appearance**. System follows the device and is the default. A
custom profile starts from an immutable built-in theme and stores a complete,
validated `ThemeColors` snapshot in browser storage. Its grouped editor exposes
every semantic token, including the app sidebar and trusted chrome. The built-in
Light and Dark profiles keep trusted chrome amber-on-dark as a stable
trust-boundary signature, while the app sidebar follows the active theme with
distinct light and dark palettes.

Custom profiles can be exported and imported as validated JSON under the same
Appearance section. The portable document includes the complete host palette
and namespaced overrides for app colors, but not the device-local profile ID.

Sandboxed app surfaces receive the resolved workspace palette as the same
`--color-*` variables, excluding protected `--color-chrome-*` tokens. An app
should use those semantic tokens instead of
copying Light/Dark literals. If its domain genuinely needs a color with no host
semantic equivalent, the package can declare `theme_colors`; Kestral exposes
each one only inside that app as `--app-color-<name>` and adds it to the custom
profile editor under the installed app's name. App tokens cannot replace host
tokens or trusted-chrome colors. Stable user content such as a drawing's chosen
stroke is data, not app chrome, and should not change with the host theme.

Only the owner can customize trusted-chrome colors, through the host-owned
Appearance screen. An app frame never receives protected chrome tokens and
cannot render, replace, or restyle the trusted approval surface.

## Responsive layout

The shell must reflow to the WCAG 1.4.10 floor of 320 CSS pixels without page
horizontal scrolling. Prefer intrinsic Grid/Flexbox, wrapping, and fluid sizes
before media queries. Use `rem` for sizes, `em` for breakpoints, and pixels only
for hairline borders.

Wide tables own their horizontal overflow. Full-height regions use `100dvh`
with a `100vh` fallback. Fixed dialogs are capped to the viewport. Keep controls
at least 24 by 24 CSS pixels and preserve labels in the accessibility tree when
the sidebar collapses.

The host-owned sidebar editor uses stable destination IDs, native visibility
checkboxes, and explicit move-up and move-down controls rather than pointer-only
dragging. Its trigger is outside the editable destination list, so hiding every
destination cannot remove the recovery path. At the reflow floor, destinations
continue to scroll in the horizontal strip while the editor remains fixed and
reachable.

## App surfaces

Custom surfaces do not inherit private host components. They run in sandboxed
frames and should provide their own accessible, responsive markup while using
only the public surface bridge and injected color variables. Test each surface with keyboard navigation,
reduced motion, light and dark host themes, 200% and 400% zoom, and a roughly
360-pixel-wide window.

The shell owns one animated Kestral loading mark. It remains visible through
custom-surface bundle lookup, surface opening, host-context loading, and iframe
handshake; reduced-motion mode keeps a static mark. Inline extension slots use
the same mark at compact size. Settings navigation group labels use text plus a
divider or group separator, not color alone, so headings remain distinct from
links in Light, Dark, zoomed, and narrow layouts.
