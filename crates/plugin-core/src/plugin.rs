//! `CorePlugin` — first-party `core` plugin implementation.

use std::sync::Arc;

use nebula_action::ActionFactory;
use nebula_action::factory::{GenericControlFactory, GenericStatelessFactory};
use nebula_metadata::{ManifestError, PluginManifest};
use nebula_plugin::Plugin;

use crate::actions::{
    Aggregate, CoreIf, CoreSwitch, DateTimeAction, Filter, JsonTransform, SetFields,
};

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
/// let plugin = Arc::new(ResolvedPlugin::from(CorePlugin::try_new()?)?);
/// let engine = engine.with_plugin(plugin)?;
/// ```
#[derive(Debug)]
pub struct CorePlugin {
    manifest: PluginManifest,
}

impl CorePlugin {
    /// Construct the core plugin with its canonical manifest.
    ///
    /// Returns `Err` if the plugin key or manifest is structurally invalid.
    /// For the built-in `core` plugin this should never fail in practice;
    /// the fallible return is required because `PluginManifest::builder().build()`
    /// validates and normalizes the key at construction time.
    pub fn try_new() -> Result<Self, ManifestError> {
        let manifest = PluginManifest::builder("core", "Core")
            .description("Built-in utility actions available in every Nebula deployment")
            .build()?;
        Ok(Self { manifest })
    }
}

impl Plugin for CorePlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
        vec![
            Arc::new(GenericStatelessFactory::<Aggregate>::new()),
            Arc::new(GenericStatelessFactory::<SetFields>::new()),
            Arc::new(GenericStatelessFactory::<JsonTransform>::new()),
            Arc::new(GenericStatelessFactory::<DateTimeAction>::new()),
            Arc::new(GenericStatelessFactory::<Filter>::new()),
            Arc::new(GenericControlFactory::<CoreIf>::new()),
            Arc::new(GenericControlFactory::<CoreSwitch>::new()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use nebula_plugin::ResolvedPlugin;

    use super::*;

    #[test]
    fn plugin_key_is_core() {
        let plugin = CorePlugin::try_new().expect("CorePlugin::try_new must succeed");
        assert_eq!(plugin.key().as_str(), "core");
    }

    #[test]
    fn resolves_aggregate_action() {
        let resolved =
            ResolvedPlugin::from(CorePlugin::try_new().expect("CorePlugin::try_new must succeed"))
                .expect("CorePlugin must resolve without errors");
        let key = nebula_core::ActionKey::new("core.aggregate").unwrap();
        assert!(
            resolved.action(&key).is_some(),
            "core.aggregate must be registered in the resolved plugin"
        );
    }

    #[test]
    fn resolves_set_fields_action() {
        let resolved =
            ResolvedPlugin::from(CorePlugin::try_new().expect("CorePlugin::try_new must succeed"))
                .expect("CorePlugin must resolve without errors");
        let key = nebula_core::ActionKey::new("core.set_fields").unwrap();
        assert!(
            resolved.action(&key).is_some(),
            "core.set_fields must be registered in the resolved plugin"
        );
    }

    #[test]
    fn resolves_json_transform_action() {
        let resolved =
            ResolvedPlugin::from(CorePlugin::try_new().expect("CorePlugin::try_new must succeed"))
                .expect("CorePlugin must resolve without errors");
        let key = nebula_core::ActionKey::new("core.json_transform").unwrap();
        assert!(
            resolved.action(&key).is_some(),
            "core.json_transform must be registered in the resolved plugin"
        );
    }

    #[test]
    fn resolves_datetime_action() {
        let resolved =
            ResolvedPlugin::from(CorePlugin::try_new().expect("CorePlugin::try_new must succeed"))
                .expect("CorePlugin must resolve without errors");
        let key = nebula_core::ActionKey::new("core.datetime").unwrap();
        assert!(
            resolved.action(&key).is_some(),
            "core.datetime must be registered in the resolved plugin"
        );
    }

    #[test]
    fn resolves_filter_action() {
        let resolved =
            ResolvedPlugin::from(CorePlugin::try_new().expect("CorePlugin::try_new must succeed"))
                .expect("CorePlugin must resolve without errors");
        let key = nebula_core::ActionKey::new("core.filter").unwrap();
        assert!(
            resolved.action(&key).is_some(),
            "core.filter must be registered in the resolved plugin"
        );
    }

    #[test]
    fn resolves_if_action() {
        let resolved =
            ResolvedPlugin::from(CorePlugin::try_new().expect("CorePlugin::try_new must succeed"))
                .expect("CorePlugin must resolve without errors");
        let key = nebula_core::ActionKey::new("core.if").unwrap();
        assert!(
            resolved.action(&key).is_some(),
            "core.if must be registered in the resolved plugin"
        );
    }

    #[test]
    fn resolves_switch_action() {
        let resolved =
            ResolvedPlugin::from(CorePlugin::try_new().expect("CorePlugin::try_new must succeed"))
                .expect("CorePlugin must resolve without errors");
        let key = nebula_core::ActionKey::new("core.switch").unwrap();
        assert!(
            resolved.action(&key).is_some(),
            "core.switch must be registered in the resolved plugin"
        );
    }

    #[test]
    fn namespace_invariant_holds() {
        // ResolvedPlugin::from validates that every action key starts with
        // "core.". A construction failure here means a key was mis-prefixed.
        let core = CorePlugin::try_new().expect("CorePlugin::try_new must succeed");
        let result = ResolvedPlugin::from(core);
        assert!(
            result.is_ok(),
            "CorePlugin must pass namespace validation: {result:?}"
        );
    }
}
