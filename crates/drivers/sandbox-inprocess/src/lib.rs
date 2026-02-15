#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula In-Process Sandbox Driver
//!
//! Implements the [`SandboxRunner`] port by executing actions directly in
//! the current process. Capability checks are enforced via the
//! [`SandboxedContext`] -- the action can only access resources that
//! were explicitly granted.
//!
//! This driver is used for desktop and trusted single-process deployments
//! where WASM isolation is not needed.
//!
//! # Architecture
//!
//! The sandbox receives an **action executor** callback at construction
//! time. The runtime provides this callback, which knows how to look up
//! the action by key and invoke it. The sandbox's responsibility is
//! limited to capability validation and execution boundaries.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::result::ActionResult;
use nebula_action::{ActionError, ActionMetadata, SandboxedContext};
use nebula_ports::SandboxRunner;

/// Boxed future returned by the action executor.
pub type ActionExecutorFuture = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = Result<ActionResult<serde_json::Value>, ActionError>>
            + Send,
    >,
>;

/// Callback type for executing an action.
///
/// The runtime provides this function, which:
/// 1. Looks up the action by key from `ActionMetadata`
/// 2. Invokes it with the given context and input
/// 3. Returns the output or error
pub type ActionExecutor = Arc<
    dyn Fn(SandboxedContext, &ActionMetadata, serde_json::Value) -> ActionExecutorFuture
        + Send
        + Sync,
>;

/// In-process sandbox that executes actions in the same process.
///
/// Capability checks are handled by [`SandboxedContext`] -- when the
/// action tries to access a resource, the context validates against
/// the granted capabilities.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_sandbox_inprocess::InProcessSandbox;
/// use std::sync::Arc;
///
/// let executor: ActionExecutor = Arc::new(|ctx, metadata, input| {
///     Box::pin(async move {
///         // Look up and execute the action...
///         Ok(nebula_action::ActionResult::success(serde_json::json!({"result": "ok"})))
///     })
/// });
///
/// let sandbox = InProcessSandbox::new(executor);
/// ```
pub struct InProcessSandbox {
    executor: ActionExecutor,
}

impl InProcessSandbox {
    /// Create a new in-process sandbox with the given action executor.
    pub fn new(executor: ActionExecutor) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl SandboxRunner for InProcessSandbox {
    async fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: serde_json::Value,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        tracing::debug!(
            action_key = %metadata.key,
            isolation = ?metadata.isolation_level,
            "executing action in-process"
        );

        // Check cancellation before starting.
        context.check_cancelled()?;

        // Delegate to the runtime-provided executor.
        let result = (self.executor)(context, metadata, input).await;

        match &result {
            Ok(_) => {
                tracing::debug!(action_key = %metadata.key, "action completed successfully");
            }
            Err(e) => {
                tracing::warn!(action_key = %metadata.key, error = %e, "action failed");
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_action::capability::{Capability, IsolationLevel};
    use nebula_action::context::ActionContext;
    use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
    use nebula_core::scope::ScopeLevel;

    fn test_metadata() -> ActionMetadata {
        ActionMetadata::new("test.echo", "Echo", "Returns input as output")
            .with_isolation(IsolationLevel::CapabilityGated)
    }

    fn test_context(caps: Vec<Capability>) -> SandboxedContext {
        let ctx = ActionContext::new(
            ExecutionId::v4(),
            NodeId::v4(),
            WorkflowId::v4(),
            ScopeLevel::Global,
        );
        SandboxedContext::new(ctx, caps)
    }

    #[tokio::test]
    async fn execute_returns_executor_result() {
        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });

        let sandbox = InProcessSandbox::new(executor);
        let metadata = test_metadata();
        let input = serde_json::json!({"hello": "world"});
        let ctx = test_context(vec![]);

        let result = sandbox.execute(ctx, &metadata, input.clone()).await;
        match result.unwrap() {
            ActionResult::Success { output } => {
                assert_eq!(output.as_value(), Some(&input));
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_propagates_executor_error() {
        let executor: ActionExecutor = Arc::new(|_ctx, _meta, _input| {
            Box::pin(async move { Err(ActionError::fatal("test error")) })
        });

        let sandbox = InProcessSandbox::new(executor);
        let metadata = test_metadata();
        let ctx = test_context(vec![]);

        let result = sandbox
            .execute(ctx, &metadata, serde_json::json!(null))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().is_fatal());
    }

    #[tokio::test]
    async fn execute_checks_cancellation() {
        let executor: ActionExecutor = Arc::new(|_ctx, _meta, _input| {
            Box::pin(
                async move { Ok(ActionResult::success(serde_json::json!("should not reach"))) },
            )
        });

        let sandbox = InProcessSandbox::new(executor);
        let metadata = test_metadata();

        let ctx = ActionContext::new(
            ExecutionId::v4(),
            NodeId::v4(),
            WorkflowId::v4(),
            ScopeLevel::Global,
        );
        // Cancel before execution.
        ctx.cancellation.cancel();
        let sandboxed = SandboxedContext::new(ctx, vec![]);

        let result = sandbox
            .execute(sandboxed, &metadata, serde_json::json!(null))
            .await;
        assert!(matches!(result, Err(ActionError::Cancelled)));
    }
}
