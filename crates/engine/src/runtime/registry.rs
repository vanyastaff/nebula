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
    Action, ActionError, ActionFactory, ActionHandler, ActionMetadata, ControlAction,
    FromWorkflowNode, GenericControlFactory, GenericResourceFactory, GenericStatefulFactory,
    GenericStatelessFactory, GenericTriggerFactory, PollAction, PollTriggerAdapter, ResourceAction,
    ResourceActionAdapter, StatefulAction, StatefulActionAdapter, StatelessAction,
    StatelessActionAdapter, TriggerAction, TriggerActionAdapter, WebhookAction,
    WebhookTriggerAdapter,
};
use nebula_core::ActionKey;
use semver::Version;

/// A single entry in the registry: metadata paired with its handler.
#[derive(Clone)]
struct ActionEntry {
    metadata: ActionMetadata,
    handler: ActionHandler,
}

/// A single factory entry in the parallel factory map (Phase 3 / Session 4).
///
/// Stored alongside the legacy `ActionEntry` so the engine can transition
/// dispatch to factory-based instantiation incrementally per ADR-0043 §6.
#[derive(Clone)]
struct FactoryEntry {
    metadata: ActionMetadata,
    factory: Arc<dyn ActionFactory>,
}

/// Type-safe registry for action handlers, keyed by `ActionKey`.
#[derive(Default)]
pub struct ActionRegistry {
    /// Map from action key to list of entries, each at a distinct version.
    actions: DashMap<ActionKey, Vec<ActionEntry>>,
    /// Parallel factory map per ADR-0043 §6 / Phase 3 Session 4. Engine
    /// dispatch consults this first and falls back to `actions` when no
    /// factory has been registered for the key.
    factories: DashMap<ActionKey, Vec<FactoryEntry>>,
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
        let version = metadata.base.version.clone();
        let mut entries = self.actions.entry(metadata.base.key.clone()).or_default();

        if let Some(pos) = entries
            .iter()
            .position(|e| e.metadata.base.version == version)
        {
            entries[pos] = ActionEntry { metadata, handler };
        } else {
            entries.push(ActionEntry { metadata, handler });
            entries.sort_by(|a, b| a.metadata.base.version.cmp(&b.metadata.base.version));
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
        version: &Version,
    ) -> Option<(ActionMetadata, ActionHandler)> {
        let entries = self.actions.get(key)?;
        let entry = entries
            .iter()
            .find(|e| e.metadata.base.version == *version)?;
        Some((entry.metadata.clone(), entry.handler.clone()))
    }

    /// Register a stateless action — wraps in `StatelessActionAdapter` automatically.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_engine::ActionRegistry;
    /// let registry = ActionRegistry::new();
    /// registry.register_stateless(my_stateless_action);
    /// ```
    ///
    /// # Errors
    ///
    /// Does not return errors. If a handler with the same `(key, version)` is
    /// already registered, it is replaced silently.
    pub fn register_stateless<A>(&self, action: A)
    where
        A: Action + StatelessAction + Send + Sync + 'static,
        <A as Action>::Input: serde::de::DeserializeOwned + Send + Sync,
        <A as Action>::Output: serde::Serialize + Send + Sync,
    {
        let metadata = <A as Action>::metadata().clone();
        let handler = ActionHandler::Stateless(Arc::new(StatelessActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a stateful action — wraps in `StatefulActionAdapter` automatically.
    ///
    /// See [`register_stateless`](Self::register_stateless) for usage and error semantics.
    ///
    /// # Errors
    ///
    /// Does not return errors. Same-version handlers are replaced silently.
    pub fn register_stateful<A>(&self, action: A)
    where
        A: Action + StatefulAction + Send + Sync + 'static,
        <A as Action>::Input: serde::de::DeserializeOwned + Send + Sync,
        <A as Action>::Output: serde::Serialize + Send + Sync,
        A::State: serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync,
    {
        let metadata = <A as Action>::metadata().clone();
        let handler = ActionHandler::Stateful(Arc::new(StatefulActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a trigger action — wraps in `TriggerActionAdapter` automatically.
    ///
    /// See [`register_stateless`](Self::register_stateless) for usage.
    ///
    /// # Errors
    ///
    /// Does not return errors. Same-version handlers are replaced silently.
    pub fn register_trigger<A>(&self, action: A)
    where
        A: TriggerAction + Send + Sync + 'static,
        A::Error: Into<ActionError>,
    {
        let metadata = <A as Action>::metadata().clone();
        let handler = ActionHandler::Trigger(Arc::new(TriggerActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a webhook action — wraps in `WebhookTriggerAdapter` automatically.
    ///
    /// See [`register_stateless`](Self::register_stateless) for usage.
    ///
    /// # Errors
    ///
    /// Does not return errors. Same-version handlers are replaced silently.
    pub fn register_webhook<A>(&self, action: A)
    where
        A: WebhookAction + Send + Sync + 'static,
        <A as WebhookAction>::State: Send + Sync,
    {
        let metadata = <A as Action>::metadata().clone();
        let handler = ActionHandler::Trigger(Arc::new(WebhookTriggerAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a poll action — wraps in `PollTriggerAdapter` automatically.
    ///
    /// See [`register_stateless`](Self::register_stateless) for usage.
    ///
    /// # Errors
    ///
    /// Does not return errors. Same-version handlers are replaced silently.
    pub fn register_poll<A>(&self, action: A)
    where
        A: PollAction + Send + Sync + 'static,
        <A as PollAction>::Cursor: Send + Sync,
        <A as PollAction>::Event: Send + Sync,
    {
        let metadata = <A as Action>::metadata().clone();
        let handler = ActionHandler::Trigger(Arc::new(PollTriggerAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register a resource action — wraps in `ResourceActionAdapter` automatically.
    ///
    /// See [`register_stateless`](Self::register_stateless) for usage.
    ///
    /// # Errors
    ///
    /// Does not return errors. Same-version handlers are replaced silently.
    pub fn register_resource<A>(&self, action: A)
    where
        A: Action + ResourceAction + Send + Sync + 'static,
    {
        let metadata = <A as Action>::metadata().clone();
        let handler = ActionHandler::Resource(Arc::new(ResourceActionAdapter::new(action)));
        self.register(metadata, handler);
    }

    /// Register an action factory (Phase 3 / Session 4 — ADR-0043 §6).
    ///
    /// The factory is consulted at dispatch time to instantiate a fresh
    /// erased action per execution. This is the new path for actions that
    /// implement [`Action`] + [`FromWorkflowNode`] (Variant A).
    ///
    /// Stored alongside any legacy `ActionHandler` registration; lookups
    /// prefer the factory entry when present.
    pub fn register_factory(&self, metadata: ActionMetadata, factory: Arc<dyn ActionFactory>) {
        let version = metadata.base.version.clone();
        let mut entries = self.factories.entry(metadata.base.key.clone()).or_default();

        if let Some(pos) = entries
            .iter()
            .position(|e| e.metadata.base.version == version)
        {
            entries[pos] = FactoryEntry { metadata, factory };
        } else {
            entries.push(FactoryEntry { metadata, factory });
            entries.sort_by(|a, b| a.metadata.base.version.cmp(&b.metadata.base.version));
        }
    }

    /// Register a stateless action via the factory pipeline (Variant A).
    ///
    /// Requires the action to implement [`FromWorkflowNode`] (auto-emitted
    /// by `#[derive(Action)]`). The factory builds a fresh `A` per
    /// dispatch via `A::from_workflow_node(node, ctx)`.
    pub fn register_stateless_factory<A>(&self)
    where
        A: StatelessAction + FromWorkflowNode<Error = ActionError>,
        <A as Action>::Input: serde::de::DeserializeOwned + Send + Sync,
        <A as Action>::Output: serde::Serialize + Send + Sync,
    {
        let metadata = <A as Action>::metadata().clone();
        let factory: Arc<dyn ActionFactory> = Arc::new(GenericStatelessFactory::<A>::new());
        self.register_factory(metadata, factory);
    }

    /// Register a stateful action via the factory pipeline (Variant A).
    pub fn register_stateful_factory<A>(&self)
    where
        A: StatefulAction + FromWorkflowNode<Error = ActionError>,
        <A as Action>::Input: serde::de::DeserializeOwned + Send + Sync,
        <A as Action>::Output: serde::Serialize + Send + Sync,
        A::State: serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync,
    {
        let metadata = <A as Action>::metadata().clone();
        let factory: Arc<dyn ActionFactory> = Arc::new(GenericStatefulFactory::<A>::new());
        self.register_factory(metadata, factory);
    }

    /// Register a trigger action via the factory pipeline (Variant A).
    pub fn register_trigger_factory<A>(&self)
    where
        A: TriggerAction + FromWorkflowNode<Error = ActionError> + Send + Sync + 'static,
        <A as TriggerAction>::Error: Into<ActionError>,
    {
        let metadata = <A as Action>::metadata().clone();
        let factory: Arc<dyn ActionFactory> = Arc::new(GenericTriggerFactory::<A>::new());
        self.register_factory(metadata, factory);
    }

    /// Register a resource action via the factory pipeline (Variant A).
    pub fn register_resource_factory<A>(&self)
    where
        A: ResourceAction + FromWorkflowNode<Error = ActionError> + Send + Sync + 'static,
    {
        let metadata = <A as Action>::metadata().clone();
        let factory: Arc<dyn ActionFactory> = Arc::new(GenericResourceFactory::<A>::new());
        self.register_factory(metadata, factory);
    }

    /// Register a control action via the factory pipeline (Variant A).
    pub fn register_control_factory<A>(&self)
    where
        A: ControlAction + FromWorkflowNode<Error = ActionError> + Send + Sync + 'static,
    {
        let metadata = <A as Action>::metadata().clone();
        let factory: Arc<dyn ActionFactory> = Arc::new(GenericControlFactory::<A>::new());
        self.register_factory(metadata, factory);
    }

    /// Look up the factory for the given key, returning the latest version.
    ///
    /// Returns `None` if no factory has been registered for this key.
    /// Engine dispatch falls back to [`get`](Self::get) for the legacy
    /// `ActionHandler` path.
    #[must_use]
    pub fn get_factory(&self, key: &ActionKey) -> Option<(ActionMetadata, Arc<dyn ActionFactory>)> {
        let entries = self.factories.get(key)?;
        let last = entries.last()?;
        Some((last.metadata.clone(), Arc::clone(&last.factory)))
    }

    /// Look up a factory by key and exact version.
    #[must_use]
    pub fn get_factory_versioned(
        &self,
        key: &ActionKey,
        version: &Version,
    ) -> Option<(ActionMetadata, Arc<dyn ActionFactory>)> {
        let entries = self.factories.get(key)?;
        let entry = entries
            .iter()
            .find(|e| e.metadata.base.version == *version)?;
        Some((entry.metadata.clone(), Arc::clone(&entry.factory)))
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
            .field("factory_count", &self.factories.len())
            .field("keys", &keys)
            .finish_non_exhaustive()
    }
}

// phase3_disabled: Variant A migration of test fixtures pending — see PHASE3_BLOCKED.md
#[cfg(any())]
mod tests {
    use nebula_action::{
        action::Action, error::ActionError, metadata::ActionMetadata, result::ActionResult,
        stateless::StatelessAction,
    };

    use super::*;

    struct NoopAction {
        meta: ActionMetadata,
    }

    impl NoopAction {
        fn new(key: &'static str, major: u64, minor: u64) -> Self {
            Self {
                meta: ActionMetadata::new(ActionKey::new(key).unwrap(), "Noop", "Does nothing")
                    .with_version(major, minor),
            }
        }
    }

    impl DeclaresDependencies for NoopAction {}
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
            _ctx: &(impl nebula_action::ActionContext + ?Sized),
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
        let v1 = Version::new(1, 0, 0);
        let v2 = Version::new(2, 0, 0);

        assert!(registry.get_versioned(&key, &v1).is_some());
        assert!(registry.get_versioned(&key, &v2).is_some());

        let (meta, _) = registry.get(&key).unwrap();
        assert_eq!(meta.base.version, v2);
    }
}
