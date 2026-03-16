//! Action runtime -- the main execution orchestrator.
//!
//! Resolves actions from the registry, executes them through the sandbox,
//! enforces data limits, and emits telemetry events.

use std::sync::Arc;
use std::time::Instant;

use crate::sandbox::SandboxRunner;
use nebula_action::ActionContext;
use nebula_action::result::ActionResult;
use nebula_metrics::naming::{
    NEBULA_ACTION_DURATION_SECONDS, NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_ACTION_FAILURES_TOTAL,
};
use nebula_telemetry::TelemetryService;
use nebula_telemetry::event::{EventBus, ExecutionEvent};
use nebula_telemetry::metrics::MetricsRegistry;

use crate::data_policy::{DataPassingPolicy, LargeDataStrategy};
use crate::error::RuntimeError;
use crate::registry::ActionRegistry;

/// The action runtime orchestrates execution of actions.
///
/// It sits between the engine (which schedules work) and the sandbox
/// (which provides isolation). The runtime:
///
/// 1. Looks up the action handler from the registry
/// 2. Resolves the isolation level
/// 3. Executes through the sandbox (if capability-gated/isolated)
///    or directly (if trusted)
/// 4. Enforces data passing policies on the output
/// 5. Emits telemetry events
pub struct ActionRuntime {
    registry: Arc<ActionRegistry>,
    #[allow(dead_code)] // reserved for sandboxed execution
    sandbox: Arc<dyn SandboxRunner>,
    data_policy: DataPassingPolicy,
    event_bus: Arc<EventBus>,
    metrics: Arc<MetricsRegistry>,
}

impl ActionRuntime {
    /// Create a new runtime with the given components.
    pub fn new(
        registry: Arc<ActionRegistry>,
        sandbox: Arc<dyn SandboxRunner>,
        data_policy: DataPassingPolicy,
        event_bus: Arc<EventBus>,
        metrics: Arc<MetricsRegistry>,
    ) -> Self {
        Self {
            registry,
            sandbox,
            data_policy,
            event_bus,
            metrics,
        }
    }

    /// Create a new runtime from a telemetry service.
    ///
    /// Uses the same event bus and metrics registry as the service for
    /// consistent observability with the engine.
    #[must_use]
    pub fn with_telemetry(
        registry: Arc<ActionRegistry>,
        sandbox: Arc<dyn SandboxRunner>,
        data_policy: DataPassingPolicy,
        telemetry: Arc<dyn TelemetryService>,
    ) -> Self {
        Self::new(
            registry,
            sandbox,
            data_policy,
            telemetry.event_bus_arc(),
            telemetry.metrics_arc(),
        )
    }

    /// Access the action registry.
    pub fn registry(&self) -> &ActionRegistry {
        &self.registry
    }

    /// Access the data passing policy.
    pub fn data_policy(&self) -> &DataPassingPolicy {
        &self.data_policy
    }

    /// Execute an action by key.
    ///
    /// # Flow
    ///
    /// 1. Look up action handler in the registry
    /// 2. Check isolation level from metadata
    /// 3. For `IsolationLevel::None` (trusted): execute directly
    /// 4. For `CapabilityGated`/`Isolated`: wrap context in
    ///    `SandboxedContext` and execute through the sandbox
    /// 5. Enforce data limits on the output
    /// 6. Emit telemetry events
    pub async fn execute_action(
        &self,
        action_key: &str,
        input: serde_json::Value,
        context: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        let handler = self.registry.get(action_key)?;
        let node_id = context.node_id.to_string();
        let execution_id = context.execution_id.to_string();

        self.event_bus.emit(ExecutionEvent::NodeStarted {
            execution_id: execution_id.clone(),
            node_id: node_id.clone(),
            trace_context: None,
        });

        let started = Instant::now();
        let action_counter = self.metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL);
        let error_counter = self.metrics.counter(NEBULA_ACTION_FAILURES_TOTAL);
        let duration_hist = self.metrics.histogram(NEBULA_ACTION_DURATION_SECONDS);

        // TODO: Restore isolation level logic once ActionMetadata has capabilities/isolation
        let result = handler.execute(input, &context).await;

        let elapsed = started.elapsed();
        duration_hist.observe(elapsed.as_secs_f64());
        action_counter.inc();

        match result {
            Ok(action_result) => {
                // Enforce data limits on the primary output value.
                self.enforce_data_limit(
                    action_key,
                    &action_result,
                    &error_counter,
                    &execution_id,
                    &node_id,
                )?;

                self.event_bus.emit(ExecutionEvent::NodeCompleted {
                    execution_id,
                    node_id,
                    duration: elapsed,
                    trace_context: None,
                });

                Ok(action_result)
            }
            Err(action_err) => {
                error_counter.inc();
                self.event_bus.emit(ExecutionEvent::NodeFailed {
                    execution_id,
                    node_id,
                    error: action_err.to_string(),
                    trace_context: None,
                });
                Err(RuntimeError::ActionError(action_err))
            }
        }
    }

    /// Check the primary output of an `ActionResult` against the data passing policy.
    ///
    /// Returns `Ok(())` if within limits or if using `SpillToBlob` strategy.
    /// Returns `Err(DataLimitExceeded)` if the output is too large and strategy is `Reject`.
    fn enforce_data_limit(
        &self,
        action_key: &str,
        action_result: &ActionResult<serde_json::Value>,
        error_counter: &nebula_telemetry::metrics::Counter,
        execution_id: &str,
        node_id: &str,
    ) -> Result<(), RuntimeError> {
        let output = match primary_output(action_result) {
            Some(o) => o,
            None => return Ok(()),
        };

        let (limit, actual) = match self.data_policy.check_output_size(output) {
            Ok(_) => return Ok(()),
            Err(exceeded) => exceeded,
        };

        error_counter.inc();
        self.event_bus.emit(ExecutionEvent::NodeFailed {
            execution_id: execution_id.to_owned(),
            node_id: node_id.to_owned(),
            error: format!("data limit exceeded: {actual} > {limit}"),
            trace_context: None,
        });

        match self.data_policy.large_data_strategy {
            LargeDataStrategy::Reject => Err(RuntimeError::DataLimitExceeded {
                limit_bytes: limit,
                actual_bytes: actual,
            }),
            LargeDataStrategy::SpillToBlob => {
                // Phase 2: spill to blob storage.
                tracing::warn!(
                    action_key,
                    actual,
                    limit,
                    "output exceeds limit, spill to blob not yet implemented"
                );
                Ok(())
            }
        }
    }
}

/// Extract the primary output value from an `ActionResult` for size checking.
fn primary_output(result: &ActionResult<serde_json::Value>) -> Option<&serde_json::Value> {
    match result {
        ActionResult::Success { output } => output.as_value(),
        ActionResult::Skip { output, .. } => output.as_ref().and_then(|o| o.as_value()),
        ActionResult::Continue { output, .. } => output.as_value(),
        ActionResult::Break { output, .. } => output.as_value(),
        ActionResult::Branch { output, .. } => output.as_value(),
        ActionResult::Route { data, .. } => data.as_value(),
        ActionResult::MultiOutput { main_output, .. } => {
            main_output.as_ref().and_then(|o| o.as_value())
        }
        ActionResult::Wait { partial_output, .. } => {
            partial_output.as_ref().and_then(|o| o.as_value())
        }
        ActionResult::Retry { .. } => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::{ActionExecutor, InProcessSandbox};
    use nebula_action::InternalHandler;
    use nebula_action::error::ActionError;
    use nebula_action::metadata::ActionMetadata;
    use nebula_action::{ActionContext, TriggerContext};
    use nebula_core::action_key;
    use nebula_core::id::{ExecutionId, NodeId, WorkflowId};

    struct EchoHandler {
        meta: ActionMetadata,
    }

    #[async_trait::async_trait]
    impl InternalHandler for EchoHandler {
        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            Ok(ActionResult::success(input))
        }
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    struct FailHandler {
        meta: ActionMetadata,
    }

    #[async_trait::async_trait]
    impl InternalHandler for FailHandler {
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            Err(ActionError::retryable("transient failure"))
        }
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    fn test_context() -> ActionContext {
        ActionContext::new(
            ExecutionId::new(),
            NodeId::new(),
            WorkflowId::new(),
            tokio_util::sync::CancellationToken::new(),
        )
    }

    fn test_trigger_context() -> TriggerContext {
        TriggerContext::new(
            WorkflowId::new(),
            NodeId::new(),
            tokio_util::sync::CancellationToken::new(),
        )
    }

    fn make_runtime(registry: Arc<ActionRegistry>) -> ActionRuntime {
        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let sandbox = Arc::new(InProcessSandbox::new(executor));
        let event_bus = Arc::new(EventBus::new(64));
        let metrics = Arc::new(MetricsRegistry::new());

        ActionRuntime::new(
            registry,
            sandbox,
            DataPassingPolicy::default(),
            event_bus,
            metrics,
        )
    }

    #[tokio::test]
    async fn execute_trusted_action() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("test.echo"), "Echo", "echoes input"),
        }));

        let rt = make_runtime(registry);
        let input = serde_json::json!({"hello": "world"});
        let result = rt
            .execute_action("test.echo", input.clone(), test_context())
            .await;
        let action_result = result.unwrap();
        match action_result {
            ActionResult::Success { output } => {
                assert_eq!(output.as_value(), Some(&input));
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_unknown_action_returns_error() {
        let registry = Arc::new(ActionRegistry::new());
        let rt = make_runtime(registry);
        let result = rt
            .execute_action("nonexistent", serde_json::json!(null), test_context())
            .await;
        assert!(matches!(result, Err(RuntimeError::ActionNotFound { .. })));
    }

    #[tokio::test]
    async fn execute_failing_action_propagates_error() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(FailHandler {
            meta: ActionMetadata::new(action_key!("test.fail"), "Fail", "always fails"),
        }));

        let rt = make_runtime(registry);
        let result = rt
            .execute_action("test.fail", serde_json::json!(null), test_context())
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().is_retryable());
    }

    #[tokio::test]
    async fn data_limit_enforcement() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("test.big"), "Big", "returns big output"),
        }));

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let sandbox = Arc::new(InProcessSandbox::new(executor));
        let event_bus = Arc::new(EventBus::new(64));
        let metrics = Arc::new(MetricsRegistry::new());

        let rt = ActionRuntime::new(
            registry,
            sandbox,
            DataPassingPolicy {
                max_node_output_bytes: 5, // very small
                ..Default::default()
            },
            event_bus,
            metrics,
        );

        let input = serde_json::json!({"big_payload": "this is way too large for 5 bytes"});
        let result = rt.execute_action("test.big", input, test_context()).await;
        assert!(matches!(
            result,
            Err(RuntimeError::DataLimitExceeded { .. })
        ));
    }

    #[tokio::test]
    async fn telemetry_events_emitted() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("test.tele"), "Tele", "test"),
        }));

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let sandbox = Arc::new(InProcessSandbox::new(executor));
        let event_bus = Arc::new(EventBus::new(64));
        let metrics = Arc::new(MetricsRegistry::new());
        let mut sub = event_bus.subscribe();

        let rt = ActionRuntime::new(
            registry,
            sandbox,
            DataPassingPolicy::default(),
            event_bus,
            metrics.clone(),
        );

        rt.execute_action("test.tele", serde_json::json!("ok"), test_context())
            .await
            .unwrap();

        // Should have emitted NodeStarted and NodeCompleted.
        let event1 = sub.try_recv().expect("should get NodeStarted");
        assert!(matches!(event1, ExecutionEvent::NodeStarted { .. }));

        let event2 = sub.try_recv().expect("should get NodeCompleted");
        assert!(matches!(event2, ExecutionEvent::NodeCompleted { .. }));

        // Metrics should be recorded.
        assert_eq!(metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL).get(), 1);
        assert_eq!(metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).get(), 0);
    }

    #[tokio::test]
    async fn trigger_context_construction_is_usable_in_runtime() {
        let ctx = test_trigger_context();
        assert!(!ctx.has_credential("missing").await);
        assert!(
            ctx.schedule_after(std::time::Duration::from_millis(1))
                .await
                .is_err()
        );
        assert!(
            ctx.emit_execution(serde_json::json!({"tick": true}))
                .await
                .is_err()
        );
    }
}
