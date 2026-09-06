import { cleanup, fireEvent, render, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tick } from "svelte";

import type {
  ActionIntent,
  CapabilityDeclaration,
  InstalledApp,
  SurfaceActionOutcome,
  SurfaceDeclaration,
  SurfaceUiBundle,
} from "$lib/api";
import * as api from "$lib/api";
import { loadSurfaceHostContext } from "$lib/surfaces/modelProfileEditorContext";
import { errorResponse, okResponse, SURFACE_BRIDGE_PROTOCOL, SURFACE_BRIDGE_VERSION } from "./surfaceBridgeProtocol";
import AppSurfaceFrame from "./AppSurfaceFrame.svelte";

// Confirmation uses its own module/DOM fixtures, separate from the startup
// suite's fake clocks and injected API failures. The real bridge is exercised.
vi.mock("$lib/stores/theme", async () => {
  const { writable } = await import("svelte/store");
  const { themes } = await import("$lib/design/colors");
  return {
    resolvedAppearance: writable({ theme: "light", colors: themes.light, appColors: {} }),
    surfaceThemeVariables: () => ({ "--color-text": themes.light.text }),
  };
});

vi.mock("$lib/api", async (importOriginal) => ({
  ...await importOriginal<typeof import("$lib/api")>(),
  openSurface: vi.fn(),
  closeSurface: vi.fn(),
  submitAction: vi.fn(),
  listGrants: vi.fn(),
  getAppConfig: vi.fn(),
}));

vi.mock("$lib/surfaces/modelProfileEditorContext", () => ({
  loadSurfaceHostContext: vi.fn(),
}));

const binding = { app_id: "weather", surface: "panel", instance_id: "confirmation-1" };
const intent: ActionIntent = {
  capability: { provider: "weather", capability: "get_forecast" },
  input: { city: "Berlin" },
  data_scope: { kind: "none" },
  goal: "Get the forecast",
};
const outcome: SurfaceActionOutcome = {
  run_id: "run-1",
  result: { kind: "completed", result: {}, artifacts: [] },
};
const surface: SurfaceDeclaration = {
  name: "panel",
  kind: "panel",
  title: "Weather panel",
  description: "Shows forecasts.",
  intents: [intent.capability],
};
const bundle: SurfaceUiBundle = {
  protocol_version: SURFACE_BRIDGE_VERSION,
  document_url: "http://127.0.0.1:41234/weather-panel",
};

beforeEach(() => {
  vi.mocked(api.openSurface).mockReset().mockResolvedValue({ ...binding });
  vi.mocked(api.closeSurface).mockReset().mockResolvedValue(undefined);
  vi.mocked(api.submitAction).mockReset().mockImplementation(async () => ({
    run_id: "run-1",
    result: { kind: "completed", result: {}, artifacts: [] },
  }));
  vi.mocked(api.listGrants).mockReset().mockResolvedValue([]);
  vi.mocked(api.getAppConfig).mockReset().mockResolvedValue({});
  vi.mocked(loadSurfaceHostContext).mockReset().mockResolvedValue({});
});

afterEach(async () => {
  await cleanup();
  await tick();
  vi.restoreAllMocks();
});

function app(effect: CapabilityDeclaration["effect"]): InstalledApp {
  return {
    manifest: {
      app_id: "weather",
      version: "0.1.0",
      display_name: "Weather",
      description: "Forecasts.",
      capabilities: [{
        name: "get_forecast",
        description: "Get the forecast for a city",
        input_schema: { type: "object" },
        effect,
      }],
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
    content_hash: "hash",
    installed_at: "2026-07-10T00:00:00Z",
  };
}

function frameMessage(source: Window, payload: Record<string, unknown>): MessageEvent {
  const event = new MessageEvent("message", {
    origin: "null",
    data: {
      protocol: SURFACE_BRIDGE_PROTOCOL,
      v: SURFACE_BRIDGE_VERSION,
      instanceId: binding.instance_id,
      ...payload,
    },
  });
  // jsdom versions disagree on accepting an iframe Window in the constructor.
  Object.defineProperty(event, "source", { value: source });
  return event;
}

async function requestOwnAction(effect: CapabilityDeclaration["effect"] = "read-only") {
  const onOutcome = vi.fn();
  const view = render(AppSurfaceFrame, { props: { app: app(effect), surface, bundle, onOutcome } });
  const fixture = within(view.container);
  await tick();
  const iframe = await fixture.findByTitle("Weather: Weather panel") as HTMLIFrameElement;
  await tick();
  const source = iframe.contentWindow!;
  const postMessage = vi.spyOn(source, "postMessage");
  // Complete the same init/ready sequence as the real iframe before invoking.
  await fireEvent.load(iframe);
  await vi.waitFor(() => expect(postMessage).toHaveBeenCalledWith(
    expect.objectContaining({ type: "init", instanceId: binding.instance_id }), "*",
  ));
  window.dispatchEvent(frameMessage(source, { type: "ready" }));
  await tick();
  await vi.waitFor(() => {
    expect(iframe.isConnected).toBe(true);
    expect(iframe.classList.contains("loading")).toBe(false);
  });
  window.dispatchEvent(frameMessage(source, {
    type: "request", requestId: 11, op: { kind: "invoke", ...intent },
  }));
  await tick();
  const dialog = await fixture.findByRole("alertdialog");
  return { ...view, iframe, dialog, fixture, postMessage, onOutcome };
}

describe("AppSurfaceFrame action confirmation", () => {
  it.each(["read-only", "local-write"] as const)(
    "forwards an approved %s outcome once, without a self-echo event",
    async (effect) => {
      const { iframe, dialog, fixture, postMessage, onOutcome } = await requestOwnAction(effect);
      expect(iframe.contains(dialog)).toBe(false);
      expect(api.submitAction).not.toHaveBeenCalled();
      expect(onOutcome).not.toHaveBeenCalled();
      expect(postMessage.mock.calls.some(([message]) =>
        message.type === "response" && message.requestId === 11,
      )).toBe(false);

      await fireEvent.click(within(dialog).getByRole("button", { name: "Continue" }));
      await vi.waitFor(() => expect(postMessage).toHaveBeenCalledWith(okResponse(11, outcome), "*"));
      expect(api.submitAction).toHaveBeenCalledOnce();
      expect(api.submitAction).toHaveBeenCalledWith(binding, intent, expect.any(Function));
      expect(onOutcome).toHaveBeenCalledOnce();
      expect(onOutcome).toHaveBeenCalledWith(outcome);
      expect(postMessage.mock.calls.filter(([message]) =>
        message.type === "response" && message.requestId === 11,
      )).toHaveLength(1);
      expect(postMessage.mock.calls.some(([message]) => message.type === "event")).toBe(false);
      expect(fixture.queryByRole("alertdialog")).toBeNull();
    },
  );

  it.each(["Cancel", "Escape"])("does not execute after %s", async (decision) => {
    const { dialog, fixture, postMessage, onOutcome } = await requestOwnAction();
    if (decision === "Cancel") {
      await fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    } else {
      await fireEvent.keyDown(dialog, { key: "Escape" });
    }
    await vi.waitFor(() => expect(postMessage).toHaveBeenCalledWith(
      errorResponse(11, "Action cancelled before execution."), "*",
    ));
    expect(api.submitAction).not.toHaveBeenCalled();
    expect(onOutcome).not.toHaveBeenCalled();
    expect(fixture.queryByRole("alertdialog")).toBeNull();
  });

  it("cancels a pending confirmation when the surface unmounts", async () => {
    const { unmount, fixture, onOutcome } = await requestOwnAction();
    await unmount();
    await tick();
    await vi.waitFor(() => expect(api.closeSurface).toHaveBeenCalledWith(binding));
    expect(api.submitAction).not.toHaveBeenCalled();
    expect(onOutcome).not.toHaveBeenCalled();
    expect(fixture.queryByRole("alertdialog")).toBeNull();
  });
});
