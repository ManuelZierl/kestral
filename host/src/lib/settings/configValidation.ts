import type { ConnectorConfigView, JsonObject, JsonValue } from "$lib/api";
import { connectorKindSemantics } from "$lib/settings/connectorProfiles";

export function validateConfigPatch(value: JsonValue): JsonObject {
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    throw new Error("config patch must be a JSON object");
  }
  return value as JsonObject;
}

export function validateConnectorConfig(connector: ConnectorConfigView): void {
  if (connector.id.trim() === "") {
    throw new Error("Connector id is required");
  }
  const parts = connector.id.split("/");
  if (parts.length !== 2 || parts.some((part) => part.trim() === "")) {
    throw new Error("Connector id must be '<provider>/<profile>'");
  }
  if (connector.base_url.trim() === "") {
    throw new Error("Base URL is required");
  }
  if (connector.default_model.trim() === "") {
    throw new Error("Default model is required");
  }
  if (connectorKindSemantics(connector.kind).apiKeyRequired && !connector.secret_refs.api_key?.trim()) {
    throw new Error("Credential storage name is required for this provider");
  }
  if (connectorKindSemantics(connector.kind).oauthRequired && !connector.secret_refs.oauth?.trim()) {
    throw new Error("OAuth storage name is required for this provider");
  }
}
