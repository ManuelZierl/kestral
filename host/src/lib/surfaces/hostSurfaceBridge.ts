// The trusted, parent side of the surface bridge. It receives raw window
// messages from a sandboxed app frame, validates them (origin/source, then
// protocol/version/shape, then surface instance, then declared intent), and
// dispatches the closed set of ops to host actions. Every op result is posted
// back over the same channel.
//
// Invariants this module upholds:
// - Untrusted input never throws out of `handleMessage`: a hostile or buggy
//   frame cannot crash the host (spec requirement: surface crash/hang must not
//   crash the host).
// - The app identity is the parent-held `binding`, never anything the frame
//   sends. A frame cannot cause actions attributed to another app.
// - Only declared-intent capabilities may be invoked; grants are still
//   enforced host-side by the kernel action path.

import type {
  ActionIntent,
  Artifact,
  AppEventView,
  CapabilityRef,
  JsonObject,
  JsonValue,
  ManagedDataCommand,
  ManagedDataRequest,
  SurfaceStateEntry,
  SurfaceActionOutcome,
  SurfaceBinding,
} from "$lib/api";
import {
  errorResponse,
  intentIsDeclared,
  okResponse,
  parseAppMessage,
  progressMessage,
  surfacePayloadFits,
  type AppToHostMessage,
  type HostToAppMessage,
  type SurfaceOp,
} from "$lib/surfaces/surfaceBridgeProtocol";

/// Host-backed actions the bridge exposes. Each is already grant-scoped and
/// app-scoped by the host command it wraps.
export interface SurfaceBridgeActions {
  invoke(
    intent: ActionIntent,
    onProgress: (value: JsonValue) => void,
  ): Promise<SurfaceActionOutcome>;
  cancelRun(runId: string): Promise<void>;
  getConfig(): Promise<JsonObject>;
  updateConfig(config: JsonObject): Promise<JsonObject>;
  getState(key: string): Promise<SurfaceStateEntry>;
  putState(
    key: string,
    expectedRevision: number,
    value: JsonObject | null,
  ): Promise<SurfaceStateEntry>;
  managedData(request: ManagedDataCommand): Promise<JsonValue>;
  listArtifacts(): Promise<Artifact[]>;
  listEvents(): Promise<AppEventView[]>;
}

export interface CreateSurfaceBridgeOptions {
  binding: SurfaceBinding;
  /// The capabilities this surface declared as intents (the only ones it may
  /// invoke). Sourced from the app manifest, not the frame.
  declaredIntents: CapabilityRef[];
  actions: SurfaceBridgeActions;
  /// Transport parent → frame. The component wires this to
  /// `iframe.contentWindow.postMessage(message, "*")`.
  post(message: HostToAppMessage): void;
  /// Origin + source guard. Returns true only for messages that genuinely
  /// came from this surface's frame. The component supplies the real check
  /// (source === iframe.contentWindow and an opaque/allowed origin).
  isTrustedSource(event: MessageEvent): boolean;
  onReady?(): void;
  onAppError?(message: string): void;
  /// The frame reported its rendered content height (CSS px). Advisory; the
  /// host decides whether and how to resize.
  onResize?(height: number): void;
  /// The frame published slot-specific state for its extension point owner
  /// (e.g. Chat). The payload is untrusted; the owner validates it against
  /// its own extension contract before acting on it.
  onExtensionState?(payload: JsonObject): void;
  /// Called when a message is dropped, with the reason. Useful for telemetry
  /// and tests; never surfaced to the frame.
  onRejected?(reason: string): void;
}

export interface SurfaceBridge {
  /// Attach as a `window` message listener. Synchronous and total: it never
  /// throws, and never returns a rejected promise, regardless of input.
  handleMessage(event: MessageEvent): void;
}

export function createSurfaceBridge(options: CreateSurfaceBridgeOptions): SurfaceBridge {
  const {
    binding,
    declaredIntents,
    actions,
    post,
    isTrustedSource,
    onReady,
    onAppError,
    onResize,
    onExtensionState,
    onRejected,
  } = options;
  const pendingRequestIds = new Set<number>();
  const maxPendingRequests = 16;

  function reject(reason: string): void {
    onRejected?.(reason);
  }

  async function runOp(op: SurfaceOp, requestId: number): Promise<JsonValue> {
    switch (op.kind) {
      case "invoke": {
        // Declared-intent gate (kernel re-checks; this is a fast, clear no).
        if (!intentIsDeclared(declaredIntents, op.capability)) {
          throw new Error(
            `capability ${op.capability.provider}/${op.capability.capability} is not declared by this surface`,
          );
        }
        const outcome = await actions.invoke(
          {
            capability: op.capability,
            input: op.input,
            data_scope: op.data_scope,
            goal: op.goal,
          },
          (value) => {
            if (surfacePayloadFits(value)) post(progressMessage(requestId, value));
            else reject("surface progress exceeded the bridge size limit");
          },
        );
        return outcome as unknown as JsonValue;
      }
      case "cancel-run":
        await actions.cancelRun(op.runId);
        return null;
      case "get-config":
        return (await actions.getConfig()) as JsonValue;
      case "update-config":
        return (await actions.updateConfig(op.config)) as JsonValue;
      case "get-state":
        return (await actions.getState(op.key)) as unknown as JsonValue;
      case "put-state":
        return (await actions.putState(op.key, op.expectedRevision, op.value)) as unknown as JsonValue;
      case "data-v1":
        return actions.managedData(op.request);
      case "data-v2":
        return actions.managedData({ contractVersion: 2, request: op.request });
      case "list-artifacts":
        return (await actions.listArtifacts()) as unknown as JsonValue;
      case "list-events":
        return (await actions.listEvents()) as unknown as JsonValue;
    }
  }

  function dispatchRequest(requestId: number, op: SurfaceOp): void {
    if (pendingRequestIds.has(requestId)) {
      post(errorResponse(requestId, "request id is already pending"));
      return;
    }
    if (pendingRequestIds.size >= maxPendingRequests) {
      post(errorResponse(requestId, "too many surface requests are pending"));
      return;
    }
    pendingRequestIds.add(requestId);
    // Fire-and-forget: the promise always resolves to a posted response, and
    // any failure becomes an error response — it never escapes as a rejection.
    void (async () => {
      try {
        const result = await runOp(op, requestId);
        if (!surfacePayloadFits(result)) {
          throw new Error("surface response exceeded the bridge size limit");
        }
        post(okResponse(requestId, result));
      } catch (failure) {
        const message =
          failure instanceof Error ? failure.message : String(failure);
        post(errorResponse(requestId, message));
      } finally {
        pendingRequestIds.delete(requestId);
      }
    })();
  }

  function dispatch(message: AppToHostMessage): void {
    switch (message.type) {
      case "ready":
        onReady?.();
        return;
      case "error":
        onAppError?.(message.message);
        return;
      case "resize":
        onResize?.(message.height);
        return;
      case "extension-state":
        onExtensionState?.(message.payload);
        return;
      case "request":
        dispatchRequest(message.requestId, message.op);
        return;
    }
  }

  function handleMessage(event: MessageEvent): void {
    try {
      // 1. Origin + source: the message must come from this surface's frame.
      if (!isTrustedSource(event)) {
        reject("untrusted message source");
        return;
      }
      // 2. Protocol / version / payload shape.
      const parsed = parseAppMessage(event.data);
      if (!parsed.ok) {
        reject(parsed.reason);
        const raw = event.data;
        if (
          typeof raw === "object" && raw !== null &&
          "type" in raw && raw.type === "request" &&
          "requestId" in raw && typeof raw.requestId === "number"
        ) {
          post(errorResponse(raw.requestId, parsed.reason));
        }
        return;
      }
      // 3. Surface instance: the echoed id must match the bound instance.
      if (parsed.message.instanceId !== binding.instance_id) {
        reject("surface instance id mismatch");
        return;
      }
      // 4. Dispatch (declared-intent gate applied per-op inside runOp).
      dispatch(parsed.message);
    } catch (failure) {
      // Defense in depth: no inbound message can throw out of the host loop.
      reject(failure instanceof Error ? failure.message : String(failure));
    }
  }

  return { handleMessage };
}

/// Build the origin+source guard for a specific frame window. A message is
/// trusted only if it came from exactly that window with the opaque origin
/// (the string "null"). Any other origin or window is rejected.
export function trustedSourceGuard(
  frameWindow: () => Window | null,
): (event: MessageEvent) => boolean {
  return (event: MessageEvent) => {
    const expected = frameWindow();
    if (expected === null || event.source !== expected) {
      return false;
    }
    return event.origin === "null";
  };
}
