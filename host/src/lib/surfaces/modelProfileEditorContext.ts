import {
  availableCapabilitiesFor,
  discoverConnectorModelsDraft,
  getChatPromptPreview,
  listConnectorConfigs,
  type ConnectorConfigView,
  type InstalledApp,
  type JsonObject,
  type ModelInfo,
} from "$lib/api";

export const MODEL_PROFILE_EXTENSION_POINT = "model-profile-editor";
export const MODEL_PROFILE_CONTRACT_VERSION = 1;

function modelsWithConfiguredDefault(connector: ConnectorConfigView, discovered: ModelInfo[]): ModelInfo[] {
  const models = discovered.map((model) => ({ ...model, variants: [...model.variants] }));
  const configured = models.find((model) => model.id === connector.default_model);
  if (configured) {
    if (connector.default_variant && !configured.variants.includes(connector.default_variant)) {
      configured.variants.push(connector.default_variant);
    }
    return models;
  }
  return [{
    id: connector.default_model,
    display_name: null,
    variants: connector.default_variant ? [connector.default_variant] : [],
    text_verbosity: connector.default_text_verbosity ? [connector.default_text_verbosity] : [],
  }, ...models];
}

function modelsForWire(connector: ConnectorConfigView, discovered: ModelInfo[]): JsonObject[] {
  return modelsWithConfiguredDefault(connector, discovered).map((model) => ({
    id: model.id,
    display_name: model.display_name,
    variants: [...model.variants],
  }));
}

async function connectorContext(connector: ConnectorConfigView): Promise<JsonObject> {
  try {
    const result = await discoverConnectorModelsDraft(
      connector.kind,
      connector.base_url,
      connector.default_model,
      connector.secret_refs.api_key ?? null,
    );
    return {
      id: connector.id,
      default_model: connector.default_model,
      default_variant: connector.default_variant ?? null,
      models: modelsForWire(connector, result.models),
      discovery_error: null,
    };
  } catch {
    return {
      id: connector.id,
      default_model: connector.default_model,
      default_variant: connector.default_variant ?? null,
      models: modelsForWire(connector, []),
      discovery_error: "Model discovery is unavailable for this provider profile.",
    };
  }
}

export async function loadSurfaceHostContext(app: InstalledApp, surfaceName: string): Promise<JsonObject> {
  const contributes = app.manifest.extension_contributions.some((contribution) =>
    contribution.target_app === "chat" &&
    contribution.extension_point === MODEL_PROFILE_EXTENSION_POINT &&
    contribution.contract_version === MODEL_PROFILE_CONTRACT_VERSION &&
    contribution.surface === surfaceName &&
    app.manifest.surfaces.some((surface) => surface.name === surfaceName)
  );
  const declaresConfig = app.manifest.config_declarations.some((config) =>
    config.name === "model-profiles"
  );
  if (!contributes || !declaresConfig) return {};

  const [connectors, capabilities, prompt] = await Promise.all([
    listConnectorConfigs(),
    availableCapabilitiesFor("chat"),
    getChatPromptPreview(),
  ]);
  const connectorContexts = await Promise.all(connectors.map(connectorContext));
  const tools = capabilities
    .filter((view) => view.capability !== "llm.generate" && view.capability !== "agent.run")
    .map((view) => ({
      reference: `${view.provider_app_id}/${view.capability}`,
      provider: view.provider_display_name,
      name: view.capability,
      description: view.description,
    }));

  return {
    kind: "model-profile-editor",
    connectors: connectorContexts,
    tools,
    prompt_layers: prompt.layers.map((layer) => ({
      id: layer.id,
      kind: layer.kind,
      title: layer.title,
      source: layer.source,
      content: layer.content,
      included: layer.included,
    })),
  };
}
