import type { DataScope } from "$lib/api";

const LLM_PROFILE_RESOURCE_PREFIX = "llm-profile:";
const LLM_ZONE_LOCAL_RESOURCE_ID = "llm-zone:local";
const LLM_ZONE_CLOUD_RESOURCE_ID = "llm-zone:cloud";

function formatResourceId(resourceId: string): string {
  if (resourceId === LLM_ZONE_LOCAL_RESOURCE_ID) {
    return "Local LLM models (llm-zone:local)";
  }
  if (resourceId === LLM_ZONE_CLOUD_RESOURCE_ID) {
    return "Cloud LLM models (llm-zone:cloud)";
  }
  if (resourceId.startsWith(LLM_PROFILE_RESOURCE_PREFIX)) {
    const profileId = resourceId.slice(LLM_PROFILE_RESOURCE_PREFIX.length);
    return `LLM profile ${profileId} (${resourceId})`;
  }
  return resourceId;
}

export function dataScopeLabel(scope: DataScope): string {
  if (scope.kind === "none") {
    return "Not tied to a registered resource";
  }
  if (scope.kind === "all-resources") {
    return "All current and future resources";
  }
  return `Resources: ${scope.resource_ids.map(formatResourceId).join(", ")}`;
}
