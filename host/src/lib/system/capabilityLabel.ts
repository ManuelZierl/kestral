import type { CapabilityRef } from "$lib/api";

export function capabilityLabel(capability: CapabilityRef): string {
  return `${capability.provider}/${capability.capability}`;
}
