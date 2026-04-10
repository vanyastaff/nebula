//! Action runtime -- the main execution orchestrator.
//!
//! Resolves actions from the registry, executes them through the sandbox,
//! enforces data limits, and records metrics.

use std::sync::Arc;
use std::time::Instant;

use crate::blob::BlobStorage;
use crate::sandbox::SandboxRunner;
use nebula_action::output::{ActionOutput, DataReference};
use nebula_action::result::ActionResult;
use nebula_action::{
    ActionContext, ActionError, ActionHandler, ActionMetadata, IsolationLevel, StatefulHandler,
    StatelessHandler,
};
use nebula_metrics::naming::{
    NEBULA_ACTION_DURATION_SECONDS, NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_ACTION_FAILURES_TOTAL,
};
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
    // Sandbox dispatch for isolated execution is deferred to Phase 7.6.
    // Currently stateless actions run directly through StatelessHandler::execute
    // regardless of isolation level.
    #[allow(dead_code)]
    sandbox: Arc<dyn SandboxRunner>,
    data_policy: DataPassingPolicy,
    metrics: MetricsRegistry,
    blob_storage: Option<Arc<dyn BlobStorage>>,
}

impl ActionRuntime {
    /// Create a new runtime with the given components.
    pub fn new(
        registry: Arc<ActionRegistry>,
        sandbox: Arc<dyn SandboxRunner>,
        data_policy: DataPassingPolicy,
        metrics: MetricsRegistry,
    ) -> Self {
        Self {
            registry,
            sandbox,
            data_policy,
            metrics,
            blob_storage: None,
        }
    }

    /// Access the action registry.
    pub fn registry(&self) -> &ActionRegistry {
        &self.registry
    }

    /// Set blob storage for the `SpillToBlob` strategy.
    ///
    /// Without blob storage, `SpillToBlob` falls back to rejecting
    /// oversized output.
    #[must_use]
    pub fn with_blob_storage(mut self, storage: Arc<dyn BlobStorage>) -> Self {
        self.blob_storage = Some(storage);
        self
    }

    /// Access the data passing policy.
    pub fn data_policy(&self) -> &DataPassingPolicy {
        &self.data_policy
    }

    /// Execute an action by key, optionally pinned to a specific interface version.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::ActionNotFound`] if no handler is registered for the
    /// key (and version, if supplied).
    pub async fn execute_action_versioned(
        &self,
        action_key: &str,
        version: Option<&nebula_action::InterfaceVersion>,
        input: serde_json::Value,
        context: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        let key = nebula_core::ActionKey::new(action_key).map_err(|e| {
            RuntimeError::InvalidActionKey {
                key: action_key.to_owned(),
                reason: e.to_string(),
            }
        })?;

        let (metadata, handler) = match version {
            Some(v) => self.registry.get_versioned(&key, v),
            None => self.registry.get(&key),
        }
        .ok_or_else(|| RuntimeError::ActionNotFound {
            key: action_key.to_owned(),
        })?;

        self.run_handler(action_key, metadata, handler, input, context)
            .await
    }

    /// Execute an action by key.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::ActionNotFound`] if the key does not resolve to a
    /// registered action, [`RuntimeError::TriggerNotExecutable`] /
    /// [`RuntimeError::ResourceNotExecutable`] /
    /// [`RuntimeError::AgentNotSupportedYet`] if the key resolves to a handler
    /// kind that is not executable through this runtime, or
    /// [`RuntimeError::ActionError`] / [`RuntimeError::DataLimitExceeded`]
    /// if execution fails.
    pub async fn execute_action(
        &self,
        action_key: &str,
        input: serde_json::Value,
        context: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        // Parse the key explicitly so we can distinguish "invalid format" from
        // "valid format but not registered". `get_by_str` collapses both into None.
        let key = nebula_core::ActionKey::new(action_key).map_err(|e| {
            RuntimeError::InvalidActionKey {
                key: action_key.to_owned(),
                reason: e.to_string(),
            }
        })?;

        let (metadata, handler) =
            self.registry
                .get(&key)
                .ok_or_else(|| RuntimeError::ActionNotFound {
                    key: action_key.to_owned(),
                })?;

        self.run_handler(action_key, metadata, handler, input, context)
            .await
    }

    /// Dispatch a resolved handler through its kind-specific execution path.
    async fn run_handler(
        &self,
        action_key: &str,
        metadata: ActionMetadata,
        handler: ActionHandler,
        input: serde_json::Value,
        context: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        let started = Instant::now();
        let action_counter = self.metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL);
        let error_counter = self.metrics.counter(NEBULA_ACTION_FAILURES_TOTAL);
        let duration_hist = self.metrics.histogram(NEBULA_ACTION_DURATION_SECONDS);

        let result = match handler {
            ActionHandler::Stateless(h) => {
                self.execute_stateless(&metadata, h, input, context).await
            }
            ActionHandler::Stateful(h) => self.execute_stateful(&metadata, h, input, context).await,
            ActionHandler::Trigger(_) => {
                // Not executable through ActionRuntime — count as a failed
                // execution attempt and record timing for observability.
                action_counter.inc();
                error_counter.inc();
                duration_hist.observe(started.elapsed().as_secs_f64());
                return Err(RuntimeError::TriggerNotExecutable {
                    key: action_key.to_owned(),
                });
            }
            ActionHandler::Resource(_) => {
                action_counter.inc();
                error_counter.inc();
                duration_hist.observe(started.elapsed().as_secs_f64());
                return Err(RuntimeError::ResourceNotExecutable {
                    key: action_key.to_owned(),
                });
            }
            ActionHandler::Agent(_) => {
                action_counter.inc();
                error_counter.inc();
                duration_hist.observe(started.elapsed().as_secs_f64());
                return Err(RuntimeError::AgentNotSupportedYet {
                    key: action_key.to_owned(),
                });
            }
            // `ActionHandler` is `#[non_exhaustive]`. Unknown future variants
            // are surfaced as an internal runtime error rather than silently
            // succeeding.
            _ => {
                action_counter.inc();
                error_counter.inc();
                duration_hist.observe(started.elapsed().as_secs_f64());
                return Err(RuntimeError::Internal(format!(
                    "unknown ActionHandler variant for action '{action_key}'"
                )));
            }
        };

        let elapsed = started.elapsed();
        duration_hist.observe(elapsed.as_secs_f64());
        action_counter.inc();

        match result {
            Ok(mut action_result) => {
                self.enforce_data_limit(action_key, &mut action_result, &error_counter)
                    .await?;
                Ok(action_result)
            }
            Err(action_err) => {
                error_counter.inc();
                Err(RuntimeError::ActionError(action_err))
            }
        }
    }

    /// Execute a stateless handler.
    ///
    /// Execute a stateless handler.
    ///
    /// Only `IsolationLevel::None` is supported in Phase 7.5. Non-`None`
    /// isolation levels return `ActionError::Fatal` to prevent silent
    /// bypassing of capability checks. Sandbox dispatch through
    /// [`SandboxRunner`] is Phase 7.6 work.
    async fn execute_stateless(
        &self,
        metadata: &ActionMetadata,
        handler: Arc<dyn StatelessHandler>,
        input: serde_json::Value,
        context: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        if !matches!(metadata.isolation_level, IsolationLevel::None) {
            return Err(ActionError::fatal(
                "sandboxed stateless execution is not yet supported (Phase 7.6) — \
                 refusing to silently bypass isolation/capability checks",
            ));
        }
        handler.execute(input, &context).await
    }

    /// Execute a stateful handler — loops through [`StatefulHandler::execute`]
    /// with in-memory state checkpointing.
    ///
    /// State persistence is post-MVP — state only lives on the stack of this
    /// call.
    async fn execute_stateful(
        &self,
        metadata: &ActionMetadata,
        handler: Arc<dyn StatefulHandler>,
        input: serde_json::Value,
        context: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        if !matches!(metadata.isolation_level, IsolationLevel::None) {
            return Err(ActionError::fatal(
                "sandboxed stateful execution is not yet supported (Phase 7.6)",
            ));
        }

        // Cancellation check BEFORE init_state — avoid the JSON round-trip if
        // the caller already cancelled.
        if context.cancellation.is_cancelled() {
            return Err(ActionError::Cancelled);
        }

        let mut state = handler.init_state()?;

        // Hard cap to prevent runaway loops.
        const MAX_ITERATIONS: u32 = 10_000;

        for _iteration in 0..MAX_ITERATIONS {
            // Cooperative cancellation check BEFORE the next iteration.
            if context.cancellation.is_cancelled() {
                return Err(ActionError::Cancelled);
            }

            let result = handler.execute(&input, &mut state, &context).await?;

            match result {
                ActionResult::Continue { delay, .. } => {
                    if let Some(d) = delay {
                        // Cancel-aware sleep — abort the delay if cancelled mid-wait.
                        tokio::select! {
                            () = tokio::time::sleep(d) => {}
                            () = context.cancellation.cancelled() => {
                                return Err(ActionError::Cancelled);
                            }
                        }
                    }
                    // Loop continues with mutated state.
                }
                other => return Ok(other),
            }
        }

        Err(ActionError::fatal(format!(
            "stateful action '{}' exceeded max iterations ({MAX_ITERATIONS})",
            metadata.key.as_str()
        )))
    }

    /// Check the primary output of an `ActionResult` against the data passing policy.
    ///
    /// Returns `Ok(())` if within limits.
    /// For `SpillToBlob`, writes the output to blob storage if configured and
    /// **rewrites** the primary output field to an [`ActionOutput::Reference`] so the
    /// large inline payload is no longer carried downstream.
    /// Returns `Err(DataLimitExceeded)` if the output is too large and cannot
    /// be spilled.
    async fn enforce_data_limit(
        &self,
        action_key: &str,
        action_result: &mut ActionResult<serde_json::Value>,
        error_counter: &nebula_telemetry::metrics::Counter,
    ) -> Result<(), RuntimeError> {
        let output = match primary_output_mut(action_result) {
            Some(o) => o,
            None => return Ok(()),
        };

        // Only inline values can exceed the size limit; binary/reference outputs
        // are already managed by their respective storage backends.
        let serialized = match &*output {
            ActionOutput::Value(v) => serde_json::to_vec(v).map_err(|e| {
                RuntimeError::Internal(format!(
                    "failed to serialize output for size limit enforcement: {e}"
                ))
            })?,
            _ => return Ok(()),
        };

        let actual = serialized.len() as u64;
        let limit = self.data_policy.max_node_output_bytes;

        if actual <= limit {
            return Ok(());
        }

        // Output is oversized — apply the configured strategy.
        match self.data_policy.large_data_strategy {
            LargeDataStrategy::Reject => {
                error_counter.inc();
                Err(RuntimeError::DataLimitExceeded {
                    limit_bytes: limit,
                    actual_bytes: actual,
                })
            }
            LargeDataStrategy::SpillToBlob => match &self.blob_storage {
                Some(storage) => {
                    let blob_ref = storage
                        .write(&serialized, "application/json")
                        .await
                        .map_err(|e| {
                            tracing::warn!(
                                action_key,
                                error = %e,
                                "blob spill failed, rejecting output"
                            );
                            error_counter.inc();
                            RuntimeError::DataLimitExceeded {
                                limit_bytes: limit,
                                actual_bytes: actual,
                            }
                        })?;
                    tracing::info!(
                        action_key,
                        uri = %blob_ref.uri,
                        size = blob_ref.size_bytes,
                        "output spilled to blob storage"
                    );
                    // Replace the large inline value with a reference so downstream
                    // nodes receive a small handle instead of the full payload.
                    *output = ActionOutput::Reference(DataReference {
                        storage_type: "blob".into(),
                        path: blob_ref.uri,
                        size: Some(blob_ref.size_bytes),
                        content_type: Some(blob_ref.content_type),
                    });
                    Ok(())
                }
                None => {
                    tracing::warn!(
                        action_key,
                        actual,
                        limit,
                        "output exceeds limit and no blob storage configured"
                    );
                    error_counter.inc();
                    Err(RuntimeError::DataLimitExceeded {
                        limit_bytes: limit,
                        actual_bytes: actual,
                    })
                }
            },
        }
    }
}

/// Extract a mutable reference to the primary output field of an `ActionResult`.
///
/// Returns `None` for variants that carry no primary output (e.g. `Retry`).
fn primary_output_mut(
    result: &mut ActionResult<serde_json::Value>,
) -> Option<&mut ActionOutput<serde_json::Value>> {
    match result {
        ActionResult::Success { output } => Some(output),
        ActionResult::Skip { output, .. } => output.as_mut(),
        ActionResult::Continue { output, .. } => Some(output),
        ActionResult::Break { output, .. } => Some(output),
        ActionResult::Branch { output, .. } => Some(output),
        ActionResult::Route { data, .. } => Some(data),
        ActionResult::MultiOutput { main_output, .. } => main_output.as_mut(),
        ActionResult::Wait { partial_output, .. } => partial_output.as_mut(),
        ActionResult::Retry { .. } => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::{ActionExecutor, InProcessSandbox};
    use nebula_action::action::Action;
    use nebula_action::context::Context;
    use nebula_action::dependency::ActionDependencies;
    use nebula_action::error::ActionError;
    use nebula_action::metadata::ActionMetadata;
    use nebula_action::stateless::StatelessAction;
    use nebula_action::{ActionContext, TriggerContext};
    use nebula_core::action_key;
    use nebula_core::id::{ExecutionId, NodeId, WorkflowId};

    struct EchoAction {
        meta: ActionMetadata,
    }

    impl ActionDependencies for EchoAction {}
    impl Action for EchoAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl StatelessAction for EchoAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        async fn execute(
            &self,
            input: Self::Input,
            _ctx: &impl Context,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    struct FailAction {
        meta: ActionMetadata,
    }

    impl ActionDependencies for FailAction {}
    impl Action for FailAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl StatelessAction for FailAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        async fn execute(
            &self,
            _input: Self::Input,
            _ctx: &impl Context,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Err(ActionError::retryable("transient failure"))
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
        let metrics = MetricsRegistry::new();

        ActionRuntime::new(registry, sandbox, DataPassingPolicy::default(), metrics)
    }

    #[tokio::test]
    async fn execute_trusted_action() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoAction {
            meta: ActionMetadata::new(action_key!("test.echo"), "Echo", "echoes input"),
        });

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
        registry.register_stateless(FailAction {
            meta: ActionMetadata::new(action_key!("test.fail"), "Fail", "always fails"),
        });

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
        registry.register_stateless(EchoAction {
            meta: ActionMetadata::new(action_key!("test.big"), "Big", "returns big output"),
        });

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let sandbox = Arc::new(InProcessSandbox::new(executor));
        let metrics = MetricsRegistry::new();

        let rt = ActionRuntime::new(
            registry,
            sandbox,
            DataPassingPolicy {
                max_node_output_bytes: 5, // very small
                ..Default::default()
            },
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
    async fn metrics_recorded_on_execution() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoAction {
            meta: ActionMetadata::new(action_key!("test.tele"), "Tele", "test"),
        });

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let sandbox = Arc::new(InProcessSandbox::new(executor));
        let metrics = MetricsRegistry::new();

        let rt = ActionRuntime::new(
            registry,
            sandbox,
            DataPassingPolicy::default(),
            metrics.clone(),
        );

        rt.execute_action("test.tele", serde_json::json!("ok"), test_context())
            .await
            .unwrap();

        // Metrics should be recorded.
        assert_eq!(metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL).get(), 1);
        assert_eq!(metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).get(), 0);
    }

    #[tokio::test]
    async fn trigger_context_construction_is_usable_in_runtime() {
        let ctx = test_trigger_context();
        assert!(!ctx.has_credential_id("missing").await);
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

    #[tokio::test]
    #[ignore = "Sandboxed dispatch is Phase 7.6 — currently bypassed"]
    async fn execute_uses_sandbox_for_capability_gated() {
        use std::sync::atomic::{AtomicBool, Ordering};

        // Track whether sandbox was invoked.
        let sandbox_called = Arc::new(AtomicBool::new(false));
        let sandbox_called_clone = sandbox_called.clone();

        let executor: ActionExecutor = Arc::new(move |_ctx, _meta, input| {
            sandbox_called_clone.store(true, Ordering::SeqCst);
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let sandbox = Arc::new(InProcessSandbox::new(executor));

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoAction {
            meta: ActionMetadata::new(action_key!("test.gated"), "Gated", "capability gated")
                .with_isolation_level(IsolationLevel::CapabilityGated),
        });

        let metrics = MetricsRegistry::new();
        let rt = ActionRuntime::new(registry, sandbox, DataPassingPolicy::default(), metrics);

        let result = rt
            .execute_action("test.gated", serde_json::json!({"data": 1}), test_context())
            .await;
        assert!(result.is_ok());
        assert!(
            sandbox_called.load(Ordering::SeqCst),
            "sandbox should have been called for CapabilityGated action"
        );
    }

    #[tokio::test]
    async fn spill_to_blob_rejects_when_no_storage() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoAction {
            meta: ActionMetadata::new(action_key!("test.spill"), "Spill", "large output"),
        });

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let sandbox = Arc::new(InProcessSandbox::new(executor));
        let metrics = MetricsRegistry::new();

        let rt = ActionRuntime::new(
            registry,
            sandbox,
            DataPassingPolicy {
                max_node_output_bytes: 5,
                large_data_strategy: LargeDataStrategy::SpillToBlob,
                ..Default::default()
            },
            metrics,
        );

        // No blob storage configured -- should reject.
        let input = serde_json::json!({"big": "this exceeds 5 bytes easily"});
        let result = rt.execute_action("test.spill", input, test_context()).await;
        assert!(
            matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
            "expected DataLimitExceeded when no blob storage configured"
        );
    }

    #[tokio::test]
    async fn spill_to_blob_succeeds_with_storage() {
        use crate::blob::{BlobRef, BlobStorage};

        struct FakeBlobStorage;

        #[async_trait::async_trait]
        impl BlobStorage for FakeBlobStorage {
            async fn write(
                &self,
                data: &[u8],
                content_type: &str,
            ) -> Result<BlobRef, RuntimeError> {
                Ok(BlobRef {
                    uri: "mem://test/blob-1".into(),
                    size_bytes: data.len() as u64,
                    content_type: content_type.into(),
                })
            }
            async fn read(&self, _blob_ref: &BlobRef) -> Result<Vec<u8>, RuntimeError> {
                Ok(vec![])
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoAction {
            meta: ActionMetadata::new(
                action_key!("test.spill_ok"),
                "SpillOk",
                "large output with storage",
            ),
        });

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let sandbox = Arc::new(InProcessSandbox::new(executor));
        let metrics = MetricsRegistry::new();

        let rt = ActionRuntime::new(
            registry,
            sandbox,
            DataPassingPolicy {
                max_node_output_bytes: 5,
                large_data_strategy: LargeDataStrategy::SpillToBlob,
                ..Default::default()
            },
            metrics,
        )
        .with_blob_storage(Arc::new(FakeBlobStorage));

        let input = serde_json::json!({"big": "this exceeds 5 bytes easily"});
        let result = rt
            .execute_action("test.spill_ok", input, test_context())
            .await;
        let action_result = result.expect("should succeed when blob storage is configured");

        // Verify the large inline payload was replaced with an external reference.
        match action_result {
            ActionResult::Success {
                output: ActionOutput::Reference(data_ref),
            } => {
                assert_eq!(data_ref.storage_type, "blob");
                assert_eq!(data_ref.path, "mem://test/blob-1");
                assert!(data_ref.size.is_some());
                assert_eq!(data_ref.content_type.as_deref(), Some("application/json"));
            }
            other => panic!("expected Success with Reference output after spill, got {other:?}"),
        }
    }
}
