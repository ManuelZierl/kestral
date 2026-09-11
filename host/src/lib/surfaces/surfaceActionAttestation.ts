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
 * Mirror the broker's effective condition for diagnostics and tests. The
 * broker prefers silent, then notify, then requires-approval among active
 * covering grants. This helper is deliberately not used as a security gate:
 * frontend grant state can change before kernel preparation.
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
    .filter((grant) => dataScopeCovered(grant.data_scope, requestedDataScope));

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
 * The grants and data-scope parameters remain in the signature because the
 * caller already has that contract and the diagnostic helper uses the same
 * inputs. They are intentionally not consulted for this security decision.
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
