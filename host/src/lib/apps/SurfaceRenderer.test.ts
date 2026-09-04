import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  CapabilityDeclaration,
  InstalledApp,
  JsonObject,
  SurfaceActionOutcome,
  SurfaceBinding,
  SurfaceDeclaration,
} from "$lib/api";
import { SURFACE_BRIDGE_VERSION } from "$lib/surfaces/surfaceBridgeProtocol";
import GenericFormSurface from "./GenericFormSurface.svelte";
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

function app(
  surface: SurfaceDeclaration,
  inputSchema: JsonObject = { type: "object", properties: {} },
): InstalledApp {
  return {
    manifest: {
      app_id: "weather",
      version: "0.1.0",
      display_name: "Weather",
      description: "",
      capabilities:
        surface.kind === "form"
          ? [{ name: "get_forecast", description: "", input_schema: inputSchema, effect: "read-only" }]
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
      document_url: "http://127.0.0.1:41234/weather-panel",
    });
    render(SurfaceRenderer, { app: app(panel), surface: panel, onOutcome: () => {} });
    const iframe = await screen.findByTitle("Weather: Weather panel");
    expect(iframe.getAttribute("sandbox")).toBe("allow-scripts allow-forms allow-downloads");
    expect(getSurfaceUi).toHaveBeenCalledWith("weather", "panel");
  });

  it("keeps the same frame when polling returns an equivalent app snapshot", async () => {
    getSurfaceUi.mockResolvedValueOnce({
      protocol_version: SURFACE_BRIDGE_VERSION,
      document_url: "http://127.0.0.1:41234/weather-panel",
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

  it("does not mix a stale opened binding with a newly selected form", async () => {
    const capabilities: CapabilityDeclaration[] = [
      { name: "first", description: "", input_schema: {
        type: "object",
        properties: { query: { type: "string" } },
      }, effect: "read-only" },
      { name: "second", description: "", input_schema: {
        type: "object",
        properties: { query: { type: "string" } },
      }, effect: "read-only" },
    ];
    availableCapabilitiesFor.mockResolvedValue(capabilities.map((capability) => ({
      provider_app_id: "weather",
      provider_display_name: "Weather",
      capability: capability.name,
      description: "",
      input_schema: capability.input_schema,
      authorizations: [{ data_scope: { kind: "none" as const }, condition: "silent" as const }],
    })));
    const pendingOpen = deferred<SurfaceBinding>();
    openSurface.mockReturnValueOnce(pendingOpen.promise);
    const firstOutcome = vi.fn();
    const secondOutcome = vi.fn();
    const view = render(GenericFormSurface, {
      appId: "weather",
      surface: "first-form",
      capability: capabilities[0],
      onOutcome: firstOutcome,
    });

    const firstInput = await screen.findByLabelText("query") as HTMLInputElement;
    await fireEvent.input(firstInput, { target: { value: "first value" } });
    const firstSubmit = screen.getByRole("button", { name: "first" });
    await waitFor(() => expect((firstSubmit as HTMLButtonElement).disabled).toBe(false));
    await fireEvent.click(firstSubmit);
    await waitFor(() => expect(openSurface).toHaveBeenCalledWith("weather", "first-form"));

    await view.rerender({
      appId: "weather",
      surface: "second-form",
      capability: capabilities[1],
      onOutcome: secondOutcome,
    });
    const secondInput = await screen.findByLabelText("query") as HTMLInputElement;
    await waitFor(() => expect((screen.getByRole("button", { name: "second" }) as HTMLButtonElement).disabled).toBe(false));
    await fireEvent.input(secondInput, { target: { value: "second draft" } });

    pendingOpen.resolve({ app_id: "weather", surface: "first-form", instance_id: "stale-binding" });
    await waitFor(() => expect(closeSurface).toHaveBeenCalledWith({
      app_id: "weather",
      surface: "first-form",
      instance_id: "stale-binding",
    }));

    expect(submitAction).not.toHaveBeenCalled();
    expect(secondInput.value).toBe("second draft");
    expect(firstOutcome).not.toHaveBeenCalled();
    expect(secondOutcome).not.toHaveBeenCalled();
  });

  it("ignores an old action completion after the form identity changes", async () => {
    const capabilities: CapabilityDeclaration[] = [
      { name: "first", description: "", input_schema: {
        type: "object",
        properties: { query: { type: "string" } },
      }, effect: "read-only" },
      { name: "second", description: "", input_schema: {
        type: "object",
        properties: { query: { type: "string" } },
      }, effect: "read-only" },
    ];
    availableCapabilitiesFor.mockResolvedValue(capabilities.map((capability) => ({
      provider_app_id: "weather",
      provider_display_name: "Weather",
      capability: capability.name,
      description: "",
      input_schema: capability.input_schema,
      authorizations: [{ data_scope: { kind: "none" as const }, condition: "silent" as const }],
    })));
    const pendingAction = deferred<SurfaceActionOutcome>();
    submitAction.mockReturnValueOnce(pendingAction.promise);
    const firstOutcome = vi.fn();
    const secondOutcome = vi.fn();
    const view = render(GenericFormSurface, {
      appId: "weather",
      surface: "first-form",
      capability: capabilities[0],
      onOutcome: firstOutcome,
    });

    const firstInput = await screen.findByLabelText("query") as HTMLInputElement;
    await fireEvent.input(firstInput, { target: { value: "first value" } });
    const firstSubmit = screen.getByRole("button", { name: "first" });
    await waitFor(() => expect((firstSubmit as HTMLButtonElement).disabled).toBe(false));
    await fireEvent.click(firstSubmit);
    await waitFor(() => expect(submitAction).toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({
        capability: { provider: "weather", capability: "first" },
        input: { query: "first value" },
      }),
    ));

    await view.rerender({
      appId: "weather",
      surface: "second-form",
      capability: capabilities[1],
      onOutcome: secondOutcome,
    });
    const secondInput = await screen.findByLabelText("query") as HTMLInputElement;
    await waitFor(() => expect((screen.getByRole("button", { name: "second" }) as HTMLButtonElement).disabled).toBe(false));
    await fireEvent.input(secondInput, { target: { value: "second draft" } });

    pendingAction.resolve({
      run_id: "old-run",
      result: { kind: "completed", result: { stale: true }, artifacts: [] },
    });
    await waitFor(() => expect(closeSurface).toHaveBeenCalled());

    expect(secondInput.value).toBe("second draft");
    expect(screen.queryByText("old-run")).toBeNull();
    expect(firstOutcome).not.toHaveBeenCalled();
    expect(secondOutcome).not.toHaveBeenCalled();
  });

  it("uses a JSON-object editor for structured schemas and submits nested values intact", async () => {
    const inputSchema = {
      type: "object",
      required: ["profiles"],
      properties: {
        profiles: {
          type: "array",
          items: {
            type: "object",
            required: ["name"],
            properties: { name: { type: "string" } },
          },
        },
        options: {
          type: "object",
          properties: { retries: { type: "integer" } },
        },
      },
    } satisfies JsonObject;
    availableCapabilitiesFor.mockResolvedValueOnce([{
      provider_app_id: "weather",
      provider_display_name: "Weather",
      capability: "get_forecast",
      description: "",
      input_schema: inputSchema,
      authorizations: [{ data_scope: { kind: "none" }, condition: "silent" }],
    }]);
    const outcome = {
      run_id: "run-nested",
      result: {
        kind: "completed" as const,
        result: { forecast: "sunny" },
        artifacts: [{
          artifact_id: "artifact-forecast",
          artifact_type: "forecast-card",
          title: "Berlin forecast",
          content: { forecast: "sunny" },
          provenance: {
            run_id: "run-nested",
            capability: { provider: "weather", capability: "get_forecast" },
            grant_id: "grant-1",
            produced_by: "weather",
            recorded_at: "2026-07-10T00:00:00Z",
          },
        }],
      },
    };
    submitAction.mockResolvedValueOnce(outcome);
    const onOutcome = vi.fn();
    render(SurfaceRenderer, { app: app(form, inputSchema), surface: form, onOutcome });

    const editor = await screen.findByRole("textbox", { name: "Structured JSON input" });
    expect(screen.getByText(/simple field editor cannot represent/)).toBeTruthy();
    expect(screen.queryByLabelText("profiles")).toBeNull();
    const guidanceId = editor.getAttribute("aria-describedby");
    const schemaId = editor.getAttribute("aria-details");
    expect(guidanceId).toBeTruthy();
    expect(schemaId).toBeTruthy();
    expect(document.getElementById(guidanceId!)?.textContent).toContain(
      "Enter a JSON object matching its input schema.",
    );
    expect(document.getElementById(guidanceId!)?.textContent).not.toContain('"profiles"');
    expect(document.getElementById(schemaId!)?.textContent).toContain('"profiles"');
    const source = `{
      "profiles": [{ "name": "local" }],
      "options": { "retries": 2 }
    }`;
    await fireEvent.input(editor, { target: { value: source } });
    const submit = screen.getByRole("button", { name: /get_forecast/ });
    await waitFor(() => expect((submit as HTMLButtonElement).disabled).toBe(false));
    await fireEvent.click(submit);

    await waitFor(() => expect(submitAction).toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({
        input: {
          profiles: [{ name: "local" }],
          options: { retries: 2 },
        },
      }),
    ));
    expect((editor as HTMLTextAreaElement).value).toBe(source);
    expect(await screen.findByText("Action completed")).toBeTruthy();
    expect(screen.getByText("run-nested")).toBeTruthy();
    expect(screen.getByText(/"forecast": "sunny"/)).toBeTruthy();
    expect(screen.getByText("Berlin forecast")).toBeTruthy();
    expect(screen.getByText("artifact-forecast")).toBeTruthy();
    expect(onOutcome).toHaveBeenCalledOnce();
  });

  it("keeps structured input and exposes the Run when a capability fails", async () => {
    const inputSchema = {
      type: "object",
      properties: { profiles: { type: "array", items: { type: "string" } } },
    } satisfies JsonObject;
    availableCapabilitiesFor.mockResolvedValueOnce([{
      provider_app_id: "weather",
      provider_display_name: "Weather",
      capability: "get_forecast",
      description: "",
      input_schema: inputSchema,
      authorizations: [{ data_scope: { kind: "none" }, condition: "silent" }],
    }]);
    const outcome = {
      run_id: "run-failed",
      result: { kind: "failed" as const, error: "weather service unavailable" },
    };
    submitAction.mockResolvedValueOnce(outcome);
    const onOutcome = vi.fn();
    render(SurfaceRenderer, { app: app(form, inputSchema), surface: form, onOutcome });

    const editor = await screen.findByRole("textbox", { name: "Structured JSON input" }) as HTMLTextAreaElement;
    const source = `{ "profiles": ["local"] }`;
    await fireEvent.input(editor, { target: { value: source } });
    const submit = screen.getByRole("button", { name: /get_forecast/ });
    await waitFor(() => expect((submit as HTMLButtonElement).disabled).toBe(false));
    await fireEvent.click(submit);

    expect((await screen.findByRole("alert")).textContent).toContain("weather service unavailable");
    expect(editor.value).toBe(source);
    expect(screen.getByText("Action failed")).toBeTruthy();
    expect(screen.getByText("run-failed")).toBeTruthy();
    expect(onOutcome).toHaveBeenCalledOnce();
  });

  it("keeps invalid structured input visible and does not invoke the capability", async () => {
    const inputSchema = {
      type: "object",
      properties: { profiles: { type: "array", items: { type: "string" } } },
    } satisfies JsonObject;
    availableCapabilitiesFor.mockResolvedValueOnce([{
      provider_app_id: "weather",
      provider_display_name: "Weather",
      capability: "get_forecast",
      description: "",
      input_schema: inputSchema,
      authorizations: [{ data_scope: { kind: "none" }, condition: "silent" }],
    }]);
    render(SurfaceRenderer, { app: app(form, inputSchema), surface: form, onOutcome: () => {} });

    const editor = await screen.findByRole("textbox", { name: "Structured JSON input" }) as HTMLTextAreaElement;
    const source = `{ "profiles": ["local" }`;
    await fireEvent.input(editor, { target: { value: source } });
    const submit = screen.getByRole("button", { name: /get_forecast/ });
    await waitFor(() => expect((submit as HTMLButtonElement).disabled).toBe(false));
    await fireEvent.click(submit);

    expect((await screen.findByRole("alert")).textContent).toContain("Enter valid JSON.");
    expect(editor.value).toBe(source);
    expect(submitAction).not.toHaveBeenCalled();
  });

  it("falls back to the degraded placeholder for a non-form surface with no bundle", async () => {
    render(SurfaceRenderer, { app: app(panel), surface: panel, onOutcome: () => {} });
    await waitFor(() => expect(screen.getByText(/reserved here/)).toBeTruthy());
  });
});
