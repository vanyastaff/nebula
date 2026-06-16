//! [`DurableExecutionEmitter`] — trigger fan-out through the durable dedup
//! inbox for the ADR-0095 trigger-dispatch slice.
//!
//! ## Responsibility
//!
//! `DurableExecutionEmitter` implements [`nebula_action::ExecutionEmitter`] and
//! wires the trigger-to-execution fan-out path end-to-end:
//!
//! 1. Mint a new [`ExecutionId`] (the candidate id).
//! 2. Build a [`JobDispatchMsg`] (Start command, routing key, etc.) and, when
//!    `event_id` is `Some`, a [`TriggerDedupRow`] guard keyed by
//!    `(scope, trigger_id, event_id)`.
//! 3. Call [`TriggerDedupInbox::claim_and_enqueue_start`] — one atomic critical
//!    section that either inserts the dedup guard + enqueues the Start job
//!    (`Dispatched`) or finds the guard already present (`Duplicate`).
//! 4. **On `Dispatched`**: create a `Created` execution row via
//!    [`ExecutionStore::create`], then return the `ExecutionId`.
//! 5. **On `Duplicate`**: return the candidate `ExecutionId` without creating
//!    any row (no orphan).
//!
//! ## Ordering invariant (R2)
//!
//! The `claim_and_enqueue_start` call happens **before** `ExecutionStore::create`.
//! Reversing the order would orphan a `Created` row on `Duplicate`:
//!
//! ```text
//! WRONG: create_row → claim_and_enqueue (Duplicate) → Created row leaked
//! RIGHT: claim_and_enqueue → Dispatched → create_row
//! ```
//!
//! ## Wiring honesty
//!
//! No prod trigger daemon installs this emitter today — all non-test
//! `TriggerRuntimeContext::new` sites use the default `NoopExecutionEmitter`.
//! Install via `ctx.with_emitter(Arc::new(DurableExecutionEmitter::new(...)))` in
//! the harness or a future trigger daemon; the integration test is the sole current
//! caller.
//!
//! ## Tracing
//!
//! The span carries `trigger_id`, `workflow_id`, `event_id`, and `outcome`.
//! No `input` payload or secrets are logged — `event_id` is non-secret per
//! `IdempotencyKey` docs.
//!
//! ## Idempotency key invariant
//!
//! Source-natural idempotency keys (`event_id`) MUST NOT carry secrets: they
//! are logged at debug level as tracing fields.

use std::future::Future;
use std::sync::Arc;

use nebula_action::{ActionError, ExecutionEmitter, IdempotencyKey};
use nebula_core::NodeKey;
use nebula_core::id::{ExecutionId, WorkflowId};
use nebula_execution::ExecutionState;
use nebula_storage_port::{
    Scope, StorageError,
    dto::{ControlCommand, DispatchOutcome, JobDispatchMsg, TriggerDedupRow},
    store::{ExecutionStore, TriggerDedupInbox},
};

use crate::daemon::routing::RoutingResolver;

/// Trigger fan-out through the durable dedup inbox.
///
/// Holds everything needed to enqueue one trigger-originated Start job:
/// - [`TriggerDedupInbox`] — atomic dedup + enqueue.
/// - [`ExecutionStore`] — create the `Created` row on `Dispatched`.
/// - [`RoutingResolver`] — derive `required_plugin_key` + `target_flavor_sha`.
/// - Identity fields captured at construction: `workflow_id`, `trigger_id`,
///   `scope` — same values on every `emit` call from this trigger context.
#[derive(Clone)]
pub struct DurableExecutionEmitter {
    dedup: Arc<dyn TriggerDedupInbox>,
    execution: Arc<dyn ExecutionStore>,
    resolver: Arc<dyn RoutingResolver>,
    // Cached at construction from `TriggerRuntimeContext`.
    workflow_id: WorkflowId,
    trigger_id: NodeKey,
    scope: Scope,
}

impl std::fmt::Debug for DurableExecutionEmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DurableExecutionEmitter")
            .field("workflow_id", &self.workflow_id)
            .field("trigger_id", &self.trigger_id)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl DurableExecutionEmitter {
    /// Construct a durable emitter.
    ///
    /// `dedup` MUST share the same [`SharedDispatchCore`] as the
    /// `JobDispatchQueue` passed to the orchestrator, so
    /// `claim_and_enqueue_start` operates in one critical section.
    ///
    /// [`SharedDispatchCore`]: nebula_storage::inmem::SharedDispatchCore
    #[must_use]
    pub fn new(
        dedup: Arc<dyn TriggerDedupInbox>,
        execution: Arc<dyn ExecutionStore>,
        resolver: Arc<dyn RoutingResolver>,
        workflow_id: WorkflowId,
        trigger_id: NodeKey,
        scope: Scope,
    ) -> Self {
        Self {
            dedup,
            execution,
            resolver,
            workflow_id,
            trigger_id,
            scope,
        }
    }

    /// Inner emit implementation with structured instrumentation.
    ///
    /// The `#[tracing::instrument]` attribute lives here (on the `async fn`)
    /// so the span is entered before the first `.await` and correctly
    /// re-entered on each poll — the DB work is fully traced.
    #[tracing::instrument(
        name = "durable_emitter.emit",
        level = "debug",
        skip(self, input),
        fields(
            trigger_id  = %self.trigger_id,
            workflow_id = %self.workflow_id,
            event_id    = event_id.as_ref().map(IdempotencyKey::as_str),
        )
    )]
    async fn do_emit(
        &self,
        input: serde_json::Value,
        event_id: Option<IdempotencyKey>,
    ) -> Result<ExecutionId, ActionError> {
        // --- step 1: resolve routing (fail-fast before minting an id) --------
        let route = self
            .resolver
            .resolve(&self.workflow_id, &self.trigger_id)
            .map_err(ActionError::fatal_from)?;

        // --- step 2: mint the candidate id + build the dispatch msg ----------
        //
        // `ExecutionId::new()` generates a fresh ULID. Minting here — before
        // the dedup call — means the returned id is always the candidate, which
        // on `Duplicate` matches no `Created` row (correct: no orphan).
        let execution_id = ExecutionId::new();

        // Mint a fresh ULID for the job-dispatch row primary key.
        // The field is documented as "16-byte ULID (raw bytes)"; time-sortable
        // ordering is required for the storage reclaim cutoff arithmetic.
        let job_id: [u8; 16] = ulid::Ulid::new().to_bytes();

        let start = JobDispatchMsg::new(
            job_id,
            execution_id.to_string(),
            ControlCommand::Start,
            self.scope.clone(),
            input.clone(),
            event_id.as_ref().map(IdempotencyKey::as_str),
            route.target_flavor_sha.clone(),
            route.required_plugin_key.clone(),
            route.capability_tags.clone(),
            None::<String>, // w3c_traceparent: future D1
            0,              // reclaim_count: 0 on first enqueue
        );

        // --- step 3: build the dedup guard row (only when event_id present) --
        let dedup_row = event_id.as_ref().map(|eid| {
            TriggerDedupRow::new(
                self.trigger_id.as_str(),
                eid.as_str(),
                self.scope.clone(),
                execution_id.to_string(),
                chrono::Utc::now().to_rfc3339(),
            )
        });

        // --- step 4: atomic dedup-insert ∧ Start-enqueue (R2 ordering) ------
        //
        // `claim_and_enqueue_start` either:
        //   Dispatched — inserted the dedup guard + enqueued the Start job
        //   Duplicate  — guard already present; no job enqueued; no-op
        //
        // `create_row` (ExecutionStore::create) happens AFTER this call.
        // Reversing the order orphans a Created row on Duplicate.
        let outcome = self
            .dedup
            .claim_and_enqueue_start(dedup_row.as_ref(), &start)
            .await
            .map_err(|e: StorageError| {
                ActionError::retryable(format!("dedup inbox storage error: {e}"))
            })?;

        tracing::debug!(
            outcome = ?outcome,
            execution_id = %execution_id,
            trigger_id   = %self.trigger_id,
            workflow_id  = %self.workflow_id,
            event_id     = event_id.as_ref().map(IdempotencyKey::as_str),
            "durable_emitter: claim_and_enqueue_start"
        );

        match outcome {
            // --- step 5a: Dispatched — seed the Created row -----------------
            DispatchOutcome::Dispatched => {
                let mut exec_state = ExecutionState::new(execution_id, self.workflow_id, &[]);
                exec_state.set_workflow_input(input);
                let state_json = serde_json::to_value(&exec_state)
                    .map_err(|e| ActionError::fatal(format!("serialize execution state: {e}")))?;
                self.execution
                    .create(
                        &self.scope,
                        &execution_id.to_string(),
                        &self.workflow_id.to_string(),
                        state_json,
                    )
                    .await
                    .map_err(|e: StorageError| {
                        ActionError::retryable(format!("create execution row: {e}"))
                    })?;

                tracing::info!(
                    execution_id = %execution_id,
                    trigger_id   = %self.trigger_id,
                    workflow_id  = %self.workflow_id,
                    event_id     = event_id.as_ref().map(IdempotencyKey::as_str),
                    "durable_emitter: dispatched"
                );

                Ok(execution_id)
            },

            // --- step 5b: Duplicate — return id, create nothing -------------
            DispatchOutcome::Duplicate => {
                tracing::debug!(
                    execution_id = %execution_id,
                    trigger_id   = %self.trigger_id,
                    workflow_id  = %self.workflow_id,
                    event_id     = event_id.as_ref().map(IdempotencyKey::as_str),
                    "durable_emitter: duplicate (no-op)"
                );
                Ok(execution_id)
            },

            // `DispatchOutcome` is #[non_exhaustive] — a future variant whose
            // enqueue semantics are unknown MUST be rejected fail-closed.
            // Seeding a Created row without knowing whether a Start job was
            // enqueued would risk orphaning or double-dispatch.
            _ => Err(ActionError::fatal(format!(
                "unknown DispatchOutcome variant {outcome:?}; refusing to seed an execution row"
            ))),
        }
    }
}

impl ExecutionEmitter for DurableExecutionEmitter {
    fn emit(
        &self,
        input: serde_json::Value,
        event_id: Option<IdempotencyKey>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<ExecutionId, ActionError>> + Send + '_>> {
        Box::pin(self.do_emit(input, event_id))
    }
}
