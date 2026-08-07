use std::collections::{BTreeMap, BTreeSet};

use app_host_kernel::ids::{AppId, ArtifactId, CapabilityName, ResourceId};
use app_host_kernel::invocation::{CapabilityHandler, CapabilityOutcome, HandlerFailure};
use app_host_kernel::kernel::{CapabilityAuthorizationView, CapabilityUseView, Kernel};
use app_host_kernel::manifest::AppManifest;
use app_host_kernel::manifest::GrantRequest;
use app_host_kernel::primitives::artifact::Provenance;
use app_host_kernel::primitives::capability::{
    CapabilityDeclaration, CapabilityEffect, CapabilityRef,
};
use app_host_kernel::primitives::grant::{DataScope, GrantCondition, GrantDuration, GrantScope};
use app_host_kernel::JsonObject;
use serde::{Deserialize, Serialize};
use serde_json::json;

const MAX_ARTIFACT_TITLE_CHARS: usize = 256;
const MAX_ARTIFACT_CONTENT_BYTES: usize =
    app_host_kernel::services::artifacts::MAX_ARTIFACT_CONTENT_BYTES;

const ARTIFACTS_APP_ID: &str = "com.ma-zierl.kestral-artifacts";
pub const ARTIFACTS_QUERY: &str = "artifacts.query";
pub const ARTIFACTS_READ: &str = "artifacts.read";

pub fn artifacts_app_id() -> AppId {
    AppId::new(ARTIFACTS_APP_ID)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ArtifactAccessTarget {
    Artifact { artifact_id: ArtifactId },
    AllArtifacts,
}

impl ArtifactAccessTarget {
    fn data_scope(&self) -> DataScope {
        match self {
            Self::Artifact { artifact_id } => {
                DataScope::resources(vec![ResourceId::new(artifact_id.as_str().to_string())])
                    .expect("one artifact resource is valid")
            }
            Self::AllArtifacts => DataScope::AllResources,
        }
    }

    fn reason(&self, holder: &AppId) -> String {
        match self {
            Self::Artifact { .. } => format!("allow {holder} to list and read this artifact"),
            Self::AllArtifacts => {
                format!("allow {holder} to list and read all current and future artifacts")
            }
        }
    }
}

pub fn artifact_access_grant_requests(
    holder: &AppId,
    target: &ArtifactAccessTarget,
) -> Vec<GrantRequest> {
    [ARTIFACTS_QUERY, ARTIFACTS_READ]
        .into_iter()
        .map(|capability| GrantRequest {
            scope: GrantScope::ExactCapability {
                provider: artifacts_app_id(),
                capability: CapabilityName::new(capability),
            },
            data_scope: target.data_scope(),
            condition: GrantCondition::RequiresApproval,
            reason: target.reason(holder),
            duration: GrantDuration::NonExpiring,
        })
        .collect()
}

pub fn validate_access_target(
    kernel: &Kernel,
    target: &ArtifactAccessTarget,
) -> Result<(), String> {
    let ArtifactAccessTarget::Artifact { artifact_id } = target else {
        return Ok(());
    };
    kernel
        .artifacts()
        .any(|artifact| &artifact.artifact_id == artifact_id)
        .then_some(())
        .ok_or_else(|| format!("unknown artifact '{artifact_id}'"))
}

pub fn validate_grant_data_scope(
    kernel: &Kernel,
    scope: &GrantScope,
    data_scope: &DataScope,
) -> Result<(), String> {
    if scope.provider() != &artifacts_app_id() {
        return Ok(());
    }
    match data_scope {
        DataScope::None => Err(
            "Artifact permissions must allow selected artifacts or all current and future artifacts"
                .into(),
        ),
        DataScope::AllResources => Ok(()),
        DataScope::Resources { resource_ids } => {
            let artifact_ids = kernel
                .artifacts()
                .map(|artifact| artifact.artifact_id.as_str())
                .collect::<BTreeSet<_>>();
            let unknown = resource_ids
                .iter()
                .find(|resource_id| !artifact_ids.contains(resource_id.as_str()));
            match unknown {
                Some(resource_id) => Err(format!("unknown artifact '{resource_id}'")),
                None => Ok(()),
            }
        }
    }
}

fn authorized_artifact_resource_ids(
    kernel: &Kernel,
    authorizations: &[CapabilityAuthorizationView],
) -> Vec<ResourceId> {
    let existing = kernel
        .artifacts()
        .map(|artifact| artifact.artifact_id.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let allow_all = authorizations
        .iter()
        .any(|authorization| authorization.data_scope == DataScope::AllResources);
    let authorized = if allow_all {
        existing
    } else {
        authorizations
            .iter()
            .flat_map(|authorization| match &authorization.data_scope {
                DataScope::Resources { resource_ids } => resource_ids.as_slice(),
                DataScope::None | DataScope::AllResources => &[],
            })
            .filter(|resource_id| existing.contains(resource_id.as_str()))
            .map(|resource_id| resource_id.as_str().to_string())
            .collect()
    };
    authorized
        .iter()
        .map(|artifact_id| ResourceId::new(artifact_id.clone()))
        .collect()
}

pub fn contextualize_tools(kernel: &Kernel, available: &mut Vec<CapabilityUseView>) {
    available.retain_mut(|view| {
        if view.provider_app_id != artifacts_app_id() {
            return true;
        }
        let resource_ids = authorized_artifact_resource_ids(kernel, &view.authorizations);
        if resource_ids.is_empty() {
            return false;
        }
        if view.capability == CapabilityName::new(ARTIFACTS_READ) {
            let values = resource_ids
                .iter()
                .map(|resource_id| serde_json::Value::String(resource_id.to_string()))
                .collect::<Vec<_>>();
            if let Some(artifact_id_schema) = view
                .input_schema
                .get_mut("properties")
                .and_then(serde_json::Value::as_object_mut)
                .and_then(|properties| properties.get_mut("artifact_id"))
                .and_then(serde_json::Value::as_object_mut)
            {
                artifact_id_schema.insert("enum".into(), serde_json::Value::Array(values));
            }
            view.description =
                "Read one authorized artifact by exact id; call artifacts.query first to list ids"
                    .into();
        }
        true
    });
}

pub fn invocation_data_scope(
    kernel: &Kernel,
    holder: &AppId,
    capability: &CapabilityRef,
    input: &JsonObject,
) -> Option<DataScope> {
    if capability.provider != artifacts_app_id() {
        return None;
    }
    if capability.capability == CapabilityName::new(ARTIFACTS_READ) {
        return Some(
            input
                .get("artifact_id")
                .and_then(serde_json::Value::as_str)
                .map(|artifact_id| DataScope::Resources {
                    resource_ids: vec![ResourceId::new(artifact_id)],
                })
                .unwrap_or(DataScope::None),
        );
    }
    if capability.capability == CapabilityName::new(ARTIFACTS_QUERY) {
        let authorizations = kernel
            .grants_for(holder)
            .into_iter()
            .filter(|grant| grant.scope.covers(capability))
            .map(|grant| CapabilityAuthorizationView {
                data_scope: grant.data_scope.clone(),
                condition: grant.condition,
            })
            .collect::<Vec<_>>();
        let resource_ids = authorized_artifact_resource_ids(kernel, &authorizations);
        return Some(DataScope::resources(resource_ids).unwrap_or(DataScope::None));
    }
    Some(DataScope::None)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryRequest {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct ArtifactSummaryView {
    artifact_id: ArtifactId,
    artifact_type: app_host_kernel::ids::ArtifactTypeName,
    title: String,
    provenance: Provenance,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct CapabilityRefView {
    provider: AppId,
    capability: CapabilityName,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct ProvenanceView {
    run_id: app_host_kernel::ids::RunId,
    capability: CapabilityRefView,
    grant_id: app_host_kernel::ids::GrantId,
    produced_by: AppId,
    recorded_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct ArtifactReadView {
    artifact_id: ArtifactId,
    artifact_type: app_host_kernel::ids::ArtifactTypeName,
    title: String,
    content: serde_json::Value,
    provenance: ProvenanceView,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct QueryResponse {
    items: Vec<ArtifactSummaryView>,
    next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadRequest {
    artifact_id: ArtifactId,
}

fn default_limit() -> usize {
    20
}

fn query_capability() -> CapabilityDeclaration {
    CapabilityDeclaration {
        name: CapabilityName::new(ARTIFACTS_QUERY),
        description: "List authorized artifact metadata without content".into(),
        input_schema: serde_json::from_value(json!({
            "type": "object",
            "properties": {
                "cursor": {"type": ["string", "null"], "minLength": 1, "maxLength": 64},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50, "default": 20}
            },
            "required": [],
            "additionalProperties": false
        }))
        .expect("valid query schema"),
        effect: CapabilityEffect::ReadOnly,
        output_schema: Some(
            serde_json::from_value(json!({
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "maxItems": 50,
                        "items": {
                            "type": "object",
                            "properties": {
                                "artifact_id": {"type": "string", "minLength": 1, "maxLength": 128},
                                "artifact_type": {"type": "string", "minLength": 1, "maxLength": 128},
                                "title": {"type": "string", "minLength": 1, "maxLength": 256},
                                "provenance": {
                                    "type": "object",
                                    "properties": {
                                        "run_id": {"type": "string", "minLength": 1, "maxLength": 128},
                                        "capability": {
                                            "type": "object",
                                            "properties": {
                                                "provider": {"type": "string", "minLength": 1, "maxLength": 128},
                                                "capability": {"type": "string", "minLength": 1, "maxLength": 128}
                                            },
                                            "required": ["provider", "capability"],
                                            "additionalProperties": false
                                        },
                                        "grant_id": {"type": "string", "minLength": 1, "maxLength": 128},
                                        "produced_by": {"type": "string", "minLength": 1, "maxLength": 128},
                                        "recorded_at": {"type": "string", "format": "date-time"}
                                    },
                                    "required": ["run_id", "capability", "grant_id", "produced_by", "recorded_at"],
                                    "additionalProperties": false
                                }
                            },
                            "required": ["artifact_id", "artifact_type", "title", "provenance"],
                            "additionalProperties": false
                        }
                    },
                    "next_cursor": {"type": ["string", "null"], "minLength": 1, "maxLength": 64}
                },
                "required": ["items", "next_cursor"],
                "additionalProperties": false
            }))
            .expect("valid query output schema"),
        ),
    }
}

fn read_capability() -> CapabilityDeclaration {
    CapabilityDeclaration {
        name: CapabilityName::new(ARTIFACTS_READ),
        description: "Read one authorized artifact by exact id".into(),
        input_schema: serde_json::from_value(json!({
            "type": "object",
            "properties": {"artifact_id": {"type": "string", "minLength": 1, "maxLength": 128}},
            "required": ["artifact_id"],
            "additionalProperties": false
        }))
        .expect("valid read schema"),
        effect: CapabilityEffect::ReadOnly,
        output_schema: Some(
            serde_json::from_value(json!({
                "type": "object",
                "properties": {
                    "artifact_id": {"type": "string", "minLength": 1, "maxLength": 128},
                    "artifact_type": {"type": "string", "minLength": 1, "maxLength": 128},
                    "title": {"type": "string", "minLength": 1, "maxLength": 256},
                    "content": {},
                    "provenance": {
                        "type": "object",
                        "properties": {
                            "run_id": {"type": "string", "minLength": 1, "maxLength": 128},
                            "capability": {
                                "type": "object",
                                "properties": {
                                    "provider": {"type": "string", "minLength": 1, "maxLength": 128},
                                    "capability": {"type": "string", "minLength": 1, "maxLength": 128}
                                },
                                "required": ["provider", "capability"],
                                "additionalProperties": false
                            },
                            "grant_id": {"type": "string", "minLength": 1, "maxLength": 128},
                            "produced_by": {"type": "string", "minLength": 1, "maxLength": 128},
                            "recorded_at": {"type": "string", "format": "date-time"}
                        },
                        "required": ["run_id", "capability", "grant_id", "produced_by", "recorded_at"],
                        "additionalProperties": false
                    }
                },
                "required": ["artifact_id", "artifact_type", "title", "content", "provenance"],
                "additionalProperties": false
            }))
                .expect("valid read output schema"),
        ),
    }
}

pub fn artifacts_manifest() -> AppManifest {
    AppManifest {
        app_id: artifacts_app_id(),
        version: "1.0.0".into(),
        display_name: "Artifacts".into(),
        description: "Read-only artifact snapshots and provenance".into(),
        capabilities: vec![query_capability(), read_capability()],
        surfaces: vec![],
        agents: vec![],
        skills: vec![],
        assistant_profiles: vec![],
        automations: vec![],
        connectors: vec![],
        config_declarations: vec![],
        artifact_types: vec![],
        extension_points: vec![],
        extension_contributions: vec![],
        grant_requests: vec![],
        event_subscriptions: vec![],
    }
}

fn require_exact_artifact_scope(scope: &DataScope) -> Result<(), HandlerFailure> {
    match scope {
        DataScope::Resources { resource_ids } if !resource_ids.is_empty() => Ok(()),
        _ => Err(HandlerFailure(
            "artifact access requires exact artifact resource IDs".into(),
        )),
    }
}

pub fn artifacts_handlers() -> BTreeMap<CapabilityName, CapabilityHandler> {
    let mut handlers = BTreeMap::new();
    let query_handler: CapabilityHandler = Box::new(
        |input: &JsonObject, ctx: &app_host_kernel::invocation::InvocationContext| {
            require_exact_artifact_scope(&ctx.authorized_data_scope)?;
            let request: QueryRequest =
                serde_json::from_value(serde_json::Value::Object(input.clone()))
                    .map_err(|e| HandlerFailure(e.to_string()))?;
            if request.limit == 0 || request.limit > 50 {
                return Err(HandlerFailure("limit out of bounds".into()));
            }
            let page = ctx
                .artifacts
                .query(request.cursor.as_deref(), request.limit)
                .map_err(|e| HandlerFailure(e.to_string()))?;
            Ok(CapabilityOutcome {
                result: serde_json::to_value(QueryResponse {
                    items: page
                        .items
                        .into_iter()
                        .map(|item| ArtifactSummaryView {
                            artifact_id: item.artifact_id,
                            artifact_type: item.artifact_type,
                            title: item.title,
                            provenance: item.provenance,
                        })
                        .collect(),
                    next_cursor: page.next_cursor,
                })
                .map_err(|e| HandlerFailure(e.to_string()))?,
                artifacts: vec![],
            })
        },
    );
    handlers.insert(CapabilityName::new(ARTIFACTS_QUERY), query_handler);
    let read_handler: CapabilityHandler = Box::new(
        |input: &JsonObject, ctx: &app_host_kernel::invocation::InvocationContext| {
            require_exact_artifact_scope(&ctx.authorized_data_scope)?;
            let request: ReadRequest =
                serde_json::from_value(serde_json::Value::Object(input.clone()))
                    .map_err(|e| HandlerFailure(e.to_string()))?;
            let artifact = ctx
                .artifacts
                .read(&request.artifact_id)
                .map_err(|e| HandlerFailure(e.to_string()))?;
            if artifact.title.chars().count() > MAX_ARTIFACT_TITLE_CHARS {
                return Err(HandlerFailure(
                    "artifact title exceeds snapshot bound".into(),
                ));
            }
            let content_bytes =
                serde_json::to_vec(&artifact.content).map_err(|e| HandlerFailure(e.to_string()))?;
            if content_bytes.len() > MAX_ARTIFACT_CONTENT_BYTES {
                return Err(HandlerFailure(
                    "artifact content exceeds snapshot bound".into(),
                ));
            }
            Ok(CapabilityOutcome {
                result: serde_json::to_value(ArtifactReadView {
                    artifact_id: artifact.artifact_id,
                    artifact_type: artifact.artifact_type,
                    title: artifact.title,
                    content: artifact.content,
                    provenance: ProvenanceView {
                        run_id: artifact.provenance.run_id,
                        capability: CapabilityRefView {
                            provider: artifact.provenance.capability.provider,
                            capability: artifact.provenance.capability.capability,
                        },
                        grant_id: artifact.provenance.grant_id,
                        produced_by: artifact.provenance.produced_by,
                        recorded_at: artifact.provenance.recorded_at,
                    },
                })
                .map_err(|e| HandlerFailure(e.to_string()))?,
                artifacts: vec![],
            })
        },
    );
    handlers.insert(CapabilityName::new(ARTIFACTS_READ), read_handler);
    handlers
}

#[cfg(test)]
mod tests;
