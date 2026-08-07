//! Registry & Identity.
//!
//! Catalog of installed apps: manifests, signatures, versions. Every kernel
//! interaction is attributed to an app identity, and the manifest is the
//! exhaustive truth about what an app may contribute — services consult the
//! registry and refuse anything undeclared.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::clock::Clock;
use crate::errors::{KernelError, KernelResult};
use crate::ids::{AppId, ArtifactTypeName, EventTopic, SurfaceName};
use crate::manifest::{
    manifest_content_hash, AppManifest, ArtifactTypeDeclaration, SealedManifest,
};
use crate::primitives::capability::{CapabilityDeclaration, CapabilityRef};
use crate::primitives::surface::SurfaceDeclaration;
use crate::schema::require_valid_schema;
use crate::services::ledger::LedgerEvent;

pub const APP_DATA_CHANGED_TOPIC: &str = "app-data-changed";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledApp {
    pub manifest: AppManifest,
    pub content_hash: String,
    pub installed_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct Registry {
    clock: Arc<dyn Clock>,
    apps: BTreeMap<AppId, InstalledApp>,
}

impl Registry {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            apps: BTreeMap::new(),
        }
    }

    pub fn install(&mut self, sealed_manifest: SealedManifest) -> KernelResult<&InstalledApp> {
        self.validate_install(&sealed_manifest)?;
        let SealedManifest {
            manifest,
            content_hash,
        } = sealed_manifest;

        let app_id = manifest.app_id.clone();
        let installed = InstalledApp {
            manifest,
            content_hash,
            installed_at: self.clock.now(),
        };
        self.apps.insert(app_id.clone(), installed);
        Ok(&self.apps[&app_id])
    }

    /// Check every install boundary without changing the registry. Kernel
    /// install uses this before trusted-chrome subscription disclosure so a
    /// denied prompt never follows an invalid manifest.
    pub fn validate_install(&self, sealed_manifest: &SealedManifest) -> KernelResult<()> {
        let manifest = &sealed_manifest.manifest;
        self.validate_local(sealed_manifest)?;
        if self.apps.contains_key(&manifest.app_id) {
            // Generic install remains a fresh-registration operation. Safe
            // compatible replacements use `Registry::upgrade`.
            return Err(KernelError::AppAlreadyInstalled(manifest.app_id.clone()));
        }
        // Cross-app contributions are allowed to remain dormant. Mounting is
        // resolved from the current catalog, so install order and a target's
        // later replacement cannot turn an installed contributor into corrupt
        // registry state.
        Ok(())
    }

    /// Replace an installed declaration without changing its registration
    /// identity. The caller must stage this registry before its durable commit.
    pub fn upgrade(&mut self, sealed_manifest: SealedManifest) -> KernelResult<()> {
        let app_id = sealed_manifest.manifest.app_id.clone();
        let installed_at = self.app(&app_id)?.installed_at;
        self.validate_local(&sealed_manifest)?;
        if !self
            .app(&app_id)?
            .manifest
            .has_same_upgrade_contract(&sealed_manifest.manifest)
        {
            return Err(KernelError::AppUpgradeContractChanged { app: app_id });
        }

        let SealedManifest {
            manifest,
            content_hash,
        } = sealed_manifest;
        self.apps.insert(
            manifest.app_id.clone(),
            InstalledApp {
                manifest,
                content_hash,
                installed_at,
            },
        );
        Ok(())
    }

    /// Validate everything that depends only on the manifest itself — content
    /// hash, internal consistency, declared schemas, and event topics — with
    /// no reference to other installed apps. Both live install and two-phase
    /// recovery run this before an app can enter the catalog.
    fn validate_local(&self, sealed_manifest: &SealedManifest) -> KernelResult<()> {
        let manifest = &sealed_manifest.manifest;
        let content_hash = &sealed_manifest.content_hash;
        if *content_hash != manifest_content_hash(manifest) {
            return Err(KernelError::ManifestContentHashMismatch(
                manifest.app_id.clone(),
            ));
        }
        manifest.require_consistent()?;
        for capability in &manifest.capabilities {
            require_valid_schema(
                &capability.input_schema,
                &format!("input schema of capability '{}'", capability.name),
            )?;
            if let Some(output_schema) = &capability.output_schema {
                require_valid_schema(
                    output_schema,
                    &format!("output schema of capability '{}'", capability.name),
                )?;
            }
        }
        for artifact_type in &manifest.artifact_types {
            require_valid_schema(
                &artifact_type.json_schema,
                &format!("schema of artifact type '{}'", artifact_type.name),
            )?;
        }
        for declaration in &manifest.config_declarations {
            require_valid_schema(
                &declaration.json_schema,
                &format!("config schema of declaration '{}'", declaration.name),
            )?;
        }
        for connector in &manifest.connectors {
            if let Some(schema) = &connector.config_schema {
                require_valid_schema(
                    schema,
                    &format!("config schema of connector '{}'", connector.name),
                )?;
            }
        }
        for point in &manifest.extension_points {
            require_valid_schema(
                &point.context_schema,
                &format!("context schema of extension point '{}'", point.name),
            )?;
        }
        for topic in &manifest.event_subscriptions {
            if !LedgerEvent::ALL_KINDS.contains(&topic.as_str())
                && topic.as_str() != APP_DATA_CHANGED_TOPIC
            {
                return Err(KernelError::UnknownEventTopic {
                    app: manifest.app_id.clone(),
                    topic: topic.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn uninstall(&mut self, app_id: &AppId) -> KernelResult<()> {
        self.apps
            .remove(app_id)
            .map(|_| ())
            .ok_or_else(|| KernelError::UnknownApp(app_id.clone()))
    }

    pub fn installed_apps(&self) -> impl Iterator<Item = &InstalledApp> {
        self.apps.values()
    }

    /// Rebuild the catalog from durable state.
    ///
    /// Each manifest is validated in isolation — hash, consistency, schemas,
    /// topics — and inserted. Cross-app extension contributions are
    /// deliberately *not* resolved here: an unresolved contribution is dormant,
    /// not invalid, so recovery never depends on the order apps are emitted in
    /// and uninstalling a target app cannot stop the kernel from restoring.
    /// The host matches contributions to extension points (exact target, point,
    /// and contract version) when it mounts them. Incompatible contributions
    /// mount nothing but remain visible as dormant in host-owned app status.
    pub fn restore(clock: Arc<dyn Clock>, apps: Vec<InstalledApp>) -> KernelResult<Self> {
        let mut registry = Self::new(clock);
        for app in apps {
            let sealed = SealedManifest {
                manifest: app.manifest.clone(),
                content_hash: app.content_hash.clone(),
            };
            registry.validate_local(&sealed)?;
            let app_id = app.manifest.app_id.clone();
            if registry.apps.insert(app_id.clone(), app).is_some() {
                return Err(KernelError::Durability(format!(
                    "duplicate installed app '{}'",
                    app_id
                )));
            }
        }
        Ok(registry)
    }

    pub fn app(&self, app_id: &AppId) -> KernelResult<&InstalledApp> {
        self.apps
            .get(app_id)
            .ok_or_else(|| KernelError::UnknownApp(app_id.clone()))
    }

    pub fn capability(
        &self,
        capability_ref: &CapabilityRef,
    ) -> KernelResult<&CapabilityDeclaration> {
        self.app(&capability_ref.provider)?
            .manifest
            .capabilities
            .iter()
            .find(|declaration| declaration.name == capability_ref.capability)
            .ok_or_else(|| KernelError::UndeclaredCapability {
                app: capability_ref.provider.clone(),
                capability: capability_ref.capability.clone(),
            })
    }

    pub fn surface(&self, app_id: &AppId, name: &SurfaceName) -> KernelResult<&SurfaceDeclaration> {
        self.app(app_id)?
            .manifest
            .surfaces
            .iter()
            .find(|declaration| declaration.name == *name)
            .ok_or_else(|| KernelError::UndeclaredSurface {
                app: app_id.clone(),
                surface: name.clone(),
            })
    }

    pub fn artifact_type(
        &self,
        app_id: &AppId,
        name: &ArtifactTypeName,
    ) -> KernelResult<&ArtifactTypeDeclaration> {
        self.app(app_id)?
            .manifest
            .artifact_types
            .iter()
            .find(|declaration| declaration.name == *name)
            .ok_or_else(|| KernelError::UndeclaredArtifactType {
                app: app_id.clone(),
                artifact_type: name.clone(),
            })
    }

    pub fn is_subscribed(&self, app_id: &AppId, topic: &EventTopic) -> bool {
        self.apps
            .get(app_id)
            .is_some_and(|app| app.manifest.event_subscriptions.contains(topic))
    }
}
