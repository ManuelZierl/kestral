import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { hostConfig } from "$lib/stores/config";
import ChatPromptSettings from "./ChatPromptSettings.svelte";

const { getChatPromptPreview, updateAppConfig } = vi.hoisted(() => ({
  getChatPromptPreview: vi.fn(),
  updateAppConfig: vi.fn(),
}));

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return { ...actual, getChatPromptPreview, updateAppConfig };
});

function preview() {
  return {
    system_prompt: "system",
    digest: "digest",
    runtime: { host_version: "1.0.0", mode: "chat", model_id: "gpt-4.1", connector_kind: "openai" },
    layers: [
      { id: "protocol", kind: "protocol", title: "Host protocol", source: null, content: "protocol", editable: false, included: true },
      { id: "assistant", kind: "assistant-instructions", title: "Your/default instructions", source: "chat config", content: "assistant", editable: true, included: true },
    ],
    available_skills: [
      {
        app_id: "notes",
        app_display_name: "Notes",
        app_version: "1.2.3",
        skill_name: "summarize",
        description: "Summarize notes",
        instructions: "Use bullet points.",
        content_hash: "hash-1",
        status: "enabled",
        status_reason: null,
      },
      {
        app_id: "planner",
        app_display_name: "Planner",
        app_version: "2.0.0",
        skill_name: "schedule",
        description: "Plan events",
        instructions: "Ask for dates.",
        content_hash: "hash-2",
        status: "review-required",
        status_reason: "Skill changed since last enable.",
      },
    ],
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  getChatPromptPreview.mockResolvedValue(preview());
  updateAppConfig.mockResolvedValue({});
  hostConfig.set({
    version: 1,
    host: { default_llm_provider: "llm", default_llm_profile: "profile", cloud_llm_egress_accepted_profiles: [], app_data_backup_retention: 1 },
    apps: {
      chat: {
        settings: {
          max_iterations: 9,
          show_metadata: false,
          show_thinking: true,
          use_default_instructions: true,
          custom_instructions: "",
          enabled_skills: [],
          show_runtime_identity: true,
          show_app_inventory: false,
          show_connection_details: false,
          record_injected_context: false,
        },
      },
    },
    connectors: {},
    mcp_servers: {},
    mcp_exports: {},
    mcp_export_transitions: {},
    mcp_gateway: { enabled: false, bind_address: "127.0.0.1:8137", allowed_origins: [], oauth_enabled: false },
  });
});

describe("ChatPromptSettings", () => {
  it("loads preview, saves merged settings, and resets custom instructions", async () => {
    render(ChatPromptSettings);

    await waitFor(() => expect(getChatPromptPreview).toHaveBeenCalled());
    const defaultMode = screen.getByRole("radio", { name: /^Kestral default/ }) as HTMLInputElement;
    const customMode = screen.getByRole("radio", { name: /^Custom/ }) as HTMLInputElement;
    expect(defaultMode.checked).toBe(true);
    expect(screen.queryByLabelText("Custom instructions")).toBeNull();

    await fireEvent.click(customMode);
    let custom = screen.getByLabelText("Custom instructions") as HTMLTextAreaElement;
    expect(customMode.checked).toBe(true);
    await fireEvent.input(custom, { target: { value: "hello" } });

    await fireEvent.click(screen.getByRole("button", { name: "Reset assistant instructions" }));
    await fireEvent.click(screen.getByRole("button", { name: "Confirm instruction reset" }));
    expect(defaultMode.checked).toBe(true);
    expect(screen.queryByLabelText("Custom instructions")).toBeNull();

    await fireEvent.click(customMode);
    custom = screen.getByLabelText("Custom instructions") as HTMLTextAreaElement;
    await fireEvent.input(custom, { target: { value: "hello" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save Chat settings" }));

    await waitFor(() => expect(updateAppConfig).toHaveBeenCalledWith("chat", expect.objectContaining({
      max_iterations: 9,
      show_thinking: true,
      use_default_instructions: false,
      custom_instructions: "hello",
      enabled_skills: [],
      show_runtime_identity: true,
      show_app_inventory: false,
      show_connection_details: false,
      record_injected_context: false,
    })));
  });

  it("keeps secondary settings and the long-form prompt preview collapsed by default", async () => {
    render(ChatPromptSettings);
    await waitFor(() => expect(getChatPromptPreview).toHaveBeenCalled());

    for (const title of ["Conversation details", "Context shared with the model", "App guidance", "Prompt preview"]) {
      const details = screen.getByText(title).closest("details");
      expect(details?.open).toBe(false);
    }
    expect(screen.queryByText("assistant")).toBeNull();
  });

  it("saves settings revealed through progressive disclosure", async () => {
    render(ChatPromptSettings);
    await waitFor(() => expect(getChatPromptPreview).toHaveBeenCalled());

    await fireEvent.click(screen.getByText("Conversation details"));
    expect(screen.getByText(/compact MCP result cards/)).toBeTruthy();
    await fireEvent.click(screen.getByRole("checkbox", { name: /^Show activity details/ }));
    await fireEvent.click(screen.getByRole("checkbox", { name: /^Record app context sent to the model/ }));
    await fireEvent.input(screen.getByRole("spinbutton", { name: /^Maximum iterations/ }), { target: { value: "12" } });

    await fireEvent.click(screen.getByText("Context shared with the model"));
    await fireEvent.click(screen.getByRole("checkbox", { name: /^Runtime identity/ }));
    expect(screen.getByText("Runtime identity is not included")).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Save Chat settings" }));
    await waitFor(() => expect(updateAppConfig).toHaveBeenCalledWith("chat", expect.objectContaining({
      max_iterations: 12,
      show_metadata: true,
      record_injected_context: true,
      show_runtime_identity: false,
    })));
  });

  it("keeps in-progress edits when the host config poll re-publishes unchanged settings", async () => {
    render(ChatPromptSettings);
    await waitFor(() => expect(getChatPromptPreview).toHaveBeenCalled());

    await fireEvent.click(screen.getByRole("radio", { name: /^Custom/ }));
    const custom = screen.getByLabelText("Custom instructions") as HTMLTextAreaElement;
    await fireEvent.input(custom, { target: { value: "half-typed instruction" } });

    // The shell polls the host every 1.5s and republishes a structurally equal
    // but referentially new config. That must not count as an outside edit.
    hostConfig.update((config) => (config ? structuredClone(config) : config));
    await waitFor(() => expect(getChatPromptPreview).toHaveBeenCalled());

    expect(custom.value).toBe("half-typed instruction");
  });

  it("reloads the draft when the saved settings actually change elsewhere", async () => {
    render(ChatPromptSettings);
    await waitFor(() => expect(getChatPromptPreview).toHaveBeenCalled());

    await fireEvent.click(screen.getByRole("radio", { name: /^Custom/ }));
    const custom = screen.getByLabelText("Custom instructions") as HTMLTextAreaElement;
    await fireEvent.input(custom, { target: { value: "local edit" } });

    hostConfig.update((config) =>
      config
        ? {
            ...config,
            apps: {
              ...config.apps,
              chat: {
                settings: {
                  ...config.apps.chat.settings,
                  use_default_instructions: false,
                  custom_instructions: "changed remotely",
                },
              },
            },
          }
        : config,
    );

    await waitFor(() => expect(custom.value).toBe("changed remotely"));
  });

  it("allows enabling skills and marks review-required skills distinctly", async () => {
    hostConfig.update((config) => config ? {
      ...config,
      apps: {
        ...config.apps,
        chat: {
          settings: {
            ...config.apps.chat.settings,
            enabled_skills: [{ app_id: "planner", skill_name: "schedule", content_hash: "old-hash" }],
          },
        },
      },
    } : config);
    render(ChatPromptSettings);

    await screen.findByText("Notes 1.2.3");
    await fireEvent.click(screen.getByText("App guidance"));
    expect(screen.getByText("review required")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Enable" }));
    await fireEvent.click(screen.getByRole("button", { name: "Re-enable" }));
    await fireEvent.click(screen.getByRole("button", { name: "Refresh preview" }));
    await waitFor(() => expect(getChatPromptPreview).toHaveBeenLastCalledWith(expect.objectContaining({
      enabled_skills: expect.arrayContaining([
        { app_id: "planner", skill_name: "schedule", content_hash: "hash-2" },
      ]),
    })));
    const candidate = getChatPromptPreview.mock.calls.at(-1)?.[0];
    expect(candidate.enabled_skills.filter((skill: { app_id: string; skill_name: string }) =>
      skill.app_id === "planner" && skill.skill_name === "schedule")).toHaveLength(1);
  });

  it("surfaces preview errors", async () => {
    getChatPromptPreview.mockRejectedValueOnce(new Error("kernel busy"));
    render(ChatPromptSettings);

    await fireEvent.click(screen.getByText("Prompt preview"));
    expect((await screen.findByRole("alert")).textContent).toContain("kernel busy");
  });
});
