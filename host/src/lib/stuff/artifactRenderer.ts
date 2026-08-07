import type { Artifact } from "$lib/api";

export function readableArtifactPreview(content: Artifact["content"]): string {
  if (typeof content === "string") {
    return content;
  }
  if (content && typeof content === "object" && !Array.isArray(content)) {
    const object = content as Record<string, unknown>;
    const readable = [object.title, object.body, object.text, object.summary, object.note].find(
      (value) => typeof value === "string" && value.trim() !== "",
    );
    if (typeof readable === "string") {
      return readable;
    }
  }
  return JSON.stringify(content, null, 2);
}

export function artifactPreview(artifact: Artifact): string {
  return readableArtifactPreview(artifact.content);
}
