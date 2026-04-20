//! `DiscoveredPlugin` — host-side `impl Plugin` wrapper over the data
//! returned by an out-of-process plugin during discovery.
//!
//! Holds the `PluginManifest` from the wire envelope (with any `plugin.toml`
//! `[plugin].id` override already applied) and the fully-resolved
//! `RemoteAction` list built from the wire `ActionDescriptor`s.
//!
//! `credentials()` and `resources()` intentionally return empty vecs.
//! Out-of-process credential and resource registration is gated on ADR-0025
//! slice 1d broker RPC.

use std::sync::Arc;

use nebula_action::Action;
use nebula_credential::AnyCredential;
use nebula_metadata::PluginManifest;
use nebula_plugin::{Plugin, PluginError};
use nebula_resource::AnyResource;

/// Host-side `impl Plugin` wrapper for an out-of-process plugin.
///
/// Constructed by discovery from a wire `MetadataResponse`. Calling
/// `ResolvedPlugin::from(discovered)` registers it in the host registry.
pub struct DiscoveredPlugin {
    manifest: PluginManifest,
    actions: Vec<Arc<dyn Action>>,
}

impl std::fmt::Debug for DiscoveredPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscoveredPlugin")
            .field("key", self.manifest.key())
            .field("version", self.manifest.version())
            .field("action_count", &self.actions.len())
            .finish()
    }
}

impl DiscoveredPlugin {
    /// Create a new `DiscoveredPlugin`.
    pub fn new(manifest: PluginManifest, actions: Vec<Arc<dyn Action>>) -> Self {
        Self { manifest, actions }
    }
}

impl Plugin for DiscoveredPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> Vec<Arc<dyn Action>> {
        self.actions.clone()
    }

    fn credentials(&self) -> Vec<Arc<dyn AnyCredential>> {
        // Out-of-process credential registration is gated on ADR-0025 slice 1d
        // broker RPC. Until that lands, discovered plugins report zero credentials.
        vec![]
    }

    fn resources(&self) -> Vec<Arc<dyn AnyResource>> {
        // Out-of-process resource registration is gated on ADR-0025 slice 1d
        // broker RPC. Until that lands, discovered plugins report zero resources.
        vec![]
    }

    fn on_load(&self) -> Result<(), PluginError> {
        Ok(())
    }

    fn on_unload(&self) -> Result<(), PluginError> {
        Ok(())
    }
}
