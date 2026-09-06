import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { InstalledApp, SurfaceDeclaration, SurfaceUiBundle } from "$lib/api";
import { SURFACE_BRIDGE_PROTOCOL, SURFACE_BRIDGE_VERSION } from "./surfaceBridgeProtocol";
import { resolvedAppearance } from "$lib/stores/theme";
import { themes } from "$lib/design/colors";
import AppSurfaceFrame from "./AppSurfaceFrame.svelte";

vi.mock("$lib/stores/theme", async () => {
  const { writable } = await import("svelte/store");
  const { themes: themeRegistry } = await import("$lib/design/colors");
  return {
    resolvedAppearance: writable({ theme: "light", colors: themeRegistry.light, appColors: {} }),
    surfaceThemeVariables: (_appId: string, declarations: { name: string; light: string; dark: string }[], appearance: { theme: "light" | "dark"; colors: Record<string, string>; appColors: Record<string, Record<string, string>> }) => ({
      "--color-text": appearance.colors.text,
      ...Object.fromEntries(declarations.map((declaration) => [
        `--app-color-${declaration.name}`,
        appearance.appColors.weather?.[declaration.name] ?? declaration[appearance.theme],
      ])),
    }),
  };
});

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    openSurface: vi.fn(async () => ({ app_id: "weather", surface: "panel", instance_id: "i-1" })),
    closeSurface: vi.fn(async () => {}),
    submitAction: vi.fn(async () => ({
      run_id: "run-1",
      result: { kind: "completed", result: {}, artifacts: [] },
    })),
    getAppConfig: vi.fn(async () => ({})),
    getSurfaceState: vi.fn(async () => ({ revision: 0, value: null })),
    putSurfaceState: vi.fn(async (_binding, _key, expectedRevision, value) => ({
      revision: expectedRevision + 1,
      value,
    })),
    requestManagedData: vi.fn(async () => null),
    updateAppConfig: vi.fn(async (_id: string, config: unknown) => config),
    listAppArtifacts: vi.fn(async () => []),
    appSurfaceEvents: vi.fn(async () => []),
  };
});

vi.mock("$lib/surfaces/modelProfileEditorContext", () => ({
  loadSurfaceHostContext: vi.fn(async () => ({})),
}));

const api = await import("$lib/api");
const editorContext = await import("$lib/surfaces/modelProfileEditorContext");
const openSurface = vi.mocked(api.openSurface);
const closeSurface = vi.mocked(api.closeSurface);
const getAppConfig = vi.mocked(api.getAppConfig);
const updateAppConfig = vi.mocked(api.updateAppConfig);
const loadSurfaceHostContext = vi.mocked(editorContext.loadSurfaceHostContext);

function surface(overrides: Partial<SurfaceDeclaration> = {}): SurfaceDeclaration {
  return {
    name: "panel",
    kind: "panel",
    title: "Weather panel",
    description: "Shows forecasts.",
    intents: [{ provider: "weather", capability: "get_forecast" }],
    ...overrides,
  };
}

function app(): InstalledApp {
  return {
    manifest: {
      app_id: "weather",
      version: "0.1.0",
      display_name: "Weather",
      description: "Forecasts.",
      capabilities: [],
      surfaces: [surface()],
      agents: [],
      skills: [],
      assistant_profiles: [],
      automations: [],
      connectors: [],
      config_declarations: [],
      artifact_types: [],
      extension_points: [],
      extension_contributions: [],
      grant_requests: [],
      event_subscriptions: [],
    },
    content_hash: "hash",
    installed_at: "2026-07-10T00:00:00Z",
  };
}

function bundle(overrides: Partial<SurfaceUiBundle> = {}): SurfaceUiBundle {
  return {
    protocol_version: SURFACE_BRIDGE_VERSION,
    document_url: "http://127.0.0.1:41234/weather-panel",
    ...overrides,
  };
}

function props(overrides: Record<string, unknown> = {}) {
  return { app: app(), surface: surface(), bundle: bundle(), ...overrides };
}

async function frame(): Promise<HTMLIFrameElement> {
  return (await screen.findByTitle("Weather: Weather panel")) as HTMLIFrameElement;
}

function readyEventFrom(source: Window | null): MessageEvent {
  const data = {
    protocol: SURFACE_BRIDGE_PROTOCOL,
    v: SURFACE_BRIDGE_VERSION,
    type: "ready",
    instanceId: "i-1",
  };
  try {
    return new MessageEvent("message", { data, origin: "null", source });
  } catch {
    const event = new MessageEvent("message", { data, origin: "null" });
    Object.defineProperty(event, "source", { value: source });
    return event;
  }
}

function requestEventFrom(source: Window | null, requestId: number, op: Record<string, unknown>): MessageEvent {
  const data = {
    protocol: SURFACE_BRIDGE_PROTOCOL,
    v: SURFACE_BRIDGE_VERSION,
    type: "request",
    instanceId: "i-1",
    requestId,
    op,
  };
  try {
    return new MessageEvent("message", { data, origin: "null", source });
  } catch {
    const event = new MessageEvent("message", { data, origin: "null" });
    Object.defineProperty(event, "source", { value: source });
    return event;
  }
}

afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
  resolvedAppearance.set({ theme: "light", colors: themes.light, appColors: {} });
});

describe("AppSurfaceFrame sandboxing", () => {
  it("renders a script-only sandbox with no same-origin access and no Tauri", async () => {
    render(AppSurfaceFrame, props());
    const iframe = await frame();
    const sandbox = iframe.getAttribute("sandbox") ?? "";
    expect(sandbox).toBe("allow-scripts allow-forms allow-downloads");
    expect(sandbox).toContain("allow-downloads");
    // The critical negative: no allow-same-origin means an opaque origin, so
    // the frame cannot reach the host window, Tauri, cookies, or storage.
    expect(sandbox).not.toContain("allow-same-origin");
    expect(iframe.getAttribute("allow")).toBe("");
    expect(iframe.getAttribute("referrerpolicy")).toBe("no-referrer");
  });

  it("loads eagerly so the hidden frame can complete its readiness handshake", async () => {
    render(AppSurfaceFrame, props());

    expect((await frame()).getAttribute("loading")).toBe("eager");
  });

  it("loads the host-owned isolated surface document instead of inherited-CSP srcdoc", async () => {
    render(AppSurfaceFrame, props());
    const iframe = await frame();
    expect(iframe.getAttribute("src")).toBe("http://127.0.0.1:41234/weather-panel");
    expect(iframe.hasAttribute("srcdoc")).toBe(false);
  });

  it("opens the surface binding through the kernel", async () => {
    render(AppSurfaceFrame, props());
    await frame();
    expect(openSurface).toHaveBeenCalledWith("weather", "panel");
  });

  it("sends the resolved theme on init and when it changes", async () => {
    resolvedAppearance.set({ theme: "dark", colors: themes.dark, appColors: {} });
    render(AppSurfaceFrame, props());
    const iframe = await frame();
    const postMessage = vi.spyOn(iframe.contentWindow!, "postMessage");

    await fireEvent.load(iframe);
    expect(postMessage).toHaveBeenCalledWith(
      expect.objectContaining({
        type: "init",
        theme: "dark",
        variables: expect.objectContaining({ "--color-text": themes.dark.text }),
      }),
      "*",
    );

    resolvedAppearance.set({ theme: "light", colors: themes.light, appColors: {} });
    await waitFor(() => expect(postMessage).toHaveBeenCalledWith(
      expect.objectContaining({
        type: "theme",
        theme: "light",
        variables: expect.objectContaining({ "--color-text": themes.light.text }),
      }),
      "*",
    ));
  });

  it("renders app identity in host chrome, outside the frame", async () => {
    render(AppSurfaceFrame, props());
    const strip = await screen.findByTestId("surface-identity");
    expect(strip.textContent).toContain("Weather");
    // The identity strip is a sibling of the iframe, never inside it.
    const iframe = await frame();
    expect(strip.contains(iframe)).toBe(false);
  });

  it("in fill mode keeps the strip only as a loading indicator", async () => {
    render(AppSurfaceFrame, props({ fill: true }));
    const iframe = await frame();
    // While loading, the strip shows who is loading…
    expect(screen.getByTestId("surface-identity")).toBeTruthy();
    // …and once ready the surrounding host chrome (top bar) carries identity,
    // so the whole workspace belongs to the app.
    window.dispatchEvent(readyEventFrom(iframe.contentWindow));
    await waitFor(() => expect(screen.queryByTestId("surface-identity")).toBeNull());
  });
});

describe("AppSurfaceFrame lifecycle", () => {
  it("goes ready when the frame reports ready from its own window", async () => {
    render(AppSurfaceFrame, props());
    const iframe = await frame();
    window.dispatchEvent(readyEventFrom(iframe.contentWindow));
    await waitFor(() => expect(screen.queryByText(/loading…/)).toBeNull());
  });

  it("ignores a spoofed ready from a foreign window", async () => {
    render(AppSurfaceFrame, props());
    await frame();
    // A different window claiming ready must not flip the surface to ready.
    window.dispatchEvent(readyEventFrom(window));
    await Promise.resolve();
    expect(screen.getByText(/loading…/)).toBeTruthy();
  });

  it("isolates a hung frame as an error instead of hanging the host", async () => {
    render(AppSurfaceFrame, props({ handshakeTimeoutMs: 20 }));
    await frame();
    // Never send ready; the hang guard should fire.
    await waitFor(() => expect(screen.getByRole("alert")).toBeTruthy(), { timeout: 500 });
    expect(screen.getByRole("alert").textContent).toContain("didn't respond");
    expect(screen.queryByTitle("Weather: Weather panel")).toBeNull();
  });

  it("releases the kernel binding immediately when a frame hangs, not only on unmount", async () => {
    render(AppSurfaceFrame, props({ handshakeTimeoutMs: 20 }));
    await frame();
    // The hang guard must tear down while the component stays mounted, so a
    // hung frame leaves no kernel binding or message listener behind.
    await waitFor(() =>
      expect(closeSurface).toHaveBeenCalledWith({
        app_id: "weather",
        surface: "panel",
        instance_id: "i-1",
      }),
    );
  });

  it("closes the surface binding on unmount", async () => {
    const { unmount } = render(AppSurfaceFrame, props());
    await frame();
    unmount();
    await waitFor(() =>
      expect(closeSurface).toHaveBeenCalledWith({
        app_id: "weather",
        surface: "panel",
        instance_id: "i-1",
      }),
    );
  });
});

describe("AppSurfaceFrame guards", () => {
  it("refuses a bundle built for a different bridge version", async () => {
    render(AppSurfaceFrame, props({ bundle: bundle({ protocol_version: 999 }) }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("needs a different host");
    expect(openSurface).not.toHaveBeenCalled();
  });

  it("shows an error (not a crash) when the surface can't be opened, e.g. after uninstall", async () => {
    openSurface.mockRejectedValueOnce("unknown app: weather");
    render(AppSurfaceFrame, props());
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("unknown app: weather");
    expect(screen.queryByTitle("Weather: Weather panel")).toBeNull();
  });

  it("retries surface startup automatically when the kernel is briefly busy", async () => {
    vi.useFakeTimers();
    openSurface
      .mockRejectedValueOnce(new Error("kernel busy: another host operation owns the kernel"))
      .mockRejectedValueOnce(new Error("kernel busy: another host operation owns the kernel"));
    render(AppSurfaceFrame, props());

    await vi.waitFor(() => expect(openSurface).toHaveBeenCalledOnce());
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByText(/loading…/)).toBeTruthy();

    await vi.advanceTimersByTimeAsync(1000);
    await vi.waitFor(() => expect(openSurface).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole("alert")).toBeNull();

    await vi.advanceTimersByTimeAsync(1000);
    await vi.waitFor(() => expect(openSurface).toHaveBeenCalledTimes(3));
    expect(await screen.findByTitle("Weather: Weather panel")).toBeTruthy();
  });

  it("cancels a pending kernel-busy retry when the surface unmounts", async () => {
    vi.useFakeTimers();
    openSurface.mockRejectedValueOnce(new Error("kernel busy: another host operation owns the kernel"));
    const { unmount } = render(AppSurfaceFrame, props());

    await vi.waitFor(() => expect(openSurface).toHaveBeenCalledOnce());
    unmount();
    await vi.advanceTimersByTimeAsync(1000);

    expect(openSurface).toHaveBeenCalledOnce();
  });

  it("retries surface startup when host context is briefly blocked by the kernel", async () => {
    vi.useFakeTimers();
    loadSurfaceHostContext.mockRejectedValueOnce("kernel busy: another host operation owns the kernel");
    render(AppSurfaceFrame, props());

    await vi.waitFor(() => expect(loadSurfaceHostContext).toHaveBeenCalledOnce());
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByText(/loading…/)).toBeTruthy();

    await vi.advanceTimersByTimeAsync(1000);
    await vi.waitFor(() => expect(loadSurfaceHostContext).toHaveBeenCalledTimes(2));
    expect(await screen.findByTitle("Weather: Weather panel")).toBeTruthy();
  });

  it("retries a config write when the kernel is briefly busy", async () => {
    vi.useFakeTimers();
    updateAppConfig
      .mockRejectedValueOnce(new Error("kernel busy: another host operation owns the kernel"))
      .mockRejectedValueOnce(new Error("kernel busy: another host operation owns the kernel"));
    render(AppSurfaceFrame, props());
    const iframe = await frame();

    window.dispatchEvent(requestEventFrom(
      iframe.contentWindow,
      7,
      { kind: "update-config", config: { profiles: [] } },
    ));
    await vi.waitFor(() => expect(updateAppConfig).toHaveBeenCalledOnce());

    await vi.advanceTimersByTimeAsync(1000);
    await vi.waitFor(() => expect(updateAppConfig).toHaveBeenCalledTimes(2));
    await vi.advanceTimersByTimeAsync(1000);
    await vi.waitFor(() => expect(updateAppConfig).toHaveBeenCalledTimes(3));
    expect(updateAppConfig).toHaveBeenLastCalledWith("weather", { profiles: [] });
  });

  it("fails explicitly and releases the binding when configuration cannot load", async () => {
    getAppConfig.mockRejectedValueOnce("config store unavailable");
    render(AppSurfaceFrame, props());
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("config store unavailable");
    expect(screen.queryByTitle("Weather: Weather panel")).toBeNull();
    await waitFor(() => expect(closeSurface).toHaveBeenCalled());
  });
});
