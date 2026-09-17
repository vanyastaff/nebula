//! Action runtime -- the main execution orchestrator.
//!
//! Resolves actions from the registry, executes them through the runner,
//! enforces data limits, and records metrics.

use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use nebula_action::{
    ActionContext, ActionError, ActionFactory, ActionHandle, ActionMetadata, AgentHandle,
    IsolationLevel, StreamHandle,
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

/// Compute a deterministic digest of the serialized stateful state for
/// stuck-state detection.
///
/// The runtime sees `state` as `serde_json::Value` — not `Hash`, but always
/// serialisable. We serialize to canonical JSON bytes and take the first 8
/// bytes of a SHA-256 hash as a `u64`. SHA-256 is stable across processes,
/// restarts, and Rust toolchain versions, making the digest cross-run
/// comparable — a requirement for stuck-state detection across retries.
///
/// `std::collections::hash_map::DefaultHasher` (SipHash with a random
/// per-process seed) was deliberately avoided: its randomization makes
/// cross-run comparison non-functional.
///
/// Errors collapse to `0` so the guard reduces to "assume the iteration
/// progressed" on unserialisable state — an author who manages to hold an
/// unserialisable `Value` has bigger problems than stuck detection.
fn stateful_state_digest(state: &serde_json::Value) -> u64 {
    use sha2::{Digest, Sha256};
    match serde_json::to_vec(state) {
        Ok(bytes) => {
            let hash = Sha256::digest(&bytes);
            // Take the first 8 bytes as a little-endian u64.
            u64::from_le_bytes(hash[..8].try_into().expect("SHA-256 output is 32 bytes"))
        },
        Err(_) => 0,
    }
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
    /// Handler state as JSON — exactly the value the runtime hands back into
    /// [`nebula_action::StatefulHandle::dispatch`] on the next loop.
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
/// The runtime does not depend on any storage crate directly — hosts that
/// want durable stateful resume implement this trait over their persistence
/// seam; the storage-port `CheckpointStore::{save,load}_stateful_checkpoint`
/// methods are the matching seam, and `clear` has no port counterpart (the
/// host maps it onto its own store).
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

    /// Execute a factory retained by the engine's exact revision witness.
    /// This path never consults the mutable action registry.
    pub(crate) async fn execute_resolved_action(
        &self,
        factory: Arc<dyn ActionFactory>,
        node: &NodeDefinition,
        input: nebula_schema::ResolvedValues,
        context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        self.run_factory(
            node.action_key.as_str(),
            factory,
            node,
            nebula_action::ActionInput::Resolved(input),
            context,
            None,
        )
        .await
    }

    /// Common dispatch entry — routes all executions through the factory path.
    ///
    /// Looks up the `Arc<dyn ActionFactory>` for the action key, instantiates a
    /// fresh `ActionHandle` via [`ActionFactory::instantiate`], and dispatches it
    /// through [`Self::run_factory`]. Returns
    /// [`RuntimeError::ActionNotFound`] if no factory is registered for the key.
    #[expect(clippy::too_many_arguments)]
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
        let (_metadata, factory) = factory_lookup.ok_or_else(|| RuntimeError::ActionNotFound {
            key: action_key_str.to_owned(),
        })?;
        self.run_factory(
            action_key_str,
            factory,
            node,
            nebula_action::ActionInput::Raw(input),
            context,
            checkpoint,
        )
        .await
    }

    /// Dispatch through the factory path — instantiate a fresh
    /// [`ActionHandle`] for the supplied workflow node and dispatch it.
    /// The factory's admitted metadata must explicitly declare no external
    /// effects, and the factory must expose no remote capability. This check
    /// precedes construction; generic dispatch cannot issue an execution-owner
    /// grant.
    ///
    /// Metric contract:
    ///
    /// - Stateless / Stateful / Control variants observe the duration histogram and increment
    ///   executions / failures.
    /// - Trigger / Resource kinds and effects requiring owner admission are
    ///   rejected before construction and increment the dispatch-rejected counter only.
    ///
    /// `factory.instantiate` returning an error is treated as an action
    /// failure (slot resolution, etc.). The duration histogram is observed
    /// for instantiate failures so dashboards reflect the per-dispatch cost
    /// regardless of whether the failure happened in instantiation or
    /// during the action itself.
    fn validate_factory_handle(
        &self,
        action_key: &str,
        factory: &dyn ActionFactory,
        handle: &ActionHandle,
    ) -> Result<(), RuntimeError> {
        if !Arc::ptr_eq(factory.metadata(), handle.metadata()) {
            self.observe_rejected("factory_handle_metadata_mismatch");
            tracing::error!(
                error_code = "RUNTIME:FACTORY_HANDLE_METADATA_MISMATCH",
                action_key,
                "sealed action factory returned a handle from another admitted contract"
            );
            return Err(RuntimeError::FactoryHandleMetadataMismatch {
                key: action_key.to_owned(),
            });
        }
        if handle.kind() != factory.metadata().kind() {
            self.observe_rejected("factory_handle_kind_mismatch");
            tracing::error!(
                error_code = "RUNTIME:FACTORY_HANDLE_KIND_MISMATCH",
                action_key,
                expected = ?factory.metadata().kind(),
                actual = ?handle.kind(),
                "sealed action factory returned a handle with the wrong structural kind"
            );
            return Err(RuntimeError::FactoryHandleKindMismatch {
                key: action_key.to_owned(),
                expected: factory.metadata().kind(),
                actual: handle.kind(),
            });
        }
        Ok(())
    }

    async fn run_factory(
        &self,
        action_key: &str,
        factory: Arc<dyn ActionFactory>,
        node: &NodeDefinition,
        input: nebula_action::ActionInput,
        context: &dyn ActionContext,
        checkpoint: Option<Arc<dyn StatefulCheckpointSink>>,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        let error_counter = &self.action_failures_total;
        let metadata = factory.metadata();
        if metadata.kind() == nebula_action::ActionKind::Trigger {
            self.observe_rejected(dispatch_reject_reason::TRIGGER_NOT_EXECUTABLE);
            return Err(RuntimeError::TriggerNotExecutable {
                key: action_key.to_owned(),
            });
        }
        if metadata.kind() == nebula_action::ActionKind::Resource {
            self.observe_rejected(dispatch_reject_reason::RESOURCE_NOT_EXECUTABLE);
            return Err(RuntimeError::ResourceNotExecutable {
                key: action_key.to_owned(),
            });
        }
        if !matches!(
            metadata.effect_contract(),
            nebula_action::effect::ActionEffectContract::NoExternalEffects
        ) || factory.remote_effect_factory().is_some()
        {
            self.observe_rejected("effect_requires_owner");
            return Err(RuntimeError::EffectRequiresOwner);
        }
        #[expect(
            clippy::unwrap_or_default,
            reason = "ExecutionId::new() != Default::default()"
        )]
        let execution_id = context
            .scope()
            .execution_id
            .unwrap_or_else(ExecutionId::new);

        let started = Instant::now();

        if context.cancellation().is_cancelled() {
            return Err(ActionError::Cancelled.into());
        }

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

        self.validate_factory_handle(action_key, factory.as_ref(), &handle)?;

        let result = match handle {
            ActionHandle::Stateless(inner) => {
                let r = self
                    .execute_stateless_handle(metadata, inner, input, context)
                    .await;
                self.observe_dispatched(started, &r);
                r
            },
            ActionHandle::Stateful(inner) => {
                let r = self
                    .execute_stateful_handle(metadata, inner, input, context, checkpoint)
                    .await;
                self.observe_dispatched(started, &r);
                r
            },
            ActionHandle::Stream(inner) => {
                let r = self
                    .execute_stream_handle(metadata, inner, input, context)
                    .await;
                self.observe_dispatched(started, &r);
                r
            },
            ActionHandle::Control(inner) => {
                let r = self
                    .execute_control_handle(metadata, inner, input, context)
                    .await;
                self.observe_dispatched(started, &r);
                r
            },
            ActionHandle::Agent(inner) => {
                let r = self
                    .execute_agent_handle(metadata, inner, input, context)
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
    /// Mirrors [`Self::execute_stateless_handle`] for the factory path. Honours
    /// the same isolation contract (`None` runs in-process; capability-gated
    /// dispatch routes through [`ActionRunner`] using the same `metadata`).
    async fn execute_stateless_handle(
        &self,
        metadata: &ActionMetadata,
        handle: Box<dyn nebula_action::StatelessHandle>,
        input: nebula_action::ActionInput,
        context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        match metadata.isolation_level() {
            IsolationLevel::None => {
                let input = handle.prepare_input(input)?;
                Ok(handle.dispatch(input, context).await?)
            },
            IsolationLevel::CapabilityGated => {
                let run_ctx = ActionRunContext::new(context);
                Ok(self
                    .runner
                    .execute_stateless(run_ctx, handle, input, context)
                    .await?)
            },
            // IsolationLevel is `#[non_exhaustive]`. Any future variant must
            // fail-closed until we explicitly wire dispatch for it.
            _ => Err(RuntimeError::Internal(format!(
                "unknown isolation level for action '{}' — refusing to dispatch",
                metadata.base().key().as_str()
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
            action.key = %metadata.base().key().as_str(),
            action.kind = "stream",
        )
    )]
    async fn execute_stream_handle(
        &self,
        metadata: &ActionMetadata,
        handle: Box<dyn StreamHandle>,
        input: nebula_action::ActionInput,
        context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        match metadata.isolation_level() {
            IsolationLevel::None => {
                let input = handle.prepare_input(input)?;
                Ok(handle.dispatch(input, context).await?)
            },
            IsolationLevel::CapabilityGated => {
                let run_ctx = ActionRunContext::new(context);
                Ok(self
                    .runner
                    .execute_stream(run_ctx, handle, input, context)
                    .await?)
            },
            // IsolationLevel is `#[non_exhaustive]`. Any future variant must
            // fail-closed until we explicitly wire dispatch for it.
            _ => Err(RuntimeError::Internal(format!(
                "unknown isolation level for stream action '{}' — refusing to dispatch",
                metadata.base().key().as_str()
            ))),
        }
    }

    /// Stateful dispatch via `Box<dyn StatefulHandle>`.
    ///
    /// The handle works on `Value` state while retaining one prepared typed
    /// input across every iteration. Cancellation, checkpoint persistence,
    /// iteration limits, and stuck-state detection remain runtime-owned.
    async fn execute_stateful_handle(
        &self,
        metadata: &ActionMetadata,
        handle: Box<dyn nebula_action::StatefulHandle>,
        input: nebula_action::ActionInput,
        context: &dyn ActionContext,
        checkpoint: Option<Arc<dyn StatefulCheckpointSink>>,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        if !matches!(metadata.isolation_level(), IsolationLevel::None) {
            return Err(ActionError::fatal(
                "capability-gated stateful execution is not yet supported",
            )
            .into());
        }

        if context.cancellation().is_cancelled() {
            return Err(ActionError::Cancelled.into());
        }

        let input = handle.prepare_input(input)?;

        let (mut state, mut iteration) = match checkpoint.as_deref() {
            Some(sink) => match sink.load().await {
                Ok(Some(cp)) => (cp.state, cp.iteration),
                Ok(None) => (handle.init_state()?, 0u32),
                Err(load_err) => {
                    tracing::warn!(
                        action_key = %metadata.base().key().as_str(),
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
                    action_key: metadata.base().key().clone(),
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
                            action_key: metadata.base().key().clone(),
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
                            action_key = %metadata.base().key().as_str(),
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
        input: nebula_action::ActionInput,
        context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        if !matches!(metadata.isolation_level(), IsolationLevel::None) {
            return Err(RuntimeError::Internal(format!(
                "control action '{}' must run with IsolationLevel::None — \
                 control nodes are flow-control desugared to stateless and \
                 never run through the runner",
                metadata.base().key().as_str()
            )));
        }
        let input = handle.prepare_input(input)?;
        Ok(handle.dispatch(input, context).await?)
    }

    /// Agent dispatch via `Box<dyn AgentHandle>`.
    ///
    /// Runs the agent's own turn loop — deliberately NOT a clone of
    /// `execute_stateful_handle`. Three properties differ by design (ADR-0103 D2):
    ///
    /// - **No `StatefulStuck` digest guard** — a turn that leaves `Turn` unchanged
    ///   is legal; an LLM thinking without mutating state is not a bug.
    /// - **`max_turns()` budget** — replaces the 10 000-iteration global cap with
    ///   the author-declared per-agent budget.
    /// - **Per-turn wall-clock timeout** — each `step` future is individually
    ///   bounded so a hung provider cannot pin a worker indefinitely.
    ///
    /// Turn state is kept as a local `serde_json::Value` variable. There is no
    /// durable checkpoint: on a worker crash the whole action re-executes from
    /// scratch with the original input. Mid-loop resume would require wiring a
    /// checkpoint sink, which is out of scope for this foundational implementation.
    ///
    /// # Cancellation
    ///
    /// Every turn races against the execution-level cancellation token via
    /// `tokio::select!`. The per-turn timeout (if any) is composed with
    /// cancellation so neither can block the other.
    #[tracing::instrument(
        name = "runtime.execute_agent_handle",
        skip_all,
        fields(
            action.key = %metadata.base().key().as_str(),
            action.kind = "agent",
            max_turns = handle.max_turns(),
        )
    )]
    async fn execute_agent_handle(
        &self,
        metadata: &ActionMetadata,
        handle: Box<dyn AgentHandle>,
        input: nebula_action::ActionInput,
        context: &dyn ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, RuntimeError> {
        if !matches!(metadata.isolation_level(), IsolationLevel::None) {
            return Err(RuntimeError::Internal(
                "capability-gated agent execution is not yet supported".into(),
            ));
        }

        if context.cancellation().is_cancelled() {
            return Err(ActionError::Cancelled.into());
        }

        let input = handle.prepare_input(input)?;
        let mut turn_state = handle.init_turn(input)?;
        let max_turns = handle.max_turns();
        let turn_timeout = handle.turn_timeout();

        let mut turn: u32 = 0;

        loop {
            if turn >= max_turns {
                return Err(RuntimeError::AgentBudgetExceeded {
                    key: metadata.base().key().as_str().to_owned(),
                    max_turns,
                });
            }

            if context.cancellation().is_cancelled() {
                return Err(ActionError::Cancelled.into());
            }

            let step_result = {
                let step_future = handle.step(&mut turn_state, context);
                tokio::pin!(step_future);

                match turn_timeout {
                    Some(deadline) => {
                        tokio::select! {
                            biased;
                            () = context.cancellation().cancelled() => {
                                return Err(ActionError::Cancelled.into());
                            }
                            timeout_result = tokio::time::timeout(deadline, &mut step_future) => {
                                match timeout_result {
                                    Ok(step_outcome) => step_outcome,
                                    Err(_elapsed) => {
                                        return Err(RuntimeError::AgentTurnTimeout {
                                            key: metadata.base().key().as_str().to_owned(),
                                            turn,
                                            timeout: deadline,
                                        });
                                    },
                                }
                            }
                        }
                    },
                    None => {
                        tokio::select! {
                            biased;
                            () = context.cancellation().cancelled() => {
                                return Err(ActionError::Cancelled.into());
                            }
                            step_outcome = &mut step_future => step_outcome,
                        }
                    },
                }
            };

            let result = step_result?;

            // Log the 0-based turn index before incrementing — consistent with
            // `AgentTurnTimeout.turn` which also carries the 0-based index.
            tracing::debug!(
                action.key = %metadata.base().key().as_str(),
                turn,
                max_turns,
                "agent turn completed"
            );

            turn = turn.saturating_add(1);

            match result {
                ActionResult::Continue { delay, .. } => {
                    if let Some(d) = delay {
                        tokio::select! {
                            () = tokio::time::sleep(d) => {}
                            () = context.cancellation().cancelled() => {
                                return Err(ActionError::Cancelled.into());
                            }
                        }
                    }
                    // Loop continues with updated turn_state (already mutated in-place
                    // by the adapter's `step` implementation).
                },
                ActionResult::Wait { .. } => {
                    // The Wait arm requires the durable park/resume machinery which
                    // is not yet wired in the engine. Surface an honest error rather
                    // than silently mishandling the result.
                    return Err(RuntimeError::AgentWaitNotSupported {
                        key: metadata.base().key().as_str().to_owned(),
                    });
                },
                terminal => {
                    // Any other terminal (Break, Success, Skip, Terminate, …) ends the loop.
                    return Ok(terminal);
                },
            }
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
/// Panics only if `action_key` is not a valid [`ActionKey`](nebula_core::ActionKey). Production
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
mod tests;
