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
    expect(supportsJsonSchemaForm({
      type: "object",
      properties: { mode: { type: "string", enum: ["safe", "fast"] } },
    })).toBe(false);
    expect(supportsJsonSchemaForm({
      type: "object",
      properties: { label: { type: "string", pattern: "^[a-z]+$" } },
    })).toBe(false);
    expect(supportsJsonSchemaForm({
      type: "object",
      properties: { count: { type: "integer", multipleOf: 2 } },
    })).toBe(false);
  });

  it("enforces constraints represented by the shared scalar controls", () => {
    const constrained = {
      type: "object",
      properties: {
        label: { type: "string", minLength: 2, maxLength: 3 },
        count: { type: "integer", minimum: 1, maximum: 5 },
      },
      required: ["label"],
    };

    expect(() => collectJsonObject(constrained, { label: "", count: "3" }))
      .toThrow("label must be at least 2 characters");
    expect(() => collectJsonObject(constrained, { label: "four", count: "3" }))
      .toThrow("label must be at most 3 characters");
    expect(() => collectJsonObject(constrained, { label: "😀", count: "3" }))
      .toThrow("label must be at least 2 characters");
    expect(collectJsonObject(constrained, { label: "😀a", count: "3" })).toEqual({
      label: "😀a",
      count: 3,
    });
    expect(() => collectJsonObject(constrained, { label: "ok", count: "0" }))
      .toThrow("count must be at least 1");
    expect(() => collectJsonObject(constrained, { label: "ok", count: "6" }))
      .toThrow("count must be at most 5");
    expect(collectJsonObject(constrained, { label: "ok", count: "3" })).toEqual({
      label: "ok",
      count: 3,
    });
    expect(supportsJsonSchemaForm(constrained)).toBe(true);
  });

  it("distinguishes missing required strings from explicit empty values", () => {
    const requiredString = {
      type: "object",
      properties: { label: { type: "string" } },
      required: ["label"],
    };

    expect(() => collectJsonObject(requiredString, {})).toThrow("label is required");
    expect(collectJsonObject(requiredString, { label: "" })).toEqual({ label: "" });
    expect(collectJsonObject(requiredString, { label: "  keep whitespace  " })).toEqual({
      label: "  keep whitespace  ",
    });
    expect(collectJsonObject(requiredString, { label: "   " })).toEqual({ label: "   " });
    expect(collectJsonObject(requiredString, { label: "", other: "ignored" })).toEqual({
      label: "",
    });
    expect(collectJsonObject({
      type: "object",
      properties: { label: { type: "string", minLength: 2 } },
    }, { label: "" })).toEqual({});
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
