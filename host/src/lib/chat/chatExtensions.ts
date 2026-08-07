import type { InstalledApp, SurfaceDeclaration } from "$lib/api";

export interface ResolvedChatExtension {
  app: InstalledApp;
  surface: SurfaceDeclaration;
}

/** Resolve only exact, version-compatible contributions for a declared slot. */
export function resolveChatExtensions(
  apps: InstalledApp[],
  pointName: string,
): ResolvedChatExtension[] {
  const chat = apps.find((app) => app.manifest.app_id === "chat");
  const point = chat?.manifest.extension_points.find((candidate) => candidate.name === pointName);
  if (!chat || !point) return [];

  return apps
    .flatMap((app) =>
      app.manifest.extension_contributions
        .filter(
          (contribution) =>
            contribution.target_app === chat.manifest.app_id &&
            contribution.extension_point === point.name &&
            contribution.contract_version === point.contract_version,
        )
        .map((contribution) => ({
          app,
          surface: app.manifest.surfaces.find((surface) => surface.name === contribution.surface),
        })),
    )
    .filter((extension): extension is ResolvedChatExtension => extension.surface !== undefined)
    .sort((left, right) =>
      left.app.manifest.display_name.localeCompare(right.app.manifest.display_name),
    );
}
