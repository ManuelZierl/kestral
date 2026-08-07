//! App manifests: exhaustive declarations of app behavior.
//!
//! A manifest declares, exhaustively, everything an app contributes and
//! needs: capabilities, surfaces, agents, skills, automations, connectors,
//! artifact types, grant requests, and event subscriptions. Undeclared
//! behavior is impossible by construction — kernel services refuse anything
//! not in the manifest.
//!
//! Agents, skills, and automations are pure data to the kernel:
//! runtime adapters and automation apps in userland give them behavior.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::errors::{KernelError, KernelResult};
use crate::ids::{
    AppId, ArtifactTypeName, ConfigName, EventTopic, ExtensionPointName, SecretName, SurfaceName,
};
use crate::primitives::capability::{CapabilityDeclaration, CapabilityRef};
use crate::primitives::grant::{DataScope, GrantCondition, GrantDuration, GrantScope};
use crate::primitives::surface::{SurfaceDeclaration, SurfaceKind};
use crate::JsonObject;

/// A permission need declared up front; issuance is the broker's decision.
///
/// `duration` is required: a manifest must say "non-expiring" explicitly
/// instead of getting the most permissive lifetime by omission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantRequest {
    pub scope: GrantScope,
    pub data_scope: DataScope,
    pub condition: GrantCondition,
    pub reason: String,
    pub duration: GrantDuration,
}

impl GrantRequest {
    pub fn validate(&self) -> KernelResult<()> {
        self.data_scope.validate()
    }
}

/// A reasoning policy: instructions plus capability bindings. Pure data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDeclaration {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub capability_bindings: Vec<CapabilityRef>,
}

/// Reusable instruction/context consumed by agents. Pure data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDeclaration {
    pub name: String,
    pub description: String,
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantProfileDeclaration {
    pub profile_name: String,
    pub title: String,
    pub description: String,
    pub instruction_skill_refs: Vec<String>,
    pub suggested_capability_refs: Vec<CapabilityRef>,
    #[serde(default)]
    pub suggested_agent_engine_contract: Option<String>,
    pub starter_prompts: Vec<String>,
}

/// A stored trigger description. The kernel never fires triggers itself; an
/// automation app in userland does, and the kernel only sees the run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationDeclaration {
    pub name: String,
    pub description: String,
    pub trigger: String,
}

/// App configuration the app declares but the host stores and renders.
/// The schema constrains values; apps never own the settings UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigDeclaration {
    pub name: ConfigName,
    pub title: String,
    pub description: String,
    pub json_schema: JsonObject,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
}

/// External system access, mediated by the broker. Names the secrets the
/// connector needs; the broker holds them and apps never see raw values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorDeclaration {
    pub name: String,
    pub description: String,
    pub secret_names: Vec<SecretName>,
    #[serde(default)]
    pub config_schema: Option<JsonObject>,
}

/// Schema for a durable object this app produces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactTypeDeclaration {
    pub name: ArtifactTypeName,
    pub description: String,
    pub json_schema: JsonObject,
}

/// A deliberate, versioned integration seam another app may contribute to.
///
/// Extension points are manifest metadata, not a sixth kernel primitive. The
/// host owns mounting and context delivery; contributed surfaces stay sandboxed
/// and intent-only. Contract versions are major versions: contributions must
/// match exactly, and a breaking contract change requires a new version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionPointDeclaration {
    pub name: ExtensionPointName,
    pub contract_version: u32,
    pub context_schema: JsonObject,
}

/// A surface contribution from this app into an extension point of another
/// installed app. It can be inactive while the target app is absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionContribution {
    pub target_app: AppId,
    pub extension_point: ExtensionPointName,
    pub contract_version: u32,
    pub surface: SurfaceName,
}

/// The exhaustive declaration of one app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppManifest {
    pub app_id: AppId,
    pub version: String,
    pub display_name: String,
    pub description: String,
    #[serde(default)]
    pub capabilities: Vec<CapabilityDeclaration>,
    #[serde(default)]
    pub surfaces: Vec<SurfaceDeclaration>,
    #[serde(default)]
    pub agents: Vec<AgentDeclaration>,
    #[serde(default)]
    pub skills: Vec<SkillDeclaration>,
    #[serde(default)]
    pub assistant_profiles: Vec<AssistantProfileDeclaration>,
    #[serde(default)]
    pub automations: Vec<AutomationDeclaration>,
    #[serde(default)]
    pub connectors: Vec<ConnectorDeclaration>,
    #[serde(default)]
    pub config_declarations: Vec<ConfigDeclaration>,
    #[serde(default)]
    pub artifact_types: Vec<ArtifactTypeDeclaration>,
    #[serde(default)]
    pub extension_points: Vec<ExtensionPointDeclaration>,
    #[serde(default)]
    pub extension_contributions: Vec<ExtensionContribution>,
    #[serde(default)]
    pub grant_requests: Vec<GrantRequest>,
    #[serde(default)]
    pub event_subscriptions: Vec<EventTopic>,
}

impl AppManifest {
    /// Compare declarations for the in-place upgrade boundary.
    ///
    /// Only the app's version and top-level presentation text may change. All
    /// capability, surface, data, grant, schema, and other contribution
    /// fields remain part of the authority or behavioral contract.
    pub fn has_same_upgrade_contract(&self, candidate: &Self) -> bool {
        let mut current = self.clone();
        let mut candidate = candidate.clone();
        for manifest in [&mut current, &mut candidate] {
            manifest.version.clear();
            manifest.display_name.clear();
            manifest.description.clear();
        }
        current == candidate
    }

    /// Identity fields must be non-empty and contribution names unique per
    /// kind. The registry enforces this once at install — the single
    /// boundary all manifests cross.
    pub fn require_consistent(&self) -> KernelResult<()> {
        for (field, value) in [
            ("app_id", self.app_id.as_str()),
            ("version", self.version.as_str()),
            ("display_name", self.display_name.as_str()),
        ] {
            if value.is_empty() {
                return Err(KernelError::ManifestIdentityInvalid { field });
            }
        }
        require_unique(
            &self.app_id,
            "capability",
            self.capabilities.iter().map(|c| c.name.as_str()),
        )?;
        require_unique(
            &self.app_id,
            "surface",
            self.surfaces.iter().map(|s| s.name.as_str()),
        )?;
        let declared_capabilities: BTreeSet<&str> =
            self.capabilities.iter().map(|c| c.name.as_str()).collect();
        for surface in &self.surfaces {
            if surface.kind == SurfaceKind::Form && surface.intents.len() != 1 {
                return Err(KernelError::ManifestSurfaceIntentInvalid {
                    app: self.app_id.clone(),
                    surface: surface.name.clone(),
                    message: format!(
                        "form surfaces must declare exactly one intent, found {}",
                        surface.intents.len()
                    ),
                });
            }
            for intent in &surface.intents {
                if intent.provider == self.app_id
                    && !declared_capabilities.contains(intent.capability.as_str())
                {
                    return Err(KernelError::ManifestSurfaceIntentInvalid {
                        app: self.app_id.clone(),
                        surface: surface.name.clone(),
                        message: format!(
                            "intent references undeclared capability '{}'",
                            intent.capability.as_str()
                        ),
                    });
                }
            }
        }
        require_unique(
            &self.app_id,
            "agent",
            self.agents.iter().map(|a| a.name.as_str()),
        )?;
        require_unique(
            &self.app_id,
            "skill",
            self.skills.iter().map(|s| s.name.as_str()),
        )?;
        let declared_skills: BTreeSet<&str> = self.skills.iter().map(|s| s.name.as_str()).collect();
        require_unique(
            &self.app_id,
            "assistant profile",
            self.assistant_profiles
                .iter()
                .map(|p| p.profile_name.as_str()),
        )?;
        for profile in &self.assistant_profiles {
            for (field, value) in [
                ("profile_name", profile.profile_name.as_str()),
                ("title", profile.title.as_str()),
                ("description", profile.description.as_str()),
            ] {
                if value.is_empty() {
                    return Err(KernelError::ManifestIdentityInvalid { field });
                }
            }
            for skill_ref in &profile.instruction_skill_refs {
                if !declared_skills.contains(skill_ref.as_str()) {
                    return Err(KernelError::ManifestContributionInvalid {
                        app: self.app_id.clone(),
                        contribution: "assistant profile instruction skill",
                        names: vec![skill_ref.clone()],
                    });
                }
            }
        }
        require_unique(
            &self.app_id,
            "automation",
            self.automations.iter().map(|a| a.name.as_str()),
        )?;
        require_unique(
            &self.app_id,
            "connector",
            self.connectors.iter().map(|c| c.name.as_str()),
        )?;
        require_unique(
            &self.app_id,
            "config declaration",
            self.config_declarations.iter().map(|c| c.name.as_str()),
        )?;
        for request in &self.grant_requests {
            request.validate()?;
        }
        require_unique(
            &self.app_id,
            "artifact type",
            self.artifact_types.iter().map(|t| t.name.as_str()),
        )?;
        require_unique(
            &self.app_id,
            "extension point",
            self.extension_points
                .iter()
                .map(|point| point.name.as_str()),
        )?;
        for contribution in &self.extension_contributions {
            if !self
                .surfaces
                .iter()
                .any(|surface| surface.name == contribution.surface)
            {
                return Err(KernelError::ManifestExtensionContributionInvalid {
                    app: self.app_id.clone(),
                    surface: contribution.surface.clone(),
                    message: "contribution surface is not declared by this app".into(),
                });
            }
            if contribution.target_app == self.app_id {
                return Err(KernelError::ManifestExtensionContributionInvalid {
                    app: self.app_id.clone(),
                    surface: contribution.surface.clone(),
                    message: "an app cannot contribute to its own extension point".into(),
                });
            }
        }
        require_unique(
            &self.app_id,
            "event subscription",
            self.event_subscriptions.iter().map(|t| t.as_str()),
        )?;
        Ok(())
    }

    pub fn declared_secret_names(&self) -> Vec<SecretName> {
        self.connectors
            .iter()
            .flat_map(|connector| connector.secret_names.iter().cloned())
            .collect()
    }
}

fn require_unique<'a>(
    app: &AppId,
    contribution: &'static str,
    names: impl Iterator<Item = &'a str>,
) -> KernelResult<()> {
    let mut seen = std::collections::BTreeSet::new();
    let mut duplicates = std::collections::BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            duplicates.insert(name.to_string());
        }
    }
    if duplicates.is_empty() {
        Ok(())
    } else {
        Err(KernelError::ManifestContributionInvalid {
            app: app.clone(),
            contribution,
            names: duplicates.into_iter().collect(),
        })
    }
}

/// A manifest plus the content hash the registry verifies before install.
///
/// A seal is tamper evidence only — anyone can compute it, so it proves the
/// manifest was not corrupted in transit, not who published it.
/// TODO: add publisher-key signing (a real SignedManifest) so app identity
/// is verifiable against a publisher, not merely tamper-evident.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedManifest {
    pub manifest: AppManifest,
    pub content_hash: String,
}

pub fn manifest_content_hash(manifest: &AppManifest) -> String {
    let canonical = serde_json::to_string(manifest).expect("manifests always serialize");
    let digest = Sha256::digest(canonical.as_bytes());
    format!("{digest:x}")
}

pub fn seal(manifest: AppManifest) -> SealedManifest {
    let content_hash = manifest_content_hash(&manifest);
    SealedManifest {
        manifest,
        content_hash,
    }
}
