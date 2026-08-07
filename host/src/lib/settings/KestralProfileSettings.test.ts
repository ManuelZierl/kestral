import { fireEvent, render, screen } from "@testing-library/svelte";
import { beforeEach, expect, it, vi } from "vitest";

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    getActiveKestralProfile: vi.fn(async () => ({
      profile_id: "profile-active",
      display_name: "Work profile",
      slug: "work",
      root: "/tmp/kestral/profiles/profile-active",
      created_at: "2026-07-18T00:00:00Z",
      current_runtime: true,
      selected_for_next_launch: false,
      source: "managed",
      launch_args: ["--profile", "work"],
      restart_instructions: "Restart Kestral with: --profile work",
    })),
    listKestralProfiles: vi.fn(async () => [
      {
        profile_id: "profile-active",
        display_name: "Work profile",
        slug: "work",
        root: "/tmp/kestral/profiles/profile-active",
        created_at: "2026-07-18T00:00:00Z",
        current_runtime: true,
        selected_for_next_launch: false,
        source: "managed",
        launch_args: ["--profile", "work"],
        restart_instructions: "Restart Kestral with: --profile work",
      },
      {
        profile_id: "profile-archive",
        display_name: "Archive profile",
        slug: "archive",
        root: "/tmp/kestral/profiles/profile-archive",
        created_at: "2026-07-17T00:00:00Z",
        current_runtime: false,
        selected_for_next_launch: true,
        source: "managed",
        launch_args: ["--profile", "archive"],
        restart_instructions: "Restart Kestral with: --profile archive",
      },
      {
        profile_id: "profile-old",
        display_name: "Old profile",
        slug: "old",
        root: "/tmp/kestral/profiles/profile-old",
        created_at: "2026-07-16T00:00:00Z",
        current_runtime: false,
        selected_for_next_launch: false,
        source: "managed",
        launch_args: ["--profile", "old"],
        restart_instructions: "Restart Kestral with: --profile old",
      },
    ]),
    createKestralProfile: vi.fn(async (request) => ({
      profile_id: "profile-new",
      display_name: request.display_name,
      slug: request.slug,
      root: "/tmp/kestral/profiles/profile-new",
      created_at: "2026-07-18T01:00:00Z",
      current_runtime: false,
      selected_for_next_launch: true,
      source: "managed",
      launch_args: ["--profile", request.slug],
      restart_instructions: `Restart Kestral with: --profile ${request.slug}`,
    })),
    deleteKestralProfile: vi.fn(async () => undefined),
  };
});

import KestralProfileSettings from "./KestralProfileSettings.svelte";

beforeEach(() => {
  vi.clearAllMocks();
});

it("distinguishes the runtime profile from the next-launch selection", async () => {
  render(KestralProfileSettings);

  expect(await screen.findByRole("heading", { name: "Work profile" })).toBeTruthy();
  await fireEvent.click(screen.getAllByText("Profile details")[0]);
  expect(screen.getByText("Restart Kestral with: --profile work")).toBeTruthy();
  expect(screen.getAllByText("Next launch").length).toBeGreaterThan(0);
  expect(screen.getByRole("button", { name: "Create profile" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Delete Old profile" })).toBeTruthy();
});

it("opens the create form and delete confirmation", async () => {
  render(KestralProfileSettings);

  await fireEvent.click(screen.getByRole("button", { name: "Create profile" }));
  expect(screen.getByRole("heading", { name: "Create profile" })).toBeTruthy();
  await fireEvent.input(screen.getByLabelText("Name"), { target: { value: "Personal Work" } });
  expect((screen.getByLabelText("Short name") as HTMLInputElement).value).toBe("personal-work");

  await fireEvent.click(screen.getByRole("button", { name: "Delete Old profile" }));
  expect(screen.getByPlaceholderText("Old profile")).toBeTruthy();
});
