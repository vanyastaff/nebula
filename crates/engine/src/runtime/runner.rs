//! Capability-gated in-process dispatch through exact action handles.

use async_trait::async_trait;
use nebula_action::{
    ActionContext, ActionError, ActionInput, ActionResult, StatelessHandle, StreamHandle,
};
use tokio_util::sync::CancellationToken;

/// Action run context wrapping an [`ActionContext`].
///
/// Provides cooperative cancellation checks around input preparation.
pub struct ActionRunContext {
    cancellation: CancellationToken,
}

impl ActionRunContext {
    /// Build run-context metadata from an action context.
    #[must_use]
    pub fn new(context: &dyn ActionContext) -> Self {
        Self {
            cancellation: context.cancellation().clone(),
        }
    }

    /// Check whether execution has been cancelled.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Cancelled`] after cancellation is requested.
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        if self.cancellation.is_cancelled() {
            Err(ActionError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Borrow the cancellation token for long-running dispatch paths.
    #[must_use]
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

/// Object-safe capability-gated dispatch boundary.
///
/// The exact receiving handle crosses this boundary with raw or schema-resolved
/// ingress. Runner implementations cannot replace handle-owned preparation with
/// schema-only validation.
#[async_trait]
pub trait ActionRunner: Send + Sync {
    /// Prepare and execute one stateless action through the same handle.
    ///
    /// # Errors
    ///
    /// Returns an action validation, cancellation, or execution error.
    async fn execute_stateless(
        &self,
        run_context: ActionRunContext,
        handle: Box<dyn StatelessHandle>,
        input: ActionInput,
        action_context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError>;

    /// Prepare and execute one stream action through the same handle.
    ///
    /// # Errors
    ///
    /// Returns an action validation, cancellation, or execution error.
    async fn execute_stream(
        &self,
        run_context: ActionRunContext,
        handle: Box<dyn StreamHandle>,
        input: ActionInput,
        action_context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError>;
}

/// Trusted in-process runner with cooperative cancellation checks.
#[derive(Debug, Default)]
pub struct InProcessRunner;

impl InProcessRunner {
    /// Construct the in-process runner.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ActionRunner for InProcessRunner {
    #[tracing::instrument(
        name = "action.runner.stateless",
        skip_all,
        fields(action_key = %handle.metadata().base().key())
    )]
    async fn execute_stateless(
        &self,
        run_context: ActionRunContext,
        handle: Box<dyn StatelessHandle>,
        input: ActionInput,
        action_context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        run_context.check_cancelled()?;
        let prepared = handle.prepare_input(input)?;
        run_context.check_cancelled()?;
        handle.dispatch(prepared, action_context).await
    }

    #[tracing::instrument(
        name = "action.runner.stream",
        skip_all,
        fields(action_key = %handle.metadata().base().key())
    )]
    async fn execute_stream(
        &self,
        run_context: ActionRunContext,
        handle: Box<dyn StreamHandle>,
        input: ActionInput,
        action_context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        run_context.check_cancelled()?;
        let prepared = handle.prepare_input(input)?;
        run_context.check_cancelled()?;
        handle.dispatch(prepared, action_context).await
    }
}
