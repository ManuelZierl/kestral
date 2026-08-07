// Turns the kernel's immutable grant facts into the view people reason about:
// one row per app + allowed action, showing the current state, with superseded
// facts kept as history instead of sibling rows. The kernel model (grants are
// append-only audit facts) stays intact underneath — this is presentation
// grouping only, kept pure for unit testing.

import type {
  DataScope,
  GrantCondition,
  GrantRequest,
  GrantScope,
  GrantView,
  InstalledApp,
} from "$lib/api";
import { scopeLabel } from "$lib/system/scopeLabel";
import { dataScopeLabel } from "$lib/system/dataScopeLabel";
import { dataScopeCovers, scopeCovers } from "$lib/apps/appMetadata";

/** One capability-scope the app holds (or held) a permission for. */
export interface PermissionEntry {
  scope: GrantScope;
  dataScope: DataScope;
  /** The fact the row represents: the newest active grant, else the newest fact. */
  current: GrantView;
  /** Older facts for the same scope, newest first. */
  history: GrantView[];
  /** The app's manifest request for this scope, when it declares one. */
  declared: GrantRequest | null;
}

export interface AppPermissionGroup {
  appId: string;
  displayName: string;
  entries: PermissionEntry[];
  /** Manifest-declared requests with no grant fact at all yet. */
  neverGranted: GrantRequest[];
}

export function scopeKey(scope: GrantScope, dataScope: DataScope): string {
  return `${scopeLabel(scope)} :: ${dataScopeLabel(dataScope)}`;
}

export function conditionLabel(condition: GrantCondition): string {
  switch (condition) {
    case "silent":
      return "Runs silently";
    case "notify":
      return "Notifies you";
    case "requires-approval":
      return "Asks for approval";
  }
}

function newestFirst(left: GrantView, right: GrantView): number {
  return right.issued_at.localeCompare(left.issued_at) || left.grant_id.localeCompare(right.grant_id);
}

export function groupPermissions(
  grants: GrantView[],
  apps: InstalledApp[],
): AppPermissionGroup[] {
  // Only currently installed apps are manageable: a grant whose holder or
  // provider was uninstalled is a dead fact (the kernel already revoked it)
  // and belongs in the audit log, not in the management list.
  const installed = new Set(apps.map((app) => app.manifest.app_id));
  const visible = grants.filter(
    (grant) => installed.has(grant.holder) && installed.has(grant.scope.provider),
  );

  // holder -> scope key -> facts
  const byHolder = new Map<string, Map<string, GrantView[]>>();
  const holderNames = new Map<string, string>();
  for (const grant of visible) {
    holderNames.set(grant.holder, grant.holder_display_name);
    let scopes = byHolder.get(grant.holder);
    if (!scopes) {
      scopes = new Map();
      byHolder.set(grant.holder, scopes);
    }
    const key = scopeKey(grant.scope, grant.data_scope);
    const facts = scopes.get(key);
    if (facts) facts.push(grant);
    else scopes.set(key, [grant]);
  }

  const declaredByHolder = new Map<string, GrantRequest[]>();
  for (const app of apps) {
    holderNames.set(app.manifest.app_id, app.manifest.display_name);
    declaredByHolder.set(app.manifest.app_id, app.manifest.grant_requests);
  }

  const holders = new Set<string>([...byHolder.keys(), ...declaredByHolder.keys()]);
  const groups: AppPermissionGroup[] = [];
  for (const holder of holders) {
    const scopes = byHolder.get(holder) ?? new Map<string, GrantView[]>();
    const declared = declaredByHolder.get(holder) ?? [];

    const entries: PermissionEntry[] = [...scopes.entries()].map(([key, facts]) => {
      const ordered = [...facts].sort(newestFirst);
      const current = ordered.find((fact) => fact.status === "active") ?? ordered[0];
      return {
        scope: current.scope,
        dataScope: current.data_scope,
        current,
        history: ordered.filter((fact) => fact !== current),
        declared: declared.find((request) => scopeKey(request.scope, request.data_scope) === key) ?? null,
      };
    }).filter((entry) =>
      entry.current.status === "active" ||
      entry.declared !== null ||
      entry.current.origin === "user-added"
    );
    entries.sort((left, right) =>
      scopeKey(left.scope, left.dataScope).localeCompare(scopeKey(right.scope, right.dataScope)),
    );

    const neverGranted = declared.filter(
      (request) =>
        !scopes.has(scopeKey(request.scope, request.data_scope)) &&
        !visible.some(
          (grant) =>
            grant.holder === holder &&
            grant.status === "active" &&
            scopeCovers(request.scope, grant.scope) &&
            dataScopeCovers(request.data_scope, grant.data_scope),
        ) &&
        installed.has(request.scope.provider),
    );

    if (entries.length === 0 && neverGranted.length === 0) continue;
    groups.push({
      appId: holder,
      displayName: holderNames.get(holder) ?? holder,
      entries,
      neverGranted,
    });
  }

  groups.sort(
    (left, right) =>
      left.displayName.localeCompare(right.displayName) || left.appId.localeCompare(right.appId),
  );
  return groups;
}
