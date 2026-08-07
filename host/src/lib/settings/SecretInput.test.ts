import { fireEvent, render, screen } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";

import SecretInput from "./SecretInput.svelte";

const { checkSecret, saveSecret, removeSecret } = vi.hoisted(() => ({
  checkSecret: vi.fn(),
  saveSecret: vi.fn(),
  removeSecret: vi.fn(),
}));

vi.mock("$lib/stores/config", () => ({
  checkSecret,
  saveSecret,
  removeSecret,
}));

beforeEach(() => {
  vi.clearAllMocks();
  checkSecret.mockResolvedValue(true);
  saveSecret.mockResolvedValue(undefined);
  removeSecret.mockResolvedValue(undefined);
});

describe("SecretInput", () => {
  it("shows presence without placing the stored secret in the input", async () => {
    render(SecretInput, { props: { owner: "llm-provider", secretName: "api-key", label: "API key" } });

    expect(await screen.findByText("Set")).toBeTruthy();
    const input = document.querySelector('input[type="password"]') as HTMLInputElement;
    expect(input.value).toBe("");
    expect(input.placeholder).toBe("Secret is set");
  });

  it("saves and clears through status-only APIs", async () => {
    render(SecretInput, { props: { owner: "llm-provider", secretName: "api-key", label: "API key" } });
    const input = document.querySelector('input[type="password"]') as HTMLInputElement;

    await screen.findByText("Set");
    await fireEvent.input(input, { target: { value: "new-secret" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save key" }));
    expect(saveSecret).toHaveBeenCalledWith("llm-provider", "api-key", "new-secret");
    expect(input.value).toBe("");

    checkSecret.mockResolvedValueOnce(false);
    await fireEvent.click(screen.getByRole("button", { name: "Clear key" }));
    expect(removeSecret).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(removeSecret).toHaveBeenCalledWith("llm-provider", "api-key");
    expect(await screen.findByText("Not set")).toBeTruthy();
  });

  it("keeps the stored secret when the inline clear confirm is dismissed", async () => {
    render(SecretInput, { props: { owner: "llm-provider", secretName: "api-key", label: "API key" } });

    await screen.findByText("Set");
    await fireEvent.click(screen.getByRole("button", { name: "Clear key" }));
    await fireEvent.click(screen.getByRole("button", { name: "Keep" }));

    expect(removeSecret).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Clear key" })).toBeTruthy();
  });

  it("does not allow clear to race an in-flight save", async () => {
    let finishSave!: () => void;
    saveSecret.mockReturnValueOnce(new Promise<void>((resolve) => { finishSave = resolve; }));
    render(SecretInput, { props: { owner: "llm-provider", secretName: "api-key", label: "API key" } });
    await screen.findByText("Set");
    const input = document.querySelector('input[type="password"]') as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "new-secret" } });

    await fireEvent.click(screen.getByRole("button", { name: "Save key" }));

    const clear = screen.getByRole("button", { name: "Clear key" }) as HTMLButtonElement;
    expect(clear.disabled).toBe(true);
    await fireEvent.click(clear);
    expect(removeSecret).not.toHaveBeenCalled();
    finishSave();
    expect(await screen.findByText("Set")).toBeTruthy();
  });
});
