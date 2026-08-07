import type { GrantView } from "$lib/api";
import { dataScopeCovers, scopeCovers } from "$lib/apps/appMetadata";

export const CHAT_APP_ID = "chat";
export const ARTIFACTS_APP_ID = "com.ma-zierl.kestral-artifacts";

const ARTIFACT_CAPABILITIES = ["artifacts.query", "artifacts.read"] as const;

function hasArtifactCapability(
  grants: GrantView[],
  capability: (typeof ARTIFACT_CAPABILITIES)[number],
  artifactId: string | null,
): boolean {
  const requestedDataScope = artifactId === null
    ? { kind: "all-resources" as const }
    : { kind: "resources" as const, resource_ids: [artifactId] };
  return grants.some((grant) =>
    grant.status === "active"
    && grant.holder === CHAT_APP_ID
    && scopeCovers(
      {
        kind: "exact-capability",
        provider: ARTIFACTS_APP_ID,
        capability,
      },
      grant.scope,
    )
    && dataScopeCovers(requestedDataScope, grant.data_scope)
  );
}

export function chatCanAccessArtifact(grants: GrantView[], artifactId: string): boolean {
  return ARTIFACT_CAPABILITIES.every((capability) =>
    hasArtifactCapability(grants, capability, artifactId)
  );
}

export function chatCanAccessAllArtifacts(grants: GrantView[]): boolean {
  return ARTIFACT_CAPABILITIES.every((capability) =>
    hasArtifactCapability(grants, capability, null)
  );
}
