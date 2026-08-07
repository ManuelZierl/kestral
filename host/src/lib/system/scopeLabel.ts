import type { GrantScope } from "$lib/api";

export function scopeLabel(scope: GrantScope): string {
  return scope.kind === "exact-capability"
    ? `${scope.provider}/${scope.capability}`
    : `${scope.provider}/*`;
}
