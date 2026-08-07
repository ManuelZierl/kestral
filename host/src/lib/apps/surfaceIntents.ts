import type { CapabilityDeclaration, InstalledApp, SurfaceDeclaration } from "$lib/api";

export function capabilityForFormSurface(
  app: InstalledApp,
  surface: SurfaceDeclaration,
): CapabilityDeclaration | undefined {
  if (surface.kind !== "form" || surface.intents.length !== 1) return undefined;
  const [intent] = surface.intents;
  if (intent.provider !== app.manifest.app_id) return undefined;
  return app.manifest.capabilities.find((capability) => capability.name === intent.capability);
}
