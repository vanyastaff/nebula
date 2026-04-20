//! The base Plugin trait.

use std::fmt::Debug;

use nebula_core::PluginKey;
use semver::Version;

use crate::{
    PluginError, PluginManifest,
    descriptor::{ActionDescriptor, CredentialDescriptor, ResourceDescriptor},
};

/// Base trait for all plugin types in Nebula.
///
/// A plugin is a user-visible, versionable packaging unit (e.g. "Slack",
/// "HTTP Request"). It provides a manifest describing the plugin's identity
/// and version, and optionally declares the actions, credentials, and resources
/// it contributes to the engine.
///
/// All methods except [`Plugin::manifest`] have default implementations so that
/// existing plugin implementations continue to compile without changes.
///
/// This trait is **object-safe** so plugins can be stored as `Arc<dyn Plugin>`.
pub trait Plugin: Send + Sync + Debug + 'static {
    /// Returns the static manifest for this plugin.
    fn manifest(&self) -> &PluginManifest;

    /// The normalized, unique key identifying this plugin type.
    fn key(&self) -> &PluginKey {
        self.manifest().key()
    }

    /// Human-readable display name.
    fn name(&self) -> &str {
        self.manifest().name()
    }

    /// Bundle semver version.
    fn version(&self) -> &Version {
        self.manifest().version()
    }

    /// Actions this plugin provides.
    ///
    /// The engine calls this once at plugin-load time to register available
    /// actions. Returns an empty list by default.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::ActionKey;
    /// use nebula_plugin::{Plugin, PluginManifest, descriptor::ActionDescriptor};
    /// use semver::Version;
    ///
    /// #[derive(Debug)]
    /// struct MyPlugin {
    ///     manifest: PluginManifest,
    /// }
    ///
    /// impl Plugin for MyPlugin {
    ///     fn manifest(&self) -> &PluginManifest {
    ///         &self.manifest
    ///     }
    ///
    ///     fn actions(&self) -> Vec<ActionDescriptor> {
    ///         vec![ActionDescriptor {
    ///             key: ActionKey::new("send_message").unwrap(),
    ///             name: "Send Message".into(),
    ///             description: "Sends a message.".into(),
    ///             version: Version::new(1, 0, 0),
    ///         }]
    ///     }
    /// }
    ///
    /// let plugin = MyPlugin {
    ///     manifest: PluginManifest::builder("my", "My").build().unwrap(),
    /// };
    /// assert_eq!(plugin.actions().len(), 1);
    /// ```
    fn actions(&self) -> Vec<ActionDescriptor> {
        vec![]
    }

    /// Credential types this plugin provides.
    ///
    /// The engine calls this once at plugin-load time to register available
    /// credential schemas. Returns an empty list by default.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::CredentialKey;
    /// use nebula_plugin::{Plugin, PluginManifest, descriptor::CredentialDescriptor};
    ///
    /// #[derive(Debug)]
    /// struct MyPlugin {
    ///     manifest: PluginManifest,
    /// }
    ///
    /// impl Plugin for MyPlugin {
    ///     fn manifest(&self) -> &PluginManifest {
    ///         &self.manifest
    ///     }
    ///
    ///     fn credentials(&self) -> Vec<CredentialDescriptor> {
    ///         vec![CredentialDescriptor {
    ///             key: CredentialKey::new("my_oauth2").unwrap(),
    ///             name: "My OAuth2".into(),
    ///             description: "OAuth2 credentials.".into(),
    ///         }]
    ///     }
    /// }
    ///
    /// let plugin = MyPlugin {
    ///     manifest: PluginManifest::builder("my", "My").build().unwrap(),
    /// };
    /// assert_eq!(plugin.credentials().len(), 1);
    /// ```
    fn credentials(&self) -> Vec<CredentialDescriptor> {
        vec![]
    }

    /// Resource types this plugin provides.
    ///
    /// The engine calls this once at plugin-load time to register available
    /// resource types. Returns an empty list by default.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::ResourceKey;
    /// use nebula_plugin::{Plugin, PluginManifest, descriptor::ResourceDescriptor};
    ///
    /// #[derive(Debug)]
    /// struct MyPlugin {
    ///     manifest: PluginManifest,
    /// }
    ///
    /// impl Plugin for MyPlugin {
    ///     fn manifest(&self) -> &PluginManifest {
    ///         &self.manifest
    ///     }
    ///
    ///     fn resources(&self) -> Vec<ResourceDescriptor> {
    ///         vec![ResourceDescriptor {
    ///             key: ResourceKey::new("my_client").unwrap(),
    ///             name: "My Client".into(),
    ///             description: "HTTP client.".into(),
    ///         }]
    ///     }
    /// }
    ///
    /// let plugin = MyPlugin {
    ///     manifest: PluginManifest::builder("my", "My").build().unwrap(),
    /// };
    /// assert_eq!(plugin.resources().len(), 1);
    /// ```
    fn resources(&self) -> Vec<ResourceDescriptor> {
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
    /// Returns [`PluginError`] if initialization fails. The engine will refuse
    /// to register the plugin and surface the error to the caller.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_plugin::{Plugin, PluginError, PluginManifest};
    ///
    /// #[derive(Debug)]
    /// struct MyPlugin {
    ///     manifest: PluginManifest,
    /// }
    ///
    /// impl Plugin for MyPlugin {
    ///     fn manifest(&self) -> &PluginManifest {
    ///         &self.manifest
    ///     }
    ///
    ///     fn on_load(&self) -> Result<(), PluginError> {
    ///         // Perform initialization here.
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let plugin = MyPlugin {
    ///     manifest: PluginManifest::builder("my", "My").build().unwrap(),
    /// };
    /// assert!(plugin.on_load().is_ok());
    /// ```
    fn on_load(&self) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called when the plugin is being unloaded from the engine.
    ///
    /// Use this hook for cleanup (e.g. flushing buffers, closing connections).
    /// The engine will call this before dropping the plugin.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError`] if cleanup fails. The engine logs the error but
    /// continues unloading regardless.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_plugin::{Plugin, PluginError, PluginManifest};
    ///
    /// #[derive(Debug)]
    /// struct MyPlugin {
    ///     manifest: PluginManifest,
    /// }
    ///
    /// impl Plugin for MyPlugin {
    ///     fn manifest(&self) -> &PluginManifest {
    ///         &self.manifest
    ///     }
    ///
    ///     fn on_unload(&self) -> Result<(), PluginError> {
    ///         // Flush buffers, close connections, etc.
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let plugin = MyPlugin {
    ///     manifest: PluginManifest::builder("my", "My").build().unwrap(),
    /// };
    /// assert!(plugin.on_unload().is_ok());
    /// ```
    fn on_unload(&self) -> Result<(), PluginError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_core::{ActionKey, CredentialKey, ResourceKey};
    use semver::Version;

    use super::*;
    use crate::descriptor::{ActionDescriptor, CredentialDescriptor, ResourceDescriptor};

    /// A minimal plugin implementation for testing (only implements `manifest`).
    #[derive(Debug)]
    struct MinimalPlugin {
        manifest: PluginManifest,
    }

    impl Plugin for MinimalPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.manifest
        }
    }

    /// A plugin that overrides all optional methods.
    #[derive(Debug)]
    struct FullPlugin {
        manifest: PluginManifest,
    }

    impl Plugin for FullPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.manifest
        }

        fn actions(&self) -> Vec<ActionDescriptor> {
            vec![ActionDescriptor {
                key: ActionKey::new("send_message").unwrap(),
                name: "Send Message".into(),
                description: "Sends a message.".into(),
                version: Version::new(1, 0, 0),
            }]
        }

        fn credentials(&self) -> Vec<CredentialDescriptor> {
            vec![CredentialDescriptor {
                key: CredentialKey::new("slack_oauth2").unwrap(),
                name: "Slack OAuth2".into(),
                description: "OAuth2 for Slack.".into(),
            }]
        }

        fn resources(&self) -> Vec<ResourceDescriptor> {
            vec![ResourceDescriptor {
                key: ResourceKey::new("slack_client").unwrap(),
                name: "Slack Client".into(),
                description: "HTTP client.".into(),
            }]
        }

        fn on_load(&self) -> Result<(), PluginError> {
            Ok(())
        }

        fn on_unload(&self) -> Result<(), PluginError> {
            Ok(())
        }
    }

    #[test]
    fn trait_default_methods() {
        let manifest = PluginManifest::builder("slack", "Slack")
            .version(Version::new(2, 0, 0))
            .description("Send messages")
            .build()
            .unwrap();

        let plugin = MinimalPlugin { manifest };

        assert_eq!(plugin.key().as_str(), "slack");
        assert_eq!(plugin.name(), "Slack");
        assert_eq!(plugin.version(), &Version::new(2, 0, 0));
    }

    #[test]
    fn existing_impl_defaults_return_empty() {
        let manifest = PluginManifest::builder("slack", "Slack").build().unwrap();
        let plugin = MinimalPlugin { manifest };

        assert!(plugin.actions().is_empty());
        assert!(plugin.credentials().is_empty());
        assert!(plugin.resources().is_empty());
        assert!(plugin.on_load().is_ok());
        assert!(plugin.on_unload().is_ok());
    }

    #[test]
    fn full_plugin_overrides_work() {
        let manifest = PluginManifest::builder("slack", "Slack").build().unwrap();
        let plugin = FullPlugin { manifest };

        let actions = plugin.actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].key.as_str(), "send_message");

        let creds = plugin.credentials();
        assert_eq!(creds.len(), 1);
        assert_eq!(creds[0].key.as_str(), "slack_oauth2");

        let resources = plugin.resources();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].key.as_str(), "slack_client");

        assert!(plugin.on_load().is_ok());
        assert!(plugin.on_unload().is_ok());
    }

    #[test]
    fn object_safety() {
        let manifest = PluginManifest::builder("test", "Test").build().unwrap();
        let plugin: Arc<dyn Plugin> = Arc::new(MinimalPlugin { manifest });

        assert_eq!(plugin.key().as_str(), "test");
        assert_eq!(plugin.version(), &Version::new(1, 0, 0));
        assert!(plugin.actions().is_empty());
    }
}
