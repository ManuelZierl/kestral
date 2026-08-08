import { beforeEach, describe, expect, it, vi } from "vitest";

const transport = vi.hoisted(() => ({
  invokeChatWithProgress: vi.fn(),
  invokeHost: vi.fn(),
  invokeHostWithProgress: vi.fn(),
  isRemoteTransport: vi.fn(() => false),
  resolveHostResourceUrl: vi.fn((value: string) => value),
}));

vi.mock("$lib/hostTransport", () => transport);

import {
  getSurfaceUi,
  submitAction,
  type ActionIntent,
  type SurfaceActionOutcome,
  type SurfaceBinding,
} from "$lib/api";

const binding: SurfaceBinding = {
  app_id: "com.example.tasks",
  surface: "tasks",
  instance_id: "surface-1",
};
const intent: ActionIntent = {
  capability: { provider: "com.example.tasks", capability: "list_tasks" },
  input: {},
  data_scope: { kind: "none" },
  goal: "List tasks",
};
const outcome: SurfaceActionOutcome = {
  run_id: "run-1",
  result: { kind: "completed", result: [], artifacts: [] },
};

describe("surface action transport", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    transport.invokeHost.mockResolvedValue(outcome);
    transport.invokeHostWithProgress.mockResolvedValue(outcome);
  });

  it("requests progress through the active host transport", async () => {
    const onProgress = vi.fn();

    await expect(submitAction(binding, intent, onProgress)).resolves.toEqual(outcome);

    expect(transport.invokeHostWithProgress).toHaveBeenCalledWith(
      "submit_action_with_progress",
      { binding, intent },
      onProgress,
    );
    expect(transport.invokeHost).not.toHaveBeenCalled();
  });

  it("uses the plain command when the caller does not request progress", async () => {
    await expect(submitAction(binding, intent)).resolves.toEqual(outcome);

    expect(transport.invokeHost).toHaveBeenCalledWith("submit_action", { binding, intent });
    expect(transport.invokeHostWithProgress).not.toHaveBeenCalled();
  });
});

describe("surface document transport", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    transport.isRemoteTransport.mockReturnValue(false);
    transport.resolveHostResourceUrl.mockImplementation((value: string) => value);
  });

  it("requests a native isolated URL without transporting app HTML", async () => {
    transport.invokeHost.mockResolvedValue({
      protocol_version: 3,
      document_url: "http://127.0.0.1:41234/token",
    });

    await expect(getSurfaceUi("weather", "panel")).resolves.toEqual({
      protocol_version: 3,
      document_url: "http://127.0.0.1:41234/token",
    });
    expect(transport.invokeHost).toHaveBeenCalledWith("get_surface_ui", {
      appId: "weather",
      surface: "panel",
      remote: false,
    });
  });

  it("resolves authenticated remote surface paths against the host URL", async () => {
    transport.isRemoteTransport.mockReturnValue(true);
    transport.invokeHost.mockResolvedValue({
      protocol_version: 3,
      document_url: "/api/surfaces/token",
    });
    transport.resolveHostResourceUrl.mockReturnValue("https://host.example/api/surfaces/token");

    await expect(getSurfaceUi("weather", "panel")).resolves.toEqual({
      protocol_version: 3,
      document_url: "https://host.example/api/surfaces/token",
    });
    expect(transport.invokeHost).toHaveBeenCalledWith("get_surface_ui", {
      appId: "weather",
      surface: "panel",
      remote: true,
    });
  });
});
