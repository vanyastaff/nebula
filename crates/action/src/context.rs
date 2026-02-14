use std::sync::Arc;

use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
use nebula_core::scope::ScopeLevel;
use parking_lot::RwLock;
use tokio_util::sync::CancellationToken;

use crate::error::ActionError;

/// Runtime context provided to every action during execution.
///
/// Constructed by the engine before invoking an action. Provides identity
/// information (which execution, workflow, and node this is), workflow-scoped
/// variables, and a cancellation token.
///
/// Actions **must** periodically call [`check_cancelled`](Self::check_cancelled)
/// in long-running loops to support cooperative cancellation.
#[non_exhaustive]
pub struct ActionContext {
    /// Unique execution run identifier.
    pub execution_id: ExecutionId,
    /// Node in the workflow graph being executed.
    pub node_id: NodeId,
    /// Workflow this execution belongs to.
    pub workflow_id: WorkflowId,
    /// Scope level for resource access control.
    pub scope: ScopeLevel,
    /// Cancellation signal — checked cooperatively by actions.
    pub cancellation: CancellationToken,
    /// Shared workflow-scoped variables.
    variables: Arc<RwLock<serde_json::Map<String, serde_json::Value>>>,
}

impl ActionContext {
    /// Create a new context with the given identifiers.
    pub fn new(
        execution_id: ExecutionId,
        node_id: NodeId,
        workflow_id: WorkflowId,
        scope: ScopeLevel,
    ) -> Self {
        Self {
            execution_id,
            node_id,
            workflow_id,
            scope,
            cancellation: CancellationToken::new(),
            variables: Arc::new(RwLock::new(serde_json::Map::new())),
        }
    }

    /// Create a context with a pre-existing cancellation token.
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancellation = token;
        self
    }

    /// Create a context with pre-populated variables.
    pub fn with_variables(mut self, vars: serde_json::Map<String, serde_json::Value>) -> Self {
        self.variables = Arc::new(RwLock::new(vars));
        self
    }

    /// Read a variable from the workflow scope.
    ///
    /// Returns `None` if the variable does not exist.
    pub fn get_variable(&self, key: &str) -> Option<serde_json::Value> {
        self.variables.read().get(key).cloned()
    }

    /// Write a variable to the workflow scope.
    ///
    /// Overwrites any existing variable with the same key.
    pub fn set_variable(&self, key: &str, value: serde_json::Value) {
        self.variables.write().insert(key.to_owned(), value);
    }

    /// Remove a variable from the workflow scope.
    ///
    /// Returns the previous value, if any.
    pub fn remove_variable(&self, key: &str) -> Option<serde_json::Value> {
        self.variables.write().remove(key)
    }

    /// Check whether execution has been cancelled.
    ///
    /// Actions **should** call this in loops and before expensive operations
    /// to support cooperative cancellation.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Cancelled`] if the token has been triggered.
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        if self.cancellation.is_cancelled() {
            Err(ActionError::Cancelled)
        } else {
            Ok(())
        }
    }
}

impl std::fmt::Debug for ActionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionContext")
            .field("execution_id", &self.execution_id)
            .field("node_id", &self.node_id)
            .field("workflow_id", &self.workflow_id)
            .field("scope", &self.scope)
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> ActionContext {
        ActionContext::new(
            ExecutionId::v4(),
            NodeId::v4(),
            WorkflowId::v4(),
            ScopeLevel::Global,
        )
    }

    #[test]
    fn get_set_variable() {
        let ctx = test_context();
        assert!(ctx.get_variable("count").is_none());

        ctx.set_variable("count", serde_json::json!(42));
        assert_eq!(ctx.get_variable("count"), Some(serde_json::json!(42)));
    }

    #[test]
    fn overwrite_variable() {
        let ctx = test_context();
        ctx.set_variable("name", serde_json::json!("alice"));
        ctx.set_variable("name", serde_json::json!("bob"));
        assert_eq!(ctx.get_variable("name"), Some(serde_json::json!("bob")));
    }

    #[test]
    fn remove_variable() {
        let ctx = test_context();
        ctx.set_variable("temp", serde_json::json!(true));
        let old = ctx.remove_variable("temp");
        assert_eq!(old, Some(serde_json::json!(true)));
        assert!(ctx.get_variable("temp").is_none());
    }

    #[test]
    fn check_cancelled_ok() {
        let ctx = test_context();
        assert!(ctx.check_cancelled().is_ok());
    }

    #[test]
    fn check_cancelled_after_cancel() {
        let ctx = test_context();
        ctx.cancellation.cancel();
        let err = ctx.check_cancelled().unwrap_err();
        assert!(matches!(err, ActionError::Cancelled));
    }

    #[test]
    fn with_cancellation_token() {
        let token = CancellationToken::new();
        let child = token.child_token();
        let ctx = test_context().with_cancellation(child);
        assert!(ctx.check_cancelled().is_ok());
        token.cancel();
        assert!(ctx.check_cancelled().is_err());
    }

    #[test]
    fn with_variables() {
        let mut vars = serde_json::Map::new();
        vars.insert("preset".into(), serde_json::json!("value"));
        let ctx = test_context().with_variables(vars);
        assert_eq!(ctx.get_variable("preset"), Some(serde_json::json!("value")));
    }

    #[test]
    fn debug_format() {
        let ctx = test_context();
        let debug = format!("{ctx:?}");
        assert!(debug.contains("ActionContext"));
        assert!(debug.contains("execution_id"));
    }
}
