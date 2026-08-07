import type { AppManifest, SurfaceDeclaration } from "$lib/api";

export function standaloneSurfaces(manifest: AppManifest): SurfaceDeclaration[] {
  const contributed = new Set(manifest.extension_contributions.map((item) => item.surface));
  return manifest.surfaces.filter(
    (surface) => surface.kind === "dashboard" || !contributed.has(surface.name),
  );
}
