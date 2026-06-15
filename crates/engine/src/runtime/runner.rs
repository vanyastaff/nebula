//! Action runner abstraction — the engine-side dispatch boundary between
//! the action dispatcher and the in-process execution transport.
//!
//! The dispatcher owns the runner trait: it decides, per `IsolationLevel`,
//! how an action is executed. Today the sole runner is [`InProcessRunner`],
//! which runs actions in the same process with a cooperative cancellation
//! check.
//!
//! ## Key types
//!
//! - [`ActionRunner`] — execute an action through the in-process dispatch boundary.
//! - [`InProcessRunner`] — the sole runner; trusted in-process dispatch.
//! - [`ActionRunContext`] — cooperative cancellation check for the runner.
//! - [`ActionExecutor`] — registry-lookup-and-invoke callback.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::{ActionContext, ActionError, ActionMetadata, result::ActionResult};
use tokio_util::sync::CancellationToken;

/// Action run context wrapping an [`ActionContext`].
///
/// Provides a cooperative cancellation check before action execution.
pub struct ActionRunContext {
    cancellation: CancellationToken,
}

impl ActionRunContext {
    /// Build run-context metadata from an action context.
    pub fn new(context: &dyn ActionContext) -> Self {
        Self {
            cancellation: context.cancellation().clone(),
        }
    }

    /// Check whether execution has been cancelled.
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        if self.cancellation.is_cancelled() {
            Err(ActionError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Borrow the cancellation token for long-running dispatch paths that
    /// need to `select!` against it.
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

/// Trait for executing actions through the in-process dispatch boundary.
///
/// [`InProcessRunner`] is the sole implementation today. The trait exists
/// as the dispatch boundary so additional execution strategies can be added
/// additively without touching call sites.
///
/// WASM and out-of-process isolation are explicit non-goals — see
/// `docs/PRODUCT_CANON.md` (ADR-0091).
#[async_trait]
pub trait ActionRunner: Send + Sync {
    /// Execute an action through the runner.
    async fn execute(
        &self,
        context: ActionRunContext,
        metadata: &ActionMetadata,
        input: serde_json::Value,
    ) -> Result<ActionResult<serde_json::Value>, ActionError>;
}

/// Boxed future returned by the action executor.
pub type ActionExecutorFuture = std::pin::Pin<
    Box<dyn Future<Output = Result<ActionResult<serde_json::Value>, ActionError>> + Send>,
>;

/// Callback type for executing an action (registry lookup + invoke).
pub type ActionExecutor = Arc<
    dyn Fn(ActionRunContext, &ActionMetadata, serde_json::Value) -> ActionExecutorFuture
        + Send
        + Sync,
>;

/// In-process runner: runs actions in the same process (cooperative
/// cancellation check only — no isolation).
///
/// The sole [`ActionRunner`] implementation. Suitable for all registered
/// actions today; additional execution strategies can be introduced
/// additively as new implementations of [`ActionRunner`].
pub struct InProcessRunner {
    executor: ActionExecutor,
}

impl InProcessRunner {
    /// Create a new in-process runner with the given action executor.
    pub fn new(executor: ActionExecutor) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl ActionRunner for InProcessRunner {
    async fn execute(
        &self,
        context: ActionRunContext,
        metadata: &ActionMetadata,
        input: serde_json::Value,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        tracing::debug!(
            action_key = %metadata.base.key,
            "executing action in-process"
        );
        context.check_cancelled()?;
        let result = (self.executor)(context, metadata, input).await;
        if let Err(e) = &result {
            tracing::warn!(action_key = %metadata.base.key, error = %e, "action failed");
        }
        result
    }
}
