import { render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { InstalledApp, SurfaceDeclaration } from "$lib/api";
import { SURFACE_BRIDGE_VERSION } from "$lib/surfaces/surfaceBridgeProtocol";
import SurfaceRenderer from "./SurfaceRenderer.svelte";

vi.mock("$lib/stores/theme", async () => {
  const { writable } = await import("svelte/store");
  const { themes } = await import("$lib/design/colors");
  return {
    resolvedAppearance: writable({ theme: "light", colors: themes.light, appColors: {} }),
    surfaceThemeVariables: (_appId: string, _declarations: unknown[], appearance: { colors: Record<string, string> }) => ({
      "--color-text": appearance.colors.text,
    }),
  };
});

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    getSurfaceUi: vi.fn(async () => null),
    // AppSurfaceFrame dependencies (only used when a bundle is present).
    openSurface: vi.fn(async (appId: string, surface: string) => ({ app_id: appId, surface, instance_id: "i-1" })),
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
    // GenericFormSurface dependency.
    availableCapabilitiesFor: vi.fn(async () => []),
  };
});

const api = await import("$lib/api");
const getSurfaceUi = vi.mocked(api.getSurfaceUi);
const availableCapabilitiesFor = vi.mocked(api.availableCapabilitiesFor);
const closeSurface = vi.mocked(api.closeSurface);
const openSurface = vi.mocked(api.openSurface);
const submitAction = vi.mocked(api.submitAction);

function app(surface: SurfaceDeclaration): InstalledApp {
  return {
    manifest: {
      app_id: "weather",
      version: "0.1.0",
      display_name: "Weather",
      description: "",
      capabilities:
        surface.kind === "form"
          ? [{ name: "get_forecast", description: "", input_schema: {}, effect: "read-only" }]
          : [],
      surfaces: [surface],
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
    content_hash: "h",
    installed_at: "2026-07-10T00:00:00Z",
  };
}

const panel: SurfaceDeclaration = {
  name: "panel",
  kind: "panel",
  title: "Weather panel",
  description: "",
  intents: [],
};

const form: SurfaceDeclaration = {
  name: "get_forecast-form",
  kind: "form",
  title: "Forecast",
  description: "",
  intents: [{ provider: "weather", capability: "get_forecast" }],
};

afterEach(() => vi.clearAllMocks());

describe("SurfaceRenderer routing", () => {
  it("renders a sandboxed frame when the surface has a custom UI bundle", async () => {
    getSurfaceUi.mockResolvedValueOnce({
      protocol_version: SURFACE_BRIDGE_VERSION,
      html: "<!doctype html><html><head></head><body><p>panel</p></body></html>",
      csp: "default-src 'none'",
    });
    render(SurfaceRenderer, { app: app(panel), surface: panel, onOutcome: () => {} });
    const iframe = await screen.findByTitle("Weather: Weather panel");
    expect(iframe.getAttribute("sandbox")).toBe("allow-scripts allow-forms allow-downloads");
    expect(getSurfaceUi).toHaveBeenCalledWith("weather", "panel");
  });

  it("keeps the same frame when polling returns an equivalent app snapshot", async () => {
    getSurfaceUi.mockResolvedValueOnce({
      protocol_version: SURFACE_BRIDGE_VERSION,
      html: "<!doctype html><html><head></head><body><p>panel</p></body></html>",
      csp: "default-src 'none'",
    });
    const installed = app(panel);
    const view = render(SurfaceRenderer, { app: installed, surface: panel, onOutcome: () => {} });
    const initialFrame = await screen.findByTitle("Weather: Weather panel");
    await waitFor(() => expect(openSurface).toHaveBeenCalledOnce());
    const closeCallsBeforeRerender = closeSurface.mock.calls.length;

    await view.rerender({
      app: structuredClone(installed),
      surface: structuredClone(panel),
      onOutcome: () => {},
    });

    expect(screen.getByTitle("Weather: Weather panel")).toBe(initialFrame);
    expect(getSurfaceUi).toHaveBeenCalledOnce();
    expect(openSurface).toHaveBeenCalledOnce();
    expect(closeSurface).toHaveBeenCalledTimes(closeCallsBeforeRerender);
  });

  it("falls back to the generic form when there is no bundle (built-ins intact)", async () => {
    availableCapabilitiesFor.mockResolvedValueOnce([{
      provider_app_id: "weather",
      provider_display_name: "Weather",
      capability: "get_forecast",
      description: "",
      input_schema: {},
      authorizations: [{ data_scope: { kind: "none" }, condition: "silent" }],
    }]);
    render(SurfaceRenderer, { app: app(form), surface: form, onOutcome: () => {} });
    // The generic form renders a submit button labelled with the capability.
    await waitFor(() => expect(screen.getByRole("button", { name: /get_forecast/ })).toBeTruthy());
    expect(screen.queryByTitle("Weather: Forecast")).toBeNull();
  });

  it("closes a generic form binding after its action settles", async () => {
    availableCapabilitiesFor.mockResolvedValueOnce([{
      provider_app_id: "weather",
      provider_display_name: "Weather",
      capability: "get_forecast",
      description: "",
      input_schema: {},
      authorizations: [{ data_scope: { kind: "none" }, condition: "silent" }],
    }]);
    render(SurfaceRenderer, { app: app(form), surface: form, onOutcome: () => {} });

    (await screen.findByRole("button", { name: /get_forecast/ })).click();

    await waitFor(() => expect(submitAction).toHaveBeenCalledOnce());
    await waitFor(() => expect(closeSurface).toHaveBeenCalledWith({
      app_id: "weather",
      surface: "get_forecast-form",
      instance_id: "i-1",
    }));
  });

  it("falls back to the degraded placeholder for a non-form surface with no bundle", async () => {
    render(SurfaceRenderer, { app: app(panel), surface: panel, onOutcome: () => {} });
    await waitFor(() => expect(screen.getByText(/reserved here/)).toBeTruthy());
  });
});
