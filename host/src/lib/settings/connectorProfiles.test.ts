import { describe, expect, it } from "vitest";

import {
  connectorEndpointKind,
  connectorCredentialLabel,
  connectorIsCloud,
  connectorKindSemantics,
  defaultApiKeySecretName,
  defaultOAuthSecretName,
} from "$lib/settings/connectorProfiles";

describe("connectorProfiles", () => {
  it("distinguishes local and cloud connectors", () => {
    expect(
      connectorEndpointKind({
        id: "llm-provider/local-ollama",
        kind: "ollama",
        base_url: "http://localhost:11434",
        default_model: "llama3.1",
        default_variant: null,
        default_text_verbosity: null,
        secret_refs: {},
      }),
    ).toBe("local-ollama");
    expect(
      connectorEndpointKind({
        id: "llm-provider/local-openai",
        kind: "open-ai-compatible",
        base_url: "http://127.0.0.1:8080/v1",
        default_model: "gpt-4.1",
        default_variant: null,
        default_text_verbosity: null,
        secret_refs: {},
      }),
    ).toBe("local-openai-compatible");
    expect(
      connectorEndpointKind({
        id: "llm-provider/cloud-openai",
        kind: "open-ai-compatible",
        base_url: "https://api.openai.com/v1",
        default_model: "gpt-4.1",
        default_variant: null,
        default_text_verbosity: null,
        secret_refs: {},
      }),
    ).toBe("cloud-openai-compatible");
  });

  it("derives a stable default secret ref name", () => {
    expect(defaultApiKeySecretName("llm-provider/work-openai")).toBe(
      "llm-provider/work-openai/api_key",
    );
    expect(defaultOAuthSecretName("llm-provider/work-codex")).toBe(
      "llm-provider/work-codex/oauth",
    );
  });

  it("defines exhaustive provider defaults and auth requirements", () => {
    expect([
      connectorKindSemantics("ollama"),
      connectorKindSemantics("open-ai-compatible"),
      connectorKindSemantics("openai"),
      connectorKindSemantics("anthropic"),
      connectorKindSemantics("anthropic-oauth"),
      connectorKindSemantics("openai-codex"),
      connectorKindSemantics("github-copilot"),
      connectorKindSemantics("openrouter"),
      connectorKindSemantics("google"),
      connectorKindSemantics("mistral"),
      connectorKindSemantics("amazon-bedrock"),
    ]).toEqual([
      { label: "Ollama", defaultBaseUrl: "http://localhost:11434", apiKeyRequired: false, oauthRequired: false },
      { label: "OpenAI-compatible", defaultBaseUrl: "https://api.openai.com/v1", apiKeyRequired: false, oauthRequired: false },
      { label: "OpenAI", defaultBaseUrl: "https://api.openai.com/v1", apiKeyRequired: true, oauthRequired: false },
      { label: "Anthropic", defaultBaseUrl: "https://api.anthropic.com", apiKeyRequired: true, oauthRequired: false },
      { label: "Anthropic", defaultBaseUrl: "https://api.anthropic.com", apiKeyRequired: false, oauthRequired: true, oauthAccountLabel: "Anthropic account" },
      {
        label: "ChatGPT (Codex subscription)",
        defaultBaseUrl: "https://chatgpt.com/backend-api",
        apiKeyRequired: false,
        oauthRequired: true,
        oauthAccountLabel: "ChatGPT account",
        oauthDescription: "Connect a ChatGPT Plus or Pro account to use its included Codex quota. This does not use OpenAI API billing.",
      },
      { label: "GitHub Copilot", defaultBaseUrl: "https://api.githubcopilot.com", apiKeyRequired: false, oauthRequired: true, oauthAccountLabel: "GitHub account" },
      { label: "OpenRouter", defaultBaseUrl: "https://openrouter.ai/api/v1", apiKeyRequired: true, oauthRequired: false },
      { label: "Google AI", defaultBaseUrl: "https://generativelanguage.googleapis.com", apiKeyRequired: true, oauthRequired: false },
      { label: "Mistral AI", defaultBaseUrl: "https://api.mistral.ai/v1", apiKeyRequired: true, oauthRequired: false },
      { label: "Amazon Bedrock", defaultBaseUrl: "https://bedrock-runtime.us-east-1.amazonaws.com", apiKeyRequired: true, oauthRequired: false },
    ]);
  });

  it("classifies every dedicated provider as cloud even with a loopback URL", () => {
    for (const kind of ["openai", "anthropic", "anthropic-oauth", "openai-codex", "github-copilot", "openrouter", "google", "mistral", "amazon-bedrock"] as const) {
      expect(connectorIsCloud({
        id: `llm-provider/${kind}`,
        kind,
        base_url: "http://localhost:8080",
        default_model: "model",
        default_variant: null,
        default_text_verbosity: null,
        secret_refs: { api_key: "credential" },
      })).toBe(true);
    }
  });

  it("labels the Bedrock credential as a bearer token without implying ambient AWS auth", () => {
    expect(connectorCredentialLabel("amazon-bedrock")).toBe("Bearer token");
    expect(connectorCredentialLabel("anthropic")).toBe("API key");
  });
});
