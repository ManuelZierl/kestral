import type { AppStatusView } from "$lib/api";

/**
 * Product onboarding is complete only when the workspace contains an active,
 * independently installed app with its own custom screen. Bundled host apps
 * keep Kestral operational but do not prove that the user has made the host
 * useful for one focused job.
 */
export function hasUsableFocusedApp(apps: AppStatusView[]): boolean {
  return apps.some((app) =>
    !app.bundled
    && app.enabled
    && app.status === "active"
    && app.surfaces.some((surface) => surface.has_custom_ui),
  );
}
