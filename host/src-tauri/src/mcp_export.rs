//! Virtual principals for the outbound MCP gateway.
//!
//! Each enabled export profile materializes as an installed app
//! `mcp-export/<profile-id>` that declares **nothing** — zero capabilities,
//! zero surfaces, zero handlers — and merely *requests* exact grants to the
//! profile's exported capabilities. The gateway then acts as this principal
//! through the ordinary action path: remote reach is exactly the
//! principal's live grants, nothing more, and every remote call is a run
//! attributed to it in the ledger.
//!
//! Installing the principal walks the same trusted-chrome grant
//! confirmation as any app install, so exporting is a local, visible,
//! per-capability decision — and `RequiresApproval` interaction keeps local
//! chrome authoritative for every single call afterwards.

use std::collections::BTreeMap;

use app_host_kernel::ids::{AppId, CapabilityName};
use app_host_kernel::kernel::Kernel;
use app_host_kernel::manifest::{seal, AppManifest, GrantRequest};
use app_host_kernel::primitives::grant::{DataScope, GrantCondition, GrantDuration, GrantScope};

use crate::config::{McpExportInteraction, McpExportProfile};

pub fn principal_app_id(profile_id: &str) -> AppId {
    AppId::new(format!("mcp-export/{profile_id}"))
}

fn grant_condition(interaction: McpExportInteraction) -> GrantCondition {
    match interaction {
        McpExportInteraction::RequiresApproval => GrantCondition::RequiresApproval,
        McpExportInteraction::Notify => GrantCondition::Notify,
        McpExportInteraction::Silent => GrantCondition::Silent,
    }
}

fn grant_duration(profile: &McpExportProfile) -> GrantDuration {
    match profile.expires_after_seconds {
        Some(seconds) => GrantDuration::ExpiresAfter { seconds },
        None => GrantDuration::NonExpiring,
    }
}

pub fn principal_install_parts(
    profile_id: &str,
    profile: &McpExportProfile,
) -> (
    app_host_kernel::manifest::SealedManifest,
    BTreeMap<CapabilityName, app_host_kernel::invocation::CapabilityHandler>,
) {
    let app_id = principal_app_id(profile_id);
    let grant_requests: Vec<GrantRequest> = profile
        .capabilities
        .iter()
        .map(|exported| GrantRequest {
            scope: GrantScope::ExactCapability {
                provider: AppId::new(&exported.provider),
                capability: CapabilityName::new(&exported.capability),
            },
            data_scope: DataScope::None,
            condition: grant_condition(profile.interaction),
            reason: format!(
                "Expose '{}/{}' to remote MCP clients through export profile '{}'",
                exported.provider, exported.capability, profile.display_name
            ),
            duration: grant_duration(profile),
        })
        .collect();
    let manifest = seal(AppManifest {
        app_id,
        version: "0.1.0".to_string(),
        display_name: format!("MCP export: {}", profile.display_name),
        description: "Virtual principal for the outbound MCP gateway. Declares no \
                      capabilities or surfaces; remote clients reach exactly this \
                      principal's live grants."
            .to_string(),
        capabilities: Vec::new(),
        surfaces: Vec::new(),
        agents: Vec::new(),
        skills: Vec::new(),
        assistant_profiles: Vec::new(),
        automations: Vec::new(),
        connectors: Vec::new(),
        config_declarations: Vec::new(),
        artifact_types: Vec::new(),
        extension_points: Vec::new(),
        extension_contributions: Vec::new(),
        grant_requests,
        event_subscriptions: Vec::new(),
    });
    (manifest, BTreeMap::new())
}

/// Uninstall the principal: the kernel revokes its grants, cancels its
/// active runs, and forgets it — remote reach ends immediately.
pub fn uninstall_principal(kernel: &mut Kernel, profile_id: &str) -> Result<(), String> {
    kernel
        .uninstall(&principal_app_id(profile_id))
        .map_err(|error| error.to_string())
}

pub fn is_principal_installed(kernel: &Kernel, profile_id: &str) -> bool {
    kernel.installed_app(&principal_app_id(profile_id)).is_ok()
}

pub fn stale_principal_ids(
    kernel: &Kernel,
    profiles: &BTreeMap<String, McpExportProfile>,
    transitions: &BTreeMap<String, bool>,
) -> Vec<String> {
    let desired = |id: &str| {
        transitions
            .get(id)
            .copied()
            .or_else(|| profiles.get(id).map(|profile| profile.enabled))
            .unwrap_or(false)
    };
    kernel
        .installed_apps()
        .filter_map(|app| {
            app.manifest
                .app_id
                .as_str()
                .strip_prefix("mcp-export/")
                .map(str::to_string)
        })
        .filter(|id| !desired(id))
        .collect()
}

pub fn pending_principal_installs(
    kernel: &Kernel,
    profiles: &BTreeMap<String, McpExportProfile>,
    transitions: &BTreeMap<String, bool>,
) -> Vec<(String, McpExportProfile)> {
    profiles
        .iter()
        .filter(|(profile_id, profile)| {
            transitions
                .get(*profile_id)
                .copied()
                .unwrap_or(profile.enabled)
                && !is_principal_installed(kernel, profile_id)
        })
        .map(|(profile_id, profile)| (profile_id.clone(), profile.clone()))
        .collect()
}
