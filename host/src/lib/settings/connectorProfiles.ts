import type { ConnectorConfigView, ConnectorKind } from "$lib/api";

export type ConnectorEndpointKind =
  | "local-ollama"
  | "local-openai-compatible"
  | "cloud-openai-compatible"
  | "cloud-provider";

export interface ConnectorKindSemantics {
  label: string;
  defaultBaseUrl: string;
  apiKeyRequired: boolean;
  oauthRequired: boolean;
  oauthAccountLabel?: string;
  oauthDescription?: string;
}

export const CHATGPT_CODEX_DEFAULT_MODEL = "gpt-5.4-mini";

export function connectorKindSemantics(kind: ConnectorKind): ConnectorKindSemantics {
  switch (kind) {
    case "ollama":
      return { label: "Ollama", defaultBaseUrl: "http://localhost:11434", apiKeyRequired: false, oauthRequired: false };
    case "open-ai-compatible":
      return {
        label: "OpenAI-compatible",
        defaultBaseUrl: "https://api.openai.com/v1",
        apiKeyRequired: false,
        oauthRequired: false,
      };
    case "openai":
      return { label: "OpenAI", defaultBaseUrl: "https://api.openai.com/v1", apiKeyRequired: true, oauthRequired: false };
    case "anthropic":
      return { label: "Anthropic", defaultBaseUrl: "https://api.anthropic.com", apiKeyRequired: true, oauthRequired: false };
    case "anthropic-oauth":
      return { label: "Anthropic", defaultBaseUrl: "https://api.anthropic.com", apiKeyRequired: false, oauthRequired: true, oauthAccountLabel: "Anthropic account" };
    case "openai-codex":
      return {
        label: "ChatGPT (Codex subscription)",
        defaultBaseUrl: "https://chatgpt.com/backend-api",
        apiKeyRequired: false,
        oauthRequired: true,
        oauthAccountLabel: "ChatGPT account",
        oauthDescription: "Connect a ChatGPT Plus or Pro account to use its included Codex quota. This does not use OpenAI API billing.",
      };
    case "github-copilot":
      return { label: "GitHub Copilot", defaultBaseUrl: "https://api.githubcopilot.com", apiKeyRequired: false, oauthRequired: true, oauthAccountLabel: "GitHub account" };
    case "openrouter":
      return {
        label: "OpenRouter",
        defaultBaseUrl: "https://openrouter.ai/api/v1",
        apiKeyRequired: true,
        oauthRequired: false,
      };
    case "google":
      return {
        label: "Google AI",
        defaultBaseUrl: "https://generativelanguage.googleapis.com",
        apiKeyRequired: true,
        oauthRequired: false,
      };
    case "mistral":
      return { label: "Mistral AI", defaultBaseUrl: "https://api.mistral.ai/v1", apiKeyRequired: true, oauthRequired: false };
    case "amazon-bedrock":
      return {
        label: "Amazon Bedrock",
        defaultBaseUrl: "https://bedrock-runtime.us-east-1.amazonaws.com",
        apiKeyRequired: true,
        oauthRequired: false,
      };
  }
}

export function connectorCredentialLabel(kind: ConnectorKind): string {
  return kind === "amazon-bedrock" ? "Bearer token" : "API key";
}

export function connectorProfileName(connectorId: string): string {
  return connectorId.split("/")[1] ?? connectorId;
}

export function defaultApiKeySecretName(connectorId: string): string {
  return `${connectorId}/api_key`;
}

export function defaultOAuthSecretName(connectorId: string): string {
  return `${connectorId}/oauth`;
}

export function connectorUsesOAuth(kind: ConnectorKind): boolean {
  return connectorKindSemantics(kind).oauthRequired;
}

export function connectorEndpointKind(connector: ConnectorConfigView): ConnectorEndpointKind {
  switch (connector.kind) {
    case "ollama":
      return "local-ollama";
    case "open-ai-compatible":
      return isLocalBaseUrl(connector.base_url)
        ? "local-openai-compatible"
        : "cloud-openai-compatible";
    case "openai":
    case "anthropic":
    case "anthropic-oauth":
    case "openai-codex":
    case "github-copilot":
    case "openrouter":
    case "google":
    case "mistral":
    case "amazon-bedrock":
      return "cloud-provider";
  }
}

export function connectorIsCloud(connector: ConnectorConfigView): boolean {
  const endpointKind = connectorEndpointKind(connector);
  return endpointKind === "cloud-openai-compatible" || endpointKind === "cloud-provider";
}

function isLocalBaseUrl(baseUrl: string): boolean {
  // Keep local-endpoint detection aligned with host/src-tauri/src/config.rs.
  try {
    const url = new URL(baseUrl);
    const hostname = url.hostname.toLowerCase();
    return (
      hostname === "localhost" ||
      hostname === "127.0.0.1" ||
      hostname === "0.0.0.0" ||
      hostname === "::1" ||
      hostname.endsWith(".local") ||
      /^10\./.test(hostname) ||
      /^192\.168\./.test(hostname) ||
      /^172\.(1[6-9]|2\d|3[0-1])\./.test(hostname)
    );
  } catch {
    return false;
  }
}
