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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_host_kernel::ids::{AppId, SurfaceName};
use serde::Serialize;
use uuid::Uuid;

/// The bridge protocol version this host speaks. A bundle declares the
/// version it targets; the frontend refuses a bundle whose major it does not
/// understand. Keep in lockstep with `SURFACE_BRIDGE_VERSION` in
/// `host/src/lib/surfaces/surfaceBridgeProtocol.ts`.
pub const SURFACE_BRIDGE_VERSION: u32 = 3;
const SURFACE_CLIENT_SDK: &str = include_str!("../../surface-runtime/surface-client.js");
const SERVER_POLL: Duration = Duration::from_millis(100);

/// A static UI bundle for one app surface. Content only — the host decides
/// how to sandbox and render it.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceUiBundle {
    /// Bridge protocol version the bundle's client code was written against.
    pub protocol_version: u32,
    /// The document served from the isolated host surface protocol. Scripts run
    /// only under the frame's `allow-scripts allow-forms allow-downloads`
    /// sandbox with an opaque origin. The response CSP keeps `form-action 'none'`,
    /// so JavaScript form handlers work without allowing form navigation.
    /// Downloads remain browser-managed writes; frames cannot choose paths or
    /// reach the host origin, Tauri, cookies, or storage.
    pub html: String,
    /// The Content-Security-Policy applied to the frame. Deny-by-default:
    /// built by [`deny_by_default_csp`] so a bundle can never widen its own
    /// policy beyond what the host allows.
    pub csp: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceUiView {
    pub protocol_version: u32,
    pub document_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceUiRoute {
    pub protocol_version: u32,
    pub route_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceDocument {
    pub html: Vec<u8>,
    pub csp: String,
}

pub struct RunningSurfaceServer {
    base_url: String,
    shutdown: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl RunningSurfaceServer {
    pub fn start(registry: Arc<Mutex<SurfaceUiRegistry>>) -> Result<Self, String> {
        let server = tiny_http::Server::http("127.0.0.1:0")
            .map_err(|error| format!("surface document server failed to bind: {error}"))?;
        let local_addr = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| "surface document server received a non-IP address".to_string())?;
        let base_url = format!("http://{local_addr}");
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let worker = std::thread::spawn(move || {
            while !worker_shutdown.load(Ordering::Relaxed) {
                match server.recv_timeout(SERVER_POLL) {
                    Ok(Some(request)) => serve_surface_request(&registry, request),
                    Ok(None) => continue,
                    Err(error) => {
                        eprintln!("surface document server stopped after a receive error: {error}");
                        break;
                    }
                }
            }
        });
        Ok(Self {
            base_url,
            shutdown,
            worker: Some(worker),
        })
    }

    pub fn document_url(&self, route_token: &str) -> String {
        format!("{}/{}", self.base_url, route_token)
    }
}

impl Drop for RunningSurfaceServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Debug)]
struct RegisteredSurfaceUi {
    route_token: String,
    bundle: SurfaceUiBundle,
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
        "frame-src 'none'",
        "object-src 'none'",
        "base-uri 'none'",
        "form-action 'none'",
    ]
    .join("; ")
}

/// Whether one package-declared value is an exact HTTP(S) or WS(S) origin.
/// Wildcards and scheme-only sources are refused because a broad HTTP source
/// would also cover WebView2's `http://ipc.localhost` Tauri transport.
pub fn is_valid_connect_src(source: &str) -> bool {
    if !is_valid_connect_src_shape(source) {
        return false;
    }
    let Ok(url) = reqwest::Url::parse(source) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    !host.contains('*') && !host.eq_ignore_ascii_case("ipc.localhost")
}

fn is_valid_connect_src_shape(source: &str) -> bool {
    const MAX_SOURCE_LEN: usize = 255;
    let lower = source.to_ascii_lowercase();
    !source.is_empty()
        && source.len() <= MAX_SOURCE_LEN
        && !lower.starts_with("ipc:")
        && !lower.starts_with("http://ipc.localhost")
        && !lower.starts_with("https://ipc.localhost")
        // Graphic ASCII only: excludes every whitespace and control character,
        // so a value can neither break the line nor smuggle a token separator.
        && source
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !br#";,'"\"#.contains(&byte))
}

/// Host-owned map of `(app, surface) -> bundle`. Empty by default; product
/// apps use bundled Svelte screens or generic renderers. Custom bundles are
/// registered explicitly after package activation.
#[derive(Debug, Default)]
pub struct SurfaceUiRegistry {
    bundles: BTreeMap<(AppId, SurfaceName), RegisteredSurfaceUi>,
    routes: BTreeMap<String, (AppId, SurfaceName)>,
}

impl SurfaceUiRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, app_id: AppId, surface: SurfaceName, bundle: SurfaceUiBundle) {
        let key = (app_id, surface);
        if let Some(previous) = self.bundles.remove(&key) {
            self.routes.remove(&previous.route_token);
        }
        let route_token = Uuid::new_v4().simple().to_string();
        self.routes.insert(route_token.clone(), key.clone());
        self.bundles.insert(
            key,
            RegisteredSurfaceUi {
                route_token,
                bundle,
            },
        );
    }

    pub fn get(&self, app_id: &AppId, surface: &SurfaceName) -> Option<SurfaceUiRoute> {
        self.bundles
            .get(&(app_id.clone(), surface.clone()))
            .map(|registered| SurfaceUiRoute {
                protocol_version: registered.bundle.protocol_version,
                route_token: registered.route_token.clone(),
            })
    }

    pub fn document(&self, route_token: &str) -> Option<SurfaceDocument> {
        let key = self.routes.get(route_token)?;
        let registered = self.bundles.get(key)?;
        Some(SurfaceDocument {
            html: inject_client_sdk(&registered.bundle.html).into_bytes(),
            csp: registered.bundle.csp.clone(),
        })
    }

    /// Drop every bundle an app registered. Called on uninstall so a removed
    /// app leaves no UI behind.
    pub fn remove_app(&mut self, app_id: &AppId) {
        let removed = self
            .bundles
            .iter()
            .filter(|((owner, _), _)| owner == app_id)
            .map(|(_, registered)| registered.route_token.clone())
            .collect::<Vec<_>>();
        self.bundles.retain(|(owner, _), _| owner != app_id);
        for route_token in removed {
            self.routes.remove(&route_token);
        }
    }
}

pub fn surface_ui_view(route: SurfaceUiRoute, document_url: String) -> SurfaceUiView {
    SurfaceUiView {
        protocol_version: route.protocol_version,
        document_url,
    }
}

fn inject_client_sdk(html: &str) -> String {
    debug_assert!(!SURFACE_CLIENT_SDK.to_ascii_lowercase().contains("</script"));
    let script = format!("<script>{SURFACE_CLIENT_SDK}</script>");
    let lower = html.to_ascii_lowercase();
    if let Some(head) = lower.find("<head") {
        if let Some(close) = lower[head..].find('>') {
            let at = head + close + 1;
            return format!("{}\n{}{}", &html[..at], script, &html[at..]);
        }
    }
    format!("<!doctype html><html><head>{script}</head><body>{html}</body></html>")
}

fn response_header(name: &str, value: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes())
        .expect("host-authored surface response header is valid")
}

fn serve_surface_request(registry: &Mutex<SurfaceUiRegistry>, request: tiny_http::Request) {
    if request.method() != &tiny_http::Method::Get {
        let _ = request.respond(
            tiny_http::Response::from_string("method not allowed")
                .with_status_code(405)
                .with_header(response_header("Content-Type", "text/plain; charset=utf-8"))
                .with_header(response_header("Cache-Control", "no-store")),
        );
        return;
    }
    let route_token = request.url().strip_prefix('/').unwrap_or(request.url());
    let document = match registry.lock() {
        Ok(registry) => registry.document(route_token),
        Err(_) => {
            let _ = request.respond(
                tiny_http::Response::from_string("surface UI registry lock poisoned")
                    .with_status_code(500)
                    .with_header(response_header("Content-Type", "text/plain; charset=utf-8"))
                    .with_header(response_header("Cache-Control", "no-store")),
            );
            return;
        }
    };
    let Some(document) = document else {
        let _ = request.respond(
            tiny_http::Response::from_string("surface document not found")
                .with_status_code(404)
                .with_header(response_header("Content-Type", "text/plain; charset=utf-8"))
                .with_header(response_header("Cache-Control", "no-store")),
        );
        return;
    };
    let mut ancestors =
        "tauri://localhost http://tauri.localhost https://tauri.localhost".to_string();
    if cfg!(debug_assertions) {
        ancestors.push_str(" http://localhost:1420");
    }
    let csp = format!("{}; frame-ancestors {ancestors}", document.csp);
    let _ = request.respond(
        tiny_http::Response::from_data(document.html)
            .with_status_code(200)
            .with_header(response_header("Content-Type", "text/html; charset=utf-8"))
            .with_header(response_header("Cache-Control", "no-store"))
            .with_header(response_header("Content-Security-Policy", &csp))
            .with_header(response_header("Referrer-Policy", "no-referrer"))
            .with_header(response_header("X-Content-Type-Options", "nosniff")),
    );
}

#[cfg(test)]
mod tests;
