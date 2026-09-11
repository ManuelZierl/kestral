import type { AppStatusView } from "$lib/api";

/**
 * Product onboarding is complete only when the workspace contains an active,
 * independently installed app with its own standalone custom screen. Bundled
 * host apps and extension-only surfaces are not that first focused app.
 */
export function hasUsableFocusedApp(apps: AppStatusView[]): boolean {
  return apps.some((app) =>
    !app.bundled
    && app.enabled
    && app.status === "active"
    && app.surfaces.some((surface) =>
      surface.has_custom_ui
      // Match standaloneSurfaces: dashboards remain independently launchable
      // even when the same surface also contributes to an extension point.
      && (surface.kind === "dashboard"
        || !app.extension_contributions.some((item) => item.surface === surface.name)),
    ),
  );
}
