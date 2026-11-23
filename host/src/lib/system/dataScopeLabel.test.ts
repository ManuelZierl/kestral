import { describe, expect, it } from "vitest";

import { dataScopeLabel } from "$lib/system/dataScopeLabel";

describe("dataScopeLabel", () => {
  it("names the unscoped form without implying broad resource access", () => {
    expect(dataScopeLabel({ kind: "none" })).toBe("Not tied to a registered resource");
  });

  it("lists resource ids for scoped grants", () => {
    expect(
      dataScopeLabel({ kind: "resources", resource_ids: ["note-1", "note-2"] }),
    ).toBe("Resources: note-1, note-2");
  });

  it("makes all current and future resources explicit", () => {
    expect(dataScopeLabel({ kind: "all-resources" })).toBe(
      "All current and future resources",
    );
  });

  it("formats LLM zone and profile resource scopes clearly", () => {
    expect(
      dataScopeLabel({
        kind: "resources",
        resource_ids: ["llm-zone:local", "llm-profile:llm-provider/local-ollama"],
      }),
    ).toBe(
      "Resources: Local LLM models (llm-zone:local), LLM profile llm-provider/local-ollama (llm-profile:llm-provider/local-ollama)",
    );
    expect(
      dataScopeLabel({
        kind: "resources",
        resource_ids: ["llm-zone:cloud"],
      }),
    ).toBe("Resources: Cloud LLM models (llm-zone:cloud)");
  });
});
