//! Registry of available actions, keyed by `ActionKey`.
//!
//! Single source of truth for action registration in nebula. The runtime
//! owns this type because registration is fundamentally an execution concern —
//! the registry holds `Arc<dyn ActionFactory>` entries for dispatch.
//!
//! # Version-aware lookup
//!
//! Multiple versions of the same action can be registered simultaneously.
//! [`ActionRegistry::get_factory`] returns the **latest** version (highest major,
//! then minor), while [`ActionRegistry::get_factory_versioned`] retrieves a
//! specific `"major.minor"` version.
//!
//! # Thread safety
//!
//! Uses `DashMap` for lock-free concurrent access. Both registration and
//! lookup use `&self` — share via `Arc<ActionRegistry>` without external
//! synchronization.

use std::sync::Arc;

use dashmap::DashMap;
use nebula_action::{
    Action, ActionError, ActionFactory, ActionMetadata, ControlAction, FromWorkflowNode,
    GenericControlFactory, GenericResourceFactory, GenericStatefulFactory, GenericStatelessFactory,
    GenericTriggerFactory, InstanceFactory, ResourceAction, StatefulAction, StatelessAction,
    TriggerAction, WebhookActionFactory,
};
use nebula_core::ActionKey;
use semver::Version;

/// A single factory entry in the registry.
#[derive(Clone)]
struct FactoryEntry {
    metadata: ActionMetadata,
    factory: Arc<dyn ActionFactory>,
}

/// Type-safe registry for action factories, keyed by `ActionKey`.
#[derive(Default)]
pub struct ActionRegistry {
    /// Map from action key to list of factory entries, each at a distinct version.
    factories: DashMap<ActionKey, Vec<FactoryEntry>>,
    /// Provider-typed webhook factory map (M3.3). Sibling to `factories` because
    /// provider kinds are coarser than `ActionKey` and arrive as runtime strings from
    /// operator-supplied storage rows. Use [`Self::register_webhook_provider`] /
    /// [`Self::lookup_webhook_factory`] to access it.
    webhook_factories: DashMap<&'static str, Arc<dyn WebhookActionFactory>>,
}

impl ActionRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an action factory.
    ///
    /// The factory is consulted at dispatch time to instantiate a fresh handle per
    /// execution. This is the canonical path for all actions implementing
    /// [`Action`] + [`FromWorkflowNode`] (Variant A) or registered via
    /// [`register_stateless_instance`](Self::register_stateless_instance).
    ///
    /// If an entry with the same key **and** the same `"major.minor"` version already
    /// exists it is replaced in-place. Otherwise the new entry is appended; entries are
    /// kept sorted from lowest to highest version so that
    /// [`get_factory`](Self::get_factory) can return the latest in O(1).
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

    /// Register a provider-typed webhook factory (M3.3).
    ///
    /// String-keyed (factory.kind()) because provider names come from
    /// operator-supplied storage rows, not Rust types. Subsequent
    /// registrations with the same key replace the previous factory.
    pub fn register_webhook_provider(&self, factory: Arc<dyn WebhookActionFactory>) {
        let kind = factory.kind();
        self.webhook_factories.insert(kind, factory);
    }

    /// Look up a registered webhook factory by provider kind.
    ///
    /// Used by the API webhook bootstrap to instantiate handlers
    /// from stored activation specs.
    #[must_use]
    pub fn lookup_webhook_factory(&self, kind: &str) -> Option<Arc<dyn WebhookActionFactory>> {
        self.webhook_factories.get(kind).map(|e| Arc::clone(&*e))
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
        let factory: Arc<dyn ActionFactory> = Arc::new(GenericStatelessFactory::<A>::new());
        let metadata = factory.metadata().clone();
        self.register_factory(metadata, factory);
    }

    /// Register a pre-built stateless action **instance** with caller-supplied
    /// metadata, via the factory pipeline.
    ///
    /// The `useValue` complement to [`register_stateless_factory`](Self::register_stateless_factory)'s
    /// `useFactory`: instead of constructing a fresh `A` from the node per
    /// dispatch, this shares the one instance across dispatches and lets the
    /// caller vary the catalog metadata (key / version / ports) per
    /// registration — so one action type can back many distinct nodes. Backed
    /// by [`InstanceFactory`].
    pub fn register_stateless_instance<A>(&self, metadata: ActionMetadata, action: A)
    where
        A: StatelessAction + Send + Sync + 'static,
        <A as Action>::Input: serde::de::DeserializeOwned + Send + Sync,
        <A as Action>::Output: serde::Serialize + Send + Sync,
    {
        let factory: Arc<dyn ActionFactory> = Arc::new(InstanceFactory::new(metadata, action));
        let meta = factory.metadata().clone();
        self.register_factory(meta, factory);
    }

    /// Register a stateful action via the factory pipeline (Variant A).
    pub fn register_stateful_factory<A>(&self)
    where
        A: StatefulAction + FromWorkflowNode<Error = ActionError>,
        <A as Action>::Input: serde::de::DeserializeOwned + Send + Sync,
        <A as Action>::Output: serde::Serialize + Send + Sync,
        A::State: serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync,
    {
        let factory: Arc<dyn ActionFactory> = Arc::new(GenericStatefulFactory::<A>::new());
        let metadata = factory.metadata().clone();
        self.register_factory(metadata, factory);
    }

    /// Register a trigger action via the factory pipeline (Variant A).
    pub fn register_trigger_factory<A>(&self)
    where
        A: TriggerAction + FromWorkflowNode<Error = ActionError> + Send + Sync + 'static,
        <A as TriggerAction>::Error: Into<ActionError>,
    {
        let factory: Arc<dyn ActionFactory> = Arc::new(GenericTriggerFactory::<A>::new());
        let metadata = factory.metadata().clone();
        self.register_factory(metadata, factory);
    }

    /// Register a resource action via the factory pipeline (Variant A).
    pub fn register_resource_factory<A>(&self)
    where
        A: ResourceAction + FromWorkflowNode<Error = ActionError> + Send + Sync + 'static,
    {
        let factory: Arc<dyn ActionFactory> = Arc::new(GenericResourceFactory::<A>::new());
        let metadata = factory.metadata().clone();
        self.register_factory(metadata, factory);
    }

    /// Register a control action via the factory pipeline (Variant A).
    pub fn register_control_factory<A>(&self)
    where
        A: ControlAction + FromWorkflowNode<Error = ActionError> + Send + Sync + 'static,
    {
        let factory: Arc<dyn ActionFactory> = Arc::new(GenericControlFactory::<A>::new());
        let metadata = factory.metadata().clone();
        self.register_factory(metadata, factory);
    }

    /// Look up the factory for the given key, returning the latest version.
    ///
    /// Returns `None` if no factory has been registered for this key.
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

    /// All registered action keys (from the factory map).
    #[must_use]
    pub fn keys(&self) -> Vec<ActionKey> {
        self.factories
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Total number of registered action keys (not counting multiple versions of the same key).
    #[must_use]
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Returns `true` if no actions have been registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }
}

impl std::fmt::Debug for ActionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let registered_keys: Vec<ActionKey> = self.keys();
        f.debug_struct("ActionRegistry")
            .field("factory_count", &self.factories.len())
            .field("keys", &registered_keys)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use nebula_action::{
        action::Action,
        error::ActionError,
        metadata::{ActionKind, ActionMetadata},
        result::ActionResult,
        stateless::StatelessAction,
    };
    use nebula_core::{Dependencies, action_key};
    use nebula_workflow::NodeDefinition;

    use super::*;

    struct NoopAction;

    impl Action for NoopAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(action_key!("test.noop"), "Noop", "Does nothing")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }
    impl StatelessAction for NoopAction {
        async fn execute(
            &self,
            input: <Self as Action>::Input,
            _ctx: &(impl nebula_action::ActionContext + ?Sized),
        ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    fn meta_with(key: &'static str, major: u64, minor: u64) -> ActionMetadata {
        ActionMetadata::new(ActionKey::new(key).unwrap(), "Noop", "Does nothing")
            .with_version(major, minor)
    }

    // Stateful fixture used to prove the factory registration path stores the
    // factory-stamped node kind. Its bare `Action::metadata()` carries the
    // default `Stateless` kind; only the factory stamps `Stateful`.
    struct NoopStateful;

    impl Action for NoopStateful {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(
                action_key!("test.noop_stateful"),
                "NoopStateful",
                "Does nothing, iteratively",
            )
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl StatefulAction for NoopStateful {
        type State = serde_json::Value;

        fn init_state(&self) -> Self::State {
            serde_json::Value::Null
        }

        async fn execute(
            &self,
            input: <Self as Action>::Input,
            _state: &mut Self::State,
            _ctx: &(impl nebula_action::ActionContext + ?Sized),
        ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
            Ok(ActionResult::break_completed(input))
        }
    }

    impl FromWorkflowNode for NoopStateful {
        type Error = ActionError;

        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn nebula_action::ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(NoopStateful)
        }
    }

    #[test]
    fn factory_registration_stores_stamped_kind() {
        // The registry must store the factory-stamped node kind, not the
        // unstamped `Action::metadata()` default — otherwise registry consumers
        // see `Stateless` for a stateful action.
        let registry = ActionRegistry::new();
        registry.register_stateful_factory::<NoopStateful>();

        let key = action_key!("test.noop_stateful");
        let (metadata, _factory) = registry.get_factory(&key).expect("factory was registered");
        assert_eq!(metadata.kind, ActionKind::Stateful);
    }

    #[test]
    fn register_and_get_action() {
        // `register_stateless_instance` lands on the factory spine, so assert
        // via the factory lookup (the surviving registration path).
        let registry = ActionRegistry::new();
        registry.register_stateless_instance(meta_with("test.noop", 1, 0), NoopAction);
        let key = ActionKey::new("test.noop").unwrap();
        assert!(registry.get_factory(&key).is_some());
        assert_eq!(registry.factories.len(), 1);
    }

    #[test]
    fn register_replaces_same_version() {
        let registry = ActionRegistry::new();
        registry.register_stateless_instance(meta_with("test.noop", 1, 0), NoopAction);
        registry.register_stateless_instance(meta_with("test.noop", 1, 0), NoopAction);
        let key = ActionKey::new("test.noop").unwrap();
        assert_eq!(
            registry.factories.get(&key).map(|entries| entries.len()),
            Some(1),
            "same (key, version) must replace in place, not append a duplicate"
        );
    }

    #[test]
    fn versioned_lookup() {
        let registry = ActionRegistry::new();
        registry.register_stateless_instance(meta_with("test.noop", 1, 0), NoopAction);
        registry.register_stateless_instance(meta_with("test.noop", 2, 0), NoopAction);

        let key = ActionKey::new("test.noop").unwrap();
        let v1 = Version::new(1, 0, 0);
        let v2 = Version::new(2, 0, 0);

        assert!(registry.get_factory_versioned(&key, &v1).is_some());
        assert!(registry.get_factory_versioned(&key, &v2).is_some());

        let (meta, _) = registry.get_factory(&key).unwrap();
        assert_eq!(
            meta.base.version, v2,
            "get_factory returns the latest version"
        );
    }
}
