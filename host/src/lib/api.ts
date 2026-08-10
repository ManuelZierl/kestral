// Typed views of the kernel's wire shapes (serde-serialized) plus thin
// invoke wrappers. The frontend talks to the kernel only through these
// commands — surfaces emit intents, they never execute.

import {
  invokeChatWithProgress,
  invokeHost as invoke,
  invokeHostWithProgress,
  isRemoteTransport,
  resolveHostResourceUrl,
} from "$lib/hostTransport";

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };
export type JsonObject = { [key: string]: JsonValue };

export interface CapabilityRef {
  provider: string;
  capability: string;
}

export type ResourceId = string;

export type DataScope =
  | { kind: "none" }
  | { kind: "all-resources" }
  | { kind: "resources"; resource_ids: ResourceId[] };

export type CapabilityEffect =
  | "unspecified"
  | "read-only"
  | "local-write"
  | "external-write"
  | "destructive";

export interface CapabilityDeclaration {
  name: string;
  description: string;
  input_schema: JsonObject;
  output_schema?: JsonObject;
  effect: CapabilityEffect;
}

export interface SurfaceDeclaration {
  name: string;
  kind: "panel" | "card" | "form" | "picker" | "dashboard";
  title: string;
  description: string;
  intents: CapabilityRef[];
}

export type GrantCondition = "silent" | "notify" | "requires-approval";

export type GrantScope =
  | { kind: "exact-capability"; provider: string; capability: string }
  | { kind: "all-provider-capabilities"; provider: string };

export type GrantDuration =
  | { kind: "non-expiring" }
  | { kind: "expires-after"; seconds: number };

export type DenialReason = "no-grant" | "expired" | "revoked";

export interface Grant {
  grant_id: string;
  holder: string;
  scope: GrantScope;
  data_scope: DataScope;
  condition: GrantCondition;
  issued_at: string;
  expires_at: string | null;
}

export type GrantStatus = "active" | "revoked" | "expired";
export type GrantOrigin = "manifest-requested" | "user-added" | "mcp-export" | "system-bundled";

export interface GrantView extends Grant {
  holder_display_name: string;
  status: GrantStatus;
  origin: GrantOrigin;
}

export interface CapabilityUseView {
  provider_app_id: string;
  provider_display_name: string;
  capability: string;
  description: string;
  input_schema: JsonObject;
  authorizations: CapabilityAuthorizationView[];
}

export interface CapabilityAuthorizationView {
  data_scope: DataScope;
  condition: GrantCondition;
}

export interface GrantRequest {
  scope: GrantScope;
  data_scope: DataScope;
  condition: GrantCondition;
  reason: string;
  duration: GrantDuration;
}

export interface GrantEditorRequest {
  holder: string;
  scope: GrantScope;
  data_scope: DataScope;
  condition: GrantCondition;
  duration: GrantDuration;
  reason: string;
  allow_all_provider_scope: boolean;
  acknowledge_less_interactive_mcp: boolean;
}

export type PermissionProposalSubmission =
  | { status: "issued"; grant_id: string; effective_condition: GrantCondition }
  | { status: "already-active"; grant_id: string; effective_condition: GrantCondition }
  | { status: "refused" };

export interface AgentDeclaration {
  name: string;
  description: string;
  instructions: string;
  capability_bindings: CapabilityRef[];
}

export interface SkillDeclaration {
  name: string;
  description: string;
  instructions: string;
}

export interface AssistantProfileDeclaration {
  profile_name: string;
  title: string;
  description: string;
  instruction_skill_refs: string[];
  suggested_capability_refs: CapabilityRef[];
  suggested_agent_engine_contract: string | null;
  starter_prompts: string[];
}

export interface AutomationDeclaration {
  name: string;
  description: string;
  trigger: string;
}

export interface ConnectorDeclaration {
  name: string;
  description: string;
  secret_names: string[];
  config_schema: JsonObject | null;
}

export interface ConfigDeclaration {
  name: string;
  title: string;
  description: string;
  json_schema: JsonObject;
  default: JsonValue | null;
}

export interface ArtifactTypeDeclaration {
  name: string;
  description: string;
  json_schema: JsonObject;
}

export interface ExtensionPointDeclaration {
  name: string;
  contract_version: number;
  context_schema: JsonObject;
}

export interface ExtensionContribution {
  target_app: string;
  extension_point: string;
  contract_version: number;
  surface: string;
}

export interface AppManifest {
  app_id: string;
  version: string;
  display_name: string;
  description: string;
  capabilities: CapabilityDeclaration[];
  surfaces: SurfaceDeclaration[];
  agents: AgentDeclaration[];
  skills: SkillDeclaration[];
  assistant_profiles: AssistantProfileDeclaration[];
  automations: AutomationDeclaration[];
  connectors: ConnectorDeclaration[];
  config_declarations: ConfigDeclaration[];
  artifact_types: ArtifactTypeDeclaration[];
  extension_points: ExtensionPointDeclaration[];
  extension_contributions: ExtensionContribution[];
  grant_requests: GrantRequest[];
  event_subscriptions: string[];
}

export interface AppConfigEntry {
  settings: JsonObject;
}

export interface ChatPromptLayerView {
  id: string;
  kind: "protocol" | "assistant-instructions" | "skill" | "runtime-context";
  title: string;
  source: string | null;
  content: string;
  editable: boolean;
  included: boolean;
}

export type ChatPromptSkillStatus = "disabled" | "enabled" | "review-required";

export interface ChatPromptSkillView {
  app_id: string;
  app_display_name: string;
  app_version: string;
  skill_name: string;
  description: string;
  instructions: string;
  content_hash: string;
  status: ChatPromptSkillStatus;
  status_reason: string | null;
}

export interface ChatPromptRuntimeView {
  host_version: string;
  mode: string;
  model_id: string | null;
  connector_kind: string | null;
  app_inventory: { app_id: string; display_name: string; version: string }[] | null;
  connection_details: { connector_id: string; profile_id: string } | null;
}

export interface ChatPromptPreview {
  system_prompt: string;
  digest: string;
  layers: ChatPromptLayerView[];
  available_skills: ChatPromptSkillView[];
  runtime: ChatPromptRuntimeView;
}

export interface HostDefaults {
  default_llm_provider: string;
  default_llm_profile: string | null;
  cloud_llm_egress_accepted_profiles: string[];
  app_data_backup_retention: number;
}

export interface HostConfig {
  version: number;
  host: HostDefaults;
  apps: Record<string, AppConfigEntry>;
  connectors: Record<string, ConnectorConfig>;
  mcp_servers: Record<string, McpServerConfig>;
  mcp_exports: Record<string, McpExportProfile>;
  mcp_export_transitions: Record<string, boolean>;
  mcp_gateway: McpGatewaySettings;
}

// MCP is an adapter protocol, not the internal ontology: these shapes exist
// only to configure which servers the host may connect on request.
export type McpTransportConfig =
  | { kind: "stdio"; command: string; args: string[] }
  | { kind: "streamable-http"; url: string; authentication: McpHttpAuthentication };

export type McpHttpAuthentication =
  | { kind: "none" }
  | { kind: "static-header"; header_name: string; value_prefix: string };

export interface McpServerConfig {
  display_name: string;
  transport: McpTransportConfig;
}

export interface McpServerConfigView {
  id: string;
  display_name: string;
  transport: McpTransportConfig;
}

export type McpExportInteraction = "requires-approval" | "notify" | "silent";
export interface McpExportedCapability { provider: string; capability: string; }
export interface McpExportProfile {
  display_name: string;
  enabled: boolean;
  capabilities: McpExportedCapability[];
  interaction: McpExportInteraction;
  expires_after_seconds: number | null;
  rate_limit_per_minute: number;
  expose_results: boolean;
  expose_artifacts: boolean;
}
export interface McpExportProfileView extends McpExportProfile { id: string; }
export interface McpGatewaySettings {
  enabled: boolean;
  bind_address: string;
  allowed_origins: string[];
  oauth_enabled: boolean;
}
export interface McpGatewayStatus { running: boolean; local_address: string | null; }
/// One recorded MCP-gateway audit event. `event` is the event kind (e.g.
/// "tool-called", "auth-failed"); the remaining fields are present depending
/// on the kind.
export interface McpExportActivity {
  at: string;
  event: string;
  profile?: string;
  tool?: string;
  outcome?: string;
  remote?: string;
}

export interface McpServerStatusView extends McpServerConfigView {
  connected: boolean;
}

export type ConnectorKind =
  | "ollama"
  | "open-ai-compatible"
  | "openai"
  | "anthropic"
  | "anthropic-oauth"
  | "openai-codex"
  | "github-copilot"
  | "openrouter"
  | "google"
  | "mistral"
  | "amazon-bedrock";

export interface ConnectorConfig {
  kind: ConnectorKind;
  base_url: string;
  default_model: string;
  default_variant: ModelVariant | null;
  default_text_verbosity: TextVerbosity | null;
  secret_refs: Record<string, string>;
}

export type ModelVariant = "minimal" | "low" | "medium" | "high" | "xhigh" | "max";
export type TextVerbosity = "low" | "medium" | "high";

export interface ConnectorConfigView extends ConnectorConfig {
  id: string;
}

export interface ConnectionTestResult {
  ok: boolean;
  message: string;
}

export interface ModelInfo {
  id: string;
  display_name: string | null;
  variants: ModelVariant[];
  text_verbosity: TextVerbosity[];
}

export interface ModelListResult {
  models: ModelInfo[];
  message: string;
}

export interface ConfigStorageInfo {
  config_path: string;
  secrets_path: string;
  chat_store_path: string;
  file_resource_registry_path: string;
  profile_registry_path: string;
}

export interface SystemResetRequestResult {
  restart_required: boolean;
}

export type KestralProfileSource = "managed" | "custom-data-dir";

export interface KestralProfileView {
  profile_id: string;
  display_name: string;
  slug: string;
  root: string;
  created_at: string;
  current_runtime: boolean;
  selected_for_next_launch: boolean;
  source: KestralProfileSource;
  launch_args: string[];
  restart_instructions: string;
}

export interface CreateKestralProfileRequest {
  display_name: string;
  slug: string;
}

export interface PortableAppRecovery {
  id: string;
  display_name: string;
  version: string;
  package_digest: string;
}

export interface PortableSecretRecovery {
  owner: string;
  name: string;
}

export interface PortableFileResourceRecovery {
  resource_id: string;
  display_name: string;
  kind: string;
}

export type PortableImportTarget =
  | { kind: "preview" }
  | { kind: "fresh"; display_name: string; slug: string }
  | { kind: "overwrite-current"; confirmation: string };

export interface PortableExportResult {
  path: string;
  sha256: string;
  size: number;
  files: number;
  excluded_secrets: number;
  reinstall_apps: number;
}

export interface PortableImportResult {
  target: string;
  restart_required: boolean;
  restart_instructions: string;
  apps: PortableAppRecovery[];
  secrets: PortableSecretRecovery[];
  file_resources: PortableFileResourceRecovery[];
}

export interface PortableRecoveryStatus {
  version: number;
  imported_at: string;
  apps: PortableAppRecovery[];
  secrets: PortableSecretRecovery[];
  file_resources: PortableFileResourceRecovery[];
}

export type FileResourceKind = "file" | "directory";
export type FileResourceStatus = "active" | "removing";
export type FileEntryKind = "file" | "directory" | "symlink" | "other";
export type FileResourceGrantOperation = "list" | "read" | "create-or-replace" | "delete";

export interface FileResourceView {
  resource_id: string;
  display_name: string;
  kind: FileResourceKind;
  created_at: string;
  status: FileResourceStatus;
}

export interface TrustedFileResourceView extends FileResourceView {
  canonical_path: string;
}

export interface FileEntryView {
  path: string;
  display_name: string;
  kind: FileEntryKind;
  size_bytes: number | null;
  modified_at: string | null;
}

export interface FileListView {
  resource_id: string;
  resource_kind: FileResourceKind;
  entries: FileEntryView[];
}

export interface FileReadView {
  resource_id: string;
  path: string;
  bytes_read: number;
  total_bytes: number;
  truncated: boolean;
  sha256: string;
  content_base64: string;
}

export interface FileWriteView {
  resource_id: string;
  path: string;
  bytes_written: number;
  replaced: boolean;
  sha256: string;
}

export interface FileDeleteView {
  resource_id: string;
  path: string;
  deleted: boolean;
}

export interface InstalledApp {
  manifest: AppManifest;
  content_hash: string;
  installed_at: string;
  icon?: AppIcon | null;
  theme_colors?: AppThemeColor[];
}

export interface AppThemeColor {
  name: string;
  title: string;
  description: string;
  light: string;
  dark: string;
}

export type KestralIconName =
  | "activity"
  | "app-grid"
  | "artifact-box"
  | "book-open"
  | "chat-bubble"
  | "check-square"
  | "pencil-ruler"
  | "settings";

export type AppIcon =
  | { kind: "asset"; media_type: string; data_base64: string }
  | { kind: "kestral"; name: KestralIconName };

export interface ManagedAppRevisionView {
  revision_id: string;
  version: string;
  display_name: string;
  description: string;
  backend_kind: string;
  publisher: string | null;
  signature_verdict: string;
  signature_key_id: string | null;
  min_host_version: string;
  installed_at: string;
  payload_dir: string;
  package_digest: string;
}

export interface Provenance {
  run_id: string;
  capability: CapabilityRef;
  grant_id: string;
  produced_by: string;
  recorded_at: string;
}

export interface Artifact {
  artifact_id: string;
  artifact_type: string;
  title: string;
  content: JsonValue;
  provenance: Provenance;
}

export type ArtifactAccessTarget =
  | { kind: "artifact"; artifact_id: string }
  | { kind: "all-artifacts" };

export type Initiator =
  | { kind: "surface-action"; app_id: string; surface: string }
  | { kind: "app"; app_id: string; reason: string }
  | { kind: "run"; app_id: string; parent_run_id: string };

export type RunTerminalState = "completed" | "failed" | "cancelled" | "interrupted";

// Mirrors the kernel's closed LedgerEvent enum, variant for variant.
export type LedgerEvent =
  | { kind: "run-started"; run_id: string; initiator: Initiator; goal: string }
  | {
      kind: "capability-invoked";
      run_id: string;
      capability: CapabilityRef;
      grant_id: string;
      input_sha256: string;
      data_scope: DataScope;
    }
  | {
      kind: "capability-completed";
      run_id: string;
      capability: CapabilityRef;
      grant_id: string;
      result_sha256: string;
      data_scope: DataScope;
    }
  | {
      kind: "capability-failed";
      run_id: string;
      capability: CapabilityRef;
      grant_id: string;
      error: string;
      data_scope: DataScope;
    }
  | {
      kind: "invocation-refused";
      run_id: string;
      capability: CapabilityRef;
      reason: DenialReason;
      data_scope: DataScope;
    }
  | {
      kind: "invocation-cancelled";
      run_id: string;
      capability: CapabilityRef;
      data_scope: DataScope;
    }
  | {
      kind: "approval-requested";
      run_id: string;
      capability: CapabilityRef;
      grant_id: string;
      data_scope: DataScope;
    }
  | {
      kind: "approval-granted";
      run_id: string;
      capability: CapabilityRef;
      grant_id: string;
      data_scope: DataScope;
    }
  | {
      kind: "approval-denied";
      run_id: string;
      capability: CapabilityRef;
      grant_id: string;
      data_scope: DataScope;
    }
  | {
      kind: "artifact-produced";
      run_id: string;
      artifact_id: string;
      artifact_type: string;
    }
  | { kind: "run-ended"; run_id: string; terminal_state: RunTerminalState };

export interface LedgerRecord {
  sequence: number;
  recorded_at: string;
  event: LedgerEvent;
}

export interface SurfaceBinding {
  app_id: string;
  surface: string;
  instance_id: string;
}

export interface SurfaceStateEntry {
  revision: number;
  value: JsonObject | null;
}

export interface ManagedDataRecord {
  id: string;
  revision: number;
  created_at: string;
  updated_at: string;
  value: JsonObject;
}

export interface ManagedDataQuery {
  index?: string;
  equals?: JsonValue;
  after?: string;
  limit?: number;
}

export type ManagedDataMutation =
  | { kind: "create"; collection: string; value: JsonObject }
  | {
      kind: "replace";
      collection: string;
      id: string;
      expectedRevision: number;
      value: JsonObject;
    }
  | {
      kind: "delete";
      collection: string;
      id: string;
      expectedRevision: number;
    };

export type ManagedDataRequest =
  | { kind: "get"; collection: string; id: string }
  | { kind: "list"; collection: string; query?: ManagedDataQuery }
  | { kind: "create"; collection: string; value: JsonObject }
  | {
      kind: "replace";
      collection: string;
      id: string;
      expectedRevision: number;
      value: JsonObject;
    }
  | {
      kind: "delete";
      collection: string;
      id: string;
      expectedRevision: number;
    }
  | { kind: "transaction"; operations: ManagedDataMutation[] };

export type ManagedDocumentOperation =
  | {
      kind: "create";
      stageId: string;
      collection: string;
      metadata: JsonObject;
      contentLength: number;
      contentSha256: string;
    }
  | {
      kind: "replace";
      stageId: string;
      collection: string;
      id: string;
      expectedRevision: number;
      metadata: JsonObject;
      contentLength: number;
      contentSha256: string;
    }
  | {
      kind: "update-metadata";
      collection: string;
      id: string;
      expectedRevision: number;
      metadata: JsonObject;
    }
  | {
      kind: "delete";
      collection: string;
      id: string;
      expectedRevision: number;
    };

export type ManagedDataV2Read =
  | { kind: "record-get"; collection: string; id: string }
  | { kind: "record-list"; collection: string; query?: ManagedDataQuery }
  | { kind: "document-get"; collection: string; id: string }
  | { kind: "document-list"; collection: string; after?: string; limit?: number }
  | { kind: "document-content"; collection: string; id: string; offset: number; length: number };

export type ManagedDataV2ReadResult =
  | { kind: "record-get"; record: ManagedDataV2Record | null }
  | { kind: "record-list"; records: ManagedDataV2Record[]; nextAfter: string | null }
  | { kind: "document-get"; document: ManagedDocumentRecord | null }
  | { kind: "document-list"; documents: ManagedDocumentRecord[]; nextAfter: string | null }
  | {
      kind: "document-content";
      document: ManagedDocumentRecord;
      offset: number;
      contentBase64: string;
      contentLength: number;
    };

export interface ManagedDataV2Record {
  id: string;
  revision: number;
  createdAt: string;
  updatedAt: string;
  value: JsonObject;
}

export interface ManagedDocumentRecord {
  id: string;
  revision: number;
  createdAt: string;
  updatedAt: string;
  metadata: JsonObject;
  contentSha256: string;
  contentLength: number;
}

export interface ManagedDataV2ReadSnapshotResult {
  generation: number;
  results: ManagedDataV2ReadResult[];
}

export type ManagedDataV2Request =
  | {
      kind: "read-snapshot";
      expectedGeneration?: number;
      reads: ManagedDataV2Read[];
    }
  | { kind: "get"; collection: string; id: string; expectedGeneration?: number }
  | {
      kind: "list";
      collection: string;
      query?: ManagedDataQuery;
      expectedGeneration?: number;
    }
  | {
      kind: "get-document";
      collection: string;
      id: string;
      offset: number;
      length: number;
      expectedGeneration?: number;
    }
  | {
      kind: "list-documents";
      collection: string;
      after?: string;
      limit?: number;
      expectedGeneration?: number;
    }
  | {
      kind: "create";
      mutationId: string;
      expectedGeneration: number;
      collection: string;
      value: JsonObject;
    }
  | {
      kind: "replace";
      mutationId: string;
      expectedGeneration: number;
      collection: string;
      id: string;
      expectedRevision: number;
      value: JsonObject;
    }
  | {
      kind: "delete";
      mutationId: string;
      expectedGeneration: number;
      collection: string;
      id: string;
      expectedRevision: number;
    }
  | {
      kind: "begin-batch";
      mutationId: string;
      expectedGeneration: number;
      operations: ManagedDataMutation[];
      documents: ManagedDocumentOperation[];
    }
  | {
      kind: "append-batch-operations";
      mutationId: string;
      batchId: string;
      operations: ManagedDataMutation[];
    }
  | {
      kind: "append-document-chunk";
      mutationId: string;
      batchId: string;
      documentId: string;
      chunkIndex: number;
      contentBase64: string;
    }
  | { kind: "commit-batch"; mutationId: string; batchId: string }
  | { kind: "abort-batch"; mutationId: string; batchId: string };

export interface ManagedDataV2BatchBeginResult {
  batchId: string;
  generation: number;
  documents: Array<{ stageId: string; documentId: string }>;
}

export type ManagedDataCommand =
  | ManagedDataRequest
  | { contractVersion: 2; request: ManagedDataV2Request };

// Minimized event view for an app's own runs: topic, attribution,
// and stable ids only — never raw ledger records.
export type AppDataChangeKind = "created" | "updated" | "deleted" | "completed" | "availability-changed";

export type AppEventView =
  | {
      kind: "run-event";
      topic: string;
      run_id: string;
      actor: string;
      capability: CapabilityRef | null;
      artifact_id: string | null;
      terminal_state: RunTerminalState | null;
    }
  | {
      kind: "app-data-changed";
      provider_app_id: string;
      resource_ref: string;
      revision: number;
      change_kind: AppDataChangeKind;
    };

// A host-owned isolated document route for one custom app surface.
export interface SurfaceUiBundle {
  protocol_version: number;
  document_url: string;
}

export interface ActionIntent {
  capability: CapabilityRef;
  input: JsonObject;
  data_scope: DataScope;
  goal: string;
}

export type RefusalReason =
  | "no-grant"
  | "grant-expired"
  | "grant-revoked"
  | "approval-denied"
  | "cancelled";

export type InvocationResult =
  | { kind: "completed"; result: JsonValue; artifacts: Artifact[] }
  | { kind: "refused"; reason: RefusalReason }
  | { kind: "failed"; error: string };

export interface SurfaceActionOutcome {
  run_id: string;
  result: InvocationResult;
}

// Trusted-chrome event payloads (emitted by the shell's ShellChrome).
export interface GrantIssuancePrompt {
  app_id: string;
  app_display_name: string;
  scope: GrantScope;
  data_scope: DataScope;
  condition: GrantCondition;
  duration: GrantDuration;
  reason: string;
}

export interface CapabilityApprovalPrompt {
  app_id: string;
  app_display_name: string;
  capability: CapabilityRef;
  data_scope: DataScope;
  grant_id: string;
  run_id: string;
  goal: string;
}

export interface EventSubscriptionPrompt {
  app_id: string;
  app_display_name: string;
  topics: string[];
}

// One app's entire install request, presented as a single checklist so the
// user grants (or denies) all of it in one decision.
export interface InstallApprovalPrompt {
  app_id: string;
  app_display_name: string;
  event: EventSubscriptionPrompt | null;
  grants: GrantIssuancePrompt[];
}

export type ChromeRequest =
  | { kind: "grant-issuance"; request_id: number; prompt: GrantIssuancePrompt }
  | {
      kind: "capability-approval";
      request_id: number;
      prompt: CapabilityApprovalPrompt;
    }
  | {
      kind: "event-subscription";
      request_id: number;
      prompt: EventSubscriptionPrompt;
    }
  | {
      kind: "install-approval";
      request_id: number;
      prompt: InstallApprovalPrompt;
    };

export type LlmOAuthPrompt =
  | { type: "text"; message: string; placeholder: string | null }
  | { type: "secret"; message: string; placeholder: string | null }
  | { type: "manual_code"; message: string; placeholder: string | null }
  | {
      type: "select";
      message: string;
      options: Array<{ id: string; label: string; description: string | null }>;
    };

export type LlmOAuthEvent =
  | { kind: "auth-url"; session_id: string; url: string; instructions: string | null }
  | {
      kind: "device-code";
      session_id: string;
      user_code: string;
      verification_uri: string;
      interval_seconds: number | null;
      expires_in_seconds: number | null;
    }
  | { kind: "progress"; session_id: string; message: string }
  | {
      kind: "prompt";
      session_id: string;
      prompt_id: string;
      prompt: LlmOAuthPrompt;
    }
  | { kind: "completed"; session_id: string }
  | { kind: "failed"; session_id: string; message: string };

export type ChromeNotice =
  | {
      kind: "grant-use";
      app_id: string;
      capability: CapabilityRef;
      grant_id: string;
      run_id: string;
    }
  | {
      kind: "lease-conflict";
      resource: string;
      holding_run: string;
      requesting_run: string;
    };

export interface TrustedNoticeRecord {
  sequence: number;
  recorded_at: string;
  acknowledged_at: string | null;
  notice: ChromeNotice;
}

// -- App manager (install / lifecycle) ---------------------------------------

export interface AppSurfaceInfo {
  name: string;
  kind: string;
  title: string;
  has_custom_ui: boolean;
}

export type BackendAuthorityMode = "sandboxed" | "unsandboxed";

export interface AppStatusView {
  id: string;
  display_name: string;
  version: string;
  description: string;
  bundled: boolean;
  enabled: boolean;
  status: "active" | "disabled" | "failed" | "needs-permissions";
  status_detail: string | null;
  backend_kind: string;
  signature: "bundled" | "unsigned" | "valid-unknown-key" | "trusted" | "invalid" | "revoked";
  publisher: string | null;
  missing_permissions: number;
  surfaces: AppSurfaceInfo[];
  min_host_version: string | null;
  installed_at: string | null;
  revisions: ManagedAppRevisionView[];
  extension_contributions: AppExtensionContributionView[];
  removable: boolean;
}

export type AppExtensionCompatibility =
  | "exact"
  | "target-missing"
  | "point-missing"
  | "contract-mismatch";

export interface AppExtensionContributionView {
  target_app: string;
  extension_point: string;
  contract_version: number;
  surface: string;
  compatibility: AppExtensionCompatibility;
  target_contract_version: number | null;
}

export type ManagedAppOperation =
  | "install"
  | "update"
  | "reinstall"
  | "version-conflict"
  | "downgrade"
  | "revert";

export type ManagedAppVersionRelation = "same" | "higher" | "lower";
export type ManagedAppPublisherContinuity = "same" | "changed" | "new" | "unknown";

export interface ManagedAppPermissionDiff<T> {
  unchanged: T[];
  added: T[];
  widened: T[];
  removed: T[];
}

export interface ManagedAppUpdateDiff {
  version_relation: ManagedAppVersionRelation;
  display_name_changed: boolean;
  description_changed: boolean;
  backend_kind_changed: boolean;
  current_backend_authority_mode: BackendAuthorityMode | null;
  target_backend_authority_mode: BackendAuthorityMode | null;
  current_data: AppDataSummary | null;
  target_data: AppDataSummary;
  publisher_key_continuity: ManagedAppPublisherContinuity;
  capabilities_added: string[];
  capabilities_removed: string[];
  surfaces_added: string[];
  surfaces_removed: string[];
  permissions: ManagedAppPermissionDiff<GrantRequestSummary>;
  consumer_permissions: ManagedAppPermissionDiff<GrantRequestSummary>;
  extension_warnings: ManagedAppExtensionWarning[];
}

export interface ManagedAppExtensionWarning {
  contributor_app_id: string;
  extension_point: string;
  surface: string;
  contribution_contract_version: number;
  current_target_contract_version: number;
  target_contract_version: number | null;
}

export interface ManagedAppTransitionRequest {
  operation: ManagedAppOperation;
  staged_id: string | null;
  package_digest: string | null;
  app_id: string | null;
  revision_id: string | null;
  acknowledge_downgrade: boolean;
  acknowledge_revert_data_caveat: boolean;
}

export interface ManagedAppTransitionPlan {
  transition_id: string;
  app_id: string;
  operation: ManagedAppOperation;
  current_revision_id: string | null;
  target_revision_id: string;
  target_version: string;
  diff: ManagedAppUpdateDiff;
  requires_explicit_approval: boolean;
  data_rollback_supported: boolean;
  data_rollback_caveat: string | null;
  data_transition: ManagedAppDataTransition | null;
  staged_id: string | null;
  package_digest: string | null;
  revision_id: string | null;
}

export interface ManagedAppDataTransition {
  source_format_version: number | null;
  target_format_version: number;
  destructive: boolean;
  reverse_migration: boolean;
}

export type SignatureStatus =
  | { kind: "unsigned" }
  | { kind: "valid-unknown-key"; key_id: string }
  | { kind: "trusted"; key_id: string; scope: TrustScope }
  | { kind: "invalid"; reason: string }
  | { kind: "revoked"; key_id: string; scope: TrustScope };

export interface CapabilitySummary {
  name: string;
  description: string;
  effect: string;
}
export interface GrantRequestSummary {
  scope_label: string;
  data_scope_label: string;
  condition: string;
  reason: string;
  duration_label: string;
}
export interface InspectionSurfaceSummary {
  name: string;
  kind: string;
  title: string;
  has_custom_ui: boolean;
}
export interface ConfigSummary {
  name: string;
  title: string;
  description: string;
}
export interface SecretSummary {
  connector: string;
  name: string;
  description: string;
}
export interface PublisherView {
  name: string;
  homepage: string | null;
  key_id: string | null;
}

export type TrustScope =
  | { kind: "app-id"; app_id: string }
  | { kind: "namespace-prefix"; namespace_prefix: string };

export type TrustStatus = "trusted" | "revoked";

export interface TrustRecord {
  key_id: string;
  public_key: string;
  scope: TrustScope;
  status: TrustStatus;
}

export interface TrustKeyRequest {
  key_id: string;
  public_key: string;
  scope: TrustScope;
}

export interface RevokeKeyRequest {
  key_id: string;
  scope: TrustScope;
}

export interface PackageInspection {
  staged_id: string;
  package_digest: string;
  id: string;
  version: string;
  display_name: string;
  description: string;
  publisher: PublisherView | null;
  license: string | null;
  signature: SignatureStatus;
  signature_public_key: string | null;
  backend_kind: string;
  backend_detail: string;
  backend_authority_mode: BackendAuthorityMode | null;
  data: AppDataSummary;
  min_host_version: string;
  host_version: string;
  host_compatible: boolean;
  capabilities: CapabilitySummary[];
  grant_requests: GrantRequestSummary[];
  extension_contributions: ExtensionContributionSummary[];
  surfaces: InspectionSurfaceSummary[];
  config: ConfigSummary[];
  secrets: SecretSummary[];
  artifact_types: string[];
  event_subscriptions: string[];
  integrity_ok: boolean;
  integrity_error: string | null;
  warnings: string[];
  installable: boolean;
  blocking_error: string | null;
}

export interface AppDataTransitionSummary {
  from: number;
  to: number;
  destructive: boolean;
}

export interface AppDataSummary {
  kind: "none" | "versioned" | "host-managed";
  format_version: number | null;
  migration_protocol_version: number | null;
  transitions: AppDataTransitionSummary[];
  contract_version: number | null;
  total_bytes: number | null;
  batch_operations: number | null;
  collections: ManagedDataCollectionSummary[];
  documents: ManagedDocumentCollectionSummary[];
  proposals: ManagedDataProposalSummary[];
}

export interface ManagedDataCollectionSummary {
  name: string;
  schema: Record<string, unknown>;
  operations: ("get" | "list" | "create" | "replace" | "delete" | "transaction")[];
  records: number;
  record_bytes: number;
  query_results: number;
  indexes: string[];
  unique_indexes: string[];
}

export interface ManagedDocumentCollectionSummary {
  name: string;
  metadata_schema: Record<string, unknown>;
  operations: ("get" | "list" | "create" | "replace" | "update-metadata" | "delete")[];
  documents: number;
  metadata_bytes: number;
  content_bytes: number;
}

export interface ManagedDataProposalSummary {
  capability: string;
  artifact_type: string;
  title: string;
  description: string;
  target_kind: "collection" | "record" | "document";
  collection: string;
  max_payload_bytes: number;
  payload_schema: Record<string, unknown>;
}

export interface ExtensionContributionSummary {
  target_app: string;
  extension_point: string;
  contract_version: number;
  surface: string;
}

export type ChatMessageRole = "user" | "assistant" | "system" | "tool-status";
export type ChatMessageStatus = "pending" | "completed" | "interrupted" | "failed" | "cancelled";

export interface ChatMessageView {
  id: string;
  role: ChatMessageRole;
  text: string;
  /** Provider-supplied reasoning, when exposed by the selected model. */
  reasoning?: string | null;
  run_id: string | null;
  artifact_ids: string[];
  status: ChatMessageStatus | null;
  /** Client idempotency key; present on user messages sent by this UI. */
  client_request_id?: string;
  /** Host-stamped RFC 3339 creation time. */
  created_at: string;
  /**
   * Host-stamped RFC 3339 time at which the full text became available. Null
   * only while a message is still pending.
   */
  completed_at: string | null;
}

export interface ChatThread {
  id: string;
  resource_id: string;
  revision: number;
  title: string;
  created_at: string;
  updated_at: string;
  messages: ChatMessageView[];
  prompt_receipts?: Record<string, ChatCompositionReceipt>;
  contributions?: ChatContribution[];
  injected_contexts: ChatInjectedContext[];
  assistant_profile_ref?: string | null;
  assistant_profile_receipt?: ChatProfileReceipt | null;
  model_profile_ref?: string | null;
  model_profile_receipt?: ChatModelProfileReceipt | null;
  chat_agent_engine_ref?: string | null;
  chat_agent_engine_receipt?: ChatAgentEngineReceipt | null;
  chat_agent_engine_state?: { status: string; fallback_reason?: string | null } | null;
}

export interface ChatThreadRef {
  resource_id: string;
  thread_id: string;
  title: string;
  revision: number;
  created_at: string;
  updated_at: string;
}

export interface ChatPublicMessageView {
  message_id: string;
  thread_resource_id: string;
  sequence: number;
  role: "user" | "assistant";
  status: ChatMessageStatus;
  text: string;
  artifact_refs: string[];
  run_ref: string | null;
  created_at: string;
  completed_at: string | null;
}

export interface ChatContextBlock {
  source_app_id: string;
  source_app_version: string;
  contract: number;
  item_id: string;
  title: string;
  snapshot_revision: number;
  completeness: "complete" | "truncated";
  content_kind: "text-snapshot" | "artifact-ref" | "resource-ref" | "app-state";
  content_digest: string;
  content: JsonValue;
}

export interface ChatCompositionReceipt {
  system_prompt_digest: string;
  assistant_profile_ref: string;
  assistant_profile_digest: string;
  enabled_skill_digests: string[];
  context_block_digests: string[];
  attachment_refs: string[];
  available_capability_refs: string[];
  provider_profile_ref: string;
  model_profile: ChatModelProfileReceipt | null;
  agent_engine_ref: string | null;
  agent_engine_version: string | null;
  agent_engine_features: string[];
  assistant_capability_refs: string[];
  created_at: string;
  system_prompt: string;
  layers: { id: string; kind: ChatPromptLayerView["kind"]; title: string; source: string | null; content: string }[];
  injected_context: ChatInjectedContextReceipt | null;
}

export interface ChatInjectedContext {
  source_app_id: string;
  source_app_version: string;
  source_app_content_hash: string;
  source_run_id: string;
  item_id: string;
  revision: number;
  content_digest: string;
  content: string;
  created_at: string;
  updated_at: string;
}

export interface ChatInjectedContextReceipt {
  message_digest: string;
  entries: {
    source_app_id: string;
    source_app_name: string;
    source_app_version: string;
    item_id: string;
    revision: number;
    source_run_id: string;
    grant_id: string;
    content_digest: string;
  }[];
  exact_message: string | null;
}

export type ChatContributionKind = "text-snapshot" | "artifact-ref" | "resource-ref" | "draft-proposal";
export type ChatContributionCompleteness = "complete" | "truncated" | "unavailable";
export type ChatContributionLifecycle = "draft" | "accepted" | "removed" | "stale" | "failed";

export interface ChatContribution {
  source_app_id: string;
  source_app_version: string;
  source_contract: number;
  item_id: string;
  revision: number;
  digest: string;
  completeness: ChatContributionCompleteness;
  lifecycle: ChatContributionLifecycle;
  kind: ChatContributionKind;
  title: string;
  body: JsonValue;
  created_at: string;
  updated_at: string;
}

export interface AttachChatArtifactResult {
  thread: ChatThread;
  contribution: ChatContribution;
}

export interface ChatProfileReceipt {
  app_id: string;
  profile_name: string;
  version: string;
  digest: string;
  reviewed_skill_digests: string[];
  capability_refs: string[];
  engine_contract: string | null;
  status: string;
}

export interface ChatProfileView extends ChatProfileReceipt {
  app_display_name: string;
  title: string;
  description: string;
  suggested_capability_refs: string[];
  suggested_agent_engine_contract: string | null;
  availability: string;
  availability_reason: string | null;
}

export interface ChatModelProfileReceipt {
  source_app_id: string;
  source_app_version: string;
  profile_id: string;
  profile_digest: string;
  title: string;
  connector_id: string;
  model: string;
  reasoning: "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | null;
  temperature: number | null;
  max_output_tokens: number | null;
  tool_refs: string[];
  prompt?: ChatModelProfilePrompt | null;
}

export interface ChatModelProfilePrompt {
  layer_ids: string[];
  custom_texts: string[];
}

export interface ChatModelProfileView extends ChatModelProfileReceipt {
  source_app_name: string;
  description: string;
  effective_tool_refs: string[];
  unavailable_tool_refs: string[];
  available: boolean;
  availability_reason: string | null;
}

export interface ChatAgentEngineView {
  app_id: string;
  display_name: string;
  version: string;
  contract: string;
  features: string[];
  available: boolean;
  availability_reason: string | null;
}

export interface ChatAgentEngineReceipt {
  app_id: string;
  version: string;
  contract: string;
}

export interface ChatThreadSummary {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  message_count: number;
}

// What the chat app answers for one message (chat_app.rs::ChatReply).
export interface SendChatMessageResult {
  thread: ChatThread;
}

export interface ChatStreamEvent {
  kind: "llm-stream-start" | "llm-stream-delta";
  content: string;
  reasoning: string;
}

export const bootstrapStartupApps = () => invoke<void>("bootstrap_startup_apps");
export const getActiveKestralProfile = () => invoke<KestralProfileView>("get_active_kestral_profile");
export const listKestralProfiles = () => invoke<KestralProfileView[]>("list_kestral_profiles");
export const createKestralProfile = (request: CreateKestralProfileRequest) =>
  invoke<KestralProfileView>("create_kestral_profile", { request });
export const deleteKestralProfile = (profileId: string) =>
  invoke<void>("delete_kestral_profile", { profileId });
export const exportPortableProfile = (destination: string) =>
  invoke<PortableExportResult>("export_portable_profile", { destination });
export const importPortableProfile = (archivePath: string, target: PortableImportTarget) =>
  invoke<PortableImportResult>("import_portable_profile", { archivePath, target });
export const getPortableRecoveryStatus = () =>
  invoke<PortableRecoveryStatus | null>("get_portable_recovery_status");
export const listChatThreads = () => invoke<ChatThreadSummary[]>("list_chat_threads");
export const getChatThread = (threadId: string) => invoke<ChatThread>("get_chat_thread", { threadId });
export const getChatPromptPreview = (candidateConfig?: JsonObject, threadId?: string) =>
  (candidateConfig
    ? invoke<ChatPromptPreview>("get_chat_prompt_preview", { candidateConfig, threadId })
    : invoke<ChatPromptPreview>("get_chat_prompt_preview", { threadId }));
export const listChatProfiles = () => invoke<ChatProfileView[]>("list_chat_profiles");
export const listChatModelProfiles = () =>
  invoke<ChatModelProfileView[]>("list_chat_model_profiles");
export const listChatAgentEngines = () => invoke<ChatAgentEngineView[]>("list_chat_agent_engines");
export const setChatModelProfile = (threadId: string, profileRef: string | null) =>
  invoke<ChatThread>("set_chat_model_profile", { threadId, profileRef });
export const setChatThreadProfile = (threadId: string, appId: string, profileName: string) =>
  invoke<ChatThread>("set_chat_thread_profile", { threadId, appId, profileName });
export const setChatAgentEngine = (threadId: string, appId: string | null) =>
  invoke<ChatThread>("set_chat_agent_engine", { threadId, appId });
export const removeChatContribution = (
  threadId: string,
  sourceAppId: string,
  kind: ChatContributionKind,
  itemId: string,
) => invoke<ChatThread>("remove_chat_contribution", { threadId, sourceAppId, kind, itemId });
export const attachChatArtifact = (threadId: string, artifactId: string, title: string) =>
  invoke<AttachChatArtifactResult>("attach_chat_artifact", { threadId, artifactId, title });
export const createChatThread = () => invoke<ChatThread>("create_chat_thread");
export const renameChatThread = (threadId: string, title: string) =>
  invoke<ChatThread>("rename_chat_thread", { threadId, title });
export const deleteChatThread = (threadId: string) =>
  invoke<void>("delete_chat_thread", { threadId });
export const sendChatMessage = (
  threadId: string,
  message: string,
  requestId: string,
  onStream: (event: ChatStreamEvent) => void,
) => invokeChatWithProgress<SendChatMessageResult, ChatStreamEvent>(
  requestId,
  { threadId, message },
  onStream,
);
export const cancelChatMessage = (threadId: string) =>
  invoke<void>("cancel_chat_message", { threadId });
export const listApps = () => invoke<InstalledApp[]>("list_apps");
export const listInstalledApps = () => invoke<AppStatusView[]>("list_installed_apps");
export const inspectPackage = (packageDir: string) =>
  invoke<PackageInspection>("inspect_package", { packageDir });
export const inspectGitPackage = (gitUrl: string) =>
  invoke<PackageInspection>("inspect_git_package", { gitUrl });
export const listManagedAppRevisions = (appId: string) =>
  invoke<ManagedAppRevisionView[]>("list_managed_app_revisions", { appId });
export const planManagedAppTransition = (request: ManagedAppTransitionRequest) =>
  invoke<ManagedAppTransitionPlan>("plan_managed_app_transition", { request });
export const applyManagedAppTransition = (plan: ManagedAppTransitionPlan) =>
  invoke<AppStatusView[]>("apply_managed_app_transition", { transitionId: plan.transition_id });
export const installApp = (stagedId: string, packageDigest: string) =>
  invoke<AppStatusView[]>("install_app", { stagedId, packageDigest });
export const setAppEnabled = (appId: string, enabled: boolean) =>
  invoke<AppStatusView[]>("set_app_enabled", { appId, enabled });
export const uninstallApp = (appId: string, purgeSecrets: boolean, purgeData: boolean) =>
  invoke<AppStatusView[]>("uninstall_app", { appId, purgeSecrets, purgeData });
export const listPublisherTrust = () => invoke<TrustRecord[]>("list_publisher_trust");
export const trustPublisherKey = (request: TrustKeyRequest) =>
  invoke<TrustRecord[]>("trust_publisher_key", { request });
export const revokePublisherKey = (request: RevokeKeyRequest) =>
  invoke<TrustRecord[]>("revoke_publisher_key", { request });
export const getHostConfig = () => invoke<HostConfig>("get_host_config");
export const getConfigStorageInfo = () => invoke<ConfigStorageInfo>("get_config_storage_info");
export const requestSystemReset = (confirmation: string) =>
  invoke<SystemResetRequestResult>("request_system_reset", { confirmation });
export const updateHostConfig = (patch: JsonObject) =>
  invoke<HostConfig>("update_host_config", { patch });
export const getAppConfig = (appId: string) =>
  invoke<JsonObject>("get_app_config", { appId });
export const updateAppConfig = (appId: string, config: JsonObject) =>
  invoke<JsonObject>("update_app_config", { appId, config });
export const listConnectorConfigs = () =>
  invoke<ConnectorConfigView[]>("list_connector_configs");
export const upsertConnectorConfig = (
  connector: ConnectorConfigView,
  acknowledgeDataEgress = false,
) => invoke<ConnectorConfigView>("upsert_connector_config", { connector, acknowledgeDataEgress });
export const deleteConnectorConfig = (connectorId: string) =>
  invoke<void>("delete_connector_config", { connectorId });
export const putSecret = (owner: string, secretName: string, value: string) =>
  invoke<void>("put_secret", { owner, secretName, value });
export const clearSecret = (owner: string, secretName: string) =>
  invoke<void>("clear_secret", { owner, secretName });
export const hasSecret = (owner: string, secretName: string) =>
  invoke<boolean>("has_secret", { owner, secretName });
export const listFileResources = () => invoke<FileResourceView[]>("list_file_resources");
export const listTrustedFileResources = () =>
  invoke<TrustedFileResourceView[]>("list_trusted_file_resources");
export const registerFileResource = (path: string) =>
  invoke<TrustedFileResourceView>("register_file_resource", { path });
export const removeFileResource = (resourceId: string) =>
  invoke<void>("remove_file_resource", { resourceId });
export const grantFileResourceAccess = (
  holder: string,
  resourceId: string,
  operations: FileResourceGrantOperation[],
) => invoke<void>("grant_file_resource_access", { holder, resourceId, operations });
export const testConnectorConfig = (connectorId: string) =>
  invoke<ConnectionTestResult>("test_connector_config", { connectorId });
export const startLlmOAuth = (connectorId: string) =>
  invoke<string>("start_llm_oauth", { connectorId });
export const resolveLlmOAuthPrompt = (
  sessionId: string,
  promptId: string,
  value: string | null,
  cancelled: boolean,
) => invoke<void>("resolve_llm_oauth_prompt", { sessionId, promptId, value, cancelled });
export const cancelLlmOAuth = (sessionId: string) =>
  invoke<void>("cancel_llm_oauth", { sessionId });
export const listMcpServers = () => invoke<McpServerStatusView[]>("list_mcp_servers");
export const upsertMcpServer = (server: McpServerConfigView) =>
  invoke<McpServerConfigView>("upsert_mcp_server", { server });
export const deleteMcpServer = (serverId: string) =>
  invoke<void>("delete_mcp_server", { serverId });
export const putMcpHttpAuthSecret = (serverId: string, value: string) =>
  invoke<void>("put_mcp_http_auth_secret", { serverId, value });
export const clearMcpHttpAuthSecret = (serverId: string) =>
  invoke<void>("clear_mcp_http_auth_secret", { serverId });
export const hasMcpHttpAuthSecret = (serverId: string) =>
  invoke<boolean>("has_mcp_http_auth_secret", { serverId });
export const connectMcpServer = (serverId: string) =>
  invoke<void>("connect_mcp_server", { serverId });
export const disconnectMcpServer = (serverId: string) =>
  invoke<void>("disconnect_mcp_server", { serverId });
export const listMcpExportProfiles = () => invoke<McpExportProfileView[]>("list_mcp_export_profiles");
export const upsertMcpExportProfile = (profile: McpExportProfileView) =>
  invoke<McpExportProfileView>("upsert_mcp_export_profile", { profile });
export const setMcpExportEnabled = (profileId: string, enabled: boolean) =>
  invoke<void>("set_mcp_export_enabled", { profileId, enabled });
export const deleteMcpExportProfile = (profileId: string) =>
  invoke<void>("delete_mcp_export_profile", { profileId });
export const rotateMcpExportToken = (profileId: string) =>
  invoke<string>("rotate_mcp_export_token", { profileId });
export const revokeMcpExportToken = (profileId: string) =>
  invoke<void>("revoke_mcp_export_token", { profileId });
export const hasMcpExportToken = (profileId: string) =>
  invoke<boolean>("has_mcp_export_token", { profileId });
export const startMcpGateway = () => invoke<McpGatewayStatus>("start_mcp_gateway");
export const stopMcpGateway = () => invoke<void>("stop_mcp_gateway");
export const mcpGatewayStatus = () => invoke<McpGatewayStatus>("mcp_gateway_status");
export const mcpExportRecentActivity = () =>
  invoke<McpExportActivity[]>("mcp_export_recent_activity");
export const discoverConnectorModelsDraft = (
  kind: ConnectorConfigView["kind"],
  baseUrl: string,
  defaultModel: string | null,
  apiKeySecretName: string | null,
) =>
  invoke<ModelListResult>("discover_connector_models_draft", {
    kind,
    baseUrl,
    defaultModel,
    apiKeySecretName,
  });
export const availableCapabilitiesFor = (appId: string) =>
  invoke<CapabilityUseView[]>("available_capabilities_for", { appId });
export const validateExtensionContext = (
  targetApp: string,
  extensionPoint: string,
  context: JsonObject,
) => invoke<void>("validate_extension_context", { targetApp, extensionPoint, context });
export const listGrants = () => invoke<GrantView[]>("list_grants");
export const ledgerRecords = () => invoke<LedgerRecord[]>("ledger_records");
export const listArtifacts = () => invoke<Artifact[]>("list_artifacts");
export const grantArtifactAccess = (holder: string, target: ArtifactAccessTarget) =>
  invoke<void>("grant_artifact_access", { holder, target });
// Sandboxed-surface data reads: scoped host-side to the app's own artifacts
// (by kernel provenance) and its own minimized events. Never all artifacts.
export const listAppArtifacts = (appId: string) =>
  invoke<Artifact[]>("list_app_artifacts", { appId });
export const appSurfaceEvents = (appId: string) =>
  invoke<AppEventView[]>("app_surface_events", { appId });
export const getSurfaceUi = async (appId: string, surface: string) => {
  const bundle = await invoke<SurfaceUiBundle | null>("get_surface_ui", {
    appId,
    surface,
    remote: isRemoteTransport(),
  });
  return bundle === null
    ? null
    : { ...bundle, document_url: resolveHostResourceUrl(bundle.document_url) };
};
export const getSurfaceState = (binding: SurfaceBinding, key: string) =>
  invoke<SurfaceStateEntry>("get_surface_state", { binding, key });
export const putSurfaceState = (
  binding: SurfaceBinding,
  key: string,
  expectedRevision: number,
  value: JsonObject | null,
) => invoke<SurfaceStateEntry>("put_surface_state", {
  binding,
  key,
  expectedRevision,
  value,
});
export const requestManagedData = (binding: SurfaceBinding, request: ManagedDataCommand) =>
  invoke<JsonValue>("managed_data_request", { binding, request });
export const requestManagedDataV2 = (binding: SurfaceBinding, request: ManagedDataV2Request) =>
  requestManagedData(binding, { contractVersion: 2, request });
export const openSurface = (appId: string, surface: string) =>
  invoke<SurfaceBinding>("open_surface", { appId, surface });
export const closeSurface = (binding: SurfaceBinding) =>
  invoke<void>("close_surface", { binding });
export const submitAction = (
  binding: SurfaceBinding,
  intent: ActionIntent,
  onProgress?: (event: JsonValue) => void,
) => onProgress
  ? invokeHostWithProgress<SurfaceActionOutcome, JsonValue>(
      "submit_action_with_progress",
      { binding, intent },
      onProgress,
    )
  : invoke<SurfaceActionOutcome>("submit_action", { binding, intent });
export const cancelSurfaceAction = (binding: SurfaceBinding, runId: string) =>
  invoke<void>("cancel_surface_action", { binding, runId });
export const revokeGrant = (grantId: string) =>
  invoke<void>("revoke_grant", { grantId });
export const requestAppGrants = (appId: string) =>
  invoke<void>("request_app_grants", { appId });
export const requestManifestGrant = (appId: string, request: GrantRequest) =>
  invoke<void>("request_manifest_grant", { appId, request });
export const submitPermissionProposal = (artifactId: string) =>
  invoke<PermissionProposalSubmission>("submit_permission_proposal", { artifactId });
export const issueEditorGrant = (request: GrantEditorRequest) =>
  invoke<void>("issue_editor_grant", { request });
export const replaceGrant = (grantId: string, request: GrantEditorRequest) =>
  invoke<void>("replace_grant", { grantId, request });
export const listTrustedNotices = () => invoke<TrustedNoticeRecord[]>("list_trusted_notices");
export const resolveApproval = (requestId: number, approved: boolean) =>
  invoke<void>("resolve_approval", { requestId, approved });
export const resolveInstallApproval = (
  requestId: number,
  eventApproved: boolean | null,
  grantApprovals: boolean[],
) =>
  invoke<void>("resolve_install_approval", {
    requestId,
    eventApproved,
    grantApprovals,
  });
