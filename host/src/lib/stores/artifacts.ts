import { get, writable } from "svelte/store";
import {
  grantArtifactAccess,
  listArtifacts,
  type Artifact,
  type ArtifactAccessTarget,
} from "$lib/api";
import { refreshGrants } from "$lib/stores/grants";

export const artifacts = writable<Artifact[]>([]);
export const artifactsLoaded = writable(false);

let nextArtifactsSequence = 0;
let appliedArtifactsSequence = 0;

async function loadArtifacts(invalidateOlder: boolean): Promise<Artifact[]> {
  const sequence = ++nextArtifactsSequence;
  if (invalidateOlder) {
    // A thread already references data absent from the current snapshot. Older
    // in-flight reads are now known stale and must not turn that gap into a
    // false "unavailable" claim if this refresh fails.
    appliedArtifactsSequence = sequence;
    artifactsLoaded.set(false);
  }
  const next = await listArtifacts();
  if (sequence < appliedArtifactsSequence) return next;
  appliedArtifactsSequence = sequence;
  artifacts.set(next);
  artifactsLoaded.set(true);
  return next;
}

export async function refreshArtifacts(): Promise<void> {
  await loadArtifacts(false);
}

export async function grantArtifactAccessAndRefresh(
  holder: string,
  target: ArtifactAccessTarget,
): Promise<void> {
  await grantArtifactAccess(holder, target);
  await refreshGrants(true);
}

export async function synchronizeArtifactReferences(artifactIds: string[]): Promise<void> {
  if (artifactIds.length === 0) return;
  const available = new Set(get(artifacts).map((artifact) => artifact.artifact_id));
  if (artifactIds.every((artifactId) => available.has(artifactId))) return;
  await loadArtifacts(true);
}
