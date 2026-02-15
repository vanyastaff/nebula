//! The base Plugin trait.

use std::fmt::Debug;

use nebula_core::PluginKey;

use crate::PluginComponents;
use crate::PluginMetadata;

/// Base trait for all plugin types in Nebula.
///
/// A plugin is a user-visible, versionable packaging unit (e.g. "Slack",
/// "HTTP Request"). It provides metadata and registers its runtime components
/// (actions, credentials) via [`PluginComponents`].
///
/// This trait is **object-safe** so plugins can be stored as `Arc<dyn Plugin>`.
pub trait Plugin: Send + Sync + Debug + 'static {
    /// Returns the static metadata for this plugin.
    fn metadata(&self) -> &PluginMetadata;

    /// Register actions and credential requirements into `components`.
    fn register(&self, components: &mut PluginComponents);

    /// The normalized, unique key identifying this plugin type.
    fn key(&self) -> &PluginKey {
        self.metadata().key()
    }

    /// Human-readable display name.
    fn name(&self) -> &str {
        self.metadata().name()
    }

    /// Version number (1-based).
    fn version(&self) -> u32 {
        self.metadata().version()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal plugin implementation for testing.
    #[derive(Debug)]
    struct TestPlugin {
        meta: PluginMetadata,
    }

    impl Plugin for TestPlugin {
        fn metadata(&self) -> &PluginMetadata {
            &self.meta
        }

        fn register(&self, _components: &mut PluginComponents) {
            // No actions to register in the test stub.
        }
    }

    #[test]
    fn trait_default_methods() {
        let meta = PluginMetadata::builder("slack", "Slack")
            .version(2)
            .description("Send messages")
            .build()
            .unwrap();

        let plugin = TestPlugin { meta };

        assert_eq!(plugin.key().as_str(), "slack");
        assert_eq!(plugin.name(), "Slack");
        assert_eq!(plugin.version(), 2);
    }

    #[test]
    fn object_safety() {
        use std::sync::Arc;

        let meta = PluginMetadata::builder("test", "Test").build().unwrap();
        let plugin: Arc<dyn Plugin> = Arc::new(TestPlugin { meta });

        assert_eq!(plugin.key().as_str(), "test");
        assert_eq!(plugin.version(), 1);
    }
}
