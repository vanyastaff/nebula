//! Registry of available actions, keyed by `ActionKey`.
//!
//! The `ActionRegistry` is the authoritative source for which action types are
//! available in a running Nebula instance. The engine consults it during workflow
//! compilation to resolve `ActionKey` references and obtain handlers for execution.
//!
//! # Version-aware lookup
//!
//! Multiple versions of the same action can be registered simultaneously. [`ActionRegistry::get`]
//! returns the **latest** version (highest major, then minor), while
//! [`ActionRegistry::get_versioned`] retrieves a specific `"major.minor"` string.
//!
//! # Examples
//!
//! ```rust
//! use nebula_action::registry::ActionRegistry;
//! use nebula_core::action_key;
//!
//! // ActionRegistry::new() creates an empty registry.
//! let registry = ActionRegistry::new();
//! assert!(registry.is_empty());
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use nebula_core::ActionKey;

use nebula_core::InterfaceVersion;

use crate::handler::ActionHandler;
use crate::metadata::ActionMetadata;

/// A single entry in the registry: metadata paired with its handler.
struct ActionEntry {
    metadata: ActionMetadata,
    handler: ActionHandler,
}

/// Type-safe registry for action handlers, keyed by `ActionKey`.
///
/// Supports version-aware lookup: when multiple versions of the same action are
/// registered, [`get`](ActionRegistry::get) returns the latest and
/// [`get_versioned`](ActionRegistry::get_versioned) returns a specific version.
///
/// # Thread safety
///
/// `ActionRegistry` is `Send + Sync` by auto-trait, so it can be shared across
/// threads for read-only access, for example via `Arc<ActionRegistry>`.
/// Mutating the registry after sharing it across threads requires
/// external synchronization, such as `Arc<RwLock<ActionRegistry>>`. Typically
/// registries are built at startup and then treated as immutable.
#[derive(Default)]
pub struct ActionRegistry {
    /// Map from action key to list of entries, each at a distinct version.
    actions: HashMap<ActionKey, Vec<ActionEntry>>,
}

impl ActionRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an action handler.
    ///
    /// If an entry with the same key **and** the same `"major.minor"` version string
    /// already exists it is replaced in-place. Otherwise the new entry is appended.
    /// Entries are kept sorted from lowest to highest version so that [`get`](Self::get)
    /// can return the latest in O(1).
    pub fn register(&mut self, metadata: ActionMetadata, handler: ActionHandler) {
        let version = metadata.version;
        let entries = self.actions.entry(metadata.key.clone()).or_default();

        // Replace existing entry with the same version, or append.
        if let Some(pos) = entries.iter().position(|e| e.metadata.version == version) {
            entries[pos] = ActionEntry { metadata, handler };
        } else {
            entries.push(ActionEntry { metadata, handler });
            // Keep sorted by (major, minor) ascending so the last element is latest.
            entries.sort_by(|a, b| {
                a.metadata
                    .version
                    .major
                    .cmp(&b.metadata.version.major)
                    .then(a.metadata.version.minor.cmp(&b.metadata.version.minor))
            });
        }
    }

    /// Look up an action by key, returning the **latest** registered version.
    ///
    /// Returns `None` if no action with this key has been registered.
    pub fn get(&self, key: &ActionKey) -> Option<(&ActionMetadata, &ActionHandler)> {
        self.actions
            .get(key)
            .and_then(|entries| entries.last())
            .map(|e| (&e.metadata, &e.handler))
    }

    /// Look up an action by key and exact version.
    ///
    /// Returns `None` if no entry matching both key and version was found.
    pub fn get_versioned(
        &self,
        key: &ActionKey,
        version: &InterfaceVersion,
    ) -> Option<(&ActionMetadata, &ActionHandler)> {
        self.actions
            .get(key)?
            .iter()
            .find(|e| e.metadata.version == *version)
            .map(|e| (&e.metadata, &e.handler))
    }

    /// Register a stateless action — wraps in [`StatelessActionAdapter`] automatically.
    ///
    /// [`StatelessActionAdapter`]: crate::handler::StatelessActionAdapter
    pub fn register_stateless<A>(&mut self, action: A)
    where
        A: crate::Action + crate::StatelessAction + Send + Sync + 'static,
        A::Input: serde::de::DeserializeOwned + Send + Sync,
        A::Output: serde::Serialize + Send + Sync,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Stateless(Arc::new(
            crate::handler::StatelessActionAdapter::new(action),
        ));
        self.register(metadata, handler);
    }

    /// Register a stateful action — wraps in [`StatefulActionAdapter`] automatically.
    ///
    /// [`StatefulActionAdapter`]: crate::handler::StatefulActionAdapter
    pub fn register_stateful<A>(&mut self, action: A)
    where
        A: crate::Action + crate::StatefulAction + Send + Sync + 'static,
        A::Input: serde::de::DeserializeOwned + Send + Sync,
        A::Output: serde::Serialize + Send + Sync,
        A::State: serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync,
    {
        let metadata = action.metadata().clone();
        let handler =
            ActionHandler::Stateful(Arc::new(crate::handler::StatefulActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a trigger action — wraps in [`TriggerActionAdapter`] automatically.
    ///
    /// [`TriggerActionAdapter`]: crate::handler::TriggerActionAdapter
    pub fn register_trigger<A>(&mut self, action: A)
    where
        A: crate::Action + crate::TriggerAction + Send + Sync + 'static,
    {
        let metadata = action.metadata().clone();
        let handler =
            ActionHandler::Trigger(Arc::new(crate::handler::TriggerActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a resource action — wraps in [`ResourceActionAdapter`] automatically.
    ///
    /// [`ResourceActionAdapter`]: crate::handler::ResourceActionAdapter
    pub fn register_resource<A>(&mut self, action: A)
    where
        A: crate::Action + crate::ResourceAction + Send + Sync + 'static,
        A::Config: Send + Sync + 'static,
        A::Instance: Send + Sync + 'static,
    {
        let metadata = action.metadata().clone();
        let handler =
            ActionHandler::Resource(Arc::new(crate::handler::ResourceActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Iterate over all registered action keys.
    pub fn keys(&self) -> impl Iterator<Item = &ActionKey> {
        self.actions.keys()
    }

    /// Total number of registered action keys (not counting multiple versions of the same key).
    #[must_use]
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Returns `true` if no actions have been registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

impl std::fmt::Debug for ActionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys: Vec<&ActionKey> = self.actions.keys().collect();
        f.debug_struct("ActionRegistry")
            .field("action_count", &self.actions.len())
            .field("keys", &keys)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use crate::action::Action;
    use crate::context::Context;
    use crate::dependency::ActionDependencies;
    use crate::error::ActionError;
    use crate::execution::StatelessAction;
    use crate::handler::ActionHandler;
    use crate::metadata::ActionMetadata;
    use crate::result::ActionResult;

    use super::*;

    // ── Minimal test action ────────────────────────────────────────────────────

    struct NoopAction {
        meta: ActionMetadata,
    }

    impl NoopAction {
        fn new(key: &'static str, major: u32, minor: u32) -> Self {
            Self {
                meta: ActionMetadata::new(
                    nebula_core::ActionKey::new(key).unwrap(),
                    "Noop",
                    "Does nothing",
                )
                .with_version(major, minor),
            }
        }
    }

    impl ActionDependencies for NoopAction {}

    impl Action for NoopAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl StatelessAction for NoopAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        async fn execute(
            &self,
            input: Self::Input,
            _ctx: &impl Context,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    fn make_entry(key: &'static str, major: u32, minor: u32) -> (ActionMetadata, ActionHandler) {
        let action = NoopAction::new(key, major, minor);
        let meta = action.metadata().clone();
        let handler = ActionHandler::Stateless(Arc::new(
            crate::handler::StatelessActionAdapter::new(action),
        ));
        (meta, handler)
    }

    // ── Tests ──────────────────────────────────────────────────────────────────

    #[test]
    fn register_and_get_action() {
        let mut registry = ActionRegistry::new();
        let (meta, handler) = make_entry("math.add", 1, 0);

        registry.register(meta, handler);

        let (found_meta, _handler) = registry
            .get(&nebula_core::ActionKey::new("math.add").unwrap())
            .expect("action should be registered");

        assert_eq!(
            found_meta.key,
            nebula_core::ActionKey::new("math.add").unwrap()
        );
        assert_eq!(found_meta.version.major, 1);
        assert_eq!(found_meta.version.minor, 0);
    }

    #[test]
    fn get_returns_none_for_unknown_key() {
        let registry = ActionRegistry::new();
        let result = registry.get(&nebula_core::ActionKey::new("unknown.action").unwrap());
        assert!(result.is_none());
    }

    #[test]
    fn keys_lists_all_registered() {
        let mut registry = ActionRegistry::new();
        let (m1, h1) = make_entry("http.get", 1, 0);
        let (m2, h2) = make_entry("http.post", 1, 0);
        let (m3, h3) = make_entry("math.add", 1, 0);
        registry.register(m1, h1);
        registry.register(m2, h2);
        registry.register(m3, h3);

        let mut keys: Vec<String> = registry.keys().map(|k| k.as_str().to_owned()).collect();
        keys.sort();

        assert_eq!(keys, vec!["http.get", "http.post", "math.add"]);
    }

    #[test]
    fn len_and_is_empty() {
        let mut registry = ActionRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);

        let (m, h) = make_entry("a.b", 1, 0);
        registry.register(m, h);
        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn get_returns_latest_version() {
        let mut registry = ActionRegistry::new();
        let (m1, h1) = make_entry("http.request", 1, 0);
        let (m2, h2) = make_entry("http.request", 2, 0);
        let (m3, h3) = make_entry("http.request", 1, 5);
        registry.register(m1, h1);
        registry.register(m2, h2);
        registry.register(m3, h3);

        let (meta, _) = registry
            .get(&nebula_core::ActionKey::new("http.request").unwrap())
            .unwrap();
        assert_eq!(meta.version.major, 2);
        assert_eq!(meta.version.minor, 0);
    }

    #[test]
    fn get_versioned_returns_specific_version() {
        let mut registry = ActionRegistry::new();
        let (m1, h1) = make_entry("http.request", 1, 0);
        let (m2, h2) = make_entry("http.request", 2, 0);
        registry.register(m1, h1);
        registry.register(m2, h2);

        let key = nebula_core::ActionKey::new("http.request").unwrap();
        let v1 = nebula_core::InterfaceVersion::new(1, 0);
        let v2 = nebula_core::InterfaceVersion::new(2, 0);
        let v3 = nebula_core::InterfaceVersion::new(3, 0);

        let (meta, _) = registry.get_versioned(&key, &v1).unwrap();
        assert_eq!(meta.version.major, 1);

        let (meta2, _) = registry.get_versioned(&key, &v2).unwrap();
        assert_eq!(meta2.version.major, 2);

        assert!(registry.get_versioned(&key, &v3).is_none());
    }

    #[test]
    fn register_replaces_same_version() {
        let mut registry = ActionRegistry::new();
        let (m1, h1) = make_entry("a.b", 1, 0);
        let (m2, h2) = make_entry("a.b", 1, 0);
        registry.register(m1, h1);
        registry.register(m2, h2);

        // Still only one version entry
        let key = nebula_core::ActionKey::new("a.b").unwrap();
        assert_eq!(registry.len(), 1);
        assert!(registry.get(&key).is_some());
    }

    #[test]
    fn register_stateless_convenience() {
        let mut registry = ActionRegistry::new();
        registry.register_stateless(NoopAction::new("test.noop", 1, 0));

        let key = nebula_core::ActionKey::new("test.noop").unwrap();
        let (meta, handler) = registry.get(&key).expect("should be registered");
        assert_eq!(meta.version.major, 1);
        assert!(handler.is_stateless());
    }
}
