import { describe, expect, it } from "vitest";
import { collectJsonObject, supportsJsonSchemaForm } from "$lib/settings/jsonSchemaFormModel";

describe("jsonSchemaFormModel", () => {
  const schema = {
    type: "object",
    properties: {
      count: { type: "integer" },
      enabled: { type: "boolean" },
      label: { type: "string" },
    },
    required: ["count", "enabled"],
  };

  it("coerces string input into typed JSON values", () => {
    expect(collectJsonObject(schema, { count: "3", enabled: "true", label: "hello" })).toEqual({
      count: 3,
      enabled: true,
      label: "hello",
    });
  });

  it("rejects schemas whose values cannot round-trip through scalar controls", () => {
    expect(supportsJsonSchemaForm(schema)).toBe(true);
    expect(supportsJsonSchemaForm({
      type: "object",
      properties: { profiles: { type: "array", items: { type: "object" } } },
    })).toBe(false);
    expect(supportsJsonSchemaForm({
      type: "object",
      properties: { prompt: { type: ["object", "null"] } },
    })).toBe(false);
    expect(supportsJsonSchemaForm({
      type: "object",
      properties: { target: { oneOf: [{ type: "string" }, { type: "object" }] } },
    })).toBe(false);
    expect(supportsJsonSchemaForm({
      oneOf: [
        { type: "object", properties: { label: { type: "string" } } },
        { type: "object", properties: { count: { type: "integer" } } },
      ],
    })).toBe(false);
  });
});
