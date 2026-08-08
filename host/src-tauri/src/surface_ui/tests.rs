use super::*;

/// A polished, self-contained showcase surface built only for this test
/// module. It exercises the same bundle shape a real MCP-bridged app would
/// register — read-only bridge ops, deny-by-default CSP, no network — to
/// prove a third-party/MCP-bridged app can reach the same UI quality as a
/// bundled Svelte screen without any privileged access.
fn demo_weather_showcase() -> SurfaceUiBundle {
    SurfaceUiBundle::new(DEMO_WEATHER_HTML, &[])
}

const DEMO_WEATHER_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<style>
  * { box-sizing: border-box; }
  body {
    margin: 0; padding: 1rem;
    font: 15px/1.5 system-ui, sans-serif;
    color: var(--color-text); background: var(--color-surface);
  }
  header { display: flex; align-items: baseline; justify-content: space-between; gap: .5rem; }
  h1 { font-size: 1.1rem; margin: 0; }
  .status { font-size: .8rem; opacity: .7; }
  .grid { margin-top: 1rem; display: grid; gap: .75rem;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 12rem), 1fr)); }
  .card { padding: .9rem 1rem; border-radius: 14px; border: 1px solid var(--color-border);
    background: var(--color-surface-raised); box-shadow: 0 8px 24px var(--color-shadow-soft); }
  .card h2 { margin: 0 0 .3rem; font-size: .95rem; }
  .temp { font-size: 1.6rem; font-weight: 700; }
  .muted { opacity: .65; font-size: .85rem; }
  .empty { margin-top: 2rem; text-align: center; opacity: .6; }
</style>
</head>
<body>
  <header>
    <h1>Weather results</h1>
    <span class="status" id="status">Connecting…</span>
  </header>
  <div class="grid" id="grid"></div>
  <p class="empty" id="empty" hidden>No forecasts yet. Run the forecast action to see cards here.</p>
<script>
  // The host injects window.appHost (the bridge SDK) before this runs.
  const grid = document.getElementById("grid");
  const status = document.getElementById("status");
  const empty = document.getElementById("empty");

  function render(artifacts) {
    grid.replaceChildren();
    if (!artifacts.length) { empty.hidden = false; return; }
    empty.hidden = true;
    for (const a of artifacts) {
      const r = (a.content && a.content.result) || {};
      const card = document.createElement("div");
      card.className = "card";
      const city = document.createElement("h2");
      city.textContent = r.city || a.title || "Forecast";
      const temp = document.createElement("div");
      temp.className = "temp";
      temp.textContent = (r.high_celsius != null) ? r.high_celsius + "°C" : "—";
      const desc = document.createElement("div");
      desc.className = "muted";
      desc.textContent = r.forecast || "";
      card.append(city, temp, desc);
      grid.append(card);
    }
  }

  async function refresh() {
    try {
      const artifacts = await window.appHost.listArtifacts();
      render(artifacts);
      status.textContent = artifacts.length + " forecast" + (artifacts.length === 1 ? "" : "s");
    } catch (err) {
      status.textContent = "Error";
      window.appHost.reportError(String(err));
    }
  }

  window.appHost.ready();
  window.appHost.onEvent(() => refresh());
  refresh();
</script>
</body>
</html>"##;

#[test]
fn deny_by_default_csp_denies_network_when_no_hosts() {
    let csp = deny_by_default_csp(&[]);
    assert!(csp.contains("default-src 'none'"));
    assert!(csp.contains("connect-src 'none'"));
    assert!(!csp.contains("frame-ancestors"));
    assert!(csp.contains("base-uri 'none'"));
    assert!(csp.contains("frame-src 'none'"));
    assert!(csp.contains("object-src 'none'"));
}

#[test]
fn csp_allowlists_declared_connect_hosts_only() {
    let csp = deny_by_default_csp(&["https://api.example.com"]);
    assert!(csp.contains("connect-src https://api.example.com"));
    assert!(!csp.contains("connect-src 'none'"));
    // default-src stays locked down regardless of connect allowlist.
    assert!(csp.contains("default-src 'none'"));
}

#[test]
fn registry_scopes_bundles_by_app_and_surface() {
    let mut registry = SurfaceUiRegistry::new();
    let app = AppId::new("mcp-weather");
    let surface = SurfaceName::new("result-cards");
    registry.register(app.clone(), surface.clone(), demo_weather_showcase());

    assert!(registry.get(&app, &surface).is_some());
    assert!(registry
        .get(&app, &SurfaceName::new("other-surface"))
        .is_none());
    assert!(registry.get(&AppId::new("notes"), &surface).is_none());
}

#[test]
fn registered_document_injects_the_host_sdk_before_app_scripts() {
    let mut registry = SurfaceUiRegistry::new();
    let app = AppId::new("mcp-weather");
    let surface = SurfaceName::new("result-cards");
    registry.register(app.clone(), surface.clone(), demo_weather_showcase());

    let route = registry.get(&app, &surface).unwrap();
    let document = registry.document(&route.route_token).unwrap();
    let html = String::from_utf8(document.html).unwrap();

    assert!(html.contains("app-host-surface-bridge"));
    assert!(html.contains("window.appHost"));
    assert!(html.find("window.appHost =").unwrap() < html.find("const grid").unwrap());
    assert!(document.csp.contains("script-src 'unsafe-inline'"));
}

#[test]
fn replacing_or_removing_a_surface_invalidates_its_route() {
    let mut registry = SurfaceUiRegistry::new();
    let app = AppId::new("mcp-weather");
    let surface = SurfaceName::new("result-cards");
    registry.register(app.clone(), surface.clone(), demo_weather_showcase());
    let first = registry.get(&app, &surface).unwrap().route_token;

    registry.register(app.clone(), surface.clone(), demo_weather_showcase());
    let second = registry.get(&app, &surface).unwrap().route_token;
    assert_ne!(first, second);
    assert!(registry.document(&first).is_none());
    assert!(registry.document(&second).is_some());

    registry.remove_app(&app);
    assert!(registry.document(&second).is_none());
}

#[test]
fn remove_app_drops_only_that_apps_bundles() {
    let mut registry = SurfaceUiRegistry::new();
    let weather = AppId::new("mcp-weather");
    let other = AppId::new("com.example.thing");
    registry.register(
        weather.clone(),
        SurfaceName::new("result-cards"),
        demo_weather_showcase(),
    );
    registry.register(
        other.clone(),
        SurfaceName::new("panel"),
        SurfaceUiBundle::new("<!doctype html><p>hi</p>", &[]),
    );

    registry.remove_app(&weather);
    assert!(registry
        .get(&weather, &SurfaceName::new("result-cards"))
        .is_none());
    assert!(registry.get(&other, &SurfaceName::new("panel")).is_some());
}

#[test]
fn demo_bundle_targets_current_protocol_and_avoids_network() {
    let bundle = demo_weather_showcase();
    assert_eq!(bundle.protocol_version, SURFACE_BRIDGE_VERSION);
    assert!(bundle.csp.contains("connect-src 'none'"));
    assert!(bundle.html.contains("window.appHost"));
}

#[test]
fn transport_views_keep_tokens_opaque_and_platform_scoped() {
    let route = SurfaceUiRoute {
        protocol_version: SURFACE_BRIDGE_VERSION,
        route_token: "abc123".into(),
    };
    let remote = surface_ui_view(route.clone(), "/api/surfaces/abc123".into());
    assert_eq!(remote.document_url, "/api/surfaces/abc123");
    let native = surface_ui_view(route, "http://127.0.0.1:41234/abc123".into());
    assert!(native.document_url.ends_with("/abc123"));
    assert!(!native.document_url.contains("mcp-weather"));
}

#[test]
fn shell_csp_allows_surface_documents_without_allowing_inline_shell_scripts() {
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    for name in ["csp", "devCsp"] {
        let csp = config["app"]["security"][name].as_str().unwrap();
        assert!(csp.contains("frame-src http://127.0.0.1:*"));
        assert!(!csp.contains("script-src 'unsafe-inline'"));
    }
}

#[test]
fn injected_sdk_matches_the_host_bridge_protocol() {
    let protocol = include_str!("../../../src/lib/surfaces/surfaceBridgeProtocol.ts");
    assert!(protocol.contains("SURFACE_BRIDGE_PROTOCOL = \"app-host-surface-bridge\""));
    assert!(protocol.contains(&format!(
        "SURFACE_BRIDGE_VERSION = {SURFACE_BRIDGE_VERSION}"
    )));
    assert!(SURFACE_CLIENT_SDK.contains("PROTOCOL = \"app-host-surface-bridge\""));
    assert!(SURFACE_CLIENT_SDK.contains("VERSION = 3"));
}

#[test]
fn connect_src_accepts_ordinary_source_expressions() {
    for source in [
        "https://example.com",
        "https://example.com:8443",
        "wss://stream.example.com",
        "http://127.0.0.1:11434",
        "ws://localhost:8080",
    ] {
        assert!(is_valid_connect_src(source), "should accept {source}");
    }
}

#[test]
fn connect_src_refuses_values_that_could_end_the_directive() {
    for source in [
        "",
        "https://x; frame-src https://evil.example",
        "https://x, default-src *",
        "https://x 'unsafe-eval'",
        "https://x\nscript-src *",
        "'self'",
        "ipc:",
        "ipc://localhost",
        "http://ipc.localhost",
        "http://ipc.localhost/callback",
        "https://ipc.localhost",
        "*",
        "http:",
        "https:",
        "*.example.com",
        "http://*",
        "http://*.localhost",
        "https://user@example.com",
        "https://example.com/path",
        "https://example.com?query=yes",
        "https://example.com#fragment",
        "ftp://example.com",
    ] {
        assert!(!is_valid_connect_src(source), "should refuse {source:?}");
    }
}

#[test]
fn loopback_server_returns_an_isolated_document_and_invalidates_removed_routes() {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn get(url: &str) -> String {
        let rest = url.strip_prefix("http://").unwrap();
        let (authority, path) = rest.split_once('/').unwrap();
        let mut stream = TcpStream::connect(authority).unwrap();
        write!(
            stream,
            "GET /{path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    let registry = Arc::new(Mutex::new(SurfaceUiRegistry::new()));
    let app = AppId::new("mcp-weather");
    let surface = SurfaceName::new("result-cards");
    registry
        .lock()
        .unwrap()
        .register(app.clone(), surface.clone(), demo_weather_showcase());
    let route = registry.lock().unwrap().get(&app, &surface).unwrap();
    let server = RunningSurfaceServer::start(registry.clone()).unwrap();
    let url = server.document_url(&route.route_token);

    let response = get(&url);
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("Content-Security-Policy:"));
    assert!(response.contains("script-src 'unsafe-inline'"));
    assert!(response.contains("frame-ancestors"));
    assert!(response.contains("http://localhost:1420"));
    assert!(response.contains("app-host-surface-bridge"));

    registry.lock().unwrap().remove_app(&app);
    assert!(get(&url).starts_with("HTTP/1.1 404"));
}

#[test]
fn injected_directive_cannot_override_the_host_policy_locks() {
    // A package entry carrying `;` must not be able to append its own
    // directives — least of all `base-uri`/`form-action`, which the host
    // writes *after* `connect-src` and which a first-occurrence override
    // would otherwise win against.
    let csp = deny_by_default_csp(&["https://ok.example; form-action https://evil.example"]);

    assert!(csp.contains("connect-src 'none'"));
    assert!(csp.contains("form-action 'none'"));
    assert!(!csp.contains("evil.example"));
    // The policy still has exactly the directives the host authored.
    assert_eq!(csp.matches("form-action").count(), 1);
    assert_eq!(csp.matches("base-uri").count(), 1);
}

#[test]
fn valid_entries_survive_alongside_a_rejected_one() {
    let csp = deny_by_default_csp(&["https://good.example", "https://bad; frame-src *"]);
    assert!(csp.contains("connect-src https://good.example;"));
    assert!(csp.contains("frame-src 'none'"));
    assert_eq!(csp.matches("frame-src").count(), 1);
}
