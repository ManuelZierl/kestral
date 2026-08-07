import type { Artifact, ChatMessageView, LedgerRecord } from "$lib/api";
import { readableArtifactPreview } from "$lib/stuff/artifactRenderer";

export const UNAVAILABLE_REFERENCE_MESSAGE =
  "Referenced data is unavailable. It may have been removed or the stored data may be inconsistent.";

// Internal execution artifacts remain available in Artifacts and System, but
// their raw payloads are not conversation content.
const INTERNAL_CHAT_ARTIFACT_TYPES = new Set([
  "agent-transcript",
  "llm-response",
  "permission-proposal",
]);
const MCP_RESULT_CARD_ARTIFACT_TYPE = "mcp-result-card";
export const MCP_CHAT_PREVIEW_CHARACTER_LIMIT = 96;

const PERMISSIONS_APP_ID = "com.ma-zierl.host.permissions";
const PROPOSE_GRANT = "permissions.propose_grant";

export interface ChatPermissionProposal {
  artifactId: string;
  holder: string;
  provider: string;
  capability: string;
  reason: string;
}

export function chatPermissionProposals(
  message: Pick<ChatMessageView, "artifact_ids">,
  artifactsLoaded: boolean,
  artifacts: Artifact[],
): ChatPermissionProposal[] {
  if (!artifactsLoaded) return [];
  return message.artifact_ids.flatMap((artifactId): ChatPermissionProposal[] => {
    const artifact = artifacts.find((candidate) => candidate.artifact_id === artifactId);
    if (
      !artifact ||
      artifact.artifact_type !== "permission-proposal" ||
      artifact.provenance.produced_by !== PERMISSIONS_APP_ID ||
      artifact.provenance.capability.provider !== PERMISSIONS_APP_ID ||
      artifact.provenance.capability.capability !== PROPOSE_GRANT ||
      typeof artifact.content !== "object" ||
      artifact.content === null ||
      Array.isArray(artifact.content)
    ) return [];
    const content = artifact.content as Record<string, unknown>;
    const scope = content.scope;
    if (
      typeof content.holder !== "string" ||
      typeof content.reason !== "string" ||
      typeof scope !== "object" ||
      scope === null ||
      Array.isArray(scope)
    ) return [];
    const exact = scope as Record<string, unknown>;
    if (
      exact.kind !== "exact-capability" ||
      typeof exact.provider !== "string" ||
      exact.provider.length === 0 ||
      typeof exact.capability !== "string" ||
      content.condition !== "requires-approval"
    ) return [];
    return [{
      artifactId,
      holder: content.holder,
      provider: exact.provider,
      capability: exact.capability,
      reason: content.reason,
    }];
  });
}

export interface SessionArtifactCard {
  id: string;
  title: string;
  type: string;
  preview: string;
  available: boolean;
}

export function isRunAvailable(
  recordsLoaded: boolean,
  records: LedgerRecord[],
  runId: string | null,
): boolean | null {
  if (runId === null || !recordsLoaded) return null;
  return records.some((record) => record.event.run_id === runId);
}

export function chatArtifactCards(
  message: Pick<ChatMessageView, "artifact_ids">,
  artifactsLoaded: boolean,
  artifacts: Artifact[],
  showMcpResultCards = true,
): SessionArtifactCard[] {
  if (!artifactsLoaded) {
    return [];
  }
  return message.artifact_ids.flatMap((artifactId): SessionArtifactCard[] => {
    const artifact = artifacts.find((candidate) => candidate.artifact_id === artifactId);
    if (
      artifact?.artifact_type === MCP_RESULT_CARD_ARTIFACT_TYPE &&
      !showMcpResultCards
    ) {
      return [];
    }
    if (artifact && INTERNAL_CHAT_ARTIFACT_TYPES.has(artifact.artifact_type)) {
      return [];
    }
    if (!artifact) {
      return [
        {
          id: artifactId,
          title: "Unavailable reference",
          type: "Unavailable",
          preview: UNAVAILABLE_REFERENCE_MESSAGE,
          available: false,
        },
      ];
    }
    const preview = readableArtifactPreview(artifact.content);
    const previewCharacters = Array.from(preview);
    return [
      {
        id: artifact.artifact_id,
        title: artifact.title,
        type: artifact.artifact_type,
        preview: artifact.artifact_type === MCP_RESULT_CARD_ARTIFACT_TYPE &&
            previewCharacters.length > MCP_CHAT_PREVIEW_CHARACTER_LIMIT
          ? `${previewCharacters.slice(0, MCP_CHAT_PREVIEW_CHARACTER_LIMIT).join("").trimEnd()}…`
          : preview,
        available: true,
      },
    ];
  });
}
