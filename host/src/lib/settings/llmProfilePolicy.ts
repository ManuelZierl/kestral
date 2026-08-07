import type { ConnectorConfigView, HostConfig } from "$lib/api";

import {
  connectorIsCloud,
  connectorProfileName,
} from "$lib/settings/connectorProfiles";

export interface SelectedCloudLlmPolicy {
  connectorId: string;
  profileId: string;
  acknowledged: boolean;
}

export function selectedCloudLlmPolicy(
  hostConfig: HostConfig | null,
  connectors: ConnectorConfigView[],
  connectorId: string | null,
): SelectedCloudLlmPolicy | null {
  if (!hostConfig || !connectorId) {
    return null;
  }
  const connector = connectors.find((candidate) => candidate.id === connectorId);
  if (!connector || !connectorIsCloud(connector)) {
    return null;
  }
  return {
    connectorId,
    profileId: connectorProfileName(connector.id),
    acknowledged: hostConfig.host.cloud_llm_egress_accepted_profiles.includes(connectorId),
  };
}

export function acknowledgeCloudLlmProfile(
  acceptedConnectorIds: string[],
  connectorId: string,
): string[] {
  return acceptedConnectorIds.includes(connectorId)
    ? acceptedConnectorIds
    : [...acceptedConnectorIds, connectorId];
}
