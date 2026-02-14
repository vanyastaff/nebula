//! Action registry for looking up handlers by key.

use std::sync::Arc;

use dashmap::DashMap;

use nebula_action::handler::InternalHandler;

use crate::error::RuntimeError;

/// Thread-safe registry of action handlers.
///
/// Actions are registered by key (e.g. `"http.request"`) and looked up
/// at execution time. Uses `DashMap` for lock-free concurrent access.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_runtime::registry::ActionRegistry;
///
/// let registry = ActionRegistry::new();
/// registry.register(Arc::new(my_handler));
/// let handler = registry.get("http.request").unwrap();
/// ```
pub struct ActionRegistry {
    handlers: DashMap<String, Arc<dyn InternalHandler>>,
}

impl ActionRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: DashMap::new(),
        }
    }

    /// Register an action handler.
    ///
    /// If a handler with the same key already exists, it is replaced.
    pub fn register(&self, handler: Arc<dyn InternalHandler>) {
        let key = handler.metadata().key.clone();
        tracing::info!(action_key = %key, "registered action handler");
        self.handlers.insert(key, handler);
    }

    /// Register a typed [`ProcessAction`](nebula_action::ProcessAction) directly.
    ///
    /// Wraps the action in a [`ProcessActionAdapter`](nebula_action::ProcessActionAdapter)
    /// automatically.
    pub fn register_process<A>(&self, action: A)
    where
        A: nebula_action::ProcessAction + Send + Sync + 'static,
        A::Input: serde::de::DeserializeOwned + Send + Sync + 'static,
        A::Output: serde::Serialize + Send + Sync + 'static,
    {
        let adapter = nebula_action::ProcessActionAdapter::new(action);
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

    /// Check if a handler is registered for the given key.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.handlers.contains_key(key)
    }

    /// Remove a handler by key. Returns the removed handler, if any.
    pub fn remove(&self, key: &str) -> Option<Arc<dyn InternalHandler>> {
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

impl Default for ActionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_action::ParameterCollection;
    use nebula_action::context::ActionContext;
    use nebula_action::error::ActionError;
    use nebula_action::metadata::{ActionMetadata, ActionType};
    use nebula_action::result::ActionResult;

    /// Minimal test handler that echoes input.
    struct EchoHandler {
        meta: ActionMetadata,
    }

    impl EchoHandler {
        fn new(key: &str) -> Self {
            Self {
                meta: ActionMetadata::new(key, key, "test handler"),
            }
        }
    }

    #[async_trait::async_trait]
    impl InternalHandler for EchoHandler {
        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: ActionContext,
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            Ok(ActionResult::success(input))
        }

        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        fn action_type(&self) -> ActionType {
            ActionType::Process
        }

        fn parameters(&self) -> Option<&ParameterCollection> {
            None
        }
    }

    #[test]
    fn register_and_lookup() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoHandler::new("test.echo")));

        assert!(reg.contains("test.echo"));
        assert_eq!(reg.len(), 1);

        let handler = reg.get("test.echo").unwrap();
        assert_eq!(handler.metadata().key, "test.echo");
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
        reg.register(Arc::new(EchoHandler::new("test.a")));
        reg.register(Arc::new(EchoHandler::new("test.a")));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn remove_handler() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoHandler::new("test.rem")));
        assert!(reg.contains("test.rem"));

        let removed = reg.remove("test.rem");
        assert!(removed.is_some());
        assert!(!reg.contains("test.rem"));
    }

    #[test]
    fn keys_lists_all() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoHandler::new("a")));
        reg.register(Arc::new(EchoHandler::new("b")));
        reg.register(Arc::new(EchoHandler::new("c")));

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
}
