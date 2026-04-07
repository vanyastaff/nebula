//! Action registry for looking up handlers by key.

use std::sync::Arc;

use dashmap::DashMap;

use nebula_action::{InterfaceVersion, InternalHandler};

use crate::error::RuntimeError;

/// A versioned handler entry: (version, handler).
type VersionedHandler = (InterfaceVersion, Arc<dyn InternalHandler>);

/// Thread-safe registry of action handlers.
///
/// Actions are registered by key (e.g. `"http.request"`) and looked up
/// at execution time. Uses `DashMap` for lock-free concurrent access.
///
/// Prefer [`ActionRegistry::register_stateless`] over the low-level
/// [`ActionRegistry::register`] — the typed helper wraps the action in a
/// [`StatelessActionAdapter`](nebula_action::StatelessActionAdapter) automatically.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_runtime::registry::ActionRegistry;
///
/// let registry = ActionRegistry::new();
/// registry.register_stateless(my_action);     // typed — preferred
/// registry.register(Arc::new(my_handler));    // raw handler — low-level
/// let handler = registry.get("http.request").unwrap();
/// ```
pub struct ActionRegistry {
    /// Primary storage: action_key → latest handler.
    handlers: DashMap<String, Arc<dyn InternalHandler>>,
    /// Version index: action_key → list of (version, handler).
    versions: DashMap<String, Vec<VersionedHandler>>,
}

impl ActionRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: DashMap::new(),
            versions: DashMap::new(),
        }
    }

    /// Register an action handler.
    ///
    /// If a handler with the same key already exists, it is replaced.
    pub fn register(&self, handler: Arc<dyn InternalHandler>) {
        let meta = handler.metadata();
        let key = meta.key.as_str().to_owned();
        let version = meta.version;

        tracing::info!(
            action_key = %key,
            version = %version,
            "registered action handler",
        );

        // Latest-wins for primary lookup.
        self.handlers.insert(key.clone(), handler.clone());

        // Add to version index (dedup: replace existing entry for same version).
        let mut entries = self.versions.entry(key).or_default();
        entries.retain(|(v, _)| v != &version);
        entries.push((version, handler));
    }

    /// Register a typed [`StatelessAction`](nebula_action::StatelessAction) directly.
    ///
    /// Wraps the action in a [`StatelessActionAdapter`](nebula_action::StatelessActionAdapter)
    /// automatically — no manual adapter construction needed.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// registry.register_stateless(MyAction::new());
    /// ```
    pub fn register_stateless<A>(&self, action: A)
    where
        A: nebula_action::StatelessAction + Send + Sync + 'static,
        A::Input: serde::de::DeserializeOwned + Send + Sync,
        A::Output: serde::Serialize + Send + Sync,
    {
        let adapter = nebula_action::StatelessActionAdapter::new(action);
        self.register(Arc::new(adapter));
    }

    /// Look up an action handler by key.
    pub fn get(&self, key: &str) -> Result<Arc<dyn InternalHandler>, RuntimeError> {
        self.handlers
            .get(key)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| RuntimeError::ActionNotFound {
                key: key.to_owned(),
            })
    }

    /// Look up an action handler by key and specific version.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::ActionNotFound`] if no handler is registered
    /// for the given key and version combination.
    pub fn get_versioned(
        &self,
        key: &str,
        version: &InterfaceVersion,
    ) -> Result<Arc<dyn InternalHandler>, RuntimeError> {
        self.versions
            .get(key)
            .and_then(|versions| {
                versions
                    .iter()
                    .find(|(v, _)| v == version)
                    .map(|(_, h)| h.clone())
            })
            .ok_or_else(|| RuntimeError::ActionNotFound {
                key: format!("{key}@{}.{}", version.major, version.minor),
            })
    }

    /// Get the latest version handler for an action key.
    ///
    /// This is what [`get()`](Self::get) already does — this is an
    /// explicit alias.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::ActionNotFound`] if no handler is registered
    /// for the given key.
    pub fn get_latest(&self, key: &str) -> Result<Arc<dyn InternalHandler>, RuntimeError> {
        self.get(key)
    }

    /// List all registered versions for an action key.
    #[must_use]
    pub fn versions(&self, key: &str) -> Vec<InterfaceVersion> {
        self.versions
            .get(key)
            .map(|v| v.iter().map(|(ver, _)| *ver).collect())
            .unwrap_or_default()
    }

    /// Check if a handler is registered for the given key.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.handlers.contains_key(key)
    }

    /// Remove a handler by key. Returns the removed handler, if any.
    pub fn remove(&self, key: &str) -> Option<Arc<dyn InternalHandler>> {
        self.versions.remove(key);
        self.handlers.remove(key).map(|(_, v)| v)
    }

    /// Number of registered handlers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// List all registered action keys.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        self.handlers.iter().map(|e| e.key().clone()).collect()
    }
}

impl std::fmt::Debug for ActionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionRegistry")
            .field("handler_count", &self.handlers.len())
            .finish_non_exhaustive()
    }
}

impl Default for ActionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_action::ActionContext;
    use nebula_action::error::ActionError;
    use nebula_action::metadata::ActionMetadata;
    use nebula_action::result::ActionResult;
    use nebula_core::ActionKey;
    use nebula_core::action_key;

    /// Minimal test handler that echoes input.
    struct EchoHandler {
        meta: ActionMetadata,
    }

    impl EchoHandler {
        fn new(key: ActionKey) -> Self {
            let name = key.to_string();
            Self {
                meta: ActionMetadata::new(key, name, "test handler"),
            }
        }

        fn with_version(key: ActionKey, major: u32, minor: u32) -> Self {
            let name = key.to_string();
            Self {
                meta: ActionMetadata::new(key, name, "test handler").with_version(major, minor),
            }
        }
    }

    #[async_trait::async_trait]
    impl InternalHandler for EchoHandler {
        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            Ok(ActionResult::success(input))
        }

        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    #[test]
    fn register_and_lookup() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoHandler::new(action_key!("test.echo"))));

        assert!(reg.contains("test.echo"));
        assert_eq!(reg.len(), 1);

        let handler = reg.get("test.echo").unwrap();
        assert_eq!(handler.metadata().key, action_key!("test.echo"));
    }

    #[test]
    fn lookup_missing_returns_error() {
        let reg = ActionRegistry::new();
        let result = reg.get("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn register_replaces_existing() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoHandler::new(action_key!("test.a"))));
        reg.register(Arc::new(EchoHandler::new(action_key!("test.a"))));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn remove_handler() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoHandler::new(action_key!("test.rem"))));
        assert!(reg.contains("test.rem"));

        let removed = reg.remove("test.rem");
        assert!(removed.is_some());
        assert!(!reg.contains("test.rem"));
    }

    #[test]
    fn keys_lists_all() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoHandler::new(action_key!("a"))));
        reg.register(Arc::new(EchoHandler::new(action_key!("b"))));
        reg.register(Arc::new(EchoHandler::new(action_key!("c"))));

        let mut keys = reg.keys();
        keys.sort();
        assert_eq!(keys, vec!["a", "b", "c"]);
    }

    #[test]
    fn empty_registry() {
        let reg = ActionRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn get_versioned_returns_specific_version() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoHandler::with_version(
            action_key!("test.echo"),
            1,
            0,
        )));
        reg.register(Arc::new(EchoHandler::with_version(
            action_key!("test.echo"),
            2,
            0,
        )));

        let v1 = nebula_action::InterfaceVersion::new(1, 0);
        let handler = reg.get_versioned("test.echo", &v1).unwrap();
        assert_eq!(handler.metadata().version, v1);

        let v2 = nebula_action::InterfaceVersion::new(2, 0);
        let handler = reg.get_versioned("test.echo", &v2).unwrap();
        assert_eq!(handler.metadata().version, v2);
    }

    #[test]
    fn get_versioned_missing_version_returns_error() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoHandler::with_version(
            action_key!("test.echo"),
            1,
            0,
        )));

        let v3 = nebula_action::InterfaceVersion::new(3, 0);
        let result = reg.get_versioned("test.echo", &v3);
        assert!(result.is_err());
    }

    #[test]
    fn get_latest_returns_last_registered() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoHandler::with_version(
            action_key!("test.echo"),
            1,
            0,
        )));
        reg.register(Arc::new(EchoHandler::with_version(
            action_key!("test.echo"),
            2,
            0,
        )));

        let handler = reg.get_latest("test.echo").unwrap();
        let v2 = nebula_action::InterfaceVersion::new(2, 0);
        assert_eq!(handler.metadata().version, v2);
    }

    #[test]
    fn versions_lists_all_registered() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoHandler::with_version(
            action_key!("test.echo"),
            1,
            0,
        )));
        reg.register(Arc::new(EchoHandler::with_version(
            action_key!("test.echo"),
            1,
            1,
        )));
        reg.register(Arc::new(EchoHandler::with_version(
            action_key!("test.echo"),
            2,
            0,
        )));

        let versions = reg.versions("test.echo");
        assert_eq!(versions.len(), 3);
        assert!(versions.contains(&nebula_action::InterfaceVersion::new(1, 0)));
        assert!(versions.contains(&nebula_action::InterfaceVersion::new(1, 1)));
        assert!(versions.contains(&nebula_action::InterfaceVersion::new(2, 0)));
    }

    #[test]
    fn versions_returns_empty_for_unknown_key() {
        let reg = ActionRegistry::new();
        assert!(reg.versions("nonexistent").is_empty());
    }

    #[test]
    fn register_same_version_replaces_handler() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoHandler::with_version(
            action_key!("test.echo"),
            1,
            0,
        )));
        // Re-register same key+version — should replace, not duplicate.
        reg.register(Arc::new(EchoHandler::with_version(
            action_key!("test.echo"),
            1,
            0,
        )));

        let versions = reg.versions("test.echo");
        assert_eq!(
            versions.len(),
            1,
            "duplicate version entry should not exist"
        );
        assert_eq!(versions[0], nebula_action::InterfaceVersion::new(1, 0));
    }

    #[test]
    fn remove_clears_versions() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoHandler::with_version(
            action_key!("test.echo"),
            1,
            0,
        )));
        reg.register(Arc::new(EchoHandler::with_version(
            action_key!("test.echo"),
            2,
            0,
        )));

        reg.remove("test.echo");

        assert!(
            reg.versions("test.echo").is_empty(),
            "versions should be cleared after remove",
        );
        let v1 = nebula_action::InterfaceVersion::new(1, 0);
        assert!(
            reg.get_versioned("test.echo", &v1).is_err(),
            "get_versioned should return NotFound after remove",
        );
    }
}
