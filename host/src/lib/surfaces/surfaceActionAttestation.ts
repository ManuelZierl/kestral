import type {
  CapabilityDeclaration,
  CapabilityRef,
  DataScope,
  GrantCondition,
  GrantScope,
  GrantView,
} from "$lib/api";

function capabilityCovered(scope: GrantScope, capability: CapabilityRef): boolean {
  if (scope.kind === "exact-capability") {
    return scope.provider === capability.provider && scope.capability === capability.capability;
  }
  return scope.provider === capability.provider;
}

function dataScopeCovered(granted: DataScope, requested: DataScope): boolean {
  if (granted.kind === "none") return requested.kind === "none";
  if (granted.kind === "all-resources") {
    return requested.kind === "all-resources" || requested.kind === "resources";
  }
  if (requested.kind !== "resources") return false;
  return requested.resource_ids.every((resourceId) => granted.resource_ids.includes(resourceId));
}

/**
 * Mirror the broker's deterministic grant selection closely enough to decide
 * whether the kernel's own-surface shortcut would otherwise consume a
 * requires-approval grant without trusted chrome. The kernel remains the
 * authority: this helper never grants access and stale data can only result in
 * an extra confirmation or a later kernel refusal.
 */
export function effectiveGrantCondition(
  grants: GrantView[],
  holder: string,
  capability: CapabilityRef,
  requestedDataScope: DataScope,
): GrantCondition | null {
  const covering = grants
    .filter((grant) => grant.status === "active")
    .filter((grant) => grant.holder === holder)
    .filter((grant) => capabilityCovered(grant.scope, capability))
    .filter((grant) => dataScopeCovered(grant.data_scope, requestedDataScope))
    .sort((left, right) => {
      const issued = left.issued_at.localeCompare(right.issued_at);
      return issued !== 0 ? issued : left.grant_id.localeCompare(right.grant_id);
    });

  if (covering.some((grant) => grant.condition === "silent")) return "silent";
  if (covering.some((grant) => grant.condition === "notify")) return "notify";
  return covering.length > 0 ? "requires-approval" : null;
}

export function needsHostGestureAttestation(
  appId: string,
  capabilities: CapabilityDeclaration[],
  grants: GrantView[],
  capability: CapabilityRef,
  dataScope: DataScope,
): boolean {
  // Cross-app work, destructive work, and external effects are already driven
  // through normal trusted chrome by the kernel. The historical shortcut only
  // applies to an app invoking its own read/local-write capability.
  if (capability.provider !== appId) return false;
  const declaration = capabilities.find((candidate) => candidate.name === capability.capability);
  if (!declaration || !["read-only", "local-write"].includes(declaration.effect)) return false;

  return effectiveGrantCondition(grants, appId, capability, dataScope) === "requires-approval";
}
