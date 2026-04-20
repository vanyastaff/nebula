//! Base `Plugin` trait — canonical per ADR-0027.
//!
//! A plugin bundles actions / credentials / resources under a versioned
//! identity. It returns the runnable trait objects directly (matching canon
//! §3.5 "Plugin = [ registry: Actions + Resources + Credentials ]"), not
//! descriptors. One plugin → one `ResolvedPlugin` after registration.

use std::{fmt::Debug, sync::Arc};

use nebula_core::PluginKey;
use nebula_metadata::PluginManifest;
use semver::Version;

use crate::PluginError;

/// Base trait for all plugin types in Nebula.
///
/// A plugin is a user-visible, versionable packaging unit (e.g. "Slack",
/// "HTTP Request"). It provides a manifest describing the plugin's identity
/// and version, and optionally declares the actions, credentials, and resources
/// it contributes to the engine.
///
/// Implementers must provide [`Plugin::manifest`]. All other methods have
/// default implementations and can be overridden as needed.
///
/// This trait is **object-safe** so plugins can be stored as `Arc<dyn Plugin>`.
pub trait Plugin: Send + Sync + Debug + 'static {
    /// Returns the static manifest for this plugin.
    fn manifest(&self) -> &PluginManifest;

    /// Actions this plugin provides.
    ///
    /// Called once at registration time by [`crate::ResolvedPlugin::from`].
    /// Returns an empty list by default.
    fn actions(&self) -> Vec<Arc<dyn nebula_action::Action>> {
        vec![]
    }

    /// Credential types this plugin provides.
    ///
    /// Called once at registration time by [`crate::ResolvedPlugin::from`].
    /// Returns an empty list by default.
    fn credentials(&self) -> Vec<Arc<dyn nebula_credential::AnyCredential>> {
        vec![]
    }

    /// Resource types this plugin provides.
    ///
    /// Called once at registration time by [`crate::ResolvedPlugin::from`].
    /// Returns an empty list by default.
    fn resources(&self) -> Vec<Arc<dyn nebula_resource::AnyResource>> {
        vec![]
    }

    /// Called once when the plugin is loaded into the engine.
    ///
    /// Use this hook for one-time initialization (e.g. validating config,
    /// opening connections). The engine will not call any other methods until
    /// this returns `Ok(())`.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError`] if initialization fails.
    fn on_load(&self) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called when the plugin is being unloaded from the engine.
    ///
    /// Use this hook for cleanup (e.g. flushing buffers, closing connections).
    fn on_unload(&self) -> Result<(), PluginError> {
        Ok(())
    }

    /// The normalized, unique key identifying this plugin type.
    fn key(&self) -> &PluginKey {
        self.manifest().key()
    }

    /// Bundle semver version.
    fn version(&self) -> &Version {
        self.manifest().version()
    }
}

#[cfg(test)]
mod tests {
    // Minimal coverage: a stub Plugin impl with zero components and defaults.
    // Rich tests live in tests/resolved_plugin.rs after PR 4 lands ResolvedPlugin.
    use nebula_metadata::PluginManifest;

    use super::*;

    #[derive(Debug)]
    struct StubPlugin(PluginManifest);

    impl Plugin for StubPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.0
        }
    }

    #[test]
    fn defaults_return_empty() {
        let manifest = PluginManifest::builder("stub", "Stub").build().unwrap();
        let plugin = StubPlugin(manifest);
        assert!(plugin.actions().is_empty());
        assert!(plugin.credentials().is_empty());
        assert!(plugin.resources().is_empty());
        assert!(plugin.on_load().is_ok());
        assert!(plugin.on_unload().is_ok());
    }

    #[test]
    fn key_and_version_forward_to_manifest() {
        let manifest = PluginManifest::builder("x", "X")
            .version(Version::new(2, 0, 0))
            .build()
            .unwrap();
        let plugin = StubPlugin(manifest);
        assert_eq!(plugin.key().as_str(), "x");
        assert_eq!(plugin.version(), &Version::new(2, 0, 0));
    }
}
