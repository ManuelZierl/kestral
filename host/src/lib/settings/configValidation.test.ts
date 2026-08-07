import { describe, expect, it } from "vitest";
import { validateConfigPatch, validateConnectorConfig } from "$lib/settings/configValidation";

describe("configValidation", () => {
  it("accepts only object patches", () => {
    expect(validateConfigPatch({ host: {} })).toEqual({ host: {} });
    expect(() => validateConfigPatch([])).toThrow();
  });

  it("rejects missing connector fields", () => {
    expect(() =>
      validateConnectorConfig({
        id: "llm-provider/openai",
        kind: "open-ai-compatible",
        base_url: "",
        default_model: "",
        default_variant: null,
        default_text_verbosity: null,
        secret_refs: {},
      }),
    ).toThrow();
  });

  it("requires credential references for dedicated providers", () => {
    expect(() => validateConnectorConfig({
      id: "llm-provider/anthropic",
      kind: "anthropic",
      base_url: "https://api.anthropic.com",
      default_model: "claude",
      default_variant: null,
      default_text_verbosity: null,
      secret_refs: {},
    })).toThrow("Credential storage name is required");
  });

  it("requires the host OAuth storage reference for OAuth providers", () => {
    expect(() => validateConnectorConfig({
      id: "llm-provider/codex",
      kind: "openai-codex",
      base_url: "https://chatgpt.com/backend-api",
      default_model: "gpt-5.4-mini",
      default_variant: null,
      default_text_verbosity: null,
      secret_refs: { api_key: "wrong-shape" },
    })).toThrow("OAuth storage name is required");
  });
});
