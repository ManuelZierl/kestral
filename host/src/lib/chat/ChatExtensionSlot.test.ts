import { render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { InstalledApp, SurfaceUiBundle } from "$lib/api";
import { apps } from "$lib/stores/apps";
import { SURFACE_BRIDGE_VERSION } from "$lib/surfaces/surfaceBridgeProtocol";
import ChatExtensionSlot from "./ChatExtensionSlot.svelte";

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    validateExtensionContext: vi.fn(async () => {}),
    getSurfaceUi: vi.fn(),
    openSurface: vi.fn(async () => ({
      app_id: "org.example.annotator",
      surface: "message-annotation",
      instance_id: "surface-1",
    })),
    closeSurface: vi.fn(async () => {}),
    getAppConfig: vi.fn(async () => ({})),
    getSurfaceState: vi.fn(async () => ({ revision: 0, value: null })),
    putSurfaceState: vi.fn(async (_binding, _key, expectedRevision, value) => ({
      revision: expectedRevision + 1,
      value,
    })),
    requestManagedData: vi.fn(async () => null),
    updateAppConfig: vi.fn(async (_id: string, config: unknown) => config),
    submitAction: vi.fn(),
    listAppArtifacts: vi.fn(async () => []),
    appSurfaceEvents: vi.fn(async () => []),
  };
});

const api = await import("$lib/api");
const getSurfaceUi = vi.mocked(api.getSurfaceUi);

function app(id: string): InstalledApp {
  return {
    content_hash: "hash",
    installed_at: "2026-07-13T00:00:00Z",
    manifest: {
      app_id: id,
      version: "1.0.0",
      display_name: id === "chat" ? "Chat" : "Text Annotator",
      description: "test",
      capabilities: [],
      surfaces: id === "chat" ? [] : [{
        name: "message-annotation",
        kind: "card",
        title: "Message annotation",
        description: "Annotate this response.",
        intents: [],
      }],
      agents: [],
      skills: [],
      assistant_profiles: [],
      automations: [],
      connectors: [],
      config_declarations: [],
      artifact_types: [],
      extension_points: id === "chat" ? [{
        name: "message-actions",
        contract_version: 6,
        context_schema: {},
      }] : [],
      extension_contributions: id === "chat" ? [] : [{
        target_app: "chat",
        extension_point: "message-actions",
        contract_version: 6,
        surface: "message-annotation",
      }],
      grant_requests: [],
      event_subscriptions: [],
    },
  };
}

const bundle: SurfaceUiBundle = {
  protocol_version: SURFACE_BRIDGE_VERSION,
  html: "<!doctype html><html><head></head><body><button>Mark as read</button></body></html>",
  csp: "default-src 'none'",
};

afterEach(() => {
  vi.clearAllMocks();
  apps.set([]);
});

describe("ChatExtensionSlot", () => {
  it("renders an installed message extension contribution", async () => {
    getSurfaceUi.mockResolvedValue(bundle);
    apps.set([app("chat"), app("org.example.annotator")]);

    render(ChatExtensionSlot, {
      props: {
        pointName: "message-actions",
        context: {
          thread_id: "thread-1",
          message_id: "message-1",
          assistant_message_number: 1,
          assistant_response_excerpt: "Hello",
          assistant_response_text: "Hello",
          created_at: "2026-07-31T10:00:00.000Z",
          completed_at: "2026-07-31T10:00:01.000Z",
          part_count: 1,
          parts: [{ index: 0, excerpt: "Hello", plain_text: "Hello" }],
          role: "assistant",
        },
      },
    });

    const frame = await screen.findByTitle("Text Annotator: Message annotation");
    expect(frame.getAttribute("srcdoc")).toContain("Mark as read");
  });

  it("retries a transient bundle-load failure", async () => {
    getSurfaceUi.mockRejectedValueOnce("kernel busy").mockResolvedValue(bundle);
    apps.set([app("chat"), app("org.example.annotator")]);

    render(ChatExtensionSlot, {
      props: {
        pointName: "message-actions",
        context: {
          thread_id: "thread-1",
          message_id: "message-1",
          assistant_message_number: 1,
          assistant_response_excerpt: "Hello",
          assistant_response_text: "Hello",
          created_at: "2026-07-31T10:00:00.000Z",
          completed_at: "2026-07-31T10:00:01.000Z",
          part_count: 1,
          parts: [{ index: 0, excerpt: "Hello", plain_text: "Hello" }],
          role: "assistant",
        },
      },
    });

    await screen.findByTitle("Text Annotator: Message annotation");
    expect(getSurfaceUi).toHaveBeenCalledTimes(2);
  });

  it("shows a permanent loading failure instead of hiding the extension", async () => {
    getSurfaceUi.mockResolvedValue(null);
    apps.set([app("chat"), app("org.example.annotator")]);

    render(ChatExtensionSlot, {
      props: {
        pointName: "message-actions",
        context: {
          thread_id: "thread-1",
          message_id: "message-1",
          assistant_message_number: 1,
          assistant_response_excerpt: "Hello",
          assistant_response_text: "Hello",
          created_at: "2026-07-31T10:00:00.000Z",
          completed_at: "2026-07-31T10:00:01.000Z",
          part_count: 1,
          parts: [{ index: 0, excerpt: "Hello", plain_text: "Hello" }],
          role: "assistant",
        },
      },
    });

    await waitFor(() => expect(screen.getByRole("status").textContent).toContain("did not register"));
  });
});
