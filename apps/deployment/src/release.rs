//! One source for the plugin set admitted by the API and executed by workers.

use std::sync::Arc;

use nebula_core::ArtifactSetDigest;
use nebula_plugin::{FrozenPluginRegistry, PluginRegistry, ResolvedPlugin, RuntimeContractVersion};
use nebula_plugin_core::CorePlugin;

/// The linked core plugin and its matching frozen dispatch registry.
#[derive(Debug)]
pub struct CoreRelease {
    pub(crate) plugin: Arc<ResolvedPlugin>,
    pub(crate) registry: Arc<FrozenPluginRegistry>,
}

/// A linked release failed admission before runtime construction.
#[derive(Debug, thiserror::Error)]
pub enum CoreReleaseError {
    /// The linked plugin manifest is invalid.
    #[error("core plugin manifest is invalid")]
    Manifest(#[from] nebula_plugin::ManifestError),
    /// Plugin resolution or registration rejected the linked plugin.
    #[error("core plugin registration failed")]
    Plugin(#[from] nebula_plugin::PluginError),
    /// The registered set cannot form an exact execution flavor.
    #[error("worker registry could not be frozen")]
    Freeze(#[from] nebula_plugin::RegistryFreezeError),
}

impl CoreRelease {
    /// Admit the statically linked core plugin for a deployment artifact.
    ///
    /// The caller supplies artifact identity from its trusted release manifest;
    /// this function does not authenticate the binary or read environment state.
    ///
    /// # Errors
    /// Returns the failed manifest, registration or registry admission stage.
    #[tracing::instrument(skip_all)]
    pub fn new(artifact: ArtifactSetDigest) -> Result<Self, CoreReleaseError> {
        let plugin = Arc::new(ResolvedPlugin::from(CorePlugin::try_new()?)?);
        let mut registry = PluginRegistry::new();
        registry.register(Arc::clone(&plugin))?;
        let registry = Arc::new(registry.freeze(artifact, RuntimeContractVersion::current())?);
        Ok(Self { plugin, registry })
    }

    /// Consume the release as the API's immutable validation/dispatch catalog.
    #[must_use]
    pub fn into_registry(self) -> Arc<FrozenPluginRegistry> {
        self.registry
    }
}
