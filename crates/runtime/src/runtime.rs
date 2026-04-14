//! Action runtime -- the main execution orchestrator.
//!
//! Resolves actions from the registry, executes them through the sandbox,
//! enforces data limits, and records metrics.

use std::{sync::Arc, time::Instant};

use nebula_action::{
    ActionContext, ActionError, ActionHandler, ActionMetadata, IsolationLevel, StatefulHandler,
    StatelessHandler,
    output::{ActionOutput, DataReference},
    result::ActionResult,
};
use nebula_metrics::naming::{
    NEBULA_ACTION_DURATION_SECONDS, NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_ACTION_FAILURES_TOTAL,
};
use nebula_telemetry::metrics::MetricsRegistry;

use crate::{
    blob::BlobStorage,
    data_policy::{DataPassingPolicy, LargeDataStrategy},
    error::RuntimeError,
    registry::ActionRegistry,
    sandbox::{SandboxRunner, SandboxedContext},
};

/// The action runtime orchestrates execution of actions.
///
/// It sits between the engine (which schedules work) and the sandbox
/// (which provides isolation). The runtime:
///
/// 1. Looks up the action handler from the registry
/// 2. Resolves the isolation level
/// 3. Executes through the sandbox (if capability-gated/isolated) or directly (if trusted)
/// 4. Enforces data passing policies on the output
/// 5. Emits telemetry events
pub struct ActionRuntime {
    registry: Arc<ActionRegistry>,
    // Used for non-None isolation in execute_stateless (Phase 0).
    // Stateful isolation dispatch remains fail-closed until the broker
    // protocol lands in Phase 1 — see execute_stateful.
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
    /// Dispatch depends on the action's [`IsolationLevel`]:
    ///
    /// - `None` — trusted in-process execution, handler invoked directly.
    /// - `CapabilityGated` / `Isolated` — routed through [`SandboxRunner`]. In Phase 0 the sandbox
    ///   is typically an `InProcessSandbox` whose `ActionExecutor` was wired at engine construction
    ///   time; the engine is responsible for passing a closure that can actually invoke the
    ///   registered handler for the given action key. Phase 1 replaces this with `ProcessSandbox`
    ///   dispatching to real plugin subprocesses over the UDS + JSON duplex v2 protocol (slices
    ///   1a–1c shipped; slice 1d supervisor + broker pending). See
    ///   `docs/plans/2026-04-13-sandbox-roadmap.md` and
    ///   `docs/plans/2026-04-13-sandbox-phase1-broker.md`.
    async fn execute_stateless(
        &self,
        metadata: &ActionMetadata,
        handler: Arc<dyn StatelessHandler>,
        input: serde_json::Value,
        context: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        match metadata.isolation_level {
            IsolationLevel::None => handler.execute(input, &context).await,
            IsolationLevel::CapabilityGated | IsolationLevel::Isolated => {
                let sandboxed = SandboxedContext::new(context);
                self.sandbox.execute(sandboxed, metadata, input).await
            }
            // IsolationLevel is `#[non_exhaustive]`. Any future variant must
            // fail-closed until we explicitly wire dispatch for it.
            _ => Err(ActionError::fatal(format!(
                "unknown isolation level for action '{}' — refusing to dispatch",
                metadata.key.as_str()
            ))),
        }
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
            // Stateful sandbox dispatch requires a long-lived broker loop to
            // persist state across iterations. The current `SandboxRunner`
            // trait is a single-shot execute call with no iteration semantics.
            // Unblocks when sandbox slice 1d ships the supervisor + broker —
            // see docs/plans/2026-04-13-sandbox-phase1-broker.md.
            return Err(ActionError::fatal(
                "sandboxed stateful execution is not yet supported — \
                 broker iteration protocol lands in sandbox slice 1d",
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

    /// Check every downstream-visible output slot against the data-passing
    /// policy.
    ///
    /// This walks *all* output fields an action can emit, not just the
    /// "primary" one:
    ///
    /// - `Success` / `Continue` / `Break` / `Route` — their single output
    /// - `Skip` / `Wait` — the optional partial output
    /// - `Branch` — the selected output **and** every alternative (previews are still shipped
    ///   downstream; a misbehaving node must not smuggle a GB-sized alternative past the limit)
    /// - `MultiOutput` — the optional main output **and** every fan-out port
    ///
    /// For each inline `Value` slot that exceeds the limit, applies the
    /// configured strategy:
    /// - `Reject` → returns `DataLimitExceeded` on the first offender
    /// - `SpillToBlob` → writes the payload to blob storage and rewrites the slot to an
    ///   `ActionOutput::Reference` so the large inline value is no longer carried downstream
    ///
    /// Non-`Value` variants (`Binary` / `Reference` / `Deferred`) are
    /// skipped — their size is managed by the owning storage backend.
    async fn enforce_data_limit(
        &self,
        action_key: &str,
        action_result: &mut ActionResult<serde_json::Value>,
        error_counter: &nebula_telemetry::metrics::Counter,
    ) -> Result<(), RuntimeError> {
        let limit = self.data_policy.max_node_output_bytes;

        // Collect disjoint mut references to every output slot in the result.
        // The Vec itself holds unique borrows of distinct struct fields, so
        // iterating and mutating each in turn is sound.
        let mut slots: Vec<&mut ActionOutput<serde_json::Value>> = Vec::new();
        collect_output_slots_mut(action_result, &mut slots);

        for slot in slots {
            // Only inline Value outputs participate in the size check;
            // Binary / Reference / Deferred are already bounded by their
            // respective backends.
            let serialized = match &*slot {
                ActionOutput::Value(v) => serde_json::to_vec(v).map_err(|e| {
                    RuntimeError::Internal(format!(
                        "failed to serialize output for size limit enforcement: {e}"
                    ))
                })?,
                _ => continue,
            };
            let actual = serialized.len() as u64;
            if actual <= limit {
                continue;
            }

            match self.data_policy.large_data_strategy {
                LargeDataStrategy::Reject => {
                    error_counter.inc();
                    return Err(RuntimeError::DataLimitExceeded {
                        limit_bytes: limit,
                        actual_bytes: actual,
                    });
                }
                LargeDataStrategy::SpillToBlob => {
                    let Some(storage) = self.blob_storage.as_ref() else {
                        tracing::warn!(
                            action_key,
                            actual,
                            limit,
                            "output exceeds limit and no blob storage configured"
                        );
                        error_counter.inc();
                        return Err(RuntimeError::DataLimitExceeded {
                            limit_bytes: limit,
                            actual_bytes: actual,
                        });
                    };
                    let blob_ref = match storage.write(&serialized, "application/json").await {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!(
                                action_key,
                                error = %e,
                                "blob spill failed, rejecting output"
                            );
                            error_counter.inc();
                            return Err(RuntimeError::DataLimitExceeded {
                                limit_bytes: limit,
                                actual_bytes: actual,
                            });
                        }
                    };
                    tracing::info!(
                        action_key,
                        uri = %blob_ref.uri,
                        size = blob_ref.size_bytes,
                        "output slot spilled to blob storage"
                    );
                    *slot = ActionOutput::Reference(DataReference {
                        storage_type: "blob".into(),
                        path: blob_ref.uri,
                        size: Some(blob_ref.size_bytes),
                        content_type: Some(blob_ref.content_type),
                    });
                }
            }
        }

        Ok(())
    }
}

/// Push a mut reference to every downstream-visible output slot in `result`
/// into `out`.
///
/// Each pushed reference borrows a distinct field of `result`, so the set
/// of references is disjoint and safe to iterate and mutate sequentially.
///
/// Variants without any output slot (`Retry`, `Drop`, `Terminate`, future
/// `#[non_exhaustive]` variants) push nothing.
fn collect_output_slots_mut<'a>(
    result: &'a mut ActionResult<serde_json::Value>,
    out: &mut Vec<&'a mut ActionOutput<serde_json::Value>>,
) {
    match result {
        ActionResult::Success { output } => out.push(output),
        ActionResult::Skip { output, .. } => {
            if let Some(o) = output.as_mut() {
                out.push(o);
            }
        }
        ActionResult::Continue { output, .. } => out.push(output),
        ActionResult::Break { output, .. } => out.push(output),
        ActionResult::Branch {
            output,
            alternatives,
            ..
        } => {
            out.push(output);
            for alt in alternatives.values_mut() {
                out.push(alt);
            }
        }
        ActionResult::Route { data, .. } => out.push(data),
        ActionResult::MultiOutput {
            outputs,
            main_output,
        } => {
            if let Some(m) = main_output.as_mut() {
                out.push(m);
            }
            for o in outputs.values_mut() {
                out.push(o);
            }
        }
        ActionResult::Wait { partial_output, .. } => {
            if let Some(o) = partial_output.as_mut() {
                out.push(o);
            }
        }
        // `ActionResult` is `#[non_exhaustive]`. Variants without a
        // downstream-visible payload (Retry, Drop, Terminate, and any
        // future additions) contribute nothing here — a slot they own
        // cannot bypass the limit because there is no slot.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use nebula_action::{
        ActionContext, TriggerContext,
        action::Action,
        context::{Context, CredentialContextExt},
        dependency::ActionDependencies,
        error::ActionError,
        metadata::ActionMetadata,
        stateless::StatelessAction,
    };
    use nebula_core::{
        action_key,
        id::{ExecutionId, NodeId, WorkflowId},
    };

    use super::*;
    use crate::sandbox::{ActionExecutor, InProcessSandbox};

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

    /// Regression: previously, `enforce_data_limit` only inspected a single
    /// "primary" output slot. A `MultiOutput` with oversized fan-out ports
    /// sailed through the limit silently — any port could carry an
    /// arbitrarily large payload downstream as long as `main_output` was
    /// small (or absent). This test pins the fix: every port slot is
    /// checked.
    #[tokio::test]
    async fn multi_output_fanout_port_respects_reject_limit() {
        use std::collections::HashMap;

        use nebula_action::{PortKey, result::ActionResult as AR};

        struct MultiOutAction {
            meta: ActionMetadata,
        }
        impl ActionDependencies for MultiOutAction {}
        impl Action for MultiOutAction {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
        }
        impl StatelessAction for MultiOutAction {
            type Input = serde_json::Value;
            type Output = serde_json::Value;
            async fn execute(
                &self,
                _input: Self::Input,
                _ctx: &impl Context,
            ) -> Result<AR<Self::Output>, ActionError> {
                // `main_output` is tiny; a fan-out port is huge. Before the
                // fix, only `main_output` was checked and this result passed
                // a byte limit of 16.
                let mut outputs: HashMap<PortKey, ActionOutput<serde_json::Value>> = HashMap::new();
                outputs.insert(
                    PortKey::from("big_port"),
                    ActionOutput::Value(serde_json::json!(
                        "this payload is definitely larger than the 16 byte limit"
                    )),
                );
                Ok(AR::MultiOutput {
                    outputs,
                    main_output: Some(ActionOutput::Value(serde_json::json!("ok"))),
                })
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(MultiOutAction {
            meta: ActionMetadata::new(
                action_key!("test.multi_out"),
                "MultiOut",
                "multi-port fan-out",
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
                max_node_output_bytes: 16,
                large_data_strategy: LargeDataStrategy::Reject,
                ..Default::default()
            },
            metrics,
        );

        let result = rt
            .execute_action("test.multi_out", serde_json::json!(null), test_context())
            .await;
        assert!(
            matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
            "MultiOutput fan-out port must not bypass the data-passing limit \
             — got {result:?}"
        );
    }

    /// Regression: `Branch.alternatives` previously bypassed the size limit
    /// too. A branch node could ship a GB-sized preview alongside the
    /// selected output and it would pass through silently.
    #[tokio::test]
    async fn branch_alternatives_respect_reject_limit() {
        use std::collections::HashMap;

        use nebula_action::result::ActionResult as AR;

        struct BranchAction {
            meta: ActionMetadata,
        }
        impl ActionDependencies for BranchAction {}
        impl Action for BranchAction {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
        }
        impl StatelessAction for BranchAction {
            type Input = serde_json::Value;
            type Output = serde_json::Value;
            async fn execute(
                &self,
                _input: Self::Input,
                _ctx: &impl Context,
            ) -> Result<AR<Self::Output>, ActionError> {
                let mut alternatives = HashMap::new();
                alternatives.insert(
                    "else".to_string(),
                    ActionOutput::Value(serde_json::json!(
                        "alternative branch holds way more than 16 bytes of data"
                    )),
                );
                Ok(AR::Branch {
                    selected: "then".to_string(),
                    output: ActionOutput::Value(serde_json::json!("ok")),
                    alternatives,
                })
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(BranchAction {
            meta: ActionMetadata::new(action_key!("test.branch"), "Branch", "branch with alts"),
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
                max_node_output_bytes: 16,
                large_data_strategy: LargeDataStrategy::Reject,
                ..Default::default()
            },
            metrics,
        );

        let result = rt
            .execute_action("test.branch", serde_json::json!(null), test_context())
            .await;
        assert!(
            matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
            "Branch.alternatives must not bypass the data-passing limit — got {result:?}"
        );
    }
}
