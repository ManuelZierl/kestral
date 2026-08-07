// The versioned message contract between a sandboxed app surface (untrusted,
// runs in an opaque-origin iframe) and the host (trusted parent). This module
// is pure and transport-agnostic: it defines the wire shapes and the parsing
// / validation that the host applies to every inbound message. The host side
// that wires this to a real window is `hostSurfaceBridge.ts`.
//
// Threat model: the frame is hostile. Nothing it sends is trusted until it is
// validated here. The frame never supplies the app identity — the host binds
// that from the SurfaceBinding it opened — so the frame cannot act as another
// app. Surfaces are intent-only and remain outside trusted chrome.

import type {
  ActionIntent,
  Artifact,
  CapabilityRef,
  DataScope,
  JsonObject,
  JsonValue,
  ManagedDataMutation,
  ManagedDataQuery,
  ManagedDataRequest,
  ManagedDataV2Read,
  ManagedDataV2Request,
  ManagedDocumentOperation,
} from "$lib/api";
import type { ThemeId } from "$lib/design/colors";

/// Wire identifier; a message missing this exact tag is not ours and is
/// ignored. Distinguishes bridge traffic from any other postMessage noise.
export const SURFACE_BRIDGE_PROTOCOL = "app-host-surface-bridge";

/// Bump on any breaking change to the message shapes below. The host refuses a
/// message whose version it does not understand. Keep in lockstep with
/// `SURFACE_BRIDGE_VERSION` in `host/src-tauri/src/surface_ui.rs`.
export const SURFACE_BRIDGE_VERSION = 3;
export const SURFACE_DATA_API_VERSION = 1;
export const MAX_SURFACE_MESSAGE_CHARS = 1024 * 1024;
export const MAX_SURFACE_TEXT_CHARS = 64 * 1024;
const MAX_SURFACE_IDENTIFIER_CHARS = 256;
const MAX_SURFACE_RESOURCE_IDS = 256;
const MAX_MANAGED_DATA_CHUNK_BASE64_CHARS = 512 * 1024;

// -- Host → app (trusted → untrusted) ----------------------------------------

/// Sent once, first, to hand the frame its identity and static context. The
/// frame must echo `instanceId` on everything it sends back.
export interface SurfaceInitMessage {
  protocol: typeof SURFACE_BRIDGE_PROTOCOL;
  v: number;
  type: "init";
  instanceId: string;
  appId: string;
  surface: string;
  /// Capabilities this surface is allowed to invoke (its declared intents).
  capabilities: CapabilityRef[];
  /// The app's declared config JSON Schema (or null if none), for the frame
  /// to render a settings form. Authoritative validation stays host-side.
  configSchema: JsonObject | null;
  config: JsonObject;
  /** Resolved host appearance; sandboxed frames cannot inherit host CSS. */
  theme: ThemeId;
  /** Host and app semantic CSS variables resolved for this surface. */
  variables: Record<string, string>;
  /** Host-validated, slot-specific data; it never originates in the frame. */
  extensionContext: JsonObject;
  /** Bounded app-specific read context assembled by the trusted host. */
  hostContext: JsonObject;
}

export interface SurfaceResponseMessage {
  protocol: typeof SURFACE_BRIDGE_PROTOCOL;
  v: number;
  type: "response";
  requestId: number;
  ok: boolean;
  /// Present when ok; the op's result value.
  result?: JsonValue;
  /// Present when !ok; a human-readable reason.
  error?: string;
}

/// Pushed when one of the app's own minimized events occurs, so a surface can
/// refresh without polling. Carries no payload — the frame re-reads via ops.
export interface SurfaceEventMessage {
  protocol: typeof SURFACE_BRIDGE_PROTOCOL;
  v: number;
  type: "event";
}

export interface SurfaceProgressMessage {
  protocol: typeof SURFACE_BRIDGE_PROTOCOL;
  v: number;
  type: "progress";
  requestId: number;
  value: JsonValue;
}

export interface SurfaceThemeMessage {
  protocol: typeof SURFACE_BRIDGE_PROTOCOL;
  v: number;
  type: "theme";
  theme: ThemeId;
  variables: Record<string, string>;
}

/// Slot-specific message from the extension point's owning app (e.g. Chat) to
/// a contributed surface. The bridge is a dumb pipe here: the payload's
/// meaning is defined by the extension point's contract, not by the bridge.
/// Additive within the current protocol version — the SDK is host-injected,
/// so host and frame can never disagree about whether this type exists.
export interface SurfaceExtensionEventMessage {
  protocol: typeof SURFACE_BRIDGE_PROTOCOL;
  v: number;
  type: "extension-event";
  payload: JsonObject;
}

export type HostToAppMessage =
  | SurfaceInitMessage
  | SurfaceResponseMessage
  | SurfaceEventMessage
  | SurfaceProgressMessage
  | SurfaceThemeMessage
  | SurfaceExtensionEventMessage;

// -- App → host (untrusted → trusted) ----------------------------------------

/// The closed set of operations a surface may ask the host to perform. Every
/// op is mediated: invocations go through the grant-checked action path, reads
/// are scoped to the app's own data. There is deliberately no filesystem,
/// secret, raw-ledger, or cross-app operation.
export type SurfaceOp =
  | {
      kind: "invoke";
      capability: CapabilityRef;
      input: JsonObject;
      data_scope: DataScope;
      goal: string;
    }
  | { kind: "cancel-run"; runId: string }
  | { kind: "get-config" }
  | { kind: "update-config"; config: JsonObject }
  | { kind: "get-state"; key: string }
  | {
      kind: "put-state";
      key: string;
      expectedRevision: number;
      value: JsonObject | null;
    }
  | { kind: "data-v1"; request: ManagedDataRequest }
  | { kind: "data-v2"; request: ManagedDataV2Request }
  | { kind: "list-artifacts" }
  | { kind: "list-events" };

export type SurfaceOpKind = SurfaceOp["kind"];

export interface SurfaceRequestMessage {
  protocol: typeof SURFACE_BRIDGE_PROTOCOL;
  v: number;
  type: "request";
  instanceId: string;
  requestId: number;
  op: SurfaceOp;
}

export interface SurfaceReadyMessage {
  protocol: typeof SURFACE_BRIDGE_PROTOCOL;
  v: number;
  type: "ready";
  instanceId: string;
}

export interface SurfaceErrorMessage {
  protocol: typeof SURFACE_BRIDGE_PROTOCOL;
  v: number;
  type: "error";
  instanceId: string;
  message: string;
}

/// Reports the frame's rendered content height so the host can size the iframe
/// to fit instead of guessing a fixed height. Advisory: a frame may send it
/// whenever its content changes, and the host decides whether to resize.
export interface SurfaceResizeMessage {
  protocol: typeof SURFACE_BRIDGE_PROTOCOL;
  v: number;
  type: "resize";
  instanceId: string;
  /// Content height in CSS pixels. Host clamps it to a sane range.
  height: number;
}

/// Slot-specific state published by a contributed surface to the app that
/// owns its extension point (the reverse direction of
/// `SurfaceExtensionEventMessage`). The payload is untrusted: the extension
/// point owner must validate it against its own contract before acting on it.
export interface SurfaceExtensionStateMessage {
  protocol: typeof SURFACE_BRIDGE_PROTOCOL;
  v: number;
  type: "extension-state";
  instanceId: string;
  payload: JsonObject;
}

export type AppToHostMessage =
  | SurfaceRequestMessage
  | SurfaceReadyMessage
  | SurfaceErrorMessage
  | SurfaceResizeMessage
  | SurfaceExtensionStateMessage;

// -- Parsing / validation (payload-schema check) -----------------------------

export type ParseResult =
  | { ok: true; message: AppToHostMessage }
  | { ok: false; reason: string };

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isPlainJsonObject(value: unknown): value is JsonObject {
  return isObject(value);
}

function hasOnlyKeys(value: Record<string, unknown>, keys: string[]): boolean {
  const allowed = new Set(keys);
  return Object.keys(value).every((key) => allowed.has(key));
}

function isManagedDataName(value: unknown): value is string {
  return typeof value === "string" && /^[a-z][a-z0-9-]{0,63}$/.test(value);
}

function isManagedDataId(value: unknown): value is string {
  return typeof value === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value);
}

function isRevision(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0;
}

function parseManagedDataQuery(value: unknown): ManagedDataQuery | null {
  if (!isObject(value) || !hasOnlyKeys(value, ["index", "equals", "after", "limit"])) return null;
  if (value.index !== undefined && !isManagedDataName(value.index)) return null;
  if (value.after !== undefined && !isManagedDataId(value.after)) return null;
  if (value.limit !== undefined &&
    (typeof value.limit !== "number" || !Number.isSafeInteger(value.limit) || value.limit < 1 || value.limit > 1000)) {
    return null;
  }
  if ((value.index === undefined) !== (value.equals === undefined)) return null;
  return {
    ...(value.index === undefined ? {} : { index: value.index as string }),
    ...(value.equals === undefined ? {} : { equals: value.equals as JsonValue }),
    ...(value.after === undefined ? {} : { after: value.after as string }),
    ...(value.limit === undefined ? {} : { limit: value.limit as number }),
  };
}

function parseManagedDataMutation(value: unknown): ManagedDataMutation | null {
  if (!isObject(value) || typeof value.kind !== "string" || !isManagedDataName(value.collection)) return null;
  switch (value.kind) {
    case "create":
      if (!hasOnlyKeys(value, ["kind", "collection", "value"]) || !isPlainJsonObject(value.value)) return null;
      return { kind: "create", collection: value.collection, value: value.value };
    case "replace":
      if (!hasOnlyKeys(value, ["kind", "collection", "id", "expectedRevision", "value"]) ||
        !isManagedDataId(value.id) || !isRevision(value.expectedRevision) || !isPlainJsonObject(value.value)) return null;
      return {
        kind: "replace",
        collection: value.collection,
        id: value.id,
        expectedRevision: value.expectedRevision,
        value: value.value,
      };
    case "delete":
      if (!hasOnlyKeys(value, ["kind", "collection", "id", "expectedRevision"]) ||
        !isManagedDataId(value.id) || !isRevision(value.expectedRevision)) return null;
      return {
        kind: "delete",
        collection: value.collection,
        id: value.id,
        expectedRevision: value.expectedRevision,
      };
    default:
      return null;
  }
}

function parseManagedDataRequest(value: unknown): ManagedDataRequest | null {
  if (!isObject(value) || typeof value.kind !== "string") return null;
  if (value.kind === "transaction") {
    if (!hasOnlyKeys(value, ["kind", "operations"]) || !Array.isArray(value.operations) ||
      value.operations.length < 1 || value.operations.length > 64) return null;
    const operations = value.operations.map(parseManagedDataMutation);
    return operations.every((operation) => operation !== null)
      ? { kind: "transaction", operations: operations as ManagedDataMutation[] }
      : null;
  }
  if (!isManagedDataName(value.collection)) return null;
  switch (value.kind) {
    case "get":
      return hasOnlyKeys(value, ["kind", "collection", "id"]) && isManagedDataId(value.id)
        ? { kind: "get", collection: value.collection, id: value.id }
        : null;
    case "list": {
      if (!hasOnlyKeys(value, ["kind", "collection", "query"])) return null;
      if (value.query === undefined) return { kind: "list", collection: value.collection };
      const query = parseManagedDataQuery(value.query);
      return query === null ? null : { kind: "list", collection: value.collection, query };
    }
    case "create":
    case "replace":
    case "delete":
      return parseManagedDataMutation(value);
    default:
      return null;
  }
}

function isMutationId(value: unknown): value is string {
  return typeof value === "string" && /^[A-Za-z0-9_.-]{1,128}$/.test(value);
}

function isStageId(value: unknown): value is string {
  return typeof value === "string" && /^[A-Za-z0-9_.-]{1,64}$/.test(value);
}

function isGeneration(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isManagedContentHash(value: unknown): value is string {
  return typeof value === "string" && /^sha256-[0-9a-f]{64}$/.test(value);
}

function isBase64Chunk(value: unknown): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= MAX_MANAGED_DATA_CHUNK_BASE64_CHARS && value.length % 4 === 0 && /^[A-Za-z0-9+/]+={0,2}$/.test(value);
}

function optionalGeneration(value: Record<string, unknown>): number | undefined | null {
  if (value.expectedGeneration === undefined) return undefined;
  return isGeneration(value.expectedGeneration) ? value.expectedGeneration : null;
}

function parseManagedDocumentOperation(value: unknown): ManagedDocumentOperation | null {
  if (!isObject(value) || typeof value.kind !== "string" || !isManagedDataName(value.collection)) return null;
  if (value.kind === "create" && hasOnlyKeys(value, ["kind", "stageId", "collection", "metadata", "contentLength", "contentSha256"]) &&
    isStageId(value.stageId) && isPlainJsonObject(value.metadata) && typeof value.contentLength === "number" && Number.isSafeInteger(value.contentLength) && value.contentLength >= 0 && value.contentLength <= 8 * 1024 * 1024 && isManagedContentHash(value.contentSha256)) {
    return { kind: "create", stageId: value.stageId, collection: value.collection, metadata: value.metadata, contentLength: value.contentLength, contentSha256: value.contentSha256 };
  }
  if (value.kind === "replace" && hasOnlyKeys(value, ["kind", "stageId", "collection", "id", "expectedRevision", "metadata", "contentLength", "contentSha256"]) &&
    isStageId(value.stageId) && isManagedDataId(value.id) && isRevision(value.expectedRevision) && isPlainJsonObject(value.metadata) && typeof value.contentLength === "number" && Number.isSafeInteger(value.contentLength) && value.contentLength >= 0 && value.contentLength <= 8 * 1024 * 1024 && isManagedContentHash(value.contentSha256)) {
    return { kind: "replace", stageId: value.stageId, collection: value.collection, id: value.id, expectedRevision: value.expectedRevision, metadata: value.metadata, contentLength: value.contentLength, contentSha256: value.contentSha256 };
  }
  if (value.kind === "update-metadata" && hasOnlyKeys(value, ["kind", "collection", "id", "expectedRevision", "metadata"]) &&
    isManagedDataId(value.id) && isRevision(value.expectedRevision) && isPlainJsonObject(value.metadata)) {
    return { kind: "update-metadata", collection: value.collection, id: value.id, expectedRevision: value.expectedRevision, metadata: value.metadata };
  }
  if (value.kind === "delete" && hasOnlyKeys(value, ["kind", "collection", "id", "expectedRevision"]) && isManagedDataId(value.id) && isRevision(value.expectedRevision)) {
    return { kind: "delete", collection: value.collection, id: value.id, expectedRevision: value.expectedRevision };
  }
  return null;
}

function parseManagedDataV2Read(value: unknown): ManagedDataV2Read | null {
  if (!isObject(value) || typeof value.kind !== "string" || !isManagedDataName(value.collection)) return null;
  switch (value.kind) {
    case "record-get":
      return hasOnlyKeys(value, ["kind", "collection", "id"]) && isManagedDataId(value.id)
        ? { kind: "record-get", collection: value.collection, id: value.id }
        : null;
    case "record-list": {
      if (!hasOnlyKeys(value, ["kind", "collection", "query"])) return null;
      const query = value.query === undefined ? undefined : parseManagedDataQuery(value.query);
      return value.query !== undefined && query === null
        ? null
        : { kind: "record-list", collection: value.collection, ...(query == null ? {} : { query }) };
    }
    case "document-get":
      return hasOnlyKeys(value, ["kind", "collection", "id"]) && isManagedDataId(value.id)
        ? { kind: "document-get", collection: value.collection, id: value.id }
        : null;
    case "document-list":
      return hasOnlyKeys(value, ["kind", "collection", "after", "limit"]) &&
        (value.after === undefined || isManagedDataId(value.after)) &&
        (value.limit === undefined || (typeof value.limit === "number" && Number.isSafeInteger(value.limit) && value.limit >= 1 && value.limit <= 100))
        ? { kind: "document-list", collection: value.collection, ...(value.after === undefined ? {} : { after: value.after }), ...(value.limit === undefined ? {} : { limit: value.limit }) }
        : null;
    case "document-content":
      return hasOnlyKeys(value, ["kind", "collection", "id", "offset", "length"]) && isManagedDataId(value.id) && isGeneration(value.offset) && typeof value.length === "number" && Number.isSafeInteger(value.length) && value.length >= 0 && value.length <= 384 * 1024
        ? { kind: "document-content", collection: value.collection, id: value.id, offset: value.offset, length: value.length }
        : null;
    default:
      return null;
  }
}

function parseManagedDataV2Request(value: unknown): ManagedDataV2Request | null {
  if (!isObject(value) || typeof value.kind !== "string") return null;
  const generation = optionalGeneration(value);
  if (generation === null) return null;
  switch (value.kind) {
    case "read-snapshot": {
      if (!hasOnlyKeys(value, ["kind", "expectedGeneration", "reads"]) || !Array.isArray(value.reads) || value.reads.length < 1 || value.reads.length > 64) return null;
      const reads = value.reads.map(parseManagedDataV2Read);
      return reads.every((read) => read !== null)
        ? { kind: "read-snapshot", ...(generation === undefined ? {} : { expectedGeneration: generation }), reads: reads as ManagedDataV2Read[] }
        : null;
    }
    case "get":
      return hasOnlyKeys(value, ["kind", "collection", "id", "expectedGeneration"]) && isManagedDataName(value.collection) && isManagedDataId(value.id)
        ? { kind: "get", collection: value.collection, id: value.id, ...(generation === undefined ? {} : { expectedGeneration: generation }) }
        : null;
    case "list": {
      if (!hasOnlyKeys(value, ["kind", "collection", "query", "expectedGeneration"]) || !isManagedDataName(value.collection)) return null;
      const query = value.query === undefined ? undefined : parseManagedDataQuery(value.query);
      return value.query !== undefined && query === null ? null : { kind: "list", collection: value.collection, ...(query == null ? {} : { query }), ...(generation === undefined ? {} : { expectedGeneration: generation }) };
    }
    case "get-document":
      return hasOnlyKeys(value, ["kind", "collection", "id", "offset", "length", "expectedGeneration"]) && isManagedDataName(value.collection) && isManagedDataId(value.id) && isGeneration(value.offset) && typeof value.length === "number" && Number.isSafeInteger(value.length) && value.length >= 0 && value.length <= 384 * 1024
        ? { kind: "get-document", collection: value.collection, id: value.id, offset: value.offset, length: value.length, ...(generation === undefined ? {} : { expectedGeneration: generation }) }
        : null;
    case "list-documents":
      return hasOnlyKeys(value, ["kind", "collection", "after", "limit", "expectedGeneration"]) && isManagedDataName(value.collection) && (value.after === undefined || isManagedDataId(value.after)) && (value.limit === undefined || (typeof value.limit === "number" && Number.isSafeInteger(value.limit) && value.limit >= 1 && value.limit <= 100))
        ? { kind: "list-documents", collection: value.collection, ...(value.after === undefined ? {} : { after: value.after }), ...(value.limit === undefined ? {} : { limit: value.limit }), ...(generation === undefined ? {} : { expectedGeneration: generation }) }
        : null;
    case "create":
      return hasOnlyKeys(value, ["kind", "mutationId", "expectedGeneration", "collection", "value"]) && isMutationId(value.mutationId) && isGeneration(value.expectedGeneration) && isManagedDataName(value.collection) && isPlainJsonObject(value.value)
        ? { kind: "create", mutationId: value.mutationId, expectedGeneration: value.expectedGeneration, collection: value.collection, value: value.value }
        : null;
    case "replace":
      return hasOnlyKeys(value, ["kind", "mutationId", "expectedGeneration", "collection", "id", "expectedRevision", "value"]) && isMutationId(value.mutationId) && isGeneration(value.expectedGeneration) && isManagedDataName(value.collection) && isManagedDataId(value.id) && isRevision(value.expectedRevision) && isPlainJsonObject(value.value)
        ? { kind: "replace", mutationId: value.mutationId, expectedGeneration: value.expectedGeneration, collection: value.collection, id: value.id, expectedRevision: value.expectedRevision, value: value.value }
        : null;
    case "delete":
      return hasOnlyKeys(value, ["kind", "mutationId", "expectedGeneration", "collection", "id", "expectedRevision"]) && isMutationId(value.mutationId) && isGeneration(value.expectedGeneration) && isManagedDataName(value.collection) && isManagedDataId(value.id) && isRevision(value.expectedRevision)
        ? { kind: "delete", mutationId: value.mutationId, expectedGeneration: value.expectedGeneration, collection: value.collection, id: value.id, expectedRevision: value.expectedRevision }
        : null;
    case "begin-batch": {
      if (!hasOnlyKeys(value, ["kind", "mutationId", "expectedGeneration", "operations", "documents"]) || !isMutationId(value.mutationId) || !isGeneration(value.expectedGeneration) || !Array.isArray(value.operations) || !Array.isArray(value.documents) || value.operations.length > 64 || value.documents.length > 64) return null;
      const records = value.operations.map(parseManagedDataMutation);
      const documents = value.documents.map(parseManagedDocumentOperation);
      const stageIds = documents
        .filter((item): item is Extract<ManagedDocumentOperation, { kind: "create" | "replace" }> => item !== null && (item.kind === "create" || item.kind === "replace"))
        .map((item) => item.stageId);
      return records.every((item) => item !== null) && documents.every((item) => item !== null) && new Set(stageIds).size === stageIds.length
        ? { kind: "begin-batch", mutationId: value.mutationId, expectedGeneration: value.expectedGeneration, operations: records as ManagedDataMutation[], documents: documents as ManagedDocumentOperation[] }
        : null;
    }
    case "append-batch-operations": {
      if (!hasOnlyKeys(value, ["kind", "mutationId", "batchId", "operations"]) || !isMutationId(value.mutationId) || typeof value.batchId !== "string" || value.batchId.length > MAX_SURFACE_IDENTIFIER_CHARS || !Array.isArray(value.operations) || value.operations.length < 1 || value.operations.length > 64) return null;
      const operations = value.operations.map(parseManagedDataMutation);
      return operations.every((item) => item !== null)
        ? { kind: "append-batch-operations", mutationId: value.mutationId, batchId: value.batchId, operations: operations as ManagedDataMutation[] }
        : null;
    }
    case "append-document-chunk":
      return hasOnlyKeys(value, ["kind", "mutationId", "batchId", "documentId", "chunkIndex", "contentBase64"]) && isMutationId(value.mutationId) && typeof value.batchId === "string" && value.batchId.length <= MAX_SURFACE_IDENTIFIER_CHARS && isManagedDataId(value.documentId) && typeof value.chunkIndex === "number" && Number.isSafeInteger(value.chunkIndex) && value.chunkIndex >= 0 && isBase64Chunk(value.contentBase64)
        ? { kind: "append-document-chunk", mutationId: value.mutationId, batchId: value.batchId, documentId: value.documentId, chunkIndex: value.chunkIndex, contentBase64: value.contentBase64 }
        : null;
    case "commit-batch":
    case "abort-batch":
      return hasOnlyKeys(value, ["kind", "mutationId", "batchId"]) && isMutationId(value.mutationId) && typeof value.batchId === "string" && value.batchId.length <= MAX_SURFACE_IDENTIFIER_CHARS
        ? { kind: value.kind, mutationId: value.mutationId, batchId: value.batchId } as ManagedDataV2Request
        : null;
    default:
      return null;
  }
}

function isCapabilityRef(value: unknown): value is CapabilityRef {
  return (
    isObject(value) &&
    typeof value.provider === "string" &&
    value.provider.length > 0 &&
    value.provider.length <= MAX_SURFACE_IDENTIFIER_CHARS &&
    typeof value.capability === "string" &&
    value.capability.length > 0 &&
    value.capability.length <= MAX_SURFACE_IDENTIFIER_CHARS
  );
}

function isDataScope(value: unknown): value is DataScope {
  if (!isObject(value) || typeof value.kind !== "string") {
    return false;
  }
  switch (value.kind) {
    case "none":
      return true;
    case "resources": {
      if (
        !Array.isArray(value.resource_ids) ||
        value.resource_ids.length === 0 ||
        value.resource_ids.length > MAX_SURFACE_RESOURCE_IDS
      ) {
        return false;
      }
      const ids = value.resource_ids;
      if (!ids.every((id) =>
        typeof id === "string" && id.length > 0 && id.length <= MAX_SURFACE_IDENTIFIER_CHARS
      )) {
        return false;
      }
      return new Set(ids).size === ids.length;
    }
    default:
      return false;
  }
}

function parseOp(value: unknown): { ok: true; op: SurfaceOp } | { ok: false; reason: string } {
  if (!isObject(value) || typeof value.kind !== "string") {
    return { ok: false, reason: "op is not an object with a string kind" };
  }
  switch (value.kind) {
    case "invoke":
      if (!isCapabilityRef(value.capability)) {
        return { ok: false, reason: "invoke op has an invalid capability ref" };
      }
      if (!isPlainJsonObject(value.input)) {
        return { ok: false, reason: "invoke op input must be an object" };
      }
      if (!isDataScope(value.data_scope)) {
        return { ok: false, reason: "invoke op data scope must be valid" };
      }
      if (typeof value.goal !== "string" || value.goal.length > MAX_SURFACE_TEXT_CHARS) {
        return { ok: false, reason: "invoke op goal must be a string" };
      }
      return {
        ok: true,
        op: {
          kind: "invoke",
          capability: { provider: value.capability.provider, capability: value.capability.capability },
          input: value.input,
          data_scope: value.data_scope,
          goal: value.goal,
        },
      };
    case "cancel-run":
      if (
        typeof value.runId !== "string" ||
        value.runId.length === 0 ||
        value.runId.length > MAX_SURFACE_IDENTIFIER_CHARS
      ) {
        return { ok: false, reason: "cancel-run op requires a run id" };
      }
      return { ok: true, op: { kind: "cancel-run", runId: value.runId } };
    case "get-config":
      return { ok: true, op: { kind: "get-config" } };
    case "update-config":
      if (!isPlainJsonObject(value.config)) {
        return { ok: false, reason: "update-config op config must be an object" };
      }
      return { ok: true, op: { kind: "update-config", config: value.config } };
    case "get-state":
      if (typeof value.key !== "string" || value.key.length > MAX_SURFACE_IDENTIFIER_CHARS) {
        return { ok: false, reason: "get-state op requires a string key" };
      }
      return { ok: true, op: { kind: "get-state", key: value.key } };
    case "put-state":
      if (
        typeof value.key !== "string" ||
        value.key.length > MAX_SURFACE_IDENTIFIER_CHARS ||
        typeof value.expectedRevision !== "number" ||
        !Number.isSafeInteger(value.expectedRevision) ||
        value.expectedRevision < 0 ||
        (value.value !== null && !isPlainJsonObject(value.value))
      ) {
        return { ok: false, reason: "put-state op has invalid key, revision, or value" };
      }
      return {
        ok: true,
        op: {
          kind: "put-state",
          key: value.key,
          expectedRevision: value.expectedRevision,
          value: value.value,
        },
      };
    case "data-v1": {
      if (!hasOnlyKeys(value, ["kind", "request"])) {
        return { ok: false, reason: "data-v1 op contains unknown fields" };
      }
      const request = parseManagedDataRequest(value.request);
      if (request === null) {
        return { ok: false, reason: "data-v1 op has an invalid request" };
      }
      return { ok: true, op: { kind: "data-v1", request } };
    }
    case "data-v2": {
      if (!hasOnlyKeys(value, ["kind", "request"])) {
        return { ok: false, reason: "data-v2 op contains unknown fields" };
      }
      const request = parseManagedDataV2Request(value.request);
      if (request === null) {
        return { ok: false, reason: "data-v2 op has an invalid request" };
      }
      return { ok: true, op: { kind: "data-v2", request } };
    }
    case "list-artifacts":
      return { ok: true, op: { kind: "list-artifacts" } };
    case "list-events":
      return { ok: true, op: { kind: "list-events" } };
    default:
      return { ok: false, reason: `unknown op kind: ${value.kind}` };
  }
}

/// Validate one raw inbound value as a well-formed, current-version app
/// message. This is the payload-schema gate: anything malformed, mistyped, or
/// from a different protocol/version is rejected with a reason (never thrown).
export function parseAppMessage(raw: unknown): ParseResult {
  if (!isObject(raw)) {
    return { ok: false, reason: "message is not an object" };
  }
  const size = surfacePayloadSize(raw);
  if (size === null) {
    return { ok: false, reason: "message is not acyclic JSON" };
  }
  if (size > MAX_SURFACE_MESSAGE_CHARS) {
    return { ok: false, reason: "message exceeds the surface bridge size limit" };
  }
  if (raw.protocol !== SURFACE_BRIDGE_PROTOCOL) {
    return { ok: false, reason: "not a surface-bridge message" };
  }
  if (typeof raw.v !== "number") {
    return { ok: false, reason: "missing protocol version" };
  }
  if (raw.v !== SURFACE_BRIDGE_VERSION) {
    return { ok: false, reason: `unsupported protocol version: ${raw.v}` };
  }
  if (typeof raw.type !== "string") {
    return { ok: false, reason: "missing message type" };
  }
  if (
    typeof raw.instanceId !== "string" ||
    raw.instanceId.length === 0 ||
    raw.instanceId.length > MAX_SURFACE_IDENTIFIER_CHARS
  ) {
    return { ok: false, reason: "missing surface instance id" };
  }
  switch (raw.type) {
    case "ready":
      return {
        ok: true,
        message: {
          protocol: SURFACE_BRIDGE_PROTOCOL,
          v: SURFACE_BRIDGE_VERSION,
          type: "ready",
          instanceId: raw.instanceId,
        },
      };
    case "error":
      if (typeof raw.message !== "string" || raw.message.length > MAX_SURFACE_TEXT_CHARS) {
        return { ok: false, reason: "error message must be a string" };
      }
      return {
        ok: true,
        message: {
          protocol: SURFACE_BRIDGE_PROTOCOL,
          v: SURFACE_BRIDGE_VERSION,
          type: "error",
          instanceId: raw.instanceId,
          message: raw.message,
        },
      };
    case "extension-state":
      if (!isPlainJsonObject(raw.payload)) {
        return { ok: false, reason: "extension-state payload must be an object" };
      }
      return {
        ok: true,
        message: {
          protocol: SURFACE_BRIDGE_PROTOCOL,
          v: SURFACE_BRIDGE_VERSION,
          type: "extension-state",
          instanceId: raw.instanceId,
          payload: raw.payload,
        },
      };
    case "resize":
      if (typeof raw.height !== "number" || !Number.isFinite(raw.height) || raw.height < 0) {
        return { ok: false, reason: "resize height must be a non-negative finite number" };
      }
      return {
        ok: true,
        message: {
          protocol: SURFACE_BRIDGE_PROTOCOL,
          v: SURFACE_BRIDGE_VERSION,
          type: "resize",
          instanceId: raw.instanceId,
          height: raw.height,
        },
      };
    case "request": {
      if (
        typeof raw.requestId !== "number" ||
        !Number.isSafeInteger(raw.requestId) ||
        raw.requestId < 0
      ) {
        return { ok: false, reason: "request requestId must be a non-negative safe integer" };
      }
      const parsedOp = parseOp(raw.op);
      if (!parsedOp.ok) {
        return { ok: false, reason: parsedOp.reason };
      }
      return {
        ok: true,
        message: {
          protocol: SURFACE_BRIDGE_PROTOCOL,
          v: SURFACE_BRIDGE_VERSION,
          type: "request",
          instanceId: raw.instanceId,
          requestId: raw.requestId,
          op: parsedOp.op,
        },
      };
    }
    default:
      return { ok: false, reason: `unknown message type: ${raw.type}` };
  }
}

export function surfacePayloadSize(value: unknown): number | null {
  try {
    const serialized = JSON.stringify(value);
    return typeof serialized === "string" ? serialized.length : null;
  } catch {
    return null;
  }
}

export function surfacePayloadFits(value: unknown): boolean {
  const size = surfacePayloadSize(value);
  return size !== null && size <= MAX_SURFACE_MESSAGE_CHARS;
}

// -- Host message builders ----------------------------------------------------

export function initMessage(args: {
  instanceId: string;
  appId: string;
  surface: string;
  capabilities: CapabilityRef[];
  configSchema: JsonObject | null;
  config: JsonObject;
  theme: ThemeId;
  variables: Record<string, string>;
  extensionContext?: JsonObject;
  hostContext?: JsonObject;
}): SurfaceInitMessage {
  return {
    protocol: SURFACE_BRIDGE_PROTOCOL,
    v: SURFACE_BRIDGE_VERSION,
    type: "init",
    ...args,
    extensionContext: args.extensionContext ?? {},
    hostContext: args.hostContext ?? {},
  };
}

export function okResponse(requestId: number, result: JsonValue): SurfaceResponseMessage {
  return {
    protocol: SURFACE_BRIDGE_PROTOCOL,
    v: SURFACE_BRIDGE_VERSION,
    type: "response",
    requestId,
    ok: true,
    result,
  };
}

export function errorResponse(requestId: number, error: string): SurfaceResponseMessage {
  return {
    protocol: SURFACE_BRIDGE_PROTOCOL,
    v: SURFACE_BRIDGE_VERSION,
    type: "response",
    requestId,
    ok: false,
    error: error.slice(0, MAX_SURFACE_TEXT_CHARS),
  };
}

export function eventMessage(): SurfaceEventMessage {
  return {
    protocol: SURFACE_BRIDGE_PROTOCOL,
    v: SURFACE_BRIDGE_VERSION,
    type: "event",
  };
}

export function progressMessage(requestId: number, value: JsonValue): SurfaceProgressMessage {
  return {
    protocol: SURFACE_BRIDGE_PROTOCOL,
    v: SURFACE_BRIDGE_VERSION,
    type: "progress",
    requestId,
    value,
  };
}

export function extensionEventMessage(payload: JsonObject): SurfaceExtensionEventMessage {
  return {
    protocol: SURFACE_BRIDGE_PROTOCOL,
    v: SURFACE_BRIDGE_VERSION,
    type: "extension-event",
    payload,
  };
}

export function themeMessage(theme: ThemeId, variables: Record<string, string>): SurfaceThemeMessage {
  return {
    protocol: SURFACE_BRIDGE_PROTOCOL,
    v: SURFACE_BRIDGE_VERSION,
    type: "theme",
    theme,
    variables,
  };
}

/// Whether the surface is allowed to invoke this capability: it must be one of
/// the surface's declared intents. This mirrors the kernel's own
/// `UndeclaredSurfaceIntent` guard and is enforced again host-side; checking
/// here gives the frame a fast, clear denial.
export function intentIsDeclared(
  declaredIntents: CapabilityRef[],
  capability: CapabilityRef,
): boolean {
  return declaredIntents.some(
    (intent) =>
      intent.provider === capability.provider && intent.capability === capability.capability,
  );
}

/// Re-exported for callers that build ActionIntents from an invoke op.
export type { ActionIntent, Artifact };
