import { describe, expect, it } from "vitest";
import { supportsJsonSchemaForm } from "$lib/settings/jsonSchemaFormModel";

describe("required keys without property schemas", () => {
  it("uses JSON input when required keys have no scalar controls", () => {
    expect(supportsJsonSchemaForm({ type: "object", required: ["payload"] })).toBe(false);
    expect(supportsJsonSchemaForm({
      type: "object", properties: {}, required: ["payload"],
    })).toBe(false);
  });

  it("preserves truly empty forms and declared scalar controls", () => {
    expect(supportsJsonSchemaForm({ type: "object" })).toBe(true);
    expect(supportsJsonSchemaForm({ type: "object", additionalProperties: false })).toBe(true);
    expect(supportsJsonSchemaForm({
      type: "object", required: ["payload"], properties: { payload: { type: "string" } },
    })).toBe(true);
    expect(supportsJsonSchemaForm({ type: "object", properties: null })).toBe(false);
  });
});
