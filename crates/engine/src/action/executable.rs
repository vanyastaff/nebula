use crate::action::{Action, ActionContext, ActionError, ActionType};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::execution::ExecutionMode;

#[async_trait]
pub trait ExecutableContext: ActionContext {}

/// Trait for actions that can be executed with optional rollback capabilities.
///
/// Simple interface for actions that perform operations and can optionally undo them.
#[async_trait]
pub trait ExecutableAction: Action {
    /// Output type produced when execution succeeds.
    type Output: Send + Sync + Clone + Serialize + for<'de> Deserialize<'de>;
    
    /// Unique identifier for this action type.
    fn action_type(&self) -> ActionType {
        ActionType::Executable
    }

    /// Execute the action with the given context.
    ///
    /// # Arguments
    ///
    /// * `context` - Execution context for callbacks
    ///
    /// # Returns
    ///
    /// * `Ok(Self::Output)` if execution succeeded
    /// * `Err(ActionError)` if execution failed
    async fn execute<C>(&self, context: &C) -> Result<Self::Output, ActionError>
    where
        C: ExecutableContext + Send + Sync;

    /// Rollback the changes made by this action.
    ///
    /// Only called if `supports_rollback()` returns true.
    ///
    /// # Arguments
    ///
    /// * `context` - Execution context for callbacks
    ///
    /// # Returns
    ///
    /// * `Ok(())` if rollback succeeded
    /// * `Err(ActionError)` if rollback failed
    async fn rollback<C>(&self, context: &C) -> Result<(), ActionError>
    where
        C: ExecutableContext + Send + Sync,
    {
        Ok(())
    }

    /// Check if this action supports rollback operations.
    fn supports_rollback(&self) -> bool {
        false 
    }
}


pub struct ExecutableExecutor {

}

impl ExecutableExecutor {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn execute<A, C>(&self, action: &A, context: &C, mode: ExecutionMode) -> Result<A::Output, ActionError>
    where
        A: ExecutableAction + Clone + Send + Sync + 'static,
        A::Output: 'static,
        C: ExecutableContext + Clone + Send + Sync + 'static,
    {
        match mode {
            
        }
    }
    
    pub async fn rollback<A, C>(&self, action: &A, context: &C) -> Result<(), ActionError>
    where
        A: ExecutableAction + Clone + Send + Sync + 'static,
        C: ExecutableContext + Clone + Send + Sync + 'static,
    {
        if action.supports_rollback() {
            action.rollback(context).await
        } else {
            Err(ActionError::UnsupportedOperation("Rollback not supported".into()))
        }
    }

    /// Execute an action with automatic rollback on failure.
    ///
    /// If execution fails and the action supports rollback, automatically
    /// performs rollback to restore the previous state.
    pub async fn execute_with_rollback<A, C>(
        &self,
        action: &A,
        context: &C,
    ) -> Result<A::Output, ActionError>
    where
        A: ExecutableAction,
        C: ExecutableContext + Send + Sync,
    {
        match action.execute(context).await {
            Ok(result) => Ok(result),
            Err(error) => {
                // Try rollback if supported
                if action.supports_rollback() {
                    if let Err(rollback_error) = action.rollback(context).await {
                        // Return both original error and rollback error
                        return Err(ActionError::Execution {
                            message: format!(
                                "Execution failed: {}. Rollback also failed: {}",
                                error, rollback_error
                            ),
                        });
                    }
                }
                Err(error)
            }
        }
    }
}