import { writable } from "svelte/store";
import {
  discoverConnectorModelsDraft,
  clearSecret,
  deleteConnectorConfig,
  getHostConfig,
  hasSecret,
  listConnectorConfigs,
  putSecret,
  testConnectorConfig,
  updateAppConfig,
  updateHostConfig,
  upsertConnectorConfig,
  type AppConfigEntry,
  type ConnectionTestResult,
  type ConnectorConfigView,
  type HostConfig,
  type JsonObject,
  type ModelListResult,
} from "$lib/api";

export const hostConfig = writable<HostConfig | null>(null);
export const connectorConfigs = writable<ConnectorConfigView[]>([]);
let refreshSequence = 0;

export async function refreshConfig() {
  const sequence = ++refreshSequence;
  const [config, connectors] = await Promise.all([
    getHostConfig(),
    listConnectorConfigs(),
  ]);
  if (sequence !== refreshSequence) return;
  hostConfig.set(config);
  connectorConfigs.set(connectors);
}

export async function saveHostPatch(patch: JsonObject) {
  const config = await updateHostConfig(patch);
  refreshSequence += 1;
  hostConfig.set(config);
}

export async function saveAppConfig(appId: string, config: JsonObject) {
  const settings = await updateAppConfig(appId, config);
  refreshSequence += 1;
  hostConfig.update((current) => {
    if (!current) return current;
    const entry = current.apps[appId] ?? { settings: {} };
    return {
      ...current,
      apps: {
        ...current.apps,
        [appId]: { ...entry, settings },
      },
    };
  });
}

export async function saveConnector(
  connector: ConnectorConfigView,
  acknowledgeDataEgress = false,
) {
  const saved = await upsertConnectorConfig(connector, acknowledgeDataEgress);
  connectorConfigs.update((current) => {
    const index = current.findIndex((existing) => existing.id === saved.id);
    if (index === -1) {
      return [...current, saved];
    }
    return current.map((existing, currentIndex) => (currentIndex === index ? saved : existing));
  });
  await refreshConfig();
  return saved;
}

export async function removeConnector(connectorId: string) {
  await deleteConnectorConfig(connectorId);
  connectorConfigs.update((current) => current.filter((connector) => connector.id !== connectorId));
  await refreshConfig();
}

export async function saveSecret(owner: string, secretName: string, value: string) {
  await putSecret(owner, secretName, value);
}

export async function removeSecret(owner: string, secretName: string) {
  await clearSecret(owner, secretName);
}

export const checkSecret = (owner: string, secretName: string) => hasSecret(owner, secretName);
export const runConnectorTest = (connectorId: string): Promise<ConnectionTestResult> =>
  testConnectorConfig(connectorId);
export const runDraftModelDiscovery = (
  kind: ConnectorConfigView["kind"],
  baseUrl: string,
  defaultModel: string | null,
  apiKeySecretName: string | null,
): Promise<ModelListResult> =>
  discoverConnectorModelsDraft(kind, baseUrl, defaultModel, apiKeySecretName);

export function appConfigEntry(config: HostConfig | null, appId: string): AppConfigEntry {
  return config?.apps[appId] ?? { settings: {} };
}
