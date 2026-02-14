use std::collections::HashMap;
use std::sync::Arc;

use crate::action::Action;
use crate::metadata::ActionMetadata;

/// Type-erased registry for discovering and retrieving actions by key.
///
/// The engine populates this at startup and uses it to resolve action
/// keys from workflow definitions to concrete implementations.
///
/// Actions are stored as `Arc<dyn Action>` to allow shared ownership
/// across concurrent executions.
///
/// # Example
///
/// ```rust
/// use std::sync::Arc;
/// use nebula_action::{ActionRegistry, ActionMetadata, ActionType, Action};
///
/// struct NoOp(ActionMetadata);
/// impl Action for NoOp {
///     fn metadata(&self) -> &ActionMetadata { &self.0 }
///     fn action_type(&self) -> ActionType { ActionType::Process }
/// }
///
/// let mut registry = ActionRegistry::new();
/// let action = Arc::new(NoOp(ActionMetadata::new("noop", "No-Op", "Does nothing")));
/// registry.register(action);
///
/// assert!(registry.get("noop").is_some());
/// assert!(registry.get("unknown").is_none());
/// assert_eq!(registry.len(), 1);
/// ```
#[derive(Default)]
pub struct ActionRegistry {
    actions: HashMap<String, Arc<dyn Action>>,
}

impl ActionRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an action. Overwrites any existing action with the same key.
    pub fn register(&mut self, action: Arc<dyn Action>) {
        let key = action.metadata().key.clone();
        self.actions.insert(key, action);
    }

    /// Look up an action by its key.
    pub fn get(&self, key: &str) -> Option<&Arc<dyn Action>> {
        self.actions.get(key)
    }

    /// Check whether an action with the given key is registered.
    pub fn contains(&self, key: &str) -> bool {
        self.actions.contains_key(key)
    }

    /// Return metadata for all registered actions.
    pub fn list(&self) -> Vec<&ActionMetadata> {
        self.actions.values().map(|a| a.metadata()).collect()
    }

    /// Number of registered actions.
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Returns `true` if no actions are registered.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Remove an action by key. Returns the removed action, if any.
    pub fn unregister(&mut self, key: &str) -> Option<Arc<dyn Action>> {
        self.actions.remove(key)
    }

    /// Iterate over all registered `(key, action)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Arc<dyn Action>)> {
        self.actions.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl std::fmt::Debug for ActionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionRegistry")
            .field("count", &self.actions.len())
            .field("keys", &self.actions.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{ActionMetadata, ActionType};

    struct DummyAction(ActionMetadata);

    impl Action for DummyAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.0
        }
        fn action_type(&self) -> ActionType {
            ActionType::Process
        }
    }

    fn make_action(key: &str, name: &str) -> Arc<dyn Action> {
        Arc::new(DummyAction(ActionMetadata::new(key, name, "test")))
    }

    #[test]
    fn empty_registry() {
        let reg = ActionRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.get("anything").is_none());
    }

    #[test]
    fn register_and_get() {
        let mut reg = ActionRegistry::new();
        reg.register(make_action("http.request", "HTTP Request"));

        assert_eq!(reg.len(), 1);
        assert!(!reg.is_empty());

        let action = reg.get("http.request").unwrap();
        assert_eq!(action.metadata().key, "http.request");
        assert_eq!(action.metadata().name, "HTTP Request");
    }

    #[test]
    fn contains() {
        let mut reg = ActionRegistry::new();
        reg.register(make_action("a", "A"));
        assert!(reg.contains("a"));
        assert!(!reg.contains("b"));
    }

    #[test]
    fn overwrite_existing() {
        let mut reg = ActionRegistry::new();
        reg.register(make_action("x", "Version 1"));
        reg.register(make_action("x", "Version 2"));

        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("x").unwrap().metadata().name, "Version 2");
    }

    #[test]
    fn list_metadata() {
        let mut reg = ActionRegistry::new();
        reg.register(make_action("a", "Action A"));
        reg.register(make_action("b", "Action B"));

        let mut names: Vec<&str> = reg.list().iter().map(|m| m.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["Action A", "Action B"]);
    }

    #[test]
    fn unregister() {
        let mut reg = ActionRegistry::new();
        reg.register(make_action("temp", "Temporary"));

        let removed = reg.unregister("temp");
        assert!(removed.is_some());
        assert!(reg.is_empty());

        let removed_again = reg.unregister("temp");
        assert!(removed_again.is_none());
    }

    #[test]
    fn iter_actions() {
        let mut reg = ActionRegistry::new();
        reg.register(make_action("a", "A"));
        reg.register(make_action("b", "B"));

        let mut keys: Vec<&str> = reg.iter().map(|(k, _)| k).collect();
        keys.sort();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[test]
    fn debug_format() {
        let mut reg = ActionRegistry::new();
        reg.register(make_action("test", "Test"));
        let debug = format!("{reg:?}");
        assert!(debug.contains("ActionRegistry"));
        assert!(debug.contains("count: 1"));
    }
}
