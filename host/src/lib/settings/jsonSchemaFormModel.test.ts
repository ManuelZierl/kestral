import { describe, expect, it } from "vitest";
import {
  coerceFieldValue,
  collectJsonObject,
  parseJsonObjectInput,
  supportsJsonSchemaForm,
} from "$lib/settings/jsonSchemaFormModel";

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
    expect(supportsJsonSchemaForm({
      type: "object",
      properties: { label: { type: "string" } },
      required: ["missing"],
    })).toBe(false);
    expect(supportsJsonSchemaForm({
      type: "object",
      properties: { label: { type: "string", not: { const: "reserved" } } },
    })).toBe(false);
    expect(supportsJsonSchemaForm({
      type: "object",
      const: { mode: "safe" },
    })).toBe(false);
    expect(supportsJsonSchemaForm({
      type: "object",
      enum: [{ mode: "safe" }, { mode: "fast" }],
    })).toBe(false);
    expect(supportsJsonSchemaForm({
      type: "object",
      minProperties: 1,
    })).toBe(false);
    expect(supportsJsonSchemaForm({
      type: "object",
      patternProperties: { "^field-": { type: "string" } },
    })).toBe(false);
    expect(supportsJsonSchemaForm({
      type: "object",
      additionalProperties: { type: "string" },
    })).toBe(false);
  });

  it("parses structured input without flattening nested JSON values", () => {
    expect(parseJsonObjectInput(`{
      "profiles": [{ "name": "primary", "tags": ["work", "local"] }],
      "options": { "retries": 2 },
      "enabled": true
    }`)).toEqual({
      profiles: [{ name: "primary", tags: ["work", "local"] }],
      options: { retries: 2 },
      enabled: true,
    });
  });

  it("rejects malformed JSON and non-object roots before invocation", () => {
    expect(() => parseJsonObjectInput("{not json}")).toThrow("Enter valid JSON.");
    expect(() => parseJsonObjectInput("null")).toThrow("Input must be a JSON object.");
    expect(() => parseJsonObjectInput("[1, 2]")).toThrow("Input must be a JSON object.");
  });

  it("rejects non-finite and unsafe integer values before they cross the bridge", () => {
    expect(parseJsonObjectInput(`{
      "safe": ${Number.MAX_SAFE_INTEGER},
      "nested": [{ "ratio": 0.25 }]
    }`)).toEqual({
      safe: Number.MAX_SAFE_INTEGER,
      nested: [{ ratio: 0.25 }],
    });
    expect(() => parseJsonObjectInput('{"unsafe": 9007199254740993}'))
      .toThrow("input.unsafe contains an integer outside JavaScript's safe range.");
    expect(() => parseJsonObjectInput('{"overflow": 1e309}'))
      .toThrow("input.overflow contains a non-finite number.");
    expect(coerceFieldValue("integer", String(Number.MAX_SAFE_INTEGER)))
      .toBe(Number.MAX_SAFE_INTEGER);
    expect(() => coerceFieldValue("integer", "9007199254740992"))
      .toThrow("must be a safe integer");
    expect(() => coerceFieldValue("number", "1e309"))
      .toThrow("must be a finite number");
    expect(() => coerceFieldValue("number", "9007199254740992"))
      .toThrow("must be within JavaScript's safe integer range");
  });
});
