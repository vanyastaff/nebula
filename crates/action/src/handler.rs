//! Dynamic handler trait and typed action adapters.
//!
//! The runtime stores all actions as `Arc<dyn InternalHandler>` — a JSON-erased
//! interface. Typed action authors write `impl StatelessAction<Input=T, Output=U>`
//! and register via [`StatelessActionAdapter`] (or the registry's helper methods),
//! which handles (de)serialization automatically.

use async_trait::async_trait;

use crate::context::ActionContext;
use crate::error::ActionError;
use crate::execution::StatelessAction;
use crate::metadata::ActionMetadata;
use crate::result::ActionResult;

/// Handler trait for action execution; runtime looks up by key and calls
/// `execute` with JSON input and [`ActionContext`].
///
/// This is the *internal* contract between registry and runtime. Action authors
/// implement typed traits ([`StatelessAction`] etc.) and use adapters to
/// convert to `dyn InternalHandler`.
#[async_trait]
pub trait InternalHandler: Send + Sync {
    /// Get action metadata.
    fn metadata(&self) -> &ActionMetadata;
    /// Execute the action with the given input and execution context.
    async fn execute(
        &self,
        input: serde_json::Value,
        context: &ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError>;
}

/// Wraps a [`StatelessAction`] as a [`dyn InternalHandler`].
///
/// Handles JSON deserialization of input and serialization of output so the
/// runtime can work with untyped JSON throughout, while action authors write
/// strongly-typed Rust.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::{StatelessActionAdapter, StatelessAction, Action, ActionResult, ActionError};
/// use nebula_action::handler::InternalHandler;
///
/// struct EchoAction { meta: ActionMetadata }
/// impl Action for EchoAction { ... }
/// impl StatelessAction for EchoAction {
///     type Input = serde_json::Value;
///     type Output = serde_json::Value;
///     async fn execute(&self, input: Self::Input, _ctx: &impl Context)
///         -> Result<ActionResult<Self::Output>, ActionError>
///     {
///         Ok(ActionResult::success(input))
///     }
/// }
///
/// let handler: Arc<dyn InternalHandler> = Arc::new(StatelessActionAdapter::new(EchoAction { ... }));
/// ```
pub struct StatelessActionAdapter<A> {
    action: A,
}

impl<A> StatelessActionAdapter<A> {
    /// Wrap a typed stateless action.
    pub fn new(action: A) -> Self {
        Self { action }
    }

    /// Consume the adapter, returning the inner action.
    pub fn into_inner(self) -> A {
        self.action
    }
}

#[async_trait]
impl<A> InternalHandler for StatelessActionAdapter<A>
where
    A: StatelessAction + Send + Sync + 'static,
    A::Input: serde::de::DeserializeOwned + Send + Sync,
    A::Output: serde::Serialize + Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        context: &ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        let typed_input: A::Input = serde_json::from_value(input)
            .map_err(|e| ActionError::validation(format!("input deserialization failed: {e}")))?;

        let result = self.action.execute(typed_input, context).await?;

        result.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde::{Deserialize, Serialize};
    use tokio_util::sync::CancellationToken;

    use crate::action::Action;
    use crate::components::ActionComponents;
    use crate::context::Context;
    use crate::execution::StatelessAction;
    use crate::metadata::ActionMetadata;
    use nebula_core::id::{ExecutionId, NodeId, WorkflowId};

    use super::*;

    // ── Test action ────────────────────────────────────────────────────────────

    #[derive(Debug, Deserialize)]
    struct AddInput {
        a: i64,
        b: i64,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct AddOutput {
        sum: i64,
    }

    struct AddAction {
        meta: ActionMetadata,
    }

    impl AddAction {
        fn new() -> Self {
            Self {
                meta: ActionMetadata::new("math.add", "Add", "Adds two numbers"),
            }
        }
    }

    impl Action for AddAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
        fn components(&self) -> ActionComponents {
            ActionComponents::new()
        }
    }

    impl StatelessAction for AddAction {
        type Input = AddInput;
        type Output = AddOutput;

        fn execute(
            &self,
            input: Self::Input,
            _ctx: &impl Context,
        ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send {
            async move { Ok(ActionResult::success(AddOutput { sum: input.a + input.b })) }
        }
    }

    fn make_ctx() -> ActionContext {
        ActionContext::new(
            ExecutionId::nil(),
            NodeId::nil(),
            WorkflowId::nil(),
            CancellationToken::new(),
        )
    }

    // ── Tests ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn adapter_executes_typed_action() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let ctx = make_ctx();

        let input = serde_json::json!({ "a": 3, "b": 7 });
        let result = adapter.execute(input, &ctx).await.unwrap();

        match result {
            ActionResult::Success { output } => {
                let v = output.into_value().unwrap();
                let out: AddOutput = serde_json::from_value(v).unwrap();
                assert_eq!(out.sum, 10);
            }
            _ => panic!("expected Success"),
        }
    }

    #[tokio::test]
    async fn adapter_returns_validation_error_on_bad_input() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let ctx = make_ctx();

        let bad_input = serde_json::json!({ "x": "not a number" });
        let err = adapter.execute(bad_input, &ctx).await.unwrap_err();
        assert!(matches!(err, ActionError::Validation(_)));
    }

    #[tokio::test]
    async fn adapter_exposes_metadata() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        assert_eq!(adapter.metadata().key, "math.add");
    }

    #[test]
    fn adapter_is_dyn_compatible() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let _: Arc<dyn InternalHandler> = Arc::new(adapter);
    }
}
