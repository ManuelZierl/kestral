import type { JsonObject, JsonValue } from "$lib/api";

export interface JsonSchemaField {
  name: string;
  title: string;
  type: string;
  input: "single-line" | "multiline";
  description: string;
  required: boolean;
  maxLength?: number;
  minimum?: number;
  maximum?: number;
}

interface PropertySchema {
  type?: string;
  title?: string;
  description?: string;
  maxLength?: number;
  minimum?: number;
  maximum?: number;
  "x-kestral-input"?: string;
}

const supportedFieldTypes = new Set(["string", "integer", "number", "boolean"]);
const schemaCompositionKeywords = ["$ref", "allOf", "anyOf", "oneOf", "if", "then", "else"];

function usesSchemaComposition(schema: Record<string, unknown>): boolean {
  return schemaCompositionKeywords.some((keyword) => keyword in schema);
}

export function supportsJsonSchemaForm(schema: JsonObject): boolean {
  if (schema.type !== "object" || usesSchemaComposition(schema)) return false;
  const properties = schema.properties;
  if (properties === undefined) return true;
  if (typeof properties !== "object" || properties === null || Array.isArray(properties)) {
    return false;
  }
  return Object.values(properties).every((property) => {
    if (typeof property !== "object" || property === null || Array.isArray(property)) return false;
    const propertySchema = property as PropertySchema & Record<string, unknown>;
    return !usesSchemaComposition(propertySchema)
      && typeof propertySchema.type === "string"
      && supportedFieldTypes.has(propertySchema.type);
  });
}

export function schemaFields(schema: JsonObject): JsonSchemaField[] {
  const properties = (schema.properties ?? {}) as Record<string, PropertySchema>;
  const required = new Set(((schema.required ?? []) as JsonValue[]).map(String));
  return Object.entries(properties).map(([name, property]) => ({
    name,
    title: property.title ?? name,
    type: property.type ?? "string",
    input: property["x-kestral-input"] === "multiline" ? "multiline" : "single-line",
    description: property.description ?? "",
    required: required.has(name),
    maxLength: property.maxLength,
    minimum: property.minimum,
    maximum: property.maximum,
  }));
}

export function toInputValue(value: JsonValue | undefined): string {
  if (value === undefined || value === null) return "";
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") return String(value);
  if (typeof value === "string") return value;
  return JSON.stringify(value);
}

export function coerceFieldValue(type: string, rawValue: string): JsonValue {
  const value = rawValue.trim();
  if (type === "integer") {
    const parsed = Number(value);
    if (!Number.isInteger(parsed)) throw new Error("must be an integer");
    return parsed;
  }
  if (type === "number") {
    const parsed = Number(value);
    if (Number.isNaN(parsed)) throw new Error("must be a number");
    return parsed;
  }
  if (type === "boolean") {
    if (value !== "true" && value !== "false") {
      throw new Error("must be true or false");
    }
    return value === "true";
  }
  return rawValue;
}

export function collectJsonObject(
  schema: JsonObject,
  values: Record<string, string>,
): JsonObject {
  const collected: JsonObject = {};
  for (const field of schemaFields(schema)) {
    const raw = values[field.name] ?? "";
    if (raw.trim() === "") {
      if (field.required) {
        throw new Error(`${field.name} is required`);
      }
      continue;
    }
    try {
      collected[field.name] = coerceFieldValue(field.type, raw);
    } catch (error) {
      throw new Error(`${field.name} ${String((error as Error).message)}`);
    }
  }
  return collected;
}
