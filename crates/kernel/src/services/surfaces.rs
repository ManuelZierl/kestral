//! Surface Manager.
//!
//! Tracks which app surfaces are open and validates that action intents come
//! from a surface the app actually declared. App UI has no direct execution
//! path: a surface can only emit an `ActionIntent`, which the kernel drives
//! through the single action path.
//!
//! Rendering itself (sandboxed webview containers) belongs to the shell; the
//! kernel side owns the binding and the intent contract.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::errors::{KernelError, KernelResult};
use crate::ids::{new_surface_instance_id, AppId, SurfaceInstanceId, SurfaceName};
use crate::services::registry::Registry;

/// One open, sandboxed surface instance of an installed app.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceBinding {
    pub app_id: AppId,
    pub surface: SurfaceName,
    pub instance_id: SurfaceInstanceId,
}

pub struct SurfaceManager {
    open: BTreeSet<SurfaceBinding>,
}

impl SurfaceManager {
    pub fn new() -> Self {
        Self {
            open: BTreeSet::new(),
        }
    }

    pub fn open(
        &mut self,
        registry: &Registry,
        app_id: &AppId,
        surface: &SurfaceName,
    ) -> KernelResult<SurfaceBinding> {
        // Fails if the app is unknown or never declared this surface.
        registry.surface(app_id, surface)?;
        let binding = SurfaceBinding {
            app_id: app_id.clone(),
            surface: surface.clone(),
            instance_id: new_surface_instance_id(),
        };
        self.open.insert(binding.clone());
        Ok(binding)
    }

    pub fn close(&mut self, binding: &SurfaceBinding) {
        self.open.remove(binding);
    }

    /// Uninstall-time cleanup: no binding may outlive its app.
    pub fn close_all_for(&mut self, app_id: &AppId) {
        self.open.retain(|binding| binding.app_id != *app_id);
    }

    pub fn require_open(&self, binding: &SurfaceBinding) -> KernelResult<()> {
        if self.open.contains(binding) {
            Ok(())
        } else {
            Err(KernelError::SurfaceNotOpen {
                app: binding.app_id.clone(),
                surface: binding.surface.clone(),
            })
        }
    }
}

impl Default for SurfaceManager {
    fn default() -> Self {
        Self::new()
    }
}
