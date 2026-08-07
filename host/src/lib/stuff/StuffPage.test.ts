import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    grantArtifactAccess: vi.fn(async () => undefined),
    listArtifacts: vi.fn(async () => []),
    listGrants: vi.fn(async () => []),
  };
});

const clipboardWriteText = vi.fn(async () => undefined);

import StuffPage from "$lib/stuff/StuffPage.svelte";
import * as api from "$lib/api";
import type { Artifact, GrantView } from "$lib/api";
import { artifacts, artifactsLoaded } from "$lib/stores/artifacts";
import { grants, grantsLoaded } from "$lib/stores/grants";
import { shellError } from "$lib/stores/hostState";
import { artifactTarget } from "$lib/stores/navigation";
import { ARTIFACTS_APP_ID, CHAT_APP_ID } from "$lib/stuff/artifactAccess";

const artifact: Artifact = {
  artifact_id: "artifact-1",
  artifact_type: "report",
  title: "Weekly report",
  content: { text: "Summary" },
  provenance: {
    run_id: "run-1",
    capability: { provider: "reports", capability: "create" },
    grant_id: "grant-producer",
    produced_by: "reports",
    recorded_at: "2026-08-02T00:00:00Z",
  },
};

function broadGrant(capability: string): GrantView {
  return {
    grant_id: `grant-${capability}`,
    holder: CHAT_APP_ID,
    holder_display_name: "Chat",
    scope: { kind: "exact-capability", provider: ARTIFACTS_APP_ID, capability },
    data_scope: { kind: "all-resources" },
    condition: "requires-approval",
    issued_at: "2026-08-02T00:00:00Z",
    expires_at: null,
    status: "active",
    origin: "user-added",
  };
}

describe("StuffPage artifact access", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: clipboardWriteText },
      configurable: true,
      writable: true,
    });
    artifacts.set([artifact]);
    artifactsLoaded.set(true);
    grants.set([]);
    grantsLoaded.set(true);
    shellError.set("");
    artifactTarget.set(null);
  });

  it("lets the user allow one artifact without entering a resource id", async () => {
    render(StuffPage);

    await fireEvent.click(screen.getByRole("button", { name: "Allow Chat" }));

    await waitFor(() => expect(api.grantArtifactAccess).toHaveBeenCalledWith(CHAT_APP_ID, {
      kind: "artifact",
      artifact_id: "artifact-1",
    }));
  });

  it("offers broad access explicitly and reports it once granted", async () => {
    const view = render(StuffPage);
    await fireEvent.click(screen.getByRole("button", { name: "Allow all artifacts" }));
    await waitFor(() => expect(api.grantArtifactAccess).toHaveBeenCalledWith(CHAT_APP_ID, {
      kind: "all-artifacts",
    }));

    grants.set([
      broadGrant("artifacts.query"),
      broadGrant("artifacts.read"),
    ]);
    await view.rerender({});
    expect(screen.getByText("Chat can use all current and future artifacts")).toBeTruthy();
    expect(screen.getByText("Available to Chat")).toBeTruthy();
  });

  it("focuses and briefly highlights an artifact opened from elsewhere", async () => {
    artifactTarget.set({ request: 1, artifactId: "artifact-1" });
    render(StuffPage);

    const card = document.getElementById("artifact-artifact-1");
    expect(card).toBeTruthy();
    await waitFor(() => {
      expect(card?.classList.contains("highlighted")).toBe(true);
      expect(document.activeElement).toBe(card);
    });
  });

  it("copies the artifact id to the clipboard from the card", async () => {
    render(StuffPage);

    await fireEvent.click(screen.getByRole("button", { name: "Copy artifact ID" }));

    await waitFor(() => expect(clipboardWriteText).toHaveBeenCalledWith("artifact-1"));
    expect(await screen.findByRole("button", { name: "Copied artifact ID" })).toBeTruthy();
  });
});
