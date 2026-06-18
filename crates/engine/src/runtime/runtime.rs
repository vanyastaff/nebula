//! Action runtime -- the main execution orchestrator.
//!
//! Resolves actions from the registry, executes them through the runner,
//! enforces data limits, and records metrics.

use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use nebula_action::{
    ActionContext, ActionError, ActionFactory, ActionHandle, ActionMetadata, IsolationLevel,
    StreamHandle,
    output::{ActionOutput, DataReference},
    result::ActionResult,
};
use nebula_core::ExecutionId;
use nebula_metrics::naming::{
    NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, NEBULA_ACTION_DURATION_SECONDS,
    NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_ACTION_FAILURES_TOTAL, dispatch_reject_reason,
};
use nebula_metrics::{Counter, Histogram, MetricsError, MetricsRegistry};
use nebula_workflow::NodeDefinition;
use serde::{Deserialize, Serialize};

use super::{
    blob::BlobStorage,
    data_policy::{DataPassingPolicy, LargeDataStrategy},
    error::RuntimeError,
    registry::ActionRegistry,
    runner::{ActionRunContext, ActionRunner},
};

/// Compute a digest of the serialized stateful state for stuck-state detection.
///
/// The runtime sees `state` as `serde_json::Value` — not `Hash`, but always
/// serialisable. We route through `serde_json::to_vec` and hash the bytes.
/// Errors collapse to `0` so the guard reduces to "assume the iteration
/// progressed" on unserialisable state — an author who manages to hold an
/// unserialisable `Value` has bigger problems than stuck detection.
fn stateful_state_digest(state: &serde_json::Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match serde_json::to_vec(state) {
        Ok(bytes) => bytes.hash(&mut hasher),
        Err(_) => 0u8.hash(&mut hasher),
    }
    hasher.finish()
}

/// Persisted iteration state for a stateful action.
///
/// Emitted by the runtime after every `Continue` before looping, consumed
/// by the runtime at the start of a fresh dispatch to resume from the last
/// recorded boundary.
///
/// The `iteration` counter is load-bearing: it is the same counter the
/// runtime uses to enforce `MAX_ITERATIONS`, so resuming a stateful action
/// after a crash keeps the cap honest across restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StatefulCheckpoint {
    /// Number of completed iterations the handler had when this checkpoint
    /// was written. The next dispatch starts counting from here.
    pub iteration: u32,
    /// Handler state as JSON — exactly the value the runtime would have
    /// handed back into `StatefulHandler::execute` on the next loop.
    pub state: serde_json::Value,
}

impl StatefulCheckpoint {
    /// Build a new checkpoint.
    #[must_use]
    pub fn new(iteration: u32, state: serde_json::Value) -> Self {
        Self { iteration, state }
    }
}

/// Engine-provided hook the runtime uses to persist stateful iteration
/// state.
///
/// The runtime does not depend on `nebula-storage` directly — the engine
/// implements this trait backed by `ExecutionRepo::{save,load,delete}_stateful_checkpoint`
/// and injects it into `execute_action_with_checkpoint`.
///
/// Methods return [`ActionError`] for sink-transport/serialization failures.
///
/// Runtime behavior differs by method:
/// - `load` errors are logged at WARN and execution falls back to `handler.init_state()`.
/// - `save` errors are propagated as action errors (retry-classified by the caller).
/// - `clear` errors are logged at WARN on terminal iterations and ignored.
#[async_trait]
pub trait StatefulCheckpointSink: Send + Sync {
    /// Return the last persisted checkpoint for the (execution, node,
    /// attempt) this runtime call serves, or `None` to start fresh.
    async fn load(&self) -> Result<Option<StatefulCheckpoint>, ActionError>;

    /// Persist the given state + iteration. Called on every successful
    /// `Continue` before the loop sleeps and recurses.
    async fn save(&self, checkpoint: &StatefulCheckpoint) -> Result<(), ActionError>;

    /// Delete the checkpoint — called once on `Break` / `Success` so a
    /// completed stateful action does not leave rows behind.
    async fn clear(&self) -> Result<(), ActionError>;
}

/// The action runtime orchestrates execution of actions.
///
/// It sits between the engine (which schedules work) and the runner
/// (which performs in-process dispatch). The runtime:
///
/// 1. Looks up the action handler from the registry
/// 2. Resolves the isolation level
/// 3. Executes through the runner (if capability-gated) or directly (if trusted)
/// 4. Enforces data passing policies on the output
/// 5. Emits telemetry events
pub struct ActionRuntime {
    registry: Arc<ActionRegistry>,
    // Used for capability-gated isolation in execute_stateless. Stateful
    // capability-gated dispatch is fail-closed — see execute_stateful.
    runner: Arc<dyn ActionRunner>,
    data_policy: DataPassingPolicy,
    metrics: MetricsRegistry,
    /// Pre-bound at construction so hot paths never propagate registry errors.
    action_failures_total: Counter,
    action_duration_seconds: Histogram,
    action_executions_total: Counter,
    blob_storage: Option<Arc<dyn BlobStorage>>,
    /// Sum of estimated output bytes per execution for
    /// [`DataPassingPolicy::max_total_execution_bytes`].
    execution_output_totals: Arc<DashMap<ExecutionId, u64>>,
}

impl ActionRuntime {
    /// Create a new runtime with the given components.
    ///
    /// # Errors
    ///
    /// Returns [`MetricsError`] if the shared registry rejects registration
    /// for the canonical action metric identities (e.g. name reused as another
    /// primitive kind).
    pub fn try_new(
        registry: Arc<ActionRegistry>,
        runner: Arc<dyn ActionRunner>,
        data_policy: DataPassingPolicy,
        metrics: MetricsRegistry,
    ) -> Result<Self, MetricsError> {
        let action_failures_total = metrics.counter(NEBULA_ACTION_FAILURES_TOTAL)?;
        let action_duration_seconds = metrics.histogram(NEBULA_ACTION_DURATION_SECONDS)?;
        let action_executions_total = metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL)?;
        Ok(Self {
            registry,
            runner,
            data_policy,
            metrics,
            action_failures_total,
            action_duration_seconds,
            action_executions_total,
            blob_storage: None,
            execution_output_totals: Arc::new(DashMap::new()),
        })
    }

    /// Clears accumulated output-byte totals for an execution.
    ///
    /// The workflow engine calls this when a run completes so accounting
    /// entries do not accumulate forever ([`DataPassingPolicy::max_total_execution_bytes`]).
    pub fn clear_execution_output_totals(&self, execution_id: ExecutionId) {
        self.execution_output_totals.remove(&execution_id);
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
        version: Option<&semver::Version>,
        input: serde_json::Value,
        context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        self.execute_action_with_checkpoint(action_key, version, input, context, None)
            .await
    }

    /// Execute an action by key, with an optional stateful checkpoint sink.
    ///
    /// Same shape as [`Self::execute_action_versioned`] but also accepts a
    /// [`StatefulCheckpointSink`]. The sink is consulted only for
    /// `ActionHandle::Stateful` dispatch (produced by stateful factories):
    ///
    /// - Before the iteration loop, `sink.load()` is called. A `Some` checkpoint resumes from the
    ///   last persisted `(iteration, state)`; `None` falls through to `handle.init_state()`.
    /// - After every successful `Continue`, `sink.save(..)` persists the mutated state and
    ///   iteration counter before looping.
    /// - After a terminal iteration (`Break`, `Success`, …), `sink.clear()` drops the checkpoint so
    ///   it cannot linger past completion.
    ///
    /// Stateless dispatches ignore the sink entirely. Pass `None` if you
    /// do not need cross-dispatch resume — behaviour matches the original
    /// `execute_action_versioned` shape.
    ///
    /// This entry point synthesizes a minimal [`NodeDefinition`] from the
    /// supplied `action_key` (and optional `version`) for callers that do
    /// not already have one (admin tooling, tests). Production engine
    /// dispatch routes through [`Self::execute_action_with_node`] which
    /// passes the real workflow node so [`ActionFactory::instantiate`] can
    /// resolve slot bindings declared on the node.
    ///
    /// # Errors
    ///
    /// Same as [`Self::execute_action_versioned`], plus `save()` sink errors
    /// surfaced as [`RuntimeError::ActionError`]. `load()` / `clear()` sink
    /// errors are logged and handled in-band (fallback/ignore) by
    /// `execute_stateful`.
    pub async fn execute_action_with_checkpoint(
        &self,
        action_key: &str,
        version: Option<&semver::Version>,
        input: serde_json::Value,
        context: &dyn ActionContext,
        checkpoint: Option<Arc<dyn StatefulCheckpointSink>>,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        let key = nebula_core::ActionKey::new(action_key).map_err(|e| {
            RuntimeError::InvalidActionKey {
                key: action_key.to_owned(),
                reason: e.to_string(),
            }
        })?;

        let synthetic_node = synthesize_node_definition("core", action_key, version);

        self.dispatch_action(
            action_key,
            &key,
            version,
            &synthetic_node,
            input,
            context,
            checkpoint,
        )
        .await
    }

    /// Execute an action by key.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::ActionNotFound`] if the key does not resolve to a
    /// registered action, [`RuntimeError::TriggerNotExecutable`] /
    /// [`RuntimeError::ResourceNotExecutable`] if the key resolves to a
    /// handler kind that is not executable through this runtime, or
    /// [`RuntimeError::ActionError`] / [`RuntimeError::DataLimitExceeded`]
    /// if execution fails.
    pub async fn execute_action(
        &self,
        action_key: &str,
        input: serde_json::Value,
        context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        // Parse the key explicitly so we can distinguish "invalid format" from
        // "valid format but not registered". `get_by_str` collapses both into None.
        let key = nebula_core::ActionKey::new(action_key).map_err(|e| {
            RuntimeError::InvalidActionKey {
                key: action_key.to_owned(),
                reason: e.to_string(),
            }
        })?;

        let synthetic_node = synthesize_node_definition("core", action_key, None);

        self.dispatch_action(
            action_key,
            &key,
            None,
            &synthetic_node,
            input,
            context,
            None,
        )
        .await
    }

    /// Execute an action by node (production dispatch entry point).
    ///
    /// Looks up the `Arc<dyn ActionFactory>` for `node.action_key` and invokes
    /// [`ActionFactory::instantiate`] with the supplied [`NodeDefinition`] +
    /// [`ActionContext`] so slot bindings declared on the node resolve correctly.
    ///
    /// `version` is optional — when `Some`, an exact version match is required;
    /// when `None`, the latest registered version of the action is dispatched.
    ///
    /// # Errors
    ///
    /// Same as [`Self::execute_action_with_checkpoint`].
    pub async fn execute_action_with_node(
        &self,
        node: &NodeDefinition,
        version: Option<&semver::Version>,
        input: serde_json::Value,
        context: &dyn ActionContext,
        checkpoint: Option<Arc<dyn StatefulCheckpointSink>>,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        let action_key_str = node.action_key.as_str();
        self.dispatch_action(
            action_key_str,
            &node.action_key,
            version,
            node,
            input,
            context,
            checkpoint,
        )
        .await
    }

    /// Common dispatch entry — routes all executions through the factory path.
    ///
    /// Looks up the `Arc<dyn ActionFactory>` for the action key, instantiates a
    /// fresh `ActionHandle` via [`ActionFactory::instantiate`], and dispatches it
    /// through [`Self::run_factory`]. Returns
    /// [`RuntimeError::ActionNotFound`] if no factory is registered for the key.
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_action(
        &self,
        action_key_str: &str,
        action_key: &nebula_core::ActionKey,
        version: Option<&semver::Version>,
        node: &NodeDefinition,
        input: serde_json::Value,
        context: &dyn ActionContext,
        checkpoint: Option<Arc<dyn StatefulCheckpointSink>>,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        let factory_lookup = match version {
            Some(v) => self.registry.get_factory_versioned(action_key, v),
            None => self.registry.get_factory(action_key),
        };
        let (metadata, factory) = factory_lookup.ok_or_else(|| RuntimeError::ActionNotFound {
            key: action_key_str.to_owned(),
        })?;
        self.run_factory(
            action_key_str,
            metadata,
            factory,
            node,
            input,
            context,
            checkpoint,
        )
        .await
    }

    /// Dispatch through the factory path — instantiate a fresh
    /// [`ActionHandle`] for the supplied workflow node and dispatch it.
    ///
    /// Metric contract:
    ///
    /// - Stateless / Stateful / Control variants observe the duration histogram and increment
    ///   executions / failures.
    /// - Trigger / Resource variants are early-rejected (not executable through `ActionRuntime`)
    ///   and increment the dispatch-rejected counter only.
    ///
    /// `factory.instantiate` returning an error is treated as an action
    /// failure (slot resolution, etc.). The duration histogram is observed
    /// for instantiate failures so dashboards reflect the per-dispatch cost
    /// regardless of whether the failure happened in instantiation or
    /// during the action itself.
    #[allow(
        clippy::too_many_arguments,
        reason = "private dispatch entry — splitting into a struct hides the metric/observe contract from the call site"
    )]
    async fn run_factory(
        &self,
        action_key: &str,
        metadata: ActionMetadata,
        factory: Arc<dyn ActionFactory>,
        node: &NodeDefinition,
        input: serde_json::Value,
        context: &dyn ActionContext,
        checkpoint: Option<Arc<dyn StatefulCheckpointSink>>,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        let error_counter = &self.action_failures_total;
        #[allow(
            clippy::unwrap_or_default,
            reason = "ExecutionId::new() != Default::default()"
        )]
        let execution_id = context
            .scope()
            .execution_id
            .unwrap_or_else(ExecutionId::new);

        let started = Instant::now();

        // Instantiate the action via the factory. Slot-binding resolution
        // (and any FromWorkflowNode user code) runs here.
        let handle = match factory.instantiate(node, context).await {
            Ok(e) => e,
            Err(e) => {
                let result: Result<ActionResult<serde_json::Value>, RuntimeError> =
                    Err(RuntimeError::ActionError(e));
                self.observe_dispatched(started, &result);
                return result;
            },
        };

        let result = match handle {
            ActionHandle::Stateless(inner) => {
                let r = self
                    .execute_stateless_handle(&metadata, inner, input, context)
                    .await;
                self.observe_dispatched(started, &r);
                r
            },
            ActionHandle::Stateful(inner) => {
                let r = self
                    .execute_stateful_handle(&metadata, inner, input, context, checkpoint)
                    .await;
                self.observe_dispatched(started, &r);
                r
            },
            ActionHandle::Stream(inner) => {
                let r = self
                    .execute_stream_handle(&metadata, inner, input, context)
                    .await;
                self.observe_dispatched(started, &r);
                r
            },
            ActionHandle::Control(inner) => {
                let r = self
                    .execute_control_handle(&metadata, inner, input, context)
                    .await;
                self.observe_dispatched(started, &r);
                r
            },
            ActionHandle::Trigger(_) => {
                self.observe_rejected(dispatch_reject_reason::TRIGGER_NOT_EXECUTABLE);
                return Err(RuntimeError::TriggerNotExecutable {
                    key: action_key.to_owned(),
                });
            },
            ActionHandle::Resource(_) => {
                self.observe_rejected(dispatch_reject_reason::RESOURCE_NOT_EXECUTABLE);
                return Err(RuntimeError::ResourceNotExecutable {
                    key: action_key.to_owned(),
                });
            },
            // `ActionHandle` is `#[non_exhaustive]`. Unknown future variants
            // surface as an internal runtime error rather than silently
            // succeeding.
            _ => {
                self.observe_rejected(dispatch_reject_reason::UNKNOWN_VARIANT);
                return Err(RuntimeError::Internal(format!(
                    "unknown ActionHandle variant for action '{action_key}'"
                )));
            },
        };

        match result {
            Ok(mut action_result) => {
                self.enforce_data_limit(
                    action_key,
                    execution_id,
                    &mut action_result,
                    error_counter,
                )
                .await?;
                Ok(action_result)
            },
            Err(runtime_err) => Err(runtime_err),
        }
    }

    /// Stateless dispatch via `Box<dyn StatelessHandle>`.
    ///
    /// Mirrors [`Self::execute_stateless`] for the factory path. Honours
    /// the same isolation contract (`None` runs in-process; capability-gated
    /// dispatch routes through [`ActionRunner`] using the same `metadata`).
    async fn execute_stateless_handle(
        &self,
        metadata: &ActionMetadata,
        handle: Box<dyn nebula_action::StatelessHandle>,
        input: serde_json::Value,
        context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        match metadata.isolation_level {
            IsolationLevel::None => Ok(handle.dispatch(input, context).await?),
            IsolationLevel::CapabilityGated => {
                let run_ctx = ActionRunContext::new(context);
                Ok(self.runner.execute(run_ctx, metadata, input).await?)
            },
            // IsolationLevel is `#[non_exhaustive]`. Any future variant must
            // fail-closed until we explicitly wire dispatch for it.
            _ => Err(RuntimeError::Internal(format!(
                "unknown isolation level for action '{}' — refusing to dispatch",
                metadata.base.key.as_str()
            ))),
        }
    }

    /// Stream dispatch via `Box<dyn StreamHandle>`.
    ///
    /// Near-clone of [`Self::execute_stateless_handle`] — the only
    /// difference is the handle trait (`StreamHandle` instead of
    /// `StatelessHandle`). The stream is driven fully in-process inside
    /// the adapter; the engine receives one folded value.
    ///
    /// Isolation contract mirrors stateless: `None` runs in-process;
    /// `CapabilityGated` routes through the [`ActionRunner`]. Future
    /// isolation variants fail-closed.
    #[tracing::instrument(
        name = "runtime.execute_stream_handle",
        skip_all,
        fields(
            action.key = %metadata.base.key.as_str(),
            action.kind = "stream",
        )
    )]
    async fn execute_stream_handle(
        &self,
        metadata: &ActionMetadata,
        handle: Box<dyn StreamHandle>,
        input: serde_json::Value,
        context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        match metadata.isolation_level {
            IsolationLevel::None => Ok(handle.dispatch(input, context).await?),
            IsolationLevel::CapabilityGated => {
                let run_ctx = ActionRunContext::new(context);
                Ok(self.runner.execute(run_ctx, metadata, input).await?)
            },
            // IsolationLevel is `#[non_exhaustive]`. Any future variant must
            // fail-closed until we explicitly wire dispatch for it.
            _ => Err(RuntimeError::Internal(format!(
                "unknown isolation level for stream action '{}' — refusing to dispatch",
                metadata.base.key.as_str()
            ))),
        }
    }

    /// Stateful dispatch via `Box<dyn StatefulHandle>`.
    ///
    /// Mirrors [`Self::execute_stateful`] for the factory path. The
    /// handle trait works on `Value` state so the iteration body matches
    /// 1:1 with the legacy `Arc<dyn StatefulHandler>` path — same cancel
    /// race, same checkpoint contract, same iteration cap, same
    /// stuck-state guard.
    async fn execute_stateful_handle(
        &self,
        metadata: &ActionMetadata,
        handle: Box<dyn nebula_action::StatefulHandle>,
        input: serde_json::Value,
        context: &dyn ActionContext,
        checkpoint: Option<Arc<dyn StatefulCheckpointSink>>,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        if !matches!(metadata.isolation_level, IsolationLevel::None) {
            return Err(ActionError::fatal(
                "capability-gated stateful execution is not yet supported",
            )
            .into());
        }

        if context.cancellation().is_cancelled() {
            return Err(ActionError::Cancelled.into());
        }

        let (mut state, mut iteration) = match checkpoint.as_deref() {
            Some(sink) => match sink.load().await {
                Ok(Some(cp)) => (cp.state, cp.iteration),
                Ok(None) => (handle.init_state()?, 0u32),
                Err(load_err) => {
                    tracing::warn!(
                        action_key = %metadata.base.key.as_str(),
                        execution_id = ?context.scope().execution_id,
                        node_key = %context.node_key(),
                        error = %load_err,
                        "stateful checkpoint load failed — falling back to init_state, \
                         iteration progress (if any) is lost"
                    );
                    (handle.init_state()?, 0u32)
                },
            },
            None => (handle.init_state()?, 0u32),
        };

        const MAX_ITERATIONS: u32 = 10_000;

        loop {
            if iteration >= MAX_ITERATIONS {
                return Err(RuntimeError::IterationCapExceeded {
                    action_key: metadata.base.key.clone(),
                    node_key: context.node_key().clone(),
                    cap: MAX_ITERATIONS,
                });
            }

            if context.cancellation().is_cancelled() {
                return Err(ActionError::Cancelled.into());
            }

            let state_digest_before = stateful_state_digest(&state);

            let iteration_result = {
                let exec_fut = handle.dispatch(&input, &mut state, context);
                tokio::pin!(exec_fut);

                tokio::select! {
                    biased;
                    () = context.cancellation().cancelled() => {
                        return Err(ActionError::Cancelled.into());
                    }
                    res = &mut exec_fut => res,
                }
            };

            let result = iteration_result?;
            iteration = iteration.saturating_add(1);

            match result {
                ActionResult::Continue { delay, .. } => {
                    let state_digest_after = stateful_state_digest(&state);
                    if state_digest_before == state_digest_after {
                        return Err(RuntimeError::StatefulStuck {
                            action_key: metadata.base.key.clone(),
                            node_key: context.node_key().clone(),
                            iteration,
                        });
                    }

                    if let Some(sink) = checkpoint.as_deref() {
                        let cp = StatefulCheckpoint::new(iteration, state.clone());
                        sink.save(&cp).await?;
                    }

                    if let Some(d) = delay {
                        tokio::select! {
                            () = tokio::time::sleep(d) => {}
                            () = context.cancellation().cancelled() => {
                                return Err(ActionError::Cancelled.into());
                            }
                        }
                    }
                },
                other => {
                    if let Some(sink) = checkpoint.as_deref()
                        && let Err(clear_err) = sink.clear().await
                    {
                        tracing::warn!(
                            action_key = %metadata.base.key.as_str(),
                            execution_id = ?context.scope().execution_id,
                            node_key = %context.node_key(),
                            error = %clear_err,
                            "stateful checkpoint clear failed on terminal iteration; \
                             orphaned row left for engine GC"
                        );
                    }
                    return Ok(other);
                },
            }
        }
    }

    /// Control dispatch via `Box<dyn ControlHandle>`.
    ///
    /// Control nodes (If / Switch / Router / Filter / NoOp / Stop / Fail)
    /// dispatch as one-shot evaluators and never run through the runner —
    /// they produce flow-control [`ActionResult`] variants but no I/O. The
    /// handle surface is intentionally identical to stateless from the
    /// runtime's POV.
    async fn execute_control_handle(
        &self,
        metadata: &ActionMetadata,
        handle: Box<dyn nebula_action::ControlHandle>,
        input: serde_json::Value,
        context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        if !matches!(metadata.isolation_level, IsolationLevel::None) {
            return Err(RuntimeError::Internal(format!(
                "control action '{}' must run with IsolationLevel::None — \
                 control nodes are flow-control desugared to stateless and \
                 never run through the runner",
                metadata.base.key.as_str()
            )));
        }
        Ok(handle.dispatch(input, context).await?)
    }

    /// Observe a dispatched handler execution.
    ///
    /// Records duration into [`NEBULA_ACTION_DURATION_SECONDS`], bumps
    /// [`NEBULA_ACTION_EXECUTIONS_TOTAL`], and — on handler-returned error
    /// — bumps [`NEBULA_ACTION_FAILURES_TOTAL`]. Rejection paths must NOT
    /// route through this helper (see [`Self::observe_rejected`]).
    fn observe_dispatched(
        &self,
        started: Instant,
        result: &Result<ActionResult<serde_json::Value>, RuntimeError>,
    ) {
        let elapsed = started.elapsed();
        self.action_duration_seconds.observe(elapsed.as_secs_f64());
        self.action_executions_total.inc();
        if result.is_err() {
            self.action_failures_total.inc();
        }
    }

    /// Observe an early-rejection path (handler never invoked).
    ///
    /// Increments [`NEBULA_ACTION_DISPATCH_REJECTED_TOTAL`] with a
    /// `reason` label and nothing else. Does NOT touch the duration
    /// histogram, executions counter, or failures counter — those would
    /// skew downstream dashboards (#305).
    fn observe_rejected(&self, reason: &'static str) {
        let labels = self.metrics.interner().label_set(&[("reason", reason)]);
        match self
            .metrics
            .counter_labeled(NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, &labels)
        {
            Ok(c) => c.inc(),
            Err(err) => tracing::warn!(
                ?err,
                reason,
                "failed to record action dispatch rejected metric"
            ),
        }
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
    /// For each output slot that exceeds the limit, applies the configured
    /// strategy:
    /// - `Reject` → returns `DataLimitExceeded` on the first offender
    /// - `SpillToBlob` → for `ActionOutput::Value` only, writes the payload to blob storage and
    ///   rewrites the slot to an `ActionOutput::Reference` so the large inline value is no longer
    ///   carried downstream.
    ///
    /// `ActionOutput::Collection` is traversed recursively. `Binary` is
    /// measured with `effective_size()`, and `Reference` is measured by
    /// serialized metadata size.
    async fn enforce_data_limit(
        &self,
        action_key: &str,
        execution_id: ExecutionId,
        action_result: &mut ActionResult<serde_json::Value>,
        error_counter: &Counter,
    ) -> Result<(), RuntimeError> {
        let limit = self.data_policy.max_node_output_bytes;
        let total_limit = self.data_policy.max_total_execution_bytes;

        // Collect disjoint mut references to every leaf output slot in the result.
        // The Vec itself holds unique borrows of distinct struct fields, so
        // iterating and mutating each in turn is sound.
        let mut slots: Vec<&mut ActionOutput<serde_json::Value>> = Vec::new();
        collect_output_slots_mut(action_result, &mut slots);

        for slot in slots {
            let actual = match &*slot {
                ActionOutput::Value(v) => serde_json::to_vec(v)
                    .map_err(|e| {
                        RuntimeError::Internal(format!(
                            "failed to serialize output for size limit enforcement: {e}"
                        ))
                    })?
                    .len() as u64,
                ActionOutput::Binary(b) => b.effective_size(),
                ActionOutput::Reference(r) => serde_json::to_vec(r)
                    .map_err(|e| {
                        RuntimeError::Internal(format!(
                            "failed to serialize reference metadata for size limit enforcement: {e}"
                        ))
                    })?
                    .len() as u64,
                // Intentional size-0: Deferred carries retry config + resolution
                // metadata (no inline payload); the real payload is sized after
                // resolution at the resolution site. Empty has no payload by
                // definition.
                ActionOutput::Deferred(_) | ActionOutput::Empty => 0,
                ActionOutput::Collection(_) => 0, // collections are flattened before this loop
                _ => 0,
            };
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
                },
                LargeDataStrategy::SpillToBlob => {
                    let ActionOutput::Value(_) = &*slot else {
                        // Non-Value large payloads (Binary/Reference) cannot be
                        // rewritten to JSON blob references safely here.
                        error_counter.inc();
                        return Err(RuntimeError::DataLimitExceeded {
                            limit_bytes: limit,
                            actual_bytes: actual,
                        });
                    };
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
                    let serialized = serde_json::to_vec(match &*slot {
                        ActionOutput::Value(v) => v,
                        _ => unreachable!("guarded above"),
                    })
                    .map_err(|e| {
                        RuntimeError::Internal(format!(
                            "failed to serialize output for blob spill: {e}"
                        ))
                    })?;
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
                        },
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
                },
            }
        }

        // Enforce max total bytes across all nodes in this execution (issue #357).
        if total_limit > 0 {
            let mut slots_after: Vec<&mut ActionOutput<serde_json::Value>> = Vec::new();
            collect_output_slots_mut(action_result, &mut slots_after);
            let node_total: u64 = slots_after
                .iter()
                .map(|s| estimated_action_output_payload_bytes(s))
                .sum();

            use dashmap::mapref::entry::Entry;
            match self.execution_output_totals.entry(execution_id) {
                Entry::Occupied(mut o) => {
                    let new_total = *o.get() + node_total;
                    if new_total > total_limit {
                        error_counter.inc();
                        return Err(RuntimeError::DataLimitExceeded {
                            limit_bytes: total_limit,
                            actual_bytes: new_total,
                        });
                    }
                    *o.get_mut() = new_total;
                },
                Entry::Vacant(v) => {
                    if node_total > total_limit {
                        error_counter.inc();
                        return Err(RuntimeError::DataLimitExceeded {
                            limit_bytes: total_limit,
                            actual_bytes: node_total,
                        });
                    }
                    v.insert(node_total);
                },
            }
        }

        Ok(())
    }
}

/// Build a minimal [`NodeDefinition`] from an action key for synthetic
/// dispatch entry points (admin tooling, tests, top-level
/// `execute_action(_versioned|_with_checkpoint)`).
///
/// The synthesized node has no parameters, no slot bindings, and no rate
/// limit — it carries just enough metadata for
/// [`ActionFactory::instantiate`] to construct the action. Production
/// dispatch routes through [`ActionRuntime::execute_action_with_node`]
/// instead so the workflow node's `slot_bindings` reach the factory.
///
/// # Panics
///
/// Panics only if `action_key` is not a valid [`ActionKey`]. Production
/// callers parse the key separately and never reach this function with an
/// invalid key.
fn synthesize_node_definition(
    plugin_key: &str,
    action_key: &str,
    interface_version: Option<&semver::Version>,
) -> NodeDefinition {
    let mut node = NodeDefinition::new(
        nebula_core::NodeKey::new("synthetic_runtime_dispatch")
            .expect("synthetic node key is valid"),
        action_key.to_owned(),
        plugin_key,
        action_key,
    )
    .unwrap_or_else(|err| {
        // Caller should have validated the key already; surface the error
        // here as a clear panic rather than silently substituting another
        // key — the synthetic-node path is admin tooling/tests only.
        panic!("synthesize_node_definition: invalid action key '{action_key}': {err}");
    });
    node.interface_version = interface_version.cloned();
    node
}

/// Best-effort size of all payload bytes represented by an output slot after
/// per-node enforcement (including nested collections).
fn estimated_action_output_payload_bytes(slot: &ActionOutput<serde_json::Value>) -> u64 {
    match slot {
        ActionOutput::Value(v) => serde_json::to_vec(v).map_or(0, |b| b.len() as u64),
        ActionOutput::Binary(b) => b.effective_size(),
        ActionOutput::Reference(r) => r
            .size
            .unwrap_or_else(|| serde_json::to_vec(r).map_or(0, |b| b.len() as u64)),
        // Size-0 is intentional — same rationale as enforce_data_limit:
        // Deferred carries retry config + resolution metadata, not inline
        // payload bytes. The real payload is measured after resolution.
        ActionOutput::Deferred(_) => 0,
        ActionOutput::Collection(items) => items
            .iter()
            .map(estimated_action_output_payload_bytes)
            .sum(),
        ActionOutput::Empty => 0,
        _ => 0,
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
    fn collect_slot<'a>(
        slot: &'a mut ActionOutput<serde_json::Value>,
        out: &mut Vec<&'a mut ActionOutput<serde_json::Value>>,
    ) {
        match slot {
            ActionOutput::Collection(items) => {
                for item in &mut *items {
                    collect_slot(item, out);
                }
            },
            _ => out.push(slot),
        }
    }

    match result {
        ActionResult::Success { output } => collect_slot(output, out),
        ActionResult::Skip { output, .. } => {
            if let Some(o) = output.as_mut() {
                collect_slot(o, out);
            }
        },
        ActionResult::Continue { output, .. } => collect_slot(output, out),
        ActionResult::Break { output, .. } => collect_slot(output, out),
        ActionResult::Branch {
            output,
            alternatives,
            ..
        } => {
            collect_slot(output, out);
            for alt in alternatives.values_mut() {
                collect_slot(alt, out);
            }
        },
        ActionResult::Route { data, .. } => collect_slot(data, out),
        ActionResult::MultiOutput {
            outputs,
            main_output,
        } => {
            if let Some(m) = main_output.as_mut() {
                collect_slot(m, out);
            }
            for o in outputs.values_mut() {
                collect_slot(o, out);
            }
        },
        ActionResult::Wait { partial_output, .. } => {
            if let Some(o) = partial_output.as_mut() {
                collect_slot(o, out);
            }
        },
        // `ActionResult` is `#[non_exhaustive]`. Variants without a
        // downstream-visible payload (Retry, Drop, Terminate, and any
        // future additions) contribute nothing here — a slot they own
        // cannot bypass the limit because there is no slot.
        _ => {},
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use nebula_action::{
        ActionRuntimeContext, FromWorkflowNode, TriggerRuntimeContext, action::Action,
        context::CredentialContextExt, error::ActionError, metadata::ActionMetadata,
        stateful::StatefulAction, stateless::StatelessAction,
    };
    use nebula_core::{
        BaseContext, Dependencies, action_key,
        context::Context,
        id::{ExecutionId, WorkflowId},
        node_key,
    };

    use crate::runtime::runner::{ActionExecutor, InProcessRunner};

    use super::*;

    /// Echo fixture — Variant A unit struct. Per-test metadata is supplied
    /// via [`ActionRegistry::register_stateless_instance`] (the
    /// R-NEW-7 test escape), so the static `<Self as Action>::metadata()`
    /// is only consulted when the test escape is bypassed.
    struct EchoAction;

    impl Action for EchoAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(action_key!("test.echo.static"), "Echo", "echoes input")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl StatelessAction for EchoAction {
        async fn execute(
            &self,
            input: <Self as Action>::Input,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    struct FailAction;

    impl Action for FailAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(action_key!("test.fail.static"), "Fail", "always fails")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl StatelessAction for FailAction {
        async fn execute(
            &self,
            _input: <Self as Action>::Input,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
            Err(ActionError::retryable("transient failure"))
        }
    }

    fn test_context() -> ActionRuntimeContext {
        ActionRuntimeContext::new(
            Arc::new(BaseContext::builder().build()),
            ExecutionId::new(),
            node_key!("test"),
            WorkflowId::new(),
        )
    }

    fn test_trigger_context() -> TriggerRuntimeContext {
        TriggerRuntimeContext::new(
            Arc::new(BaseContext::builder().build()),
            WorkflowId::new(),
            node_key!("test"),
        )
    }

    fn make_runtime(registry: Arc<ActionRegistry>) -> ActionRuntime {
        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();

        ActionRuntime::try_new(registry, runner, DataPassingPolicy::default(), metrics).unwrap()
    }

    /// Build a runtime with a metrics registry we hand back to the caller,
    /// so tests can assert on counters/histograms that the runtime wrote
    /// through its private `metrics` field.
    fn make_runtime_with_metrics(
        registry: Arc<ActionRegistry>,
    ) -> (ActionRuntime, MetricsRegistry) {
        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();
        let rt = ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .unwrap();
        (rt, metrics)
    }

    #[tokio::test]
    async fn max_total_execution_bytes_across_dispatches() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(action_key!("test.echo"), "Echo", "echoes input"),
            EchoAction,
        );

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();
        let rt = ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy {
                max_node_output_bytes: 1024,
                max_total_execution_bytes: 10,
                ..Default::default()
            },
            metrics,
        )
        .unwrap();

        let eid = ExecutionId::new();
        let ctx = ActionRuntimeContext::new(
            Arc::new(BaseContext::builder().build()),
            eid,
            node_key!("test"),
            WorkflowId::new(),
        );

        rt.execute_action("test.echo", serde_json::json!(null), &ctx)
            .await
            .expect("first dispatch under total cap");

        let err = rt
            .execute_action("test.echo", serde_json::json!("1234567890"), &ctx)
            .await
            .expect_err("second dispatch exceeds max_total_execution_bytes");

        assert!(
            matches!(
                err,
                RuntimeError::DataLimitExceeded {
                    limit_bytes: 10,
                    ..
                }
            ),
            "expected total cap error, got {err:?}"
        );

        rt.clear_execution_output_totals(eid);
    }

    #[tokio::test]
    async fn execute_trusted_action() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(action_key!("test.echo"), "Echo", "echoes input"),
            EchoAction,
        );

        let rt = make_runtime(registry);
        let input = serde_json::json!({"hello": "world"});
        let result = rt
            .execute_action("test.echo", input.clone(), &test_context())
            .await;
        let action_result = result.unwrap();
        match action_result {
            ActionResult::Success { output } => {
                assert_eq!(output.as_value(), Some(&input));
            },
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_unknown_action_returns_error() {
        let registry = Arc::new(ActionRegistry::new());
        let rt = make_runtime(registry);
        let result = rt
            .execute_action("nonexistent", serde_json::json!(null), &test_context())
            .await;
        assert!(matches!(result, Err(RuntimeError::ActionNotFound { .. })));
    }

    #[tokio::test]
    async fn execute_failing_action_propagates_error() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(action_key!("test.fail"), "Fail", "always fails"),
            FailAction,
        );

        let rt = make_runtime(registry);
        let result = rt
            .execute_action("test.fail", serde_json::json!(null), &test_context())
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().is_retryable());
    }

    #[tokio::test]
    async fn data_limit_enforcement() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(action_key!("test.big"), "Big", "returns big output"),
            EchoAction,
        );

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();

        let rt = ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy {
                max_node_output_bytes: 5, // very small
                ..Default::default()
            },
            metrics,
        )
        .unwrap();

        let input = serde_json::json!({"big_payload": "this is way too large for 5 bytes"});
        let result = rt.execute_action("test.big", input, &test_context()).await;
        assert!(matches!(
            result,
            Err(RuntimeError::DataLimitExceeded { .. })
        ));
    }

    #[tokio::test]
    async fn metrics_recorded_on_execution() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(action_key!("test.tele"), "Tele", "test"),
            EchoAction,
        );

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();

        let rt = ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .unwrap();

        rt.execute_action("test.tele", serde_json::json!("ok"), &test_context())
            .await
            .unwrap();

        // Metrics should be recorded.
        assert_eq!(
            metrics
                .counter(NEBULA_ACTION_EXECUTIONS_TOTAL)
                .unwrap()
                .get(),
            1
        );
        assert_eq!(
            metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).unwrap().get(),
            0
        );
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
            ctx.emit_execution(serde_json::json!({"tick": true}), None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn execute_uses_runner_for_capability_gated() {
        use std::sync::atomic::{AtomicBool, Ordering};

        // Track whether the runner was invoked.
        let runner_called = Arc::new(AtomicBool::new(false));
        let runner_called_clone = runner_called.clone();

        let executor: ActionExecutor = Arc::new(move |_ctx, _meta, input| {
            runner_called_clone.store(true, Ordering::SeqCst);
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(action_key!("test.gated"), "Gated", "capability gated")
                .with_isolation_level(IsolationLevel::CapabilityGated),
            EchoAction,
        );

        let metrics = MetricsRegistry::new();
        let rt = ActionRuntime::try_new(registry, runner, DataPassingPolicy::default(), metrics)
            .unwrap();

        let result = rt
            .execute_action(
                "test.gated",
                serde_json::json!({"data": 1}),
                &test_context(),
            )
            .await;
        assert!(result.is_ok());
        assert!(
            runner_called.load(Ordering::SeqCst),
            "runner should have been called for CapabilityGated action"
        );
    }

    #[tokio::test]
    async fn spill_to_blob_rejects_when_no_storage() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(action_key!("test.spill"), "Spill", "large output"),
            EchoAction,
        );

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();

        let rt = ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy {
                max_node_output_bytes: 5,
                large_data_strategy: LargeDataStrategy::SpillToBlob,
                ..Default::default()
            },
            metrics,
        )
        .unwrap();

        // No blob storage configured -- should reject.
        let input = serde_json::json!({"big": "this exceeds 5 bytes easily"});
        let result = rt
            .execute_action("test.spill", input, &test_context())
            .await;
        assert!(
            matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
            "expected DataLimitExceeded when no blob storage configured"
        );
    }

    #[tokio::test]
    async fn spill_to_blob_succeeds_with_storage() {
        use super::super::blob::{BlobRef, BlobStorage};

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
        registry.register_stateless_instance(
            ActionMetadata::new(
                action_key!("test.spill_ok"),
                "SpillOk",
                "large output with storage",
            ),
            EchoAction,
        );

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();

        let rt = ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy {
                max_node_output_bytes: 5,
                large_data_strategy: LargeDataStrategy::SpillToBlob,
                ..Default::default()
            },
            metrics,
        )
        .unwrap()
        .with_blob_storage(Arc::new(FakeBlobStorage));

        let input = serde_json::json!({"big": "this exceeds 5 bytes easily"});
        let result = rt
            .execute_action("test.spill_ok", input, &test_context())
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
            },
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

        struct MultiOutAction;
        impl Action for MultiOutAction {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new(
                    action_key!("test.multi_out.static"),
                    "MultiOut",
                    "multi-port fan-out",
                )
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }
        impl StatelessAction for MultiOutAction {
            async fn execute(
                &self,
                _input: <Self as Action>::Input,
                _ctx: &(impl ActionContext + ?Sized),
            ) -> Result<AR<<Self as Action>::Output>, ActionError> {
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
        registry.register_stateless_instance(
            ActionMetadata::new(
                action_key!("test.multi_out"),
                "MultiOut",
                "multi-port fan-out",
            ),
            MultiOutAction,
        );

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();
        let rt = ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy {
                max_node_output_bytes: 16,
                large_data_strategy: LargeDataStrategy::Reject,
                ..Default::default()
            },
            metrics,
        )
        .unwrap();

        let result = rt
            .execute_action("test.multi_out", serde_json::json!(null), &test_context())
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

        struct BranchAction;
        impl Action for BranchAction {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new(action_key!("test.branch.static"), "Branch", "static")
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }
        impl StatelessAction for BranchAction {
            async fn execute(
                &self,
                _input: <Self as Action>::Input,
                _ctx: &(impl ActionContext + ?Sized),
            ) -> Result<AR<<Self as Action>::Output>, ActionError> {
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
        registry.register_stateless_instance(
            ActionMetadata::new(action_key!("test.branch"), "Branch", "branch with alts"),
            BranchAction,
        );

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();
        let rt = ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy {
                max_node_output_bytes: 16,
                large_data_strategy: LargeDataStrategy::Reject,
                ..Default::default()
            },
            metrics,
        )
        .unwrap();

        let result = rt
            .execute_action("test.branch", serde_json::json!(null), &test_context())
            .await;
        assert!(
            matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
            "Branch.alternatives must not bypass the data-passing limit — got {result:?}"
        );
    }

    #[tokio::test]
    async fn collection_children_respect_reject_limit() {
        use nebula_action::result::ActionResult as AR;

        struct CollectionAction;
        impl Action for CollectionAction {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new(action_key!("test.collection.static"), "Coll", "static")
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }
        impl StatelessAction for CollectionAction {
            async fn execute(
                &self,
                _input: <Self as Action>::Input,
                _ctx: &(impl ActionContext + ?Sized),
            ) -> Result<AR<<Self as Action>::Output>, ActionError> {
                Ok(AR::Success {
                    output: ActionOutput::Collection(vec![
                        ActionOutput::Value(serde_json::json!("ok")),
                        ActionOutput::Value(serde_json::json!(
                            "this payload is larger than 16 bytes"
                        )),
                    ]),
                })
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(
                action_key!("test.collection"),
                "Collection",
                "nested values",
            ),
            CollectionAction,
        );
        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();
        let rt = ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy {
                max_node_output_bytes: 16,
                large_data_strategy: LargeDataStrategy::Reject,
                ..Default::default()
            },
            metrics,
        )
        .unwrap();

        let result = rt
            .execute_action("test.collection", serde_json::json!(null), &test_context())
            .await;
        assert!(
            matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
            "nested collection values must not bypass the data-passing limit — got {result:?}"
        );
    }

    #[tokio::test]
    async fn binary_inline_respects_reject_limit() {
        use nebula_action::{
            output::{BinaryData, BinaryStorage},
            result::ActionResult as AR,
        };

        struct BinaryAction;
        impl Action for BinaryAction {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new(action_key!("test.binary.static"), "Bin", "static")
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }
        impl StatelessAction for BinaryAction {
            async fn execute(
                &self,
                _input: <Self as Action>::Input,
                _ctx: &(impl ActionContext + ?Sized),
            ) -> Result<AR<<Self as Action>::Output>, ActionError> {
                Ok(AR::Success {
                    output: ActionOutput::Binary(BinaryData {
                        content_type: "application/octet-stream".to_owned(),
                        data: BinaryStorage::Inline(vec![0_u8; 64]),
                        size: 1, // intentionally wrong; effective_size() must win
                        metadata: None,
                    }),
                })
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(action_key!("test.binary"), "Binary", "inline bytes"),
            BinaryAction,
        );
        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();
        let rt = ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy {
                max_node_output_bytes: 16,
                large_data_strategy: LargeDataStrategy::Reject,
                ..Default::default()
            },
            metrics,
        )
        .unwrap();

        let result = rt
            .execute_action("test.binary", serde_json::json!(null), &test_context())
            .await;
        assert!(
            matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
            "inline binary output must be checked via effective_size() — got {result:?}"
        );
    }

    #[tokio::test]
    async fn reference_metadata_respects_reject_limit() {
        use nebula_action::result::ActionResult as AR;

        struct RefAction;
        impl Action for RefAction {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new(action_key!("test.ref.static"), "Ref", "static")
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }
        impl StatelessAction for RefAction {
            async fn execute(
                &self,
                _input: <Self as Action>::Input,
                _ctx: &(impl ActionContext + ?Sized),
            ) -> Result<AR<<Self as Action>::Output>, ActionError> {
                Ok(AR::Success {
                    output: ActionOutput::Reference(DataReference {
                        storage_type: "blob".to_owned(),
                        path: "x".repeat(128),
                        size: Some(1),
                        content_type: Some("application/json".to_owned()),
                    }),
                })
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(action_key!("test.ref"), "Reference", "large metadata"),
            RefAction,
        );
        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();
        let rt = ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy {
                max_node_output_bytes: 32,
                large_data_strategy: LargeDataStrategy::Reject,
                ..Default::default()
            },
            metrics,
        )
        .unwrap();

        let result = rt
            .execute_action("test.ref", serde_json::json!(null), &test_context())
            .await;
        assert!(
            matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
            "reference metadata must be included in size enforcement — got {result:?}"
        );
    }

    // ── #305 regression: dispatch-rejection paths do not skew histogram ─────

    /// Register an action that resolves to a kind not executable via
    /// `ActionRuntime` — trigger or resource — and assert that `run_factory`
    /// does not record duration samples or bump the executions / failures
    /// counters. Instead the dispatch-rejected counter increments once with the
    /// correct reason label.
    #[tokio::test]
    async fn trigger_rejection_does_not_observe_histogram() {
        use nebula_action::{FromWorkflowNode, TriggerAction, TriggerEventOutcome, TriggerSource};

        // Minimal TriggerAction fixture — never invoked, only its ActionHandle
        // variant matters for the rejection test.
        struct FakeTrigger;
        struct FakeTriggerSource;
        impl TriggerSource for FakeTriggerSource {
            type Event = ();
        }

        impl Action for FakeTrigger {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new(
                    action_key!("test.trigger_reject"),
                    "FakeTrigger",
                    "rejection fixture",
                )
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }

        impl TriggerAction for FakeTrigger {
            type Source = FakeTriggerSource;
            type Error = ActionError;

            async fn start(
                &self,
                _ctx: &(impl nebula_action::TriggerContext + ?Sized),
            ) -> Result<(), Self::Error> {
                Ok(())
            }

            async fn stop(
                &self,
                _ctx: &(impl nebula_action::TriggerContext + ?Sized),
            ) -> Result<(), Self::Error> {
                Ok(())
            }

            async fn handle(
                &self,
                _ctx: &(impl nebula_action::TriggerContext + ?Sized),
                _event: (),
            ) -> Result<TriggerEventOutcome, Self::Error> {
                Err(ActionError::fatal(
                    "trigger does not accept external events",
                ))
            }
        }

        impl FromWorkflowNode for FakeTrigger {
            type Error = ActionError;

            async fn from_workflow_node(
                _node: &NodeDefinition,
                _ctx: &dyn ActionContext,
            ) -> Result<Self, Self::Error> {
                Ok(FakeTrigger)
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        registry.register_trigger_factory::<FakeTrigger>();
        let (rt, metrics) = make_runtime_with_metrics(registry);

        let result = rt
            .execute_action(
                "test.trigger_reject",
                serde_json::json!(null),
                &test_context(),
            )
            .await;
        assert!(
            matches!(result, Err(RuntimeError::TriggerNotExecutable { .. })),
            "expected TriggerNotExecutable, got {result:?}"
        );

        // Histogram and execution/failure counters must NOT observe this path.
        assert_eq!(
            metrics
                .histogram(NEBULA_ACTION_DURATION_SECONDS)
                .unwrap()
                .count(),
            0,
            "duration histogram must not sample rejection paths"
        );
        assert_eq!(
            metrics
                .counter(NEBULA_ACTION_EXECUTIONS_TOTAL)
                .unwrap()
                .get(),
            0,
            "executions counter must not bump on rejection"
        );
        assert_eq!(
            metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).unwrap().get(),
            0,
            "failures counter must not bump on rejection"
        );

        // Dispatch-rejected counter MUST be labelled and bumped exactly once.
        let labels = metrics
            .interner()
            .label_set(&[("reason", dispatch_reject_reason::TRIGGER_NOT_EXECUTABLE)]);
        assert_eq!(
            metrics
                .counter_labeled(NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, &labels)
                .unwrap()
                .get(),
            1,
            "dispatch-rejected counter should be bumped once with reason=trigger_not_executable"
        );
    }

    #[tokio::test]
    async fn resource_rejection_does_not_increment_execution_metrics() {
        use nebula_action::{FromWorkflowNode, ResourceAction};

        // Minimal ResourceAction fixture — never invoked, only its ActionHandle
        // variant matters for the rejection test.
        struct FakeResource;

        impl Action for FakeResource {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new(
                    action_key!("test.resource_reject"),
                    "FakeResource",
                    "rejection fixture",
                )
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }

        impl ResourceAction for FakeResource {
            type Resource = serde_json::Value;

            async fn configure(
                &self,
                _ctx: &(impl ActionContext + ?Sized),
            ) -> Result<Self::Resource, ActionError> {
                Ok(serde_json::json!(null))
            }

            async fn cleanup(
                &self,
                _resource: Self::Resource,
                _ctx: &(impl ActionContext + ?Sized),
            ) -> Result<(), ActionError> {
                Ok(())
            }
        }

        impl FromWorkflowNode for FakeResource {
            type Error = ActionError;

            async fn from_workflow_node(
                _node: &NodeDefinition,
                _ctx: &dyn ActionContext,
            ) -> Result<Self, Self::Error> {
                Ok(FakeResource)
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        registry.register_resource_factory::<FakeResource>();
        let (rt, metrics) = make_runtime_with_metrics(registry);

        let result = rt
            .execute_action(
                "test.resource_reject",
                serde_json::json!(null),
                &test_context(),
            )
            .await;
        assert!(
            matches!(result, Err(RuntimeError::ResourceNotExecutable { .. })),
            "expected ResourceNotExecutable, got {result:?}"
        );

        assert_eq!(
            metrics
                .histogram(NEBULA_ACTION_DURATION_SECONDS)
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            metrics
                .counter(NEBULA_ACTION_EXECUTIONS_TOTAL)
                .unwrap()
                .get(),
            0
        );
        assert_eq!(
            metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).unwrap().get(),
            0
        );

        let labels = metrics
            .interner()
            .label_set(&[("reason", dispatch_reject_reason::RESOURCE_NOT_EXECUTABLE)]);
        assert_eq!(
            metrics
                .counter_labeled(NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, &labels)
                .unwrap()
                .get(),
            1
        );
    }

    /// Counterpart to the rejection test: a successful stateless dispatch
    /// must observe the histogram and bump the executions counter. Pin
    /// both so the rejection fix does not regress the dispatched path.
    #[tokio::test]
    async fn dispatched_stateless_observes_histogram_and_counter() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(action_key!("test.dispatched"), "Disp", "dispatched"),
            EchoAction,
        );
        let (rt, metrics) = make_runtime_with_metrics(registry);

        rt.execute_action("test.dispatched", serde_json::json!("ok"), &test_context())
            .await
            .expect("dispatched execution must succeed");
        assert_eq!(
            metrics
                .histogram(NEBULA_ACTION_DURATION_SECONDS)
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            metrics
                .counter(NEBULA_ACTION_EXECUTIONS_TOTAL)
                .unwrap()
                .get(),
            1
        );
        assert_eq!(
            metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).unwrap().get(),
            0
        );

        let labels = metrics
            .interner()
            .label_set(&[("reason", dispatch_reject_reason::TRIGGER_NOT_EXECUTABLE)]);
        assert_eq!(
            metrics
                .counter_labeled(NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, &labels)
                .unwrap()
                .get(),
            0,
            "dispatch-rejected counter must stay at zero for successful dispatch"
        );
    }

    // ── Stream action dispatch ───────────────────────────────────────────────

    /// Prove the end-to-end stream dispatch path:
    /// register via `register_stream_factory` → execute → folded value reaches
    /// the result. If the `ActionHandle::Stream` arm in `dispatch_action` is
    /// reverted, this test goes red (the action would hit `_ => UNKNOWN_VARIANT`
    /// and return `RuntimeError::Internal`).
    #[tokio::test]
    async fn stream_action_dispatch_yields_folded_value() {
        use futures::stream;
        use nebula_action::{FromWorkflowNode, stream::StreamAction};

        struct CountingStream;

        impl Action for CountingStream {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new(
                    action_key!("test.stream.counting"),
                    "CountingStream",
                    "yields 1,2,3 and sums",
                )
            }

            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }

        impl StreamAction for CountingStream {
            type Chunk = u64;

            fn open_stream(
                &self,
                _input: serde_json::Value,
                _ctx: &(impl ActionContext + ?Sized),
            ) -> impl futures::Stream<Item = Result<u64, ActionError>> + Send {
                stream::iter([Ok(1u64), Ok(2), Ok(3)])
            }

            fn init(&self) -> serde_json::Value {
                serde_json::json!(0u64)
            }

            fn fold(&self, acc: serde_json::Value, chunk: u64) -> serde_json::Value {
                let running = acc.as_u64().unwrap_or(0);
                serde_json::json!(running + chunk)
            }
        }

        impl FromWorkflowNode for CountingStream {
            type Error = ActionError;

            async fn from_workflow_node(
                _node: &NodeDefinition,
                _ctx: &dyn ActionContext,
            ) -> Result<Self, Self::Error> {
                Ok(CountingStream)
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stream_factory::<CountingStream>();
        let rt = make_runtime(registry);

        let result = rt
            .execute_action(
                "test.stream.counting",
                serde_json::json!(null),
                &test_context(),
            )
            .await
            .expect("stream dispatch must succeed");

        match result {
            ActionResult::Success { output } => {
                let value = output.into_value().expect("output must be inline Value");
                assert_eq!(value, serde_json::json!(6u64), "1+2+3 must fold to 6");
            },
            other => panic!("expected Success, got {other:?}"),
        }
    }

    /// D-2 regression: a stream-produced payload that exceeds the per-node
    /// limit must be rejected (or spilled). The folded Value goes through
    /// `ActionOutput::Value`, which IS measured by `enforce_data_limit`.
    /// This test proves the D-2 path is not bypassed by the new kind.
    #[tokio::test]
    async fn stream_output_respects_data_limit() {
        use futures::stream;
        use nebula_action::{FromWorkflowNode, stream::StreamAction};

        struct BigStream;

        impl Action for BigStream {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new(
                    action_key!("test.stream.big"),
                    "BigStream",
                    "produces an oversized folded value",
                )
            }

            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }

        impl StreamAction for BigStream {
            type Chunk = String;

            fn open_stream(
                &self,
                _input: serde_json::Value,
                _ctx: &(impl ActionContext + ?Sized),
            ) -> impl futures::Stream<Item = Result<String, ActionError>> + Send {
                // One chunk whose folded form exceeds any tiny limit.
                stream::iter([Ok("x".repeat(1024))])
            }

            fn init(&self) -> serde_json::Value {
                serde_json::json!("")
            }

            fn fold(&self, _acc: serde_json::Value, chunk: String) -> serde_json::Value {
                serde_json::json!(chunk)
            }
        }

        impl FromWorkflowNode for BigStream {
            type Error = ActionError;

            async fn from_workflow_node(
                _node: &NodeDefinition,
                _ctx: &dyn ActionContext,
            ) -> Result<Self, Self::Error> {
                Ok(BigStream)
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        registry.register_stream_factory::<BigStream>();

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();
        let rt = ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy {
                max_node_output_bytes: 10, // far below the 1024-char chunk
                ..Default::default()
            },
            metrics,
        )
        .unwrap();

        let err = rt
            .execute_action("test.stream.big", serde_json::json!(null), &test_context())
            .await
            .expect_err("oversized stream output must be rejected");

        assert!(
            matches!(err, RuntimeError::DataLimitExceeded { .. }),
            "expected DataLimitExceeded, got {err:?}"
        );
    }

    // ── #304 + #308 regression: stateful cancel + checkpoint ────────────────

    use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

    use serde_json::Value as JsonValue;
    use tokio::sync::Mutex as TokioMutex;

    // ── Shared counting logic used by multiple fixtures ─────────────────────

    fn counting_step(state: &mut JsonValue, target: u32) -> ActionResult<JsonValue> {
        let count = state
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        let next = count + 1;
        *state = serde_json::json!({ "count": next });
        if next >= target {
            ActionResult::Break {
                output: ActionOutput::Value(serde_json::json!({ "final": next })),
                reason: nebula_action::result::BreakReason::Completed,
            }
        } else {
            ActionResult::Continue {
                output: ActionOutput::Value(serde_json::json!({ "step": next })),
                progress: None,
                delay: None,
            }
        }
    }

    // ── CountingTo3 — #308 checkpoint test (3-iteration break) ───────────────

    /// Counts `state.count` from 0 to 3; used by checkpoint + resume tests.
    struct CountingTo3;

    impl Action for CountingTo3 {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(action_key!("test.count"), "CountTo3", "counts to 3")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }
    impl StatefulAction for CountingTo3 {
        type State = JsonValue;
        fn init_state(&self) -> Self::State {
            serde_json::json!({ "count": 0u32 })
        }
        async fn execute(
            &self,
            _input: Self::Input,
            state: &mut Self::State,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(counting_step(state, 3))
        }
    }
    impl FromWorkflowNode for CountingTo3 {
        type Error = ActionError;
        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(CountingTo3)
        }
    }

    // ── CountingTo5 — #308 resume test (5-iteration break) ───────────────────

    struct CountingTo5;

    impl Action for CountingTo5 {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(action_key!("test.count5"), "CountTo5", "counts to 5")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }
    impl StatefulAction for CountingTo5 {
        type State = JsonValue;
        fn init_state(&self) -> Self::State {
            serde_json::json!({ "count": 0u32 })
        }
        async fn execute(
            &self,
            _input: Self::Input,
            state: &mut Self::State,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(counting_step(state, 5))
        }
    }
    impl FromWorkflowNode for CountingTo5 {
        type Error = ActionError;
        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(CountingTo5)
        }
    }

    // ── CountingTo2 — #308 resume-from-checkpoint test (breaks at 2) ─────────

    struct CountingTo2;

    impl Action for CountingTo2 {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(action_key!("test.count2"), "CountTo2", "counts to 2")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }
    impl StatefulAction for CountingTo2 {
        type State = JsonValue;
        fn init_state(&self) -> Self::State {
            serde_json::json!({ "count": 0u32 })
        }
        async fn execute(
            &self,
            _input: Self::Input,
            state: &mut Self::State,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(counting_step(state, 2))
        }
    }
    impl FromWorkflowNode for CountingTo2 {
        type Error = ActionError;
        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(CountingTo2)
        }
    }

    // ── SleepyStateful — #304 cancel-aborts-handler test ─────────────────────

    /// Awaits a 1-hour sleep inside `execute`; used to prove cancellation aborts it.
    struct SleepyStateful;

    impl Action for SleepyStateful {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(
                action_key!("test.sleepy"),
                "SleepyStateful",
                "hangs in execute",
            )
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }
    impl StatefulAction for SleepyStateful {
        type State = JsonValue;
        fn init_state(&self) -> Self::State {
            serde_json::json!({})
        }
        async fn execute(
            &self,
            _input: Self::Input,
            _state: &mut Self::State,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            tokio::time::sleep(std::time::Duration::from_hours(1)).await;
            Ok(ActionResult::Break {
                output: ActionOutput::Value(serde_json::json!(null)),
                reason: nebula_action::result::BreakReason::Completed,
            })
        }
    }
    impl FromWorkflowNode for SleepyStateful {
        type Error = ActionError;
        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(SleepyStateful)
        }
    }

    /// Recording sink — stores every save/clear call so tests can assert
    /// on the exact sequence of checkpoint operations.
    #[derive(Default)]
    struct RecordingSink {
        preload: std::sync::Mutex<Option<StatefulCheckpoint>>,
        saves: TokioMutex<Vec<StatefulCheckpoint>>,
        clears: AtomicU32,
        fail_load: std::sync::atomic::AtomicBool,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self::default()
        }
        fn with_preload(cp: StatefulCheckpoint) -> Self {
            let s = Self::default();
            *s.preload.lock().unwrap() = Some(cp);
            s
        }
        fn with_failing_load() -> Self {
            let s = Self::default();
            s.fail_load
                .store(true, std::sync::atomic::Ordering::Relaxed);
            s
        }
    }

    #[async_trait::async_trait]
    impl StatefulCheckpointSink for RecordingSink {
        async fn load(&self) -> Result<Option<StatefulCheckpoint>, ActionError> {
            if self.fail_load.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(ActionError::fatal("simulated checkpoint load failure"));
            }
            Ok(self.preload.lock().unwrap().clone())
        }
        async fn save(&self, cp: &StatefulCheckpoint) -> Result<(), ActionError> {
            self.saves.lock().await.push(cp.clone());
            Ok(())
        }
        async fn clear(&self) -> Result<(), ActionError> {
            self.clears.fetch_add(1, AtomicOrdering::Relaxed);
            Ok(())
        }
    }

    /// #304 regression: a stateful action that awaits a 1-hour sleep inside
    /// `execute` must abort the moment the cancellation token fires — not
    /// 1 hour later. Uses `start_paused = true` so the sleep never
    /// naturally advances.
    #[tokio::test(start_paused = true)]
    async fn execute_stateful_aborts_handler_on_cancel() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateful_factory::<SleepyStateful>();
        let rt = Arc::new(make_runtime(registry));

        let ctx = test_context();
        let cancel = ctx.cancellation().clone();

        // Dispatch on a task so we can cancel after 10ms of virtual time.
        let rt_clone = Arc::clone(&rt);
        let handle = tokio::spawn(async move {
            rt_clone
                .execute_action("test.sleepy", serde_json::json!(null), &ctx)
                .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        cancel.cancel();

        // Use a bounded timeout so a broken fix presents as a hang, not a
        // success. 500ms of virtual time is three orders of magnitude
        // less than the handler's 1-hour sleep.
        let result = tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("execute_stateful must observe cancel inside handler.execute()")
            .expect("task panicked");
        assert!(
            matches!(
                result,
                Err(RuntimeError::ActionError(ActionError::Cancelled))
            ),
            "expected ActionError::Cancelled, got {result:?}"
        );
    }

    /// #308 regression: every iteration boundary is checkpointed.
    /// Counting 0→3 produces two `save()` calls (at iterations 1 and 2)
    /// and one `clear()` call on the terminal `Break` at iteration 3.
    #[tokio::test]
    async fn execute_stateful_checkpoints_each_iteration() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateful_factory::<CountingTo3>();
        let rt = make_runtime(registry);

        let sink = Arc::new(RecordingSink::new());
        let result = rt
            .execute_action_with_checkpoint(
                "test.count",
                None,
                serde_json::json!(null),
                &test_context(),
                Some(Arc::clone(&sink) as Arc<dyn StatefulCheckpointSink>),
            )
            .await;
        assert!(
            matches!(result, Ok(ActionResult::Break { .. })),
            "{result:?}"
        );

        let saves = sink.saves.lock().await;
        assert_eq!(
            saves.len(),
            2,
            "expected 2 saves (iterations 1 and 2), got {:?}",
            *saves
        );
        assert_eq!(saves[0].iteration, 1);
        assert_eq!(saves[0].state, serde_json::json!({"count": 1u32}));
        assert_eq!(saves[1].iteration, 2);
        assert_eq!(saves[1].state, serde_json::json!({"count": 2u32}));
        assert_eq!(
            sink.clears.load(AtomicOrdering::Relaxed),
            1,
            "expected exactly one clear() on terminal iteration"
        );
    }

    /// #308 regression: seeding the sink with a checkpoint at iteration 3
    /// must make the handler visibly resume from `count=3`, not 0.
    /// Counting to 5 from a checkpoint of 3 is 2 more iterations: one
    /// `Continue` at 4 (save), one `Break` at 5 (clear).
    #[tokio::test]
    async fn execute_stateful_resumes_from_checkpoint() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateful_factory::<CountingTo5>();
        let rt = make_runtime(registry);

        let seed = StatefulCheckpoint::new(3, serde_json::json!({ "count": 3u32 }));
        let sink = Arc::new(RecordingSink::with_preload(seed));

        let result = rt
            .execute_action_with_checkpoint(
                "test.count5",
                None,
                serde_json::json!(null),
                &test_context(),
                Some(Arc::clone(&sink) as Arc<dyn StatefulCheckpointSink>),
            )
            .await
            .expect("execute should succeed");
        match result {
            ActionResult::Break { output, .. } => {
                assert_eq!(output.as_value(), Some(&serde_json::json!({"final": 5u32})));
            },
            other => panic!("expected Break, got {other:?}"),
        }

        let saves = sink.saves.lock().await;
        assert_eq!(
            saves.len(),
            1,
            "only one Continue should have happened on resume, got {:?}",
            *saves
        );
        assert_eq!(saves[0].iteration, 4);
        assert_eq!(sink.clears.load(AtomicOrdering::Relaxed), 1);
    }

    /// #308 gotcha regression: a checkpoint sink that fails `load()` must
    /// still complete via fallback to `init_state`. This test pins the
    /// functional fallback path and checkpoint side effects.
    #[tokio::test]
    async fn execute_stateful_load_failure_falls_back_to_init_state() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateful_factory::<CountingTo2>();
        let rt = make_runtime(registry);

        let sink = Arc::new(RecordingSink::with_failing_load());
        let result = rt
            .execute_action_with_checkpoint(
                "test.count2",
                None,
                serde_json::json!(null),
                &test_context(),
                Some(Arc::clone(&sink) as Arc<dyn StatefulCheckpointSink>),
            )
            .await
            .expect("fallback must still run the action to completion");

        match result {
            ActionResult::Break { output, .. } => {
                assert_eq!(output.as_value(), Some(&serde_json::json!({"final": 2u32})));
            },
            other => panic!("expected Break, got {other:?}"),
        }
        // One Continue at 1, one Break at 2, starting from init_state.
        let saves = sink.saves.lock().await;
        assert_eq!(saves.len(), 1, "expected one save at iteration 1");
        assert_eq!(saves[0].iteration, 1);
        assert_eq!(sink.clears.load(AtomicOrdering::Relaxed), 1);
    }

    // ── NoProgressStateful — spec 28 stuck-state guard ───────────────────────

    /// Returns `Continue` on every iteration without mutating state — pins the
    /// spec 28 stuck-state guard (a `Continue` with byte-identical state must
    /// surface as `RuntimeError::StatefulStuck`).
    struct NoProgressStateful;

    impl Action for NoProgressStateful {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(
                action_key!("test.stuck"),
                "NoProgress",
                "never advances state",
            )
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }
    impl StatefulAction for NoProgressStateful {
        type State = JsonValue;
        fn init_state(&self) -> Self::State {
            serde_json::json!({ "cursor": 0u32 })
        }
        async fn execute(
            &self,
            _input: Self::Input,
            _state: &mut Self::State,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::Continue {
                output: ActionOutput::Value(serde_json::json!(null)),
                progress: None,
                delay: None,
            })
        }
    }
    impl FromWorkflowNode for NoProgressStateful {
        type Error = ActionError;
        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(NoProgressStateful)
        }
    }

    /// Spec 28: a stateful action that Continues without mutating its state
    /// must surface as a typed `RuntimeError::StatefulStuck`, NOT as an opaque
    /// `ActionError::Fatal`. Retry/error routing depends on the typed
    /// classification.
    #[tokio::test]
    async fn execute_stateful_stuck_surfaces_typed_variant() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateful_factory::<NoProgressStateful>();
        let rt = make_runtime(registry);

        let result = rt
            .execute_action("test.stuck", serde_json::json!(null), &test_context())
            .await;

        match result {
            Err(RuntimeError::StatefulStuck {
                action_key,
                iteration,
                ..
            }) => {
                assert_eq!(action_key.as_str(), "test.stuck");
                assert_eq!(iteration, 1, "stall detected on the first Continue");
            },
            other => panic!("expected RuntimeError::StatefulStuck, got {other:?}"),
        }
    }

    // ── EndlessStateful — iteration-cap test ─────────────────────────────────

    /// Advances `state.count` on every iteration but never breaks — exercises
    /// the iteration cap without tripping the stuck-state guard.
    struct EndlessStateful;

    impl Action for EndlessStateful {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(
                action_key!("test.endless"),
                "EndlessStateful",
                "never breaks",
            )
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }
    impl StatefulAction for EndlessStateful {
        type State = JsonValue;
        fn init_state(&self) -> Self::State {
            serde_json::json!({ "count": 0u32 })
        }
        async fn execute(
            &self,
            _input: Self::Input,
            state: &mut Self::State,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            let count = state
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as u32;
            *state = serde_json::json!({ "count": count + 1 });
            Ok(ActionResult::Continue {
                output: ActionOutput::Value(serde_json::json!({ "n": count + 1 })),
                progress: None,
                delay: None,
            })
        }
    }
    impl FromWorkflowNode for EndlessStateful {
        type Error = ActionError;
        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(EndlessStateful)
        }
    }

    /// A stateful action whose state evolves every iteration must still be
    /// capped at `MAX_ITERATIONS` and surface as a typed
    /// `RuntimeError::IterationCapExceeded` — not a generic action fatal.
    #[tokio::test(flavor = "current_thread")]
    async fn execute_stateful_iteration_cap_surfaces_typed_variant() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateful_factory::<EndlessStateful>();
        let rt = make_runtime(registry);

        let result = rt
            .execute_action("test.endless", serde_json::json!(null), &test_context())
            .await;

        match result {
            Err(RuntimeError::IterationCapExceeded {
                action_key, cap, ..
            }) => {
                assert_eq!(action_key.as_str(), "test.endless");
                assert_eq!(cap, 10_000);
            },
            other => panic!("expected RuntimeError::IterationCapExceeded, got {other:?}"),
        }
    }
}
