import type { DataScope } from "$lib/api";

export function dataScopeLabel(scope: DataScope): string {
  if (scope.kind === "none") {
    return "Not tied to a registered resource";
  }
  if (scope.kind === "all-resources") {
    return "All current and future resources";
  }
  return `Resources: ${scope.resource_ids.join(", ")}`;
}
