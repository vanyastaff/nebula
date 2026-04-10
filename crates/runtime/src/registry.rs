//! Registry of available actions, keyed by `ActionKey`.
//!
//! Single source of truth for action registration in nebula. The runtime
//! owns this type because registration is fundamentally an execution concern —
//! the registry holds `Arc`-wrapped handlers for dispatch.
//!
//! # Version-aware lookup
//!
//! Multiple versions of the same action can be registered simultaneously.
//! [`ActionRegistry::get`] returns the **latest** version (highest major,
//! then minor), while [`ActionRegistry::get_versioned`] retrieves a specific
//! `"major.minor"` version.
//!
//! # Thread safety
//!
//! Uses `DashMap` for lock-free concurrent access. Both registration and
//! lookup use `&self` — share via `Arc<ActionRegistry>` without external
//! synchronization.

use std::sync::Arc;

use dashmap::DashMap;

use nebula_action::{
    Action, ActionHandler, ActionMetadata, PollAction, PollTriggerAdapter, ResourceAction,
    ResourceActionAdapter, StatefulAction, StatefulActionAdapter, StatelessAction,
    StatelessActionAdapter, TriggerAction, TriggerActionAdapter, WebhookAction,
    WebhookTriggerAdapter,
};
use nebula_core::{ActionKey, InterfaceVersion};

/// A single entry in the registry: metadata paired with its handler.
#[derive(Clone)]
struct ActionEntry {
    metadata: ActionMetadata,
    handler: ActionHandler,
}

/// Type-safe registry for action handlers, keyed by `ActionKey`.
#[derive(Default)]
pub struct ActionRegistry {
    /// Map from action key to list of entries, each at a distinct version.
    actions: DashMap<ActionKey, Vec<ActionEntry>>,
}

impl ActionRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an action handler.
    ///
    /// If an entry with the same key **and** the same `"major.minor"` version
    /// string already exists it is replaced in-place. Otherwise the new entry
    /// is appended. Entries are kept sorted from lowest to highest version so
    /// that [`get`](Self::get) can return the latest in O(1).
    pub fn register(&self, metadata: ActionMetadata, handler: ActionHandler) {
        let version = metadata.version;
        let mut entries = self.actions.entry(metadata.key.clone()).or_default();

        if let Some(pos) = entries.iter().position(|e| e.metadata.version == version) {
            entries[pos] = ActionEntry { metadata, handler };
        } else {
            entries.push(ActionEntry { metadata, handler });
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
    /// Returns owned `(metadata, handler)` — `ActionHandler` is `Arc` inside,
    /// so cloning is a cheap pointer copy. Owned values avoid borrowing
    /// `DashMap` guards across `.await` boundaries.
    pub fn get(&self, key: &ActionKey) -> Option<(ActionMetadata, ActionHandler)> {
        let entries = self.actions.get(key)?;
        let last = entries.last()?;
        Some((last.metadata.clone(), last.handler.clone()))
    }

    /// Look up an action by string key (parses into `ActionKey` first).
    ///
    /// Returns `None` for both unregistered actions AND invalid key strings.
    /// Callers that need to distinguish should use [`ActionKey::new`] explicitly
    /// before calling [`get`](Self::get), or use a higher-level wrapper like
    /// `ActionRuntime::execute_action` which surfaces parse errors as
    /// `RuntimeError::InvalidActionKey`.
    pub fn get_by_str(&self, key: &str) -> Option<(ActionMetadata, ActionHandler)> {
        ActionKey::new(key).ok().and_then(|k| self.get(&k))
    }

    /// Look up an action by key and exact version.
    pub fn get_versioned(
        &self,
        key: &ActionKey,
        version: &InterfaceVersion,
    ) -> Option<(ActionMetadata, ActionHandler)> {
        let entries = self.actions.get(key)?;
        let entry = entries.iter().find(|e| e.metadata.version == *version)?;
        Some((entry.metadata.clone(), entry.handler.clone()))
    }

    /// Register a stateless action — wraps in `StatelessActionAdapter` automatically.
    pub fn register_stateless<A>(&self, action: A)
    where
        A: Action + StatelessAction + Send + Sync + 'static,
        A::Input: serde::de::DeserializeOwned + Send + Sync,
        A::Output: serde::Serialize + Send + Sync,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Stateless(Arc::new(StatelessActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a stateful action — wraps in `StatefulActionAdapter` automatically.
    pub fn register_stateful<A>(&self, action: A)
    where
        A: Action + StatefulAction + Send + Sync + 'static,
        A::Input: serde::de::DeserializeOwned + Send + Sync,
        A::Output: serde::Serialize + Send + Sync,
        A::State: serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Stateful(Arc::new(StatefulActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a trigger action — wraps in `TriggerActionAdapter` automatically.
    pub fn register_trigger<A>(&self, action: A)
    where
        A: Action + TriggerAction + Send + Sync + 'static,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Trigger(Arc::new(TriggerActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a webhook action — wraps in `WebhookTriggerAdapter` automatically.
    pub fn register_webhook<A>(&self, action: A)
    where
        A: WebhookAction + Send + Sync + 'static,
        <A as WebhookAction>::State: Send + Sync,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Trigger(Arc::new(WebhookTriggerAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a poll action — wraps in `PollTriggerAdapter` automatically.
    pub fn register_poll<A>(&self, action: A)
    where
        A: PollAction + Send + Sync + 'static,
        <A as PollAction>::Cursor: Send + Sync,
        <A as PollAction>::Event: Send + Sync,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Trigger(Arc::new(PollTriggerAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a resource action — wraps in `ResourceActionAdapter` automatically.
    pub fn register_resource<A>(&self, action: A)
    where
        A: Action + ResourceAction + Send + Sync + 'static,
        A::Config: Send + Sync + 'static,
        A::Instance: Send + Sync + 'static,
    {
        let metadata = action.metadata().clone();
        let handler = ActionHandler::Resource(Arc::new(ResourceActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// All registered action keys.
    #[must_use]
    pub fn keys(&self) -> Vec<ActionKey> {
        self.actions
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
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
        let keys: Vec<ActionKey> = self.keys();
        f.debug_struct("ActionRegistry")
            .field("action_count", &self.actions.len())
            .field("keys", &keys)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_action::action::Action;
    use nebula_action::context::Context;
    use nebula_action::dependency::ActionDependencies;
    use nebula_action::error::ActionError;
    use nebula_action::execution::StatelessAction;
    use nebula_action::metadata::ActionMetadata;
    use nebula_action::result::ActionResult;

    struct NoopAction {
        meta: ActionMetadata,
    }

    impl NoopAction {
        fn new(key: &'static str, major: u32, minor: u32) -> Self {
            Self {
                meta: ActionMetadata::new(ActionKey::new(key).unwrap(), "Noop", "Does nothing")
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

    #[test]
    fn register_and_get_action() {
        let registry = ActionRegistry::new();
        registry.register_stateless(NoopAction::new("test.noop", 1, 0));
        assert_eq!(registry.len(), 1);
        let key = ActionKey::new("test.noop").unwrap();
        let result = registry.get(&key);
        assert!(result.is_some());
    }

    #[test]
    fn register_replaces_same_version() {
        let registry = ActionRegistry::new();
        registry.register_stateless(NoopAction::new("test.noop", 1, 0));
        registry.register_stateless(NoopAction::new("test.noop", 1, 0));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn versioned_lookup() {
        let registry = ActionRegistry::new();
        registry.register_stateless(NoopAction::new("test.noop", 1, 0));
        registry.register_stateless(NoopAction::new("test.noop", 2, 0));

        let key = ActionKey::new("test.noop").unwrap();
        let v1 = InterfaceVersion::new(1, 0);
        let v2 = InterfaceVersion::new(2, 0);

        assert!(registry.get_versioned(&key, &v1).is_some());
        assert!(registry.get_versioned(&key, &v2).is_some());

        let (meta, _) = registry.get(&key).unwrap();
        assert_eq!(meta.version, v2);
    }
}
