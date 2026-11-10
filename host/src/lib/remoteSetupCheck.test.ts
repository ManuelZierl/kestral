import { describe, expect, it, vi } from "vitest";
import { checkRemoteSetup } from "$lib/remoteSetupCheck";

const browser = "https://kestral.home.arpa";

function status(paired: boolean): Response {
  return new Response(JSON.stringify({ paired, authenticated: false }), { status: 200 });
}

describe("remote LAN setup check", () => {
  it("checks the API without signing in and reports the pairing state", async () => {
    const request = vi.fn().mockResolvedValue(status(false));
    const result = await checkRemoteSetup(browser, browser, request);
    expect(result).toEqual({
      ok: true,
      paired: false,
      message: expect.stringContaining("first passkey"),
    });
    expect(request).toHaveBeenCalledWith(
      "https://kestral.home.arpa/api/auth/status",
      { credentials: "include" },
    );
  });

  it("recognizes an already-paired host", async () => {
    const request = vi.fn().mockResolvedValue(status(true));
    const result = await checkRemoteSetup(browser, browser, request);
    expect(result.ok).toBe(true);
    expect(result.paired).toBe(true);
  });

  it("blocks LAN HTTP without sending a request", async () => {
    const request = vi.fn();
    const result = await checkRemoteSetup("http://192.168.1.20:4310", browser, request);
    expect(result.ok).toBe(false);
    expect(result.message).toContain("HTTPS");
    expect(request).not.toHaveBeenCalled();
  });

  it("permits local loopback development on different ports", async () => {
    const request = vi.fn().mockResolvedValue(status(false));
    const result = await checkRemoteSetup("http://localhost:4310", "http://localhost:1420", request);
    expect(result.ok).toBe(true);
    expect(request).toHaveBeenCalledWith(
      "http://localhost:4310/api/auth/status",
      { credentials: "include" },
    );
  });

  it("rejects insecure browser origins and localhost on a remote device", async () => {
    const request = vi.fn();
    const insecure = await checkRemoteSetup(browser, "http://homeserver:1420", request);
    const wrongMachine = await checkRemoteSetup("http://localhost:4310", browser, request);
    expect(insecure.message).toContain("browser client over trusted HTTPS");
    expect(wrongMachine.message).toContain("this browser's device");
    expect(request).not.toHaveBeenCalled();
  });

  it("rejects paths, credentials, query strings and invalid URLs", async () => {
    const request = vi.fn();
    for (const input of [
      "https://user:password@kestral.home.arpa",
      "https://kestral.home.arpa/api",
      "https://kestral.home.arpa/?token=abc",
      "not a url",
    ]) {
      expect((await checkRemoteSetup(input, browser, request)).ok).toBe(false);
    }
    expect(request).not.toHaveBeenCalled();
  });

  it("identifies a rejected browser origin", async () => {
    const request = vi.fn().mockResolvedValue(new Response(null, { status: 403 }));
    expect((await checkRemoteSetup(browser, browser, request)).message).toContain("HOST_REMOTE_ORIGIN");
  });

  it("identifies a missing API or an HTML page served instead of the API", async () => {
    const notFound = vi.fn().mockResolvedValue(new Response(null, { status: 404 }));
    const html = vi.fn().mockResolvedValue(new Response("<html>client</html>", { status: 200 }));
    expect((await checkRemoteSetup(browser, browser, notFound)).message).toContain("/api");
    expect((await checkRemoteSetup(browser, browser, html)).message).toContain("unexpected response");
  });

  it("does not pretend to distinguish DNS, TLS and CORS failures", async () => {
    const request = vi.fn().mockRejectedValue(new TypeError("Failed to fetch"));
    const result = await checkRemoteSetup(browser, browser, request);
    expect(result.ok).toBe(false);
    expect(result.message).toContain("DNS");
    expect(result.message).toContain("certificate");
  });
});
