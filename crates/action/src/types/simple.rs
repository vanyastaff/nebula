//! Simplified action trait for the common case.
//!
//! [`SimpleAction`] reduces boilerplate for actions that take input,
//! produce output, and don't need flow-control (branching, waiting, etc.).
//! A blanket impl automatically adapts `SimpleAction` into `ProcessAction`,
//! so simple actions work with the entire execution pipeline unchanged.

use async_trait::async_trait;

use crate::action::Action;
use crate::context::ActionContext;
use crate::error::ActionError;
use crate::result::ActionResult;
use crate::types::ProcessAction;

/// Simplified action trait — the 80% case.
///
/// Action authors implement [`run`](SimpleAction::run) returning
/// `Result<Output, ActionError>`. The framework wraps the output in
/// `ActionResult::success(output)` automatically.
///
/// For flow-control (branching, waiting, retrying) implement
/// [`ProcessAction`] directly instead.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::prelude::*;
/// use nebula_action::SimpleAction;
/// use async_trait::async_trait;
///
/// struct DoubleAction { meta: ActionMetadata }
///
/// impl Action for DoubleAction {
///     fn metadata(&self) -> &ActionMetadata { &self.meta }
/// }
///
/// #[async_trait]
/// impl SimpleAction for DoubleAction {
///     type Input = i64;
///     type Output = i64;
///
///     async fn run(&self, input: i64, _ctx: &ActionContext) -> Result<i64, ActionError> {
///         Ok(input * 2)
///     }
/// }
/// ```
#[async_trait]
pub trait SimpleAction: Action {
    /// Input data type received from upstream nodes.
    type Input: Send + Sync + 'static;
    /// Output data type passed to downstream nodes.
    type Output: Send + Sync + 'static;

    /// Execute the action, returning the output directly.
    ///
    /// The blanket `ProcessAction` impl wraps the return value in
    /// `ActionResult::success(output)`.
    async fn run(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<Self::Output, ActionError>;
}

/// Blanket impl: every `SimpleAction` is automatically a `ProcessAction`.
#[async_trait]
impl<T> ProcessAction for T
where
    T: SimpleAction + Send + Sync + 'static,
    T::Input: Send + Sync + 'static,
    T::Output: Send + Sync + 'static,
{
    type Input = T::Input;
    type Output = T::Output;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let output = self.run(input, ctx).await?;
        Ok(ActionResult::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::ProcessActionAdapter;
    use crate::handler::InternalHandler;
    use crate::metadata::{ActionMetadata, ActionType};

    #[derive(Debug)]
    struct AddOneAction {
        meta: ActionMetadata,
    }

    impl Action for AddOneAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    #[async_trait]
    impl SimpleAction for AddOneAction {
        type Input = i64;
        type Output = i64;

        async fn run(&self, input: i64, _ctx: &ActionContext) -> Result<i64, ActionError> {
            Ok(input + 1)
        }
    }

    fn test_ctx() -> ActionContext {
        use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
        use nebula_core::scope::ScopeLevel;
        ActionContext::new(
            ExecutionId::v4(),
            NodeId::v4(),
            WorkflowId::v4(),
            ScopeLevel::Global,
        )
    }

    #[tokio::test]
    async fn simple_action_as_process_action() {
        let action = AddOneAction {
            meta: ActionMetadata::new("test.add_one", "Add One", "Adds 1"),
        };
        // SimpleAction satisfies ProcessAction via blanket impl.
        let result = ProcessAction::execute(&action, 41, &test_ctx())
            .await
            .unwrap();
        assert!(result.is_success());
        assert_eq!(result.into_primary_value(), Some(42));
    }

    #[tokio::test]
    async fn simple_action_through_adapter() {
        let action = AddOneAction {
            meta: ActionMetadata::new("test.add_one", "Add One", "Adds 1"),
        };
        let adapter = ProcessActionAdapter::new(action);
        let result = adapter
            .execute(serde_json::json!(99), test_ctx())
            .await
            .unwrap();
        match result {
            ActionResult::Success { output } => {
                assert_eq!(output.as_value(), Some(&serde_json::json!(100)));
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn simple_action_default_action_type() {
        let action = AddOneAction {
            meta: ActionMetadata::new("test.add_one", "Add One", "Adds 1"),
        };
        assert_eq!(action.action_type(), ActionType::Process);
    }

    #[tokio::test]
    async fn simple_action_error_propagation() {
        #[derive(Debug)]
        struct FailAction {
            meta: ActionMetadata,
        }
        impl Action for FailAction {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
        }
        #[async_trait]
        impl SimpleAction for FailAction {
            type Input = i64;
            type Output = i64;
            async fn run(&self, _input: i64, _ctx: &ActionContext) -> Result<i64, ActionError> {
                Err(ActionError::fatal("intentional failure"))
            }
        }

        let action = FailAction {
            meta: ActionMetadata::new("test.fail", "Fail", "Always fails"),
        };
        let result = ProcessAction::execute(&action, 0, &test_ctx()).await;
        assert!(result.is_err());
    }
}
