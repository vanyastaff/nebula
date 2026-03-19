//! Ergonomic authoring helpers for common action patterns.
//!
//! This module provides low-boilerplate adapters for action authors.

use std::future::Future;
use std::marker::PhantomData;

use crate::dependency::ActionDependencies;
use crate::{Action, ActionError, ActionMetadata, ActionResult, Context, StatelessAction};

/// Stateless action adapter backed by an async function/closure.
///
/// This removes boilerplate for the common "pure stateless transform" case.
pub struct FnStatelessAction<F, Input, Output> {
    metadata: ActionMetadata,
    func: F,
    _marker: PhantomData<fn(Input) -> Output>,
}

impl<F, Input, Output> FnStatelessAction<F, Input, Output> {
    /// Create a new function-backed stateless action.
    #[must_use]
    pub fn new(metadata: ActionMetadata, func: F) -> Self {
        Self {
            metadata,
            func,
            _marker: PhantomData,
        }
    }
}

impl<F, Input, Output> ActionDependencies for FnStatelessAction<F, Input, Output>
where
    F: Send + Sync + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
}

impl<F, Input, Output> Action for FnStatelessAction<F, Input, Output>
where
    F: Send + Sync + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl<F, Fut, Input, Output> StatelessAction for FnStatelessAction<F, Input, Output>
where
    F: Fn(Input) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Output, ActionError>> + Send + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
    type Input = Input;
    type Output = Output;

    fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send {
        let fut = (self.func)(input);
        async move { fut.await.map(ActionResult::success) }
    }
}

/// Build a function-backed stateless action.
#[must_use]
pub fn stateless_fn<F, Input, Output>(
    metadata: ActionMetadata,
    func: F,
) -> FnStatelessAction<F, Input, Output> {
    FnStatelessAction::new(metadata, func)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ActionContext;
    use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn fn_stateless_action_executes_with_low_boilerplate() {
        let action = stateless_fn::<_, serde_json::Value, serde_json::Value>(
            ActionMetadata::new(
                nebula_core::action_key!("example.fn"),
                "Fn",
                "Function-backed action",
            ),
            |input| async move { Ok(input) },
        );

        let ctx = ActionContext::new(
            ExecutionId::new(),
            NodeId::new(),
            WorkflowId::new(),
            CancellationToken::new(),
        );

        let result = action
            .execute(serde_json::json!({"hello":"world"}), &ctx)
            .await
            .unwrap();
        match result {
            ActionResult::Success { output } => {
                assert_eq!(
                    output.as_value(),
                    Some(&serde_json::json!({"hello":"world"}))
                );
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }
}
