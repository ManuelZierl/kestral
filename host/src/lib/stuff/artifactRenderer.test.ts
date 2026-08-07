import { describe, expect, it } from "vitest";
import { artifactPreview } from "$lib/stuff/artifactRenderer";

describe("artifactPreview", () => {
  it("prefers readable note text over raw JSON", () => {
    expect(
      artifactPreview({
        artifact_id: "artifact-1",
        artifact_type: "note",
        title: "Note",
        content: { text: "hello" },
        provenance: {
          run_id: "run-1",
          capability: { provider: "notes", capability: "create_note" },
          grant_id: "grant-1",
          produced_by: "notes",
          recorded_at: new Date().toISOString(),
        },
      }),
    ).toBe("hello");
  });
});
