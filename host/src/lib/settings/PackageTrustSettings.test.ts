import { render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";

vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    listPublisherTrust: vi.fn(async () => [
      {
        key_id: "ed25519:key-1",
        public_key: "BASE64KEY",
        scope: { kind: "app-id", app_id: "com.example.app" },
        status: "trusted",
      },
      {
        key_id: "ed25519:key-2",
        public_key: "BASE64KEY2",
        scope: { kind: "app-id", app_id: "com.example.revoked" },
        status: "revoked",
      },
    ]),
    trustPublisherKey: vi.fn(async () => []),
    revokePublisherKey: vi.fn(async () => []),
  };
});

import * as api from "$lib/api";
import PackageTrustSettings from "./PackageTrustSettings.svelte";

const trustPublisherKey = vi.mocked(api.trustPublisherKey);
const revokePublisherKey = vi.mocked(api.revokePublisherKey);

beforeEach(() => {
  vi.clearAllMocks();
});

it("lists trusted and revoked keys and can trust an exact app id", async () => {
  const user = userEvent.setup();
  render(PackageTrustSettings);

  expect(await screen.findByText("ed25519:key-1")).toBeTruthy();
  await user.type(screen.getByLabelText("App id"), "com.example.new");
  await user.type(screen.getByLabelText("Key id"), "ed25519:key-3");
  await user.type(screen.getByLabelText("Public key"), "BASE64KEY3");
  await user.click(screen.getByRole("button", { name: "Trust key" }));

  await waitFor(() => expect(trustPublisherKey).toHaveBeenCalledWith({
    key_id: "ed25519:key-3",
    public_key: "BASE64KEY3",
    scope: { kind: "app-id", app_id: "com.example.new" },
  }));
});

it("revokes a trusted key from settings after an inline confirm", async () => {
  const user = userEvent.setup();
  render(PackageTrustSettings);

  await screen.findByText("ed25519:key-1");

  await waitFor(() => expect(screen.getAllByRole("button", { name: "Revoke" })).toHaveLength(2));
  await user.click(screen.getAllByRole("button", { name: "Revoke" }).find((button) => !button.hasAttribute("disabled"))!);
  expect(revokePublisherKey).not.toHaveBeenCalled();

  await user.click(screen.getAllByRole("button", { name: "Revoke" }).find((button) => !button.hasAttribute("disabled"))!);

  expect(revokePublisherKey).toHaveBeenCalledWith({
    key_id: "ed25519:key-1",
    scope: { kind: "app-id", app_id: "com.example.app" },
  });
});

it("keeps a trusted key when the inline revoke confirm is dismissed", async () => {
  const user = userEvent.setup();
  render(PackageTrustSettings);

  await screen.findByText("ed25519:key-1");
  await waitFor(() => expect(screen.getAllByRole("button", { name: "Revoke" })).toHaveLength(2));

  await user.click(screen.getAllByRole("button", { name: "Revoke" }).find((button) => !button.hasAttribute("disabled"))!);
  await user.click(screen.getByRole("button", { name: "Keep" }));

  expect(revokePublisherKey).not.toHaveBeenCalled();
  expect(screen.getAllByRole("button", { name: "Revoke" })).toHaveLength(2);
});
