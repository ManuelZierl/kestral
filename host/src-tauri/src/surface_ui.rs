//! Static custom app-surface UI bundles, served to sandboxed webviews.
//!
//! An app may ship a static UI bundle (HTML + inlined assets) for one of its
//! declared surfaces. The host renders that bundle inside a **sandboxed**
//! iframe/webview with an opaque origin, a per-app deny-by-default CSP, and
//! **no** Tauri API — the only channel out is the versioned surface message
//! bridge (see `host/src/lib/surfaces/`).
//!
//! This registry is the host-owned seam where bundles live. The installable
//! package format (`docs/writing-apps.md`, `surfaces[].ui`) registers third-party
//! bundles here at activation. Nothing about a bundle enters the kernel:
//! the kernel surface stays intent-only, and a bundle is
//! pure host presentation over the same grant-checked action path bundled
//! Svelte screens use.

use std::collections::BTreeMap;

use serde::Serialize;

use app_host_kernel::ids::{AppId, SurfaceName};

/// The bridge protocol version this host speaks. A bundle declares the
/// version it targets; the frontend refuses a bundle whose major it does not
/// understand. Keep in lockstep with `SURFACE_BRIDGE_VERSION` in
/// `host/src/lib/surfaces/surfaceBridgeProtocol.ts`.
pub const SURFACE_BRIDGE_VERSION: u32 = 3;

/// A static UI bundle for one app surface. Content only — the host decides
/// how to sandbox and render it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceUiBundle {
    /// Bridge protocol version the bundle's client code was written against.
    pub protocol_version: u32,
    /// The document loaded into the sandboxed frame (via `srcdoc`). Scripts
    /// run only under the frame's `allow-scripts allow-forms allow-downloads`
    /// sandbox with an opaque origin. The host CSP keeps `form-action 'none'`,
    /// so JavaScript form handlers work without allowing form navigation.
    /// Downloads remain browser-managed writes; frames cannot choose paths or
    /// reach the host origin, Tauri, cookies, or storage.
    pub html: String,
    /// The Content-Security-Policy applied to the frame. Deny-by-default:
    /// built by [`deny_by_default_csp`] so a bundle can never widen its own
    /// policy beyond what the host allows.
    pub csp: String,
}

impl SurfaceUiBundle {
    /// Build a bundle with a host-authored, deny-by-default CSP over the given
    /// allowed network hosts (empty = no network at all).
    pub fn new(html: impl Into<String>, connect_src: &[&str]) -> Self {
        Self {
            protocol_version: SURFACE_BRIDGE_VERSION,
            html: html.into(),
            csp: deny_by_default_csp(connect_src),
        }
    }
}

/// A deny-by-default CSP for a sandboxed app surface. Everything is `'none'`
/// unless a source is explicitly required to render self-contained UI:
/// inline script/style (the frame is already origin-isolated by the sandbox),
/// `data:` images/fonts, and only the explicitly allowlisted `connect-src`
/// hosts. Network is denied unless the app declares hosts.
pub fn deny_by_default_csp(connect_src: &[&str]) -> String {
    // Anything that is not a well-formed source expression is dropped rather
    // than spliced in: a single `;` inside one entry would close `connect-src`
    // and let the value append its own directives, including ones the host
    // sets *after* this point (`base-uri`, `form-action`). Packages are
    // refused loudly at the install boundary; this is the fail-closed backstop
    // for every other caller.
    let allowed: Vec<&str> = connect_src
        .iter()
        .copied()
        .filter(|source| is_valid_connect_src(source))
        .collect();
    let connect = if allowed.is_empty() {
        "'none'".to_string()
    } else {
        allowed.join(" ")
    };
    [
        "default-src 'none'",
        "script-src 'unsafe-inline'",
        "style-src 'unsafe-inline'",
        "img-src data:",
        "font-src data:",
        &format!("connect-src {connect}"),
        "base-uri 'none'",
        "form-action 'none'",
    ]
    .join("; ")
}

/// Whether one package-declared value is a CSP source expression that can be
/// spliced into a directive without changing the policy's shape.
///
/// This is deliberately a shape check, not a URL parse: the only thing that
/// makes a value dangerous here is its ability to terminate the directive
/// (`;`) or the whole policy (`,`), so those, quotes (which would forge a
/// keyword source such as `'unsafe-eval'`), and any whitespace are refused.
/// Everything the host itself needs — `https://example.com`, `https://a:8443`,
/// `wss://x.example`, `https:`, `*.example.com` — passes.
pub fn is_valid_connect_src(source: &str) -> bool {
    const MAX_SOURCE_LEN: usize = 255;
    !source.is_empty()
        && source.len() <= MAX_SOURCE_LEN
        // Graphic ASCII only: excludes every whitespace and control character,
        // so a value can neither break the line nor smuggle a token separator.
        && source
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !br#";,'"\"#.contains(&byte))
}

/// Host-owned map of `(app, surface) -> bundle`. Empty by default; product
/// apps use bundled Svelte screens or generic renderers. Custom bundles are
/// registered explicitly (developer demo now, package installer later).
#[derive(Debug, Default)]
pub struct SurfaceUiRegistry {
    bundles: BTreeMap<(AppId, SurfaceName), SurfaceUiBundle>,
}

impl SurfaceUiRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, app_id: AppId, surface: SurfaceName, bundle: SurfaceUiBundle) {
        self.bundles.insert((app_id, surface), bundle);
    }

    pub fn get(&self, app_id: &AppId, surface: &SurfaceName) -> Option<&SurfaceUiBundle> {
        self.bundles.get(&(app_id.clone(), surface.clone()))
    }

    /// Drop every bundle an app registered. Called on uninstall so a removed
    /// app leaves no UI behind.
    pub fn remove_app(&mut self, app_id: &AppId) {
        self.bundles.retain(|(owner, _), _| owner != app_id);
    }
}

#[cfg(test)]
mod tests;
