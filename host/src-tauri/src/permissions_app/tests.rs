use app_host_kernel::ids::{AppId, ArtifactId, GrantId, RunId};
use app_host_kernel::primitives::artifact::{Artifact, Provenance};
use app_host_kernel::primitives::grant::{DataScope, GrantCondition, GrantDuration, GrantScope};
use chrono::Utc;
use serde_json::json;

use super::*;

fn proposal_artifact(content: serde_json::Value) -> Artifact {
    Artifact {
        artifact_id: ArtifactId::new("artifact-1"),
        artifact_type: ArtifactTypeName::new(PERMISSION_PROPOSAL_ARTIFACT),
        title: "Permission request".into(),
        content,
        provenance: Provenance {
            run_id: RunId::new("run-1"),
            capability: propose_grant_ref(),
            grant_id: GrantId::new("grant-1"),
            produced_by: permissions_app_id(),
            recorded_at: Utc::now(),
        },
    }
}

#[test]
fn manifest_exposes_read_only_introspection_and_non_authoritative_proposals() {
    let manifest = permissions_manifest();
    assert_eq!(manifest.app_id, permissions_app_id());
    assert_eq!(manifest.version, "0.3.0");
    assert_eq!(manifest.capabilities.len(), 3);
    assert_eq!(manifest.capabilities[0].name.as_str(), LIST_ACTIVE);
    assert_eq!(manifest.capabilities[0].effect, CapabilityEffect::ReadOnly);
    assert_eq!(manifest.capabilities[1].name.as_str(), LIST_REQUESTABLE);
    assert_eq!(manifest.capabilities[1].effect, CapabilityEffect::ReadOnly);
    assert_eq!(manifest.capabilities[2].name.as_str(), PROPOSE_GRANT);
    assert_eq!(manifest.artifact_types.len(), 1);
    assert!(manifest.grant_requests.is_empty());
    assert!(manifest.surfaces.is_empty());
}

#[test]
fn proposal_artifact_round_trips_the_fixed_general_policy() {
    let proposal = PermissionProposal {
        holder: AppId::new("chat"),
        scope: GrantScope::ExactCapability {
            provider: AppId::new("notes"),
            capability: CapabilityName::new("notes.create"),
        },
        data_scope: DataScope::None,
        condition: GrantCondition::RequiresApproval,
        duration: GrantDuration::NonExpiring,
        reason: "Create the event requested by the user".into(),
    };
    let artifact = proposal_artifact(serde_json::to_value(&proposal).unwrap());

    assert_eq!(proposal_from_artifact(&artifact).unwrap(), proposal);
}

#[test]
fn proposal_artifact_rejects_broad_or_less_interactive_authority() {
    for content in [
        json!({
            "holder": "chat",
            "scope": {"kind": "all-provider-capabilities", "provider": "notes"},
            "data_scope": {"kind": "none"},
            "condition": "requires-approval",
            "duration": {"kind": "non-expiring"},
            "reason": "Write a note"
        }),
        json!({
            "holder": "chat",
            "scope": {"kind": "exact-capability", "provider": "notes", "capability": "notes.create"},
            "data_scope": {"kind": "none"},
            "condition": "silent",
            "duration": {"kind": "non-expiring"},
            "reason": "Create an event"
        }),
    ] {
        assert!(proposal_from_artifact(&proposal_artifact(content)).is_err());
    }

    let mut forged = proposal_artifact(json!({
        "holder": "chat",
        "scope": {"kind": "exact-capability", "provider": "notes", "capability": "notes.create"},
        "data_scope": {"kind": "none"},
        "condition": "requires-approval",
        "duration": {"kind": "non-expiring"},
        "reason": "Create an event"
    }));
    forged.provenance.produced_by = AppId::new("untrusted-app");
    assert!(proposal_from_artifact(&forged).is_err());
}

#[test]
fn proposal_input_must_use_the_callers_host_bound_candidate_catalog() {
    let candidate = RequestablePermissionView {
        provider: AppId::new("notes"),
        provider_display_name: "Notes".into(),
        capability: CapabilityName::new("notes.create"),
        description: "Create a note".into(),
        effect: CapabilityEffect::LocalWrite,
    };
    let mut input = ProposalInput {
        provider: candidate.provider.clone(),
        capability: candidate.capability.clone(),
        reason: "Create the requested note".into(),
        snapshot: RequestablePermissionsSnapshot {
            holder: AppId::new("chat"),
            permissions: vec![candidate],
            omitted_count: 0,
        },
    };

    assert!(validate_proposal_input(&input, &AppId::new("chat")).is_ok());
    input.provider = AppId::new("calendar");
    assert!(validate_proposal_input(&input, &AppId::new("chat")).is_err());
    assert!(validate_proposal_input(&input, &AppId::new("other-app")).is_err());
}
