//! `CorePlugin` — first-party `core` plugin implementation.

use std::sync::Arc;

use nebula_action::ActionFactory;
use nebula_action::factory::GenericStatelessFactory;
use nebula_metadata::PluginManifest;
use nebula_plugin::Plugin;

use crate::actions::SetFields;

/// First-party core plugin.
///
/// Provides foundational utility actions under the `core` plugin key. Wire it
/// into the engine with `WorkflowEngine::with_plugin`:
///
/// ```rust,ignore
/// use std::sync::Arc;
/// use nebula_engine::WorkflowEngine;
/// use nebula_plugin::ResolvedPlugin;
/// use nebula_plugin_core::CorePlugin;
///
/// let plugin = Arc::new(ResolvedPlugin::from(CorePlugin::new())?);
/// let engine = engine.with_plugin(plugin)?;
/// ```
#[derive(Debug)]
pub struct CorePlugin {
    manifest: PluginManifest,
}

impl CorePlugin {
    /// Construct the core plugin with its canonical manifest.
    #[must_use]
    pub fn new() -> Self {
        let manifest = PluginManifest::builder("core", "Core")
            .description("Built-in utility actions available in every Nebula deployment")
            .build()
            .expect("CorePlugin manifest is statically valid");
        Self { manifest }
    }
}

impl Default for CorePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for CorePlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
        vec![Arc::new(GenericStatelessFactory::<SetFields>::new())]
    }
}

#[cfg(test)]
mod tests {
    use nebula_plugin::ResolvedPlugin;

    use super::*;

    #[test]
    fn plugin_key_is_core() {
        let plugin = CorePlugin::new();
        assert_eq!(plugin.key().as_str(), "core");
    }

    #[test]
    fn resolves_set_fields_action() {
        let resolved = ResolvedPlugin::from(CorePlugin::new())
            .expect("CorePlugin must resolve without errors");
        let key = nebula_core::ActionKey::new("core.set_fields").unwrap();
        assert!(
            resolved.action(&key).is_some(),
            "core.set_fields must be registered in the resolved plugin"
        );
    }

    #[test]
    fn namespace_invariant_holds() {
        // ResolvedPlugin::from validates that every action key starts with
        // "core.". A construction failure here means a key was mis-prefixed.
        let result = ResolvedPlugin::from(CorePlugin::new());
        assert!(
            result.is_ok(),
            "CorePlugin must pass namespace validation: {result:?}"
        );
    }
}
