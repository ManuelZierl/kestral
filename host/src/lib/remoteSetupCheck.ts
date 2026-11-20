export interface RemoteSetupCheck {
  ok: boolean;
  message: string;
  paired?: boolean;
}

function loopbackHost(hostname: string): boolean {
  return hostname === "localhost" || hostname === "127.0.0.1" || hostname === "[::1]";
}

function trustedTransport(url: URL): boolean {
  return url.protocol === "https:" || (url.protocol === "http:" && loopbackHost(url.hostname));
}

/**
 * Read-only check before pairing. It proves API reachability, not that the
 * device or its network can be trusted. Browser fetch errors deliberately
 * don't claim to distinguish DNS, TLS, firewall and CORS failures.
 */
export async function checkRemoteSetup(
  hostUrl: string,
  browserOrigin: string,
  request: typeof fetch = fetch,
): Promise<RemoteSetupCheck> {
  let host: URL;
  let browser: URL;
  try {
    host = new URL(hostUrl.trim());
    browser = new URL(browserOrigin);
  } catch {
    return { ok: false, message: "Enter a complete host URL, such as https://kestral.home.arpa." };
  }

  if (!trustedTransport(browser)) {
    return {
      ok: false,
      message: "Open the browser client over trusted HTTPS. Plain HTTP is only supported on loopback for local development.",
    };
  }
  if (!trustedTransport(host)) {
    return {
      ok: false,
      message: "Remote passkeys require HTTPS with a certificate trusted by this device. LAN HTTP is not a secure alternative.",
    };
  }
  if (host.username || host.password || host.pathname !== "/" || host.search || host.hash) {
    return {
      ok: false,
      message: "Use only the host origin (scheme, hostname and optional port), with no path, query or credentials.",
    };
  }
  if (loopbackHost(host.hostname) && !loopbackHost(browser.hostname)) {
    return {
      ok: false,
      message: "localhost refers to this browser's device, not your LAN server. Enter the server's stable HTTPS hostname.",
    };
  }

  let response: Response;
  try {
    response = await request(new URL("/api/auth/status", host).href, { credentials: "include" });
  } catch {
    return {
      ok: false,
      message: "The Kestral API could not be reached. Check this device's DNS, HTTPS certificate trust, firewall, proxy /api routing and the configured browser origin.",
    };
  }

  if (response.status === 403) {
    return {
      ok: false,
      message: "The backend rejected this browser origin. HOST_REMOTE_ORIGIN must exactly match the browser address (scheme, host and port).",
    };
  }
  if (response.status === 404) {
    return {
      ok: false,
      message: "This address does not serve the Kestral API. Check the Host URL and the reverse proxy's /api routing.",
    };
  }
  if (!response.ok) {
    return {
      ok: false,
      message: "The host returned HTTP " + response.status + ". Check the reverse proxy and backend logs.",
    };
  }

  const payload: unknown = await response.json().catch(() => null);
  if (
    typeof payload !== "object" ||
    payload === null ||
    !("paired" in payload) ||
    typeof payload.paired !== "boolean" ||
    !("authenticated" in payload) ||
    typeof payload.authenticated !== "boolean"
  ) {
    return {
      ok: false,
      message: "The address returned a page or unexpected response, not a Kestral API status. Check the /api proxy.",
    };
  }

  return {
    ok: true,
    paired: payload.paired,
    message: payload.paired
      ? "Kestral API reachable. Sign in, or create a pairing code on the host for this device."
      : "Kestral API reachable. Create a pairing code on the host to register the first passkey.",
  };
}
