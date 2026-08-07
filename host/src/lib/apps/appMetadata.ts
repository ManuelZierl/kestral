import type { AppManifest, DataScope, GrantScope, GrantStatus, GrantView } from "$lib/api";
import { dataScopeLabel } from "$lib/system/dataScopeLabel";
import { scopeLabel } from "$lib/system/scopeLabel";

export interface RequestedCapabilityView {
  label: string;
  dataScopeLabel: string;
  status: GrantStatus | "missing";
}

export function scopeCovers(request: GrantScope, grant: GrantScope): boolean {
  if (grant.kind === "all-provider-capabilities") {
    return request.provider === grant.provider;
  }
  if (request.kind === "all-provider-capabilities") {
    return false;
  }
  return request.provider === grant.provider && request.capability === grant.capability;
}

export function dataScopeCovers(request: DataScope, grant: DataScope): boolean {
  if (grant.kind === "all-resources") {
    return request.kind === "all-resources" || request.kind === "resources";
  }
  if (grant.kind === "none") {
    return request.kind === "none";
  }
  if (request.kind !== "resources") {
    return false;
  }
  return request.resource_ids.every((resourceId) => grant.resource_ids.includes(resourceId));
}

function requestedCapabilities(manifest: AppManifest, grants: GrantView[]): RequestedCapabilityView[] {
  return manifest.grant_requests.map((request) => {
    const match = grants.find(
      (grant) =>
        grant.holder === manifest.app_id &&
        scopeCovers(request.scope, grant.scope) &&
        dataScopeCovers(request.data_scope, grant.data_scope),
    );
    return {
      label: scopeLabel(request.scope),
      dataScopeLabel: dataScopeLabel(request.data_scope),
      status: match?.status ?? "missing",
    };
  });
}

export function missingRequestedCapabilities(manifest: AppManifest, grants: GrantView[]): RequestedCapabilityView[] {
  return requestedCapabilities(manifest, grants).filter((request) => request.status !== "active");
}
