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
 * Mirror the broker's deterministic grant selection for diagnostics and tests.
 * This helper is not used as a security decision: grant state can change
 * between a frontend read and kernel preparation, so a frontend snapshot must
 * never decide whether a custom surface needs physical user attestation.
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

/**
 * A sandboxed custom surface cannot attest that an invoke request came from a
 * human gesture. Until the kernel owns an attestation token (or removes its
 * direct-provider surface exemption), require host-owned confirmation for
 * every own-provider read/local-write invoke. This is intentionally stricter
 * than the standing grant condition: consulting grants here creates a TOCTOU
 * gap where authority can change after the frontend check but before kernel
 * preparation.
 *
 * Cross-app, external-write, destructive and unspecified effects are already
 * routed through the kernel's normal trusted-chrome policy and must not gain a
 * parallel frontend approval path.
 */
export function needsHostGestureAttestation(
  appId: string,
  capabilities: CapabilityDeclaration[],
  _grants: GrantView[],
  capability: CapabilityRef,
  _dataScope: DataScope,
): boolean {
  if (capability.provider !== appId) return false;
  const declaration = capabilities.find((candidate) => candidate.name === capability.capability);
  return declaration !== undefined && ["read-only", "local-write"].includes(declaration.effect);
}
