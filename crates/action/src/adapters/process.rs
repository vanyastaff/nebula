//! Adapter for [`ProcessAction`] to [`InternalHandler`].

use std::sync::Arc;

use async_trait::async_trait;
use nebula_parameter::collection::ParameterCollection;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::action::Action;
use crate::context::ActionContext;
use crate::error::ActionError;
use crate::handler::InternalHandler;
use crate::metadata::{ActionMetadata, ActionType};
use crate::result::ActionResult;
use crate::types::ProcessAction;

/// Adapter that wraps a typed [`ProcessAction`] as an [`InternalHandler`].
///
/// Handles JSON-to-typed conversion:
/// 1. Deserializes `serde_json::Value` into `A::Input`
/// 2. Calls `validate_input()` then `execute()`
/// 3. Serializes `ActionResult<A::Output>` into `ActionResult<serde_json::Value>` via `map_output()`
pub struct ProcessActionAdapter<A> {
    action: Arc<A>,
}

impl<A> ProcessActionAdapter<A> {
    /// Wrap a process action in an adapter.
    pub fn new(action: A) -> Self {
        Self {
            action: Arc::new(action),
        }
    }
}

#[async_trait]
impl<A> InternalHandler for ProcessActionAdapter<A>
where
    A: ProcessAction + Send + Sync + 'static,
    A::Input: DeserializeOwned + Send + Sync + 'static,
    A::Output: Serialize + Send + Sync + 'static,
{
    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        // 1. Deserialize JSON into typed input
        let typed_input: A::Input = serde_json::from_value(input)
            .map_err(|e| ActionError::validation(format!("input deserialization failed: {e}")))?;

        // 2. Validate input
        self.action.validate_input(&typed_input).await?;

        // 3. Execute
        let result = self.action.execute(typed_input, &ctx).await?;

        // 4. Serialize typed output into JSON via try_map_output
        result.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))
        })
    }

    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    fn action_type(&self) -> ActionType {
        self.action.action_type()
    }

    fn parameters(&self) -> Option<&ParameterCollection> {
        self.action.parameters()
    }
}

impl<A: Action> std::fmt::Debug for ProcessActionAdapter<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessActionAdapter")
            .field("action_key", &self.action.metadata().key)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::ActionMetadata;

    #[derive(Debug)]
    struct DoubleAction {
        meta: ActionMetadata,
    }

    impl Action for DoubleAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        fn action_type(&self) -> ActionType {
            ActionType::Process
        }
    }

    #[async_trait]
    impl ProcessAction for DoubleAction {
        type Input = i64;
        type Output = i64;

        async fn execute(
            &self,
            input: Self::Input,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::success(input * 2))
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

    fn double_action() -> DoubleAction {
        DoubleAction {
            meta: ActionMetadata::new("test.double", "Double", "Doubles input"),
        }
    }

    #[tokio::test]
    async fn adapter_executes_typed_action() {
        let adapter = ProcessActionAdapter::new(double_action());
        let result = adapter
            .execute(serde_json::json!(21), test_ctx())
            .await
            .unwrap();
        match result {
            ActionResult::Success { output } => assert_eq!(output, serde_json::json!(42)),
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn adapter_returns_validation_error_on_bad_input() {
        let adapter = ProcessActionAdapter::new(double_action());
        let err = adapter
            .execute(serde_json::json!("not a number"), test_ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::Validation(_)));
    }

    #[tokio::test]
    async fn adapter_delegates_metadata() {
        let adapter = ProcessActionAdapter::new(double_action());
        assert_eq!(adapter.metadata().key, "test.double");
        assert_eq!(adapter.action_type(), ActionType::Process);
        assert!(adapter.parameters().is_none());
    }

    #[tokio::test]
    async fn adapter_debug_shows_action_key() {
        let adapter = ProcessActionAdapter::new(double_action());
        let debug = format!("{adapter:?}");
        assert!(debug.contains("test.double"));
    }
}
