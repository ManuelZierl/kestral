import { beforeEach, describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";

import type { ConnectorConfigView, HostConfig } from "$lib/api";

const api = vi.hoisted(() => ({
  getHostConfig: vi.fn(),
  listConnectorConfigs: vi.fn(),
}));

vi.mock("$lib/api", () => ({
  ...api,
  discoverConnectorModelsDraft: vi.fn(),
  clearSecret: vi.fn(),
  deleteConnectorConfig: vi.fn(),
  hasSecret: vi.fn(),
  putSecret: vi.fn(),
  testConnectorConfig: vi.fn(),
  updateAppConfig: vi.fn(),
  updateHostConfig: vi.fn(),
  upsertConnectorConfig: vi.fn(),
}));

import { connectorConfigs, hostConfig, refreshConfig } from "$lib/stores/config";

function config(defaultProfile: string): HostConfig {
  return {
    version: 1,
    host: {
      default_llm_provider: "llm-provider",
      default_llm_profile: defaultProfile,
      cloud_llm_egress_accepted_profiles: [],
      app_data_backup_retention: 1,
    },
    apps: {},
    connectors: {},
    mcp_servers: {},
    mcp_exports: {},
    mcp_export_transitions: {},
    mcp_gateway: {
      enabled: false,
      bind_address: "127.0.0.1:8137",
      allowed_origins: [],
      oauth_enabled: false,
    },
  };
}

describe("config store", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    hostConfig.set(null);
    connectorConfigs.set([]);
  });

  it("drops an older refresh that resolves after a newer snapshot", async () => {
    let releaseOld!: () => void;
    const oldGate = new Promise<void>((resolve) => { releaseOld = resolve; });
    api.getHostConfig
      .mockImplementationOnce(async () => {
        await oldGate;
        return config("old");
      })
      .mockResolvedValueOnce(config("new"));
    api.listConnectorConfigs
      .mockImplementationOnce(async () => {
        await oldGate;
        return [{ id: "old" }] as ConnectorConfigView[];
      })
      .mockResolvedValueOnce([{ id: "new" }] as ConnectorConfigView[]);

    const oldRefresh = refreshConfig();
    await refreshConfig();
    releaseOld();
    await oldRefresh;

    expect(get(hostConfig)?.host.default_llm_profile).toBe("new");
    expect(get(connectorConfigs)[0]?.id).toBe("new");
  });
});
