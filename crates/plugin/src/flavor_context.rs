//! Execution-facing view of a validated worker flavor activation.

use nebula_core::{PluginKey, WorkerFlavorRevisionId};

use crate::FrozenPluginRegistry;

/// Immutable worker-flavor identity and registered plugin keys.
///
/// Construction requires a successfully frozen registry. Trust still depends
/// on the activation boundary supplying the artifact digest and runtime
/// contract version from trusted deployment inputs; this context is not a
/// capability or authenticity proof by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerFlavorContext {
    revision_id: WorkerFlavorRevisionId,
    plugin_keys: Box<[PluginKey]>,
}

impl WorkerFlavorContext {
    /// Derives the immutable context from a frozen activation product.
    #[must_use]
    pub fn from_registry(registry: &FrozenPluginRegistry) -> Self {
        let mut plugin_keys: Vec<_> = registry.iter().map(|(key, _)| key.clone()).collect();
        plugin_keys.sort_unstable();
        Self {
            revision_id: registry.revision().id(),
            plugin_keys: plugin_keys.into_boxed_slice(),
        }
    }

    /// Immutable flavor revision advertised by this worker.
    #[must_use]
    pub const fn revision_id(&self) -> WorkerFlavorRevisionId {
        self.revision_id
    }

    /// Canonically ordered plugin keys registered in this flavor.
    #[must_use]
    pub fn plugin_keys(&self) -> &[PluginKey] {
        &self.plugin_keys
    }
}
