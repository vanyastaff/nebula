//! Action runtime -- the main execution orchestrator.
//!
//! Resolves actions from the registry, executes them through the sandbox,
//! enforces data limits, and records metrics.

use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use nebula_action::{
    ActionContext, ActionError, ActionHandler, ActionMetadata, IsolationLevel, StatefulHandler,
    StatelessHandler,
    output::{ActionOutput, DataReference},
    result::ActionResult,
};
use nebula_core::ExecutionId;
use nebula_metrics::naming::{
    NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, NEBULA_ACTION_DURATION_SECONDS,
    NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_ACTION_FAILURES_TOTAL, dispatch_reject_reason,
};
use nebula_sandbox::{SandboxRunner, SandboxedContext};
use nebula_telemetry::metrics::{Counter, Histogram, MetricsRegistry};
use serde::{Deserialize, Serialize};

use crate::{
    blob::BlobStorage,
    data_policy::{DataPassingPolicy, LargeDataStrategy},
    error::RuntimeError,
    registry::ActionRegistry,
};

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
    /// Sum of estimated output bytes per execution for
    /// [`DataPassingPolicy::max_total_execution_bytes`].
    execution_output_totals: Arc<DashMap<ExecutionId, u64>>,
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
            execution_output_totals: Arc::new(DashMap::new()),
        }
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
        context: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        self.execute_action_with_checkpoint(action_key, version, input, context, None)
            .await
    }

    /// Execute an action by key, with an optional stateful checkpoint sink.
    ///
    /// Same shape as [`Self::execute_action_versioned`] but also accepts a
    /// [`StatefulCheckpointSink`]. The sink is consulted only when the
    /// resolved handler is `ActionHandler::Stateful`:
    ///
    /// - Before the iteration loop, `sink.load()` is called. A `Some` checkpoint resumes from the
    ///   last persisted `(iteration, state)`; `None` falls through to `handler.init_state()`.
    /// - After every successful `Continue`, `sink.save(..)` persists the mutated state and
    ///   iteration counter before looping.
    /// - After a terminal iteration (`Break`, `Success`, …), `sink.clear()` drops the checkpoint so
    ///   it cannot linger past completion.
    ///
    /// Stateless dispatches ignore the sink entirely. Pass `None` if you
    /// do not need cross-dispatch resume — behaviour matches the original
    /// `execute_action_versioned` shape.
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
        context: ActionContext,
        checkpoint: Option<Arc<dyn StatefulCheckpointSink>>,
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

        self.run_handler(action_key, metadata, handler, input, context, checkpoint)
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

        self.run_handler(action_key, metadata, handler, input, context, None)
            .await
    }

    /// Dispatch a resolved handler through its kind-specific execution path.
    ///
    /// # Metrics contract (#305 regression)
    ///
    /// Only the *dispatched* paths observe
    /// [`NEBULA_ACTION_DURATION_SECONDS`] and increment
    /// [`NEBULA_ACTION_EXECUTIONS_TOTAL`] /
    /// [`NEBULA_ACTION_FAILURES_TOTAL`]. Early-rejection paths (trigger /
    /// resource / agent / unknown variants) never reach a handler and would
    /// skew the p50/p99 histogram toward zero if sampled; they increment
    /// [`NEBULA_ACTION_DISPATCH_REJECTED_TOTAL`] with a `reason` label
    /// instead so mis-routing is still visible in dashboards.
    async fn run_handler(
        &self,
        action_key: &str,
        metadata: ActionMetadata,
        handler: ActionHandler,
        input: serde_json::Value,
        context: ActionContext,
        checkpoint: Option<Arc<dyn StatefulCheckpointSink>>,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        let error_counter = self.metrics.counter(NEBULA_ACTION_FAILURES_TOTAL);
        let execution_id = context.execution_id;

        // Commit to a dispatched path or a rejection path, then branch.
        // The rejection arms never sample the histogram — they only touch
        // the dispatch-rejected counter (see module-level contract).
        let result = match handler {
            ActionHandler::Stateless(h) => {
                let started = Instant::now();
                let r = self.execute_stateless(&metadata, h, input, context).await;
                self.observe_dispatched(started, &r);
                r
            },
            ActionHandler::Stateful(h) => {
                let started = Instant::now();
                let r = self
                    .execute_stateful(&metadata, h, input, context, checkpoint)
                    .await;
                self.observe_dispatched(started, &r);
                r
            },
            ActionHandler::Trigger(_) => {
                self.observe_rejected(dispatch_reject_reason::TRIGGER_NOT_EXECUTABLE);
                return Err(RuntimeError::TriggerNotExecutable {
                    key: action_key.to_owned(),
                });
            },
            ActionHandler::Resource(_) => {
                self.observe_rejected(dispatch_reject_reason::RESOURCE_NOT_EXECUTABLE);
                return Err(RuntimeError::ResourceNotExecutable {
                    key: action_key.to_owned(),
                });
            },
            ActionHandler::Agent(_) => {
                self.observe_rejected(dispatch_reject_reason::AGENT_NOT_SUPPORTED);
                return Err(RuntimeError::AgentNotSupportedYet {
                    key: action_key.to_owned(),
                });
            },
            // `ActionHandler` is `#[non_exhaustive]`. Unknown future variants
            // are surfaced as an internal runtime error rather than silently
            // succeeding.
            _ => {
                self.observe_rejected(dispatch_reject_reason::UNKNOWN_VARIANT);
                return Err(RuntimeError::Internal(format!(
                    "unknown ActionHandler variant for action '{action_key}'"
                )));
            },
        };

        match result {
            Ok(mut action_result) => {
                self.enforce_data_limit(
                    action_key,
                    execution_id,
                    &mut action_result,
                    &error_counter,
                )
                .await?;
                Ok(action_result)
            },
            Err(action_err) => Err(RuntimeError::ActionError(action_err)),
        }
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
        result: &Result<ActionResult<serde_json::Value>, ActionError>,
    ) {
        let elapsed = started.elapsed();
        self.duration_histogram().observe(elapsed.as_secs_f64());
        self.executions_counter().inc();
        if result.is_err() {
            self.failures_counter().inc();
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
        self.metrics
            .counter_labeled(NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, &labels)
            .inc();
    }

    fn duration_histogram(&self) -> Histogram {
        self.metrics.histogram(NEBULA_ACTION_DURATION_SECONDS)
    }

    fn executions_counter(&self) -> Counter {
        self.metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL)
    }

    fn failures_counter(&self) -> Counter {
        self.metrics.counter(NEBULA_ACTION_FAILURES_TOTAL)
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
            },
            // IsolationLevel is `#[non_exhaustive]`. Any future variant must
            // fail-closed until we explicitly wire dispatch for it.
            _ => Err(ActionError::fatal(format!(
                "unknown isolation level for action '{}' — refusing to dispatch",
                metadata.key.as_str()
            ))),
        }
    }

    /// Execute a stateful handler — loops through [`StatefulHandler::execute`]
    /// with cross-iteration checkpointing.
    ///
    /// # Cancellation contract (#304 regression)
    ///
    /// The iteration body races `handler.execute(..)` against
    /// `context.cancellation.cancelled()` via `tokio::select!`. A stuck
    /// handler future that does not return on its own is aborted at its
    /// next `.await` point by dropping the pinned future. Handlers whose
    /// mid-`await` state cannot safely be dropped must document that and
    /// guard their critical sections internally — the runtime will drop
    /// them the moment cancellation fires.
    ///
    /// # Checkpoint contract (#308 regression)
    ///
    /// When a `checkpoint` sink is provided:
    ///
    /// - Before `init_state()`, `sink.load()` is consulted. A successful `Some` resumes at
    ///   `(iteration, state)`; deserialization failure at the sink boundary (schema drift, etc.)
    ///   logs a WARN with the action key, execution id and node id, and falls through to
    ///   `init_state()` — iteration progress is lost, but the loss is visible in logs instead of
    ///   silently swallowed.
    /// - After every successful `Continue`, `sink.save(..)` persists the mutated state and
    ///   iteration counter before the next loop turn.
    /// - After any terminal iteration (`Break`, `Success`, `Skip`, …), `sink.clear()` deletes the
    ///   checkpoint so a completed stateful action does not leave rows behind.
    /// - On handler error, the checkpoint is **not** cleared — the engine decides whether to retry
    ///   (reuse checkpoint) or fail the attempt (new attempt gets a new checkpoint row).
    async fn execute_stateful(
        &self,
        metadata: &ActionMetadata,
        handler: Arc<dyn StatefulHandler>,
        input: serde_json::Value,
        context: ActionContext,
        checkpoint: Option<Arc<dyn StatefulCheckpointSink>>,
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

        // Cancellation check BEFORE init_state / load — avoid the JSON
        // round-trip if the caller already cancelled.
        if context.cancellation.is_cancelled() {
            return Err(ActionError::Cancelled);
        }

        // Attempt to resume from the checkpoint if a sink is configured.
        //
        // Three outcomes:
        //
        //   1. sink is None              → init_state fresh.
        //   2. sink.load() -> None       → init_state fresh (no prior run).
        //   3. sink.load() -> Some(cp)   → try to adopt cp.state directly. If the handler's current
        //      schema rejects cp.state on the first iteration (StateDeserialization / migration
        //      failure), the iteration body will surface that as ActionError. We do NOT
        //      pre-validate here — the handler owns the schema.
        //
        // Any error from the sink itself (transport, serialization, etc.)
        // is logged at WARN with full (action_key, execution_id, node_key)
        // context and we fall through to init_state. This is the
        // "checkpoint-deser-failure" contract: the runtime MUST NOT
        // silently swallow sink errors — losing iteration progress has to
        // be visible.
        let (mut state, mut iteration) = match checkpoint.as_deref() {
            Some(sink) => match sink.load().await {
                Ok(Some(cp)) => (cp.state, cp.iteration),
                Ok(None) => (handler.init_state()?, 0u32),
                Err(load_err) => {
                    tracing::warn!(
                        action_key = %metadata.key.as_str(),
                        execution_id = %context.execution_id,
                        node_key = %context.node_key,
                        error = %load_err,
                        "stateful checkpoint load failed — falling back to init_state, \
                         iteration progress (if any) is lost"
                    );
                    (handler.init_state()?, 0u32)
                },
            },
            None => (handler.init_state()?, 0u32),
        };

        // Hard cap to prevent runaway loops. Resumed iteration counts
        // carry forward — the cap is per (execution, node, attempt), not
        // per dispatch.
        const MAX_ITERATIONS: u32 = 10_000;

        loop {
            if iteration >= MAX_ITERATIONS {
                return Err(ActionError::fatal(format!(
                    "stateful action '{}' exceeded max iterations ({MAX_ITERATIONS})",
                    metadata.key.as_str()
                )));
            }

            // Cooperative cancellation check BEFORE the next iteration.
            if context.cancellation.is_cancelled() {
                return Err(ActionError::Cancelled);
            }

            // Race handler.execute against cancellation (#304). A stuck
            // handler future dropped here aborts its work at the next
            // .await point — this is the whole point of the fix.
            //
            // The select!'s `handler.execute(..)` arm borrows `state`
            // mutably for the duration of the future. We scope the pin
            // tightly so the borrow ends before the post-iteration
            // checkpoint save, which also needs to read `state`.
            let iteration_result = {
                let exec_fut = handler.execute(&input, &mut state, &context);
                tokio::pin!(exec_fut);

                tokio::select! {
                    biased;
                    () = context.cancellation.cancelled() => {
                        return Err(ActionError::Cancelled);
                    }
                    res = &mut exec_fut => res,
                }
            };

            let result = iteration_result?;
            iteration = iteration.saturating_add(1);

            match result {
                ActionResult::Continue { delay, .. } => {
                    // Persist the new state BEFORE sleeping. If the
                    // process dies during the delay, the next dispatch
                    // resumes from this iteration boundary — not from
                    // init_state.
                    if let Some(sink) = checkpoint.as_deref() {
                        let cp = StatefulCheckpoint::new(iteration, state.clone());
                        sink.save(&cp).await?;
                    }

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
                },
                other => {
                    // Terminal iteration — drop the checkpoint so a
                    // completed stateful node does not leave rows behind.
                    // Failure to clear is not fatal for this dispatch —
                    // the engine's attempt lifecycle will garbage-collect
                    // orphaned checkpoints on terminal transitions — but
                    // we surface it as WARN so it is visible.
                    if let Some(sink) = checkpoint.as_deref()
                        && let Err(clear_err) = sink.clear().await
                    {
                        tracing::warn!(
                            action_key = %metadata.key.as_str(),
                            execution_id = %context.execution_id,
                            node_key = %context.node_key,
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
        error_counter: &nebula_telemetry::metrics::Counter,
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
                ActionOutput::Deferred(_) | ActionOutput::Streaming(_) | ActionOutput::Empty => 0,
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

/// Best-effort size of all payload bytes represented by an output slot after
/// per-node enforcement (including nested collections).
fn estimated_action_output_payload_bytes(slot: &ActionOutput<serde_json::Value>) -> u64 {
    match slot {
        ActionOutput::Value(v) => serde_json::to_vec(v).map(|b| b.len() as u64).unwrap_or(0),
        ActionOutput::Binary(b) => b.effective_size(),
        ActionOutput::Reference(r) => r
            .size
            .unwrap_or_else(|| serde_json::to_vec(r).map(|b| b.len() as u64).unwrap_or(0)),
        ActionOutput::Deferred(_) => 0,
        ActionOutput::Streaming(_) => 0,
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
                for item in items.iter_mut() {
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
        id::{ExecutionId, WorkflowId},
        node_key,
    };
    use nebula_sandbox::{ActionExecutor, InProcessSandbox};

    use super::*;

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
            node_key!("test"),
            WorkflowId::new(),
            tokio_util::sync::CancellationToken::new(),
        )
    }

    fn test_trigger_context() -> TriggerContext {
        TriggerContext::new(
            WorkflowId::new(),
            node_key!("test"),
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

    /// Build a runtime with a metrics registry we hand back to the caller,
    /// so tests can assert on counters/histograms that the runtime wrote
    /// through its private `metrics` field.
    fn make_runtime_with_metrics(
        registry: Arc<ActionRegistry>,
    ) -> (ActionRuntime, MetricsRegistry) {
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
        (rt, metrics)
    }

    #[tokio::test]
    async fn max_total_execution_bytes_across_dispatches() {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(EchoAction {
            meta: ActionMetadata::new(action_key!("test.echo"), "Echo", "echoes input"),
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
                max_node_output_bytes: 1024,
                max_total_execution_bytes: 10,
                ..Default::default()
            },
            metrics,
        );

        let eid = ExecutionId::new();
        let ctx = ActionContext::new(
            eid,
            node_key!("test"),
            WorkflowId::new(),
            tokio_util::sync::CancellationToken::new(),
        );

        rt.execute_action("test.echo", serde_json::json!(null), ctx.clone())
            .await
            .expect("first dispatch under total cap");

        let err = rt
            .execute_action("test.echo", serde_json::json!("1234567890"), ctx)
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
            },
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

    #[tokio::test]
    async fn collection_children_respect_reject_limit() {
        use nebula_action::result::ActionResult as AR;

        struct CollectionAction {
            meta: ActionMetadata,
        }
        impl ActionDependencies for CollectionAction {}
        impl Action for CollectionAction {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
        }
        impl StatelessAction for CollectionAction {
            type Input = serde_json::Value;
            type Output = serde_json::Value;
            async fn execute(
                &self,
                _input: Self::Input,
                _ctx: &impl Context,
            ) -> Result<AR<Self::Output>, ActionError> {
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
        registry.register_stateless(CollectionAction {
            meta: ActionMetadata::new(
                action_key!("test.collection"),
                "Collection",
                "nested values",
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
            .execute_action("test.collection", serde_json::json!(null), test_context())
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

        struct BinaryAction {
            meta: ActionMetadata,
        }
        impl ActionDependencies for BinaryAction {}
        impl Action for BinaryAction {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
        }
        impl StatelessAction for BinaryAction {
            type Input = serde_json::Value;
            type Output = serde_json::Value;
            async fn execute(
                &self,
                _input: Self::Input,
                _ctx: &impl Context,
            ) -> Result<AR<Self::Output>, ActionError> {
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
        registry.register_stateless(BinaryAction {
            meta: ActionMetadata::new(action_key!("test.binary"), "Binary", "inline bytes"),
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
            .execute_action("test.binary", serde_json::json!(null), test_context())
            .await;
        assert!(
            matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
            "inline binary output must be checked via effective_size() — got {result:?}"
        );
    }

    #[tokio::test]
    async fn reference_metadata_respects_reject_limit() {
        use nebula_action::result::ActionResult as AR;

        struct RefAction {
            meta: ActionMetadata,
        }
        impl ActionDependencies for RefAction {}
        impl Action for RefAction {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
        }
        impl StatelessAction for RefAction {
            type Input = serde_json::Value;
            type Output = serde_json::Value;
            async fn execute(
                &self,
                _input: Self::Input,
                _ctx: &impl Context,
            ) -> Result<AR<Self::Output>, ActionError> {
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
        registry.register_stateless(RefAction {
            meta: ActionMetadata::new(action_key!("test.ref"), "Reference", "large metadata"),
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
                max_node_output_bytes: 32,
                large_data_strategy: LargeDataStrategy::Reject,
                ..Default::default()
            },
            metrics,
        );

        let result = rt
            .execute_action("test.ref", serde_json::json!(null), test_context())
            .await;
        assert!(
            matches!(result, Err(RuntimeError::DataLimitExceeded { .. })),
            "reference metadata must be included in size enforcement — got {result:?}"
        );
    }

    // ── #305 regression: dispatch-rejection paths do not skew histogram ─────

    /// Register a handler that resolves to a kind which is *not* executable
    /// via `ActionRuntime` — trigger, resource, agent — and assert that
    /// `run_handler` does not record duration samples or bump the
    /// executions / failures counters. Instead, the dispatch-rejected
    /// counter increments once, labeled with the correct reason.
    #[tokio::test]
    async fn trigger_rejection_does_not_observe_histogram() {
        use nebula_action::{
            TriggerContext, handler::ActionHandler as AH, trigger::TriggerHandler as TH,
        };

        // Fake trigger handler — never actually invoked, only its variant
        // matters. We implement the minimal surface the registry expects.
        struct FakeTriggerHandler {
            meta: ActionMetadata,
        }

        #[async_trait::async_trait]
        impl TH for FakeTriggerHandler {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
            async fn start(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
                Ok(())
            }
            async fn stop(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
                Ok(())
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        let meta = ActionMetadata::new(action_key!("test.trigger_reject"), "Trig", "reject case");
        registry.register(
            meta.clone(),
            AH::Trigger(Arc::new(FakeTriggerHandler { meta })),
        );
        let (rt, metrics) = make_runtime_with_metrics(registry);

        let result = rt
            .execute_action(
                "test.trigger_reject",
                serde_json::json!(null),
                test_context(),
            )
            .await;
        assert!(
            matches!(result, Err(RuntimeError::TriggerNotExecutable { .. })),
            "expected TriggerNotExecutable, got {result:?}"
        );

        // Histogram and execution/failure counters must NOT observe this path.
        assert_eq!(
            metrics.histogram(NEBULA_ACTION_DURATION_SECONDS).count(),
            0,
            "duration histogram must not sample rejection paths"
        );
        assert_eq!(
            metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL).get(),
            0,
            "executions counter must not bump on rejection"
        );
        assert_eq!(
            metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).get(),
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
                .get(),
            1,
            "dispatch-rejected counter should be bumped once with reason=trigger_not_executable"
        );
    }

    #[tokio::test]
    async fn resource_rejection_does_not_increment_execution_metrics() {
        use std::any::Any;

        use nebula_action::{handler::ActionHandler, resource::ResourceHandler};

        struct FakeResourceHandler {
            meta: ActionMetadata,
        }

        #[async_trait::async_trait]
        impl ResourceHandler for FakeResourceHandler {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
            async fn configure(
                &self,
                _config: serde_json::Value,
                _ctx: &ActionContext,
            ) -> Result<Box<dyn Any + Send + Sync>, ActionError> {
                Ok(Box::new(()))
            }
            async fn cleanup(
                &self,
                _instance: Box<dyn Any + Send + Sync>,
                _ctx: &ActionContext,
            ) -> Result<(), ActionError> {
                Ok(())
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        let meta = ActionMetadata::new(action_key!("test.resource_reject"), "Res", "reject case");
        registry.register(
            meta.clone(),
            ActionHandler::Resource(Arc::new(FakeResourceHandler { meta })),
        );
        let (rt, metrics) = make_runtime_with_metrics(registry);

        let result = rt
            .execute_action(
                "test.resource_reject",
                serde_json::json!(null),
                test_context(),
            )
            .await;
        assert!(
            matches!(result, Err(RuntimeError::ResourceNotExecutable { .. })),
            "expected ResourceNotExecutable, got {result:?}"
        );

        assert_eq!(metrics.histogram(NEBULA_ACTION_DURATION_SECONDS).count(), 0);
        assert_eq!(metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL).get(), 0);
        assert_eq!(metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).get(), 0);

        let labels = metrics
            .interner()
            .label_set(&[("reason", dispatch_reject_reason::RESOURCE_NOT_EXECUTABLE)]);
        assert_eq!(
            metrics
                .counter_labeled(NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, &labels)
                .get(),
            1
        );
    }

    #[tokio::test]
    async fn agent_rejection_does_not_increment_execution_metrics() {
        use nebula_action::handler::{ActionHandler, AgentHandler};

        struct FakeAgentHandler {
            meta: ActionMetadata,
        }

        #[async_trait::async_trait]
        impl AgentHandler for FakeAgentHandler {
            fn metadata(&self) -> &ActionMetadata {
                &self.meta
            }
            async fn execute(
                &self,
                _input: serde_json::Value,
                _ctx: &ActionContext,
            ) -> Result<ActionResult<serde_json::Value>, ActionError> {
                Ok(ActionResult::success(serde_json::json!(null)))
            }
        }

        let registry = Arc::new(ActionRegistry::new());
        let meta = ActionMetadata::new(action_key!("test.agent_reject"), "Agent", "reject case");
        registry.register(
            meta.clone(),
            ActionHandler::Agent(Arc::new(FakeAgentHandler { meta })),
        );
        let (rt, metrics) = make_runtime_with_metrics(registry);

        let result = rt
            .execute_action("test.agent_reject", serde_json::json!(null), test_context())
            .await;
        assert!(
            matches!(result, Err(RuntimeError::AgentNotSupportedYet { .. })),
            "expected AgentNotSupportedYet, got {result:?}"
        );

        assert_eq!(metrics.histogram(NEBULA_ACTION_DURATION_SECONDS).count(), 0);
        assert_eq!(metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL).get(), 0);
        assert_eq!(metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).get(), 0);

        let labels = metrics
            .interner()
            .label_set(&[("reason", dispatch_reject_reason::AGENT_NOT_SUPPORTED)]);
        assert_eq!(
            metrics
                .counter_labeled(NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, &labels)
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
        registry.register_stateless(EchoAction {
            meta: ActionMetadata::new(action_key!("test.dispatched"), "Disp", "dispatched"),
        });
        let (rt, metrics) = make_runtime_with_metrics(registry);

        rt.execute_action("test.dispatched", serde_json::json!("ok"), test_context())
            .await
            .expect("dispatched execution must succeed");
        assert_eq!(metrics.histogram(NEBULA_ACTION_DURATION_SECONDS).count(), 1);
        assert_eq!(metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL).get(), 1);
        assert_eq!(metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).get(), 0);

        let labels = metrics
            .interner()
            .label_set(&[("reason", dispatch_reject_reason::TRIGGER_NOT_EXECUTABLE)]);
        assert_eq!(
            metrics
                .counter_labeled(NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, &labels)
                .get(),
            0,
            "dispatch-rejected counter must stay at zero for successful dispatch"
        );
    }

    // ── #304 + #308 regression: stateful cancel + checkpoint ────────────────

    use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

    use nebula_action::stateful::StatefulHandler;
    use serde_json::Value as JsonValue;
    use tokio::sync::Mutex as TokioMutex;

    /// Counting stateful handler — counts from `state.count` to `target`,
    /// emitting `Continue` until it reaches the target, then `Break`.
    /// Used by #308 checkpoint + resume tests.
    struct CountingHandler {
        meta: ActionMetadata,
        target: u32,
    }

    #[async_trait::async_trait]
    impl StatefulHandler for CountingHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        fn init_state(&self) -> Result<JsonValue, ActionError> {
            Ok(serde_json::json!({ "count": 0u32 }))
        }

        async fn execute(
            &self,
            _input: &JsonValue,
            state: &mut JsonValue,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<JsonValue>, ActionError> {
            let count = state.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let next = count + 1;
            *state = serde_json::json!({ "count": next });
            if next >= self.target {
                Ok(ActionResult::Break {
                    output: nebula_action::output::ActionOutput::Value(
                        serde_json::json!({ "final": next }),
                    ),
                    reason: nebula_action::result::BreakReason::Completed,
                })
            } else {
                Ok(ActionResult::Continue {
                    output: nebula_action::output::ActionOutput::Value(
                        serde_json::json!({ "step": next }),
                    ),
                    progress: None,
                    delay: None,
                })
            }
        }
    }

    /// Sleepy stateful handler — awaits a very long sleep inside
    /// `execute`. Used by the #304 cancel-aborts-handler test.
    struct SleepyHandler {
        meta: ActionMetadata,
    }

    #[async_trait::async_trait]
    impl StatefulHandler for SleepyHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
        fn init_state(&self) -> Result<JsonValue, ActionError> {
            Ok(serde_json::json!({}))
        }
        async fn execute(
            &self,
            _input: &JsonValue,
            _state: &mut JsonValue,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<JsonValue>, ActionError> {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            Ok(ActionResult::Break {
                output: nebula_action::output::ActionOutput::Value(serde_json::json!(null)),
                reason: nebula_action::result::BreakReason::Completed,
            })
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

    /// #304 regression: a handler that awaits a 1-hour sleep inside
    /// `execute` must abort the moment the cancellation token fires — not
    /// 1 hour later. Uses `start_paused = true` so the sleep never
    /// naturally advances.
    #[tokio::test(start_paused = true)]
    async fn execute_stateful_aborts_handler_on_cancel() {
        let registry = Arc::new(ActionRegistry::new());
        let meta = ActionMetadata::new(action_key!("test.sleepy"), "Sleepy", "hangs in execute");
        registry.register(
            meta.clone(),
            nebula_action::handler::ActionHandler::Stateful(Arc::new(SleepyHandler { meta })),
        );
        let rt = Arc::new(make_runtime(registry));

        let ctx = test_context();
        let cancel = ctx.cancellation.clone();

        // Dispatch on a task so we can cancel after 10ms of virtual time.
        let rt_clone = Arc::clone(&rt);
        let handle = tokio::spawn(async move {
            rt_clone
                .execute_action("test.sleepy", serde_json::json!(null), ctx)
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
        let meta = ActionMetadata::new(action_key!("test.count"), "Count", "counts");
        registry.register(
            meta.clone(),
            nebula_action::handler::ActionHandler::Stateful(Arc::new(CountingHandler {
                meta,
                target: 3,
            })),
        );
        let rt = make_runtime(registry);

        let sink = Arc::new(RecordingSink::new());
        let result = rt
            .execute_action_with_checkpoint(
                "test.count",
                None,
                serde_json::json!(null),
                test_context(),
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
        let meta = ActionMetadata::new(action_key!("test.resume"), "Resume", "resumes");
        registry.register(
            meta.clone(),
            nebula_action::handler::ActionHandler::Stateful(Arc::new(CountingHandler {
                meta,
                target: 5,
            })),
        );
        let rt = make_runtime(registry);

        let seed = StatefulCheckpoint::new(3, serde_json::json!({ "count": 3u32 }));
        let sink = Arc::new(RecordingSink::with_preload(seed));

        let result = rt
            .execute_action_with_checkpoint(
                "test.resume",
                None,
                serde_json::json!(null),
                test_context(),
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
        let meta = ActionMetadata::new(action_key!("test.load_fail"), "LoadFail", "load fails");
        registry.register(
            meta.clone(),
            nebula_action::handler::ActionHandler::Stateful(Arc::new(CountingHandler {
                meta,
                target: 2,
            })),
        );
        let rt = make_runtime(registry);

        let sink = Arc::new(RecordingSink::with_failing_load());
        let result = rt
            .execute_action_with_checkpoint(
                "test.load_fail",
                None,
                serde_json::json!(null),
                test_context(),
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
}
