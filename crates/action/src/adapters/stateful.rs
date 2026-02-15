//! Adapter for [`StatefulAction`] to [`InternalHandler`].

use std::sync::Arc;

use async_trait::async_trait;
use nebula_parameter::collection::ParameterCollection;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::action::Action;
use crate::context::ActionContext;
use crate::error::ActionError;
use crate::handler::InternalHandler;
use crate::metadata::ActionMetadata;
use crate::output::ActionOutput;
use crate::result::{ActionResult, BreakReason};
use crate::types::StatefulAction;

/// Default maximum iterations to prevent infinite Continue loops.
const DEFAULT_MAX_ITERATIONS: usize = 10_000;

/// Adapter that wraps a typed [`StatefulAction`] as an [`InternalHandler`].
///
/// Embeds the Continue/Break loop internally — the engine never sees
/// `ActionResult::Continue`. Each iteration calls `execute_with_state`,
/// and the adapter loops until a non-Continue result is produced or the
/// max iteration guard triggers.
pub struct StatefulActionAdapter<A> {
    action: Arc<A>,
    max_iterations: usize,
}

impl<A> StatefulActionAdapter<A> {
    /// Wrap a stateful action in an adapter with the default max iterations (10,000).
    pub fn new(action: A) -> Self {
        Self {
            action: Arc::new(action),
            max_iterations: DEFAULT_MAX_ITERATIONS,
        }
    }

    /// Wrap a stateful action with a custom max iterations guard.
    pub fn with_max_iterations(action: A, max_iterations: usize) -> Self {
        Self {
            action: Arc::new(action),
            max_iterations,
        }
    }
}

#[async_trait]
impl<A> InternalHandler for StatefulActionAdapter<A>
where
    A: StatefulAction + Send + Sync + 'static,
    A::Input: DeserializeOwned + Clone + Send + Sync + 'static,
    A::Output: Serialize + Send + Sync + 'static,
    A::State: Send + Sync + 'static,
{
    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        // 1. Deserialize JSON into typed input
        let typed_input: A::Input = serde_json::from_value(input)
            .map_err(|e| ActionError::validation(format!("input deserialization failed: {e}")))?;

        // 2. Initialize state
        let mut state = self.action.initialize_state(&typed_input, &ctx).await?;

        // 3. Continue/Break loop
        for _ in 0..self.max_iterations {
            ctx.check_cancelled()?;

            let result = self
                .action
                .execute_with_state(typed_input.clone(), &mut state, &ctx)
                .await?;

            match result {
                ActionResult::Continue { delay, .. } => {
                    if let Some(delay) = delay {
                        tokio::time::sleep(delay).await;
                    }
                    // Loop continues
                }
                other => {
                    // Any non-Continue result: serialize and return
                    return other.try_map_output(|output| {
                        serde_json::to_value(output).map_err(|e| {
                            ActionError::fatal(format!("output serialization failed: {e}"))
                        })
                    });
                }
            }
        }

        // Max iterations exceeded
        Ok(ActionResult::Break {
            output: ActionOutput::Empty,
            reason: BreakReason::MaxIterations,
        })
    }

    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    fn action_type(&self) -> crate::metadata::ActionType {
        self.action.action_type()
    }

    fn parameters(&self) -> Option<&ParameterCollection> {
        self.action.parameters()
    }
}

impl<A: Action> std::fmt::Debug for StatefulActionAdapter<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatefulActionAdapter")
            .field("action_key", &self.action.metadata().key)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{ActionMetadata, ActionType};

    use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
    use nebula_core::scope::ScopeLevel;
    use serde::{Deserialize, Serialize};
    use tokio_util::sync::CancellationToken;

    fn test_ctx() -> ActionContext {
        ActionContext::new(
            ExecutionId::v4(),
            NodeId::v4(),
            WorkflowId::v4(),
            ScopeLevel::Global,
        )
    }

    // ── Counter action: counts up via Continue, then Break ──

    #[derive(Debug, Serialize, Deserialize)]
    struct CounterState {
        count: u32,
    }

    #[derive(Debug)]
    struct CounterAction {
        meta: ActionMetadata,
        target: u32,
    }

    impl CounterAction {
        fn new(target: u32) -> Self {
            Self {
                meta: ActionMetadata::new("test.counter", "Counter", "Count to target"),
                target,
            }
        }
    }

    impl Action for CounterAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
        fn action_type(&self) -> ActionType {
            ActionType::Stateful
        }
    }

    #[async_trait]
    impl StatefulAction for CounterAction {
        type State = CounterState;
        type Input = u32;
        type Output = u32;

        async fn initialize_state(
            &self,
            _input: &Self::Input,
            _ctx: &ActionContext,
        ) -> Result<Self::State, ActionError> {
            Ok(CounterState { count: 0 })
        }

        async fn execute_with_state(
            &self,
            _input: Self::Input,
            state: &mut Self::State,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            state.count += 1;
            if state.count >= self.target {
                Ok(ActionResult::Break {
                    output: ActionOutput::Value(state.count),
                    reason: BreakReason::Completed,
                })
            } else {
                Ok(ActionResult::Continue {
                    output: ActionOutput::Value(state.count),
                    progress: None,
                    delay: None,
                })
            }
        }
    }

    // ── Immediate action: returns Success on first iteration ──

    #[derive(Debug)]
    struct ImmediateAction {
        meta: ActionMetadata,
    }

    impl ImmediateAction {
        fn new() -> Self {
            Self {
                meta: ActionMetadata::new("test.immediate", "Immediate", "Immediate success"),
            }
        }
    }

    impl Action for ImmediateAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    #[async_trait]
    impl StatefulAction for ImmediateAction {
        type State = CounterState;
        type Input = String;
        type Output = String;

        async fn initialize_state(
            &self,
            _input: &Self::Input,
            _ctx: &ActionContext,
        ) -> Result<Self::State, ActionError> {
            Ok(CounterState { count: 0 })
        }

        async fn execute_with_state(
            &self,
            input: Self::Input,
            _state: &mut Self::State,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    // ── Failing init action ──

    #[derive(Debug)]
    struct FailInitAction {
        meta: ActionMetadata,
    }

    impl Action for FailInitAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    #[async_trait]
    impl StatefulAction for FailInitAction {
        type State = CounterState;
        type Input = u32;
        type Output = u32;

        async fn initialize_state(
            &self,
            _input: &Self::Input,
            _ctx: &ActionContext,
        ) -> Result<Self::State, ActionError> {
            Err(ActionError::fatal("init failed"))
        }

        async fn execute_with_state(
            &self,
            _input: Self::Input,
            _state: &mut Self::State,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            unreachable!()
        }
    }

    // ── Delay action ──

    #[derive(Debug)]
    struct DelayAction {
        meta: ActionMetadata,
    }

    impl Action for DelayAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    #[async_trait]
    impl StatefulAction for DelayAction {
        type State = CounterState;
        type Input = u32;
        type Output = u32;

        async fn initialize_state(
            &self,
            _input: &Self::Input,
            _ctx: &ActionContext,
        ) -> Result<Self::State, ActionError> {
            Ok(CounterState { count: 0 })
        }

        async fn execute_with_state(
            &self,
            _input: Self::Input,
            state: &mut Self::State,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            state.count += 1;
            if state.count >= 2 {
                Ok(ActionResult::Break {
                    output: ActionOutput::Value(state.count),
                    reason: BreakReason::Completed,
                })
            } else {
                Ok(ActionResult::Continue {
                    output: ActionOutput::Value(state.count),
                    progress: None,
                    delay: Some(std::time::Duration::from_secs(5)),
                })
            }
        }
    }

    // ── Tests ──

    #[tokio::test]
    async fn counter_action_counts_to_target() {
        let adapter = StatefulActionAdapter::new(CounterAction::new(3));
        let result = adapter
            .execute(serde_json::json!(0), test_ctx())
            .await
            .unwrap();
        match result {
            ActionResult::Break { output, reason } => {
                assert_eq!(output.as_value(), Some(&serde_json::json!(3)));
                assert_eq!(reason, BreakReason::Completed);
            }
            other => panic!("expected Break, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn immediate_success_passthrough() {
        let adapter = StatefulActionAdapter::new(ImmediateAction::new());
        let result = adapter
            .execute(serde_json::json!("hello"), test_ctx())
            .await
            .unwrap();
        match result {
            ActionResult::Success { output } => {
                assert_eq!(output.as_value(), Some(&serde_json::json!("hello")));
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn max_iterations_guard() {
        let adapter = StatefulActionAdapter::with_max_iterations(
            CounterAction::new(u32::MAX), // never reaches target
            5,
        );
        let result = adapter
            .execute(serde_json::json!(0), test_ctx())
            .await
            .unwrap();
        match result {
            ActionResult::Break { output, reason } => {
                assert!(output.is_empty());
                assert_eq!(reason, BreakReason::MaxIterations);
            }
            other => panic!("expected Break with MaxIterations, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancellation_between_iterations() {
        let token = CancellationToken::new();
        let ctx = test_ctx().with_cancellation(token.clone());

        // Action that never finishes
        let adapter = StatefulActionAdapter::new(CounterAction::new(u32::MAX));

        // Cancel immediately — the loop checks cancellation at the top of each iteration
        token.cancel();

        let err = adapter
            .execute(serde_json::json!(0), ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::Cancelled));
    }

    #[tokio::test(start_paused = true)]
    async fn delay_honored() {
        let adapter = StatefulActionAdapter::new(DelayAction {
            meta: ActionMetadata::new("test.delay", "Delay", "Test delay"),
        });

        let start = tokio::time::Instant::now();
        let result = adapter
            .execute(serde_json::json!(0), test_ctx())
            .await
            .unwrap();

        // The action requests a 5s delay before iteration 2
        assert!(start.elapsed() >= std::time::Duration::from_secs(5));
        match result {
            ActionResult::Break { output, .. } => {
                assert_eq!(output.as_value(), Some(&serde_json::json!(2)));
            }
            other => panic!("expected Break, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn input_deserialization_failure() {
        let adapter = StatefulActionAdapter::new(CounterAction::new(3));
        let err = adapter
            .execute(serde_json::json!("not a number"), test_ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::Validation(_)));
    }

    #[tokio::test]
    async fn state_initialization_failure() {
        let adapter = StatefulActionAdapter::new(FailInitAction {
            meta: ActionMetadata::new("test.fail_init", "FailInit", "Fails on init"),
        });
        let err = adapter
            .execute(serde_json::json!(0), test_ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::Fatal { .. }));
    }

    #[tokio::test]
    async fn output_serialization_via_try_map() {
        let adapter = StatefulActionAdapter::new(CounterAction::new(1));
        let result = adapter
            .execute(serde_json::json!(0), test_ctx())
            .await
            .unwrap();
        // Verify the output is valid JSON (serialization worked)
        match result {
            ActionResult::Break { output, .. } => {
                assert!(output.as_value().is_some());
            }
            other => panic!("expected Break, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn metadata_delegation() {
        let adapter = StatefulActionAdapter::new(CounterAction::new(1));
        assert_eq!(adapter.metadata().key, "test.counter");
        assert_eq!(adapter.action_type(), ActionType::Stateful);
        assert!(adapter.parameters().is_none());
    }

    #[tokio::test]
    async fn debug_display() {
        let adapter = StatefulActionAdapter::new(CounterAction::new(1));
        let debug = format!("{adapter:?}");
        assert!(debug.contains("test.counter"));
    }
}
