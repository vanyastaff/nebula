//! Action runtime -- the main execution orchestrator.
//!
//! Resolves actions from the registry, executes them through the sandbox,
//! enforces data limits, and records metrics.

use std::sync::Arc;
use std::time::Instant;

use crate::blob::BlobStorage;
use crate::sandbox::{SandboxRunner, SandboxedContext};
use nebula_action::ActionContext;
use nebula_action::metadata::IsolationLevel;
use nebula_action::output::{ActionOutput, DataReference};
use nebula_action::result::ActionResult;
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
    /// When `version` is `Some`, the registry resolves the handler registered for
    /// that exact version. When `version` is `None`, the latest registered handler
    /// is used (same behaviour as [`execute_action`]).
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
        let handler = match version {
            Some(v) => self.registry.get_versioned(action_key, v)?,
            None => self.registry.get(action_key)?,
        };
        self.run_handler(action_key, handler, input, context).await
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
        self.run_handler(action_key, handler, input, context).await
    }

    /// Internal: run a resolved handler through the sandbox and data policy.
    async fn run_handler(
        &self,
        action_key: &str,
        handler: Arc<dyn nebula_action::InternalHandler>,
        input: serde_json::Value,
        context: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        let started = Instant::now();
        let action_counter = self.metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL);
        let error_counter = self.metrics.counter(NEBULA_ACTION_FAILURES_TOTAL);
        let duration_hist = self.metrics.histogram(NEBULA_ACTION_DURATION_SECONDS);

        let metadata = handler.metadata();
        let result = match metadata.isolation_level {
            IsolationLevel::None => {
                // Direct execution -- no sandbox overhead for trusted actions.
                handler.execute(input, &context).await
            }
            _ => {
                // Wrap context with sandbox for capability checking / isolation.
                // All non-`None` variants, including future ones, route through the sandbox for safety.
                let sandboxed = SandboxedContext::new(context);
                self.sandbox.execute(sandboxed, metadata, input).await
            }
        };

        let elapsed = started.elapsed();
        duration_hist.observe(elapsed.as_secs_f64());
        action_counter.inc();

        match result {
            Ok(mut action_result) => {
                // Enforce data limits on the primary output value.
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
        let metrics = MetricsRegistry::new();

        ActionRuntime::new(registry, sandbox, DataPassingPolicy::default(), metrics)
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
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("test.tele"), "Tele", "test"),
        }));

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

    #[tokio::test]
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
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("test.gated"), "Gated", "capability gated")
                .with_isolation_level(IsolationLevel::CapabilityGated),
        }));

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
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(action_key!("test.spill"), "Spill", "large output"),
        }));

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
        registry.register(Arc::new(EchoHandler {
            meta: ActionMetadata::new(
                action_key!("test.spill_ok"),
                "SpillOk",
                "large output with storage",
            ),
        }));

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
