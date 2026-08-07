import { describe, expect, it } from "vitest";

import type { ConnectorConfigView, HostConfig } from "$lib/api";
import {
  acknowledgeCloudLlmProfile,
  selectedCloudLlmPolicy,
} from "$lib/settings/llmProfilePolicy";

const connectors: ConnectorConfigView[] = [
  {
    id: "llm-provider/local-ollama",
    kind: "ollama",
    base_url: "http://localhost:11434",
    default_model: "llama3.1",
    default_variant: null,
    default_text_verbosity: null,
    secret_refs: {},
  },
  {
    id: "llm-provider/anthropic",
    kind: "anthropic",
    base_url: "http://localhost:8080",
    default_model: "claude",
    default_variant: null,
    default_text_verbosity: null,
    secret_refs: { api_key: "anthropic-key" },
  },
  {
    id: "llm-provider/work-openai",
    kind: "open-ai-compatible",
    base_url: "https://example.test/v1",
    default_model: "gpt-4.1",
    default_variant: null,
    default_text_verbosity: null,
    secret_refs: {},
  },
];

function hostConfig(
  acceptedProfiles: string[] = [],
  defaultLlmProfile: string = "local-ollama",
): HostConfig {
  return {
    version: 1,
    host: {
      default_llm_provider: "llm-provider",
      default_llm_profile: defaultLlmProfile,
      cloud_llm_egress_accepted_profiles: acceptedProfiles,
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

describe("llmProfilePolicy", () => {
  it("flags a cloud default profile selection for Settings warnings", () => {
    expect(
      selectedCloudLlmPolicy(hostConfig(), connectors, "llm-provider/work-openai"),
    ).toEqual({
      connectorId: "llm-provider/work-openai",
      profileId: "work-openai",
      acknowledged: false,
    });
  });

  it("ignores local default profile selections", () => {
    expect(
      selectedCloudLlmPolicy(hostConfig(), connectors, "llm-provider/local-ollama"),
    ).toBeNull();
  });

  it("flags dedicated providers as cloud regardless of configured URL", () => {
    expect(
      selectedCloudLlmPolicy(hostConfig(), connectors, "llm-provider/anthropic"),
    ).toEqual({
      connectorId: "llm-provider/anthropic",
      profileId: "anthropic",
      acknowledged: false,
    });
  });

  it("deduplicates cloud-profile acknowledgements", () => {
    expect(
      acknowledgeCloudLlmProfile(["llm-provider/work-openai"], "llm-provider/work-openai"),
    ).toEqual(["llm-provider/work-openai"]);
  });

  it("treats the active cloud default profile as a data-egress warning state", () => {
    const config = hostConfig([], "work-openai");

    expect(
      selectedCloudLlmPolicy(
        config,
        connectors,
        `${config.host.default_llm_provider}/${config.host.default_llm_profile}`,
      ),
    ).toEqual({
      connectorId: "llm-provider/work-openai",
      profileId: "work-openai",
      acknowledged: false,
    });
  });
});
