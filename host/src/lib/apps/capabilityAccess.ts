import type { CapabilityUseView, GrantCondition } from "$lib/api";

export interface CapabilityAccessState {
  available: boolean;
  grantCondition: GrantCondition | null;
}

export function capabilityAccessState(
  availableCapabilities: CapabilityUseView[],
  provider: string,
  capability: string,
): CapabilityAccessState {
  const match = availableCapabilities.find(
    (item) => item.provider_app_id === provider && item.capability === capability,
  );
  return {
    available: match !== undefined,
    grantCondition: match ? mostInteractiveCondition(match) : null,
  };
}

export function mostInteractiveCondition(view: CapabilityUseView): GrantCondition {
  if (view.authorizations.some((entry) => entry.condition === "requires-approval")) {
    return "requires-approval";
  }
  if (view.authorizations.some((entry) => entry.condition === "notify")) return "notify";
  return "silent";
}

export function capabilityAccessBadge(condition: GrantCondition | null): string | null {
  if (condition === null) return null;
  if (condition === "requires-approval") return "Requires approval";
  if (condition === "notify") return "Notifies on use";
  return "Allowed";
}

export function missingCapabilityWarning(provider: string, capability: string): string {
  return `${humanizeCapability(provider, capability)} isn't allowed right now. Enable it in Settings → Permissions.`;
}

function humanizeCapability(provider: string, capability: string): string {
  const action = capability.replaceAll("_", " ").replaceAll("-", " ");
  const app = provider.charAt(0).toUpperCase() + provider.slice(1);
  return `${app}: ${action}`;
}
