//! [`DurableExecutionEmitter`] — trigger fan-out through the durable dedup
//! inbox for the ADR-0095 trigger-dispatch slice.
//!
//! ## Responsibility
//!
//! `DurableExecutionEmitter` implements [`nebula_action::ExecutionEmitter`] and
//! wires the trigger-to-execution fan-out path end-to-end:
//!
//! 1. Resolve routing from the [`ValidatedWorkflow`] (fail-fast before minting
//!    an id): `required_plugin_key` + `required_plugins` + flavor SHA.
//! 2. Mint a new [`ExecutionId`] candidate and build the initial
//!    [`ExecutionState`].
//! 3. Build a [`JobDispatchMsg`] (Start command, routing key, etc.), a
//!    [`NewExecution`] carrying the serialised initial state, and — when
//!    `event_id` is `Some` — a [`TriggerDedupRow`] guard keyed by
//!    `(scope, trigger_id, event_id)`.
//! 4. Call [`TriggerDedupInbox::claim_and_materialize_start`] — one atomic
//!    critical section that either:
//!    - **`Dispatched`**: inserts the dedup guard + the `Created` execution row
//!      + enqueues the Start job; returns the effective execution id.
//!    - **`Duplicate`**: returns the *original winner's* execution id without
//!      touching any row.
//! 5. Parse the returned `outcome.execution_id` back to [`ExecutionId`] and
//!    return it to the caller.
//!
//! ## Atomicity guarantee
//!
//! The dedup guard, `Created` execution row, and Start job are written in a
//! single database transaction inside `claim_and_materialize_start`. A
//! concurrently-polling orchestrator can never see the Start job before the
//! execution row — the race window that existed when the Created-row was a
//! second separate write is closed.
//!
//! ## Wiring honesty
//!
//! No prod trigger daemon installs this emitter today — all non-test
//! `TriggerRuntimeContext::new` sites use the default `NoopExecutionEmitter`.
//! Install via `ctx.with_emitter(Arc::new(DurableExecutionEmitter::new(...)))` in
//! the harness or a future trigger daemon; the integration test is the sole
//! current caller.
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
use nebula_core::id::ExecutionId;
use nebula_execution::ExecutionState;
use nebula_storage_port::{
    Scope, StorageError,
    dto::{ControlCommand, DispatchKind, JobDispatchMsg, NewExecution, TriggerDedupRow},
    store::TriggerDedupInbox,
};
use nebula_workflow::ValidatedWorkflow;

use crate::daemon::routing::RoutingResolver;

/// Trigger fan-out through the durable dedup inbox.
///
/// Holds everything needed to enqueue one trigger-originated Start job:
/// - [`ValidatedWorkflow`] — the validated workflow being triggered.  Passed
///   to [`RoutingResolver`] so routing is derived from the definition without a
///   registry lookup.  Using a validation witness ensures this seam can never
///   be reached with an unvalidated definition.
/// - [`TriggerDedupInbox`] — atomic dedup + execution-row materialise + job
///   enqueue, all in one transaction.
/// - [`RoutingResolver`] — derive `required_plugin_key` + `required_plugins`
///   + `target_flavor_sha` from the validated definition.
/// - Identity fields captured at construction: `trigger_id`, `scope` — same
///   values on every `emit` call from this trigger context.
#[derive(Clone)]
pub struct DurableExecutionEmitter {
    workflow: Arc<ValidatedWorkflow>,
    dedup: Arc<dyn TriggerDedupInbox>,
    resolver: Arc<dyn RoutingResolver>,
    // Cached at construction from `TriggerRuntimeContext`.
    trigger_id: NodeKey,
    scope: Scope,
}

impl std::fmt::Debug for DurableExecutionEmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DurableExecutionEmitter")
            .field("workflow_id", &self.workflow.definition().id)
            .field("trigger_id", &self.trigger_id)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl DurableExecutionEmitter {
    /// Construct a durable emitter.
    ///
    /// `dedup` MUST be backed by the same store (or share the same
    /// `Arc<Mutex<…>>` for the InMemory adapter) as the `JobDispatchQueue`
    /// passed to the orchestrator, so `claim_and_materialize_start` is atomic
    /// across all three writes.
    #[must_use]
    pub fn new(
        dedup: Arc<dyn TriggerDedupInbox>,
        resolver: Arc<dyn RoutingResolver>,
        workflow: Arc<ValidatedWorkflow>,
        trigger_id: NodeKey,
        scope: Scope,
    ) -> Self {
        Self {
            workflow,
            dedup,
            resolver,
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
            workflow_id = %self.workflow.definition().id,
            event_id    = event_id.as_ref().map(IdempotencyKey::as_str),
        )
    )]
    async fn do_emit(
        &self,
        input: serde_json::Value,
        event_id: Option<IdempotencyKey>,
    ) -> Result<ExecutionId, ActionError> {
        let workflow_id = self.workflow.definition().id;

        // --- step 1: resolve routing from validated definition (fail-fast) ----
        let route = self
            .resolver
            .resolve(&self.workflow, &self.trigger_id)
            .map_err(ActionError::fatal_from)?;

        // --- step 2: mint the candidate id + serialise the initial state -----
        //
        // The candidate id is passed to `claim_and_materialize_start`.
        // On `Dispatched` the store inserts the execution row with this id.
        // On `Duplicate` the store returns the *winner's* id — which may differ.
        let candidate_id = ExecutionId::new();

        let mut exec_state = ExecutionState::new(candidate_id, workflow_id, &[]);
        exec_state.set_workflow_input(input.clone());
        let state_json = serde_json::to_value(&exec_state)
            .map_err(|e| ActionError::fatal(format!("serialize execution state: {e}")))?;

        // Mint a fresh ULID for the job-dispatch row primary key.
        // The field is documented as "16-byte ULID (raw bytes)"; time-sortable
        // ordering is required for the storage reclaim cutoff arithmetic.
        let job_id: [u8; 16] = ulid::Ulid::new().to_bytes();

        let start = JobDispatchMsg::new(
            job_id,
            candidate_id.to_string(),
            ControlCommand::Start,
            self.scope.clone(),
            input,
            event_id.as_ref().map(IdempotencyKey::as_str),
            route.target_flavor_sha.clone(),
            route.required_plugin_key.clone(),
            route.required_plugins.clone(),
            None::<String>, // w3c_traceparent: future D1
            0,              // reclaim_count: 0 on first enqueue
        );

        let workflow_id_str = workflow_id.to_string();
        let new_execution = NewExecution::new(&workflow_id_str, &state_json);

        // --- step 3: build the dedup guard row (only when event_id present) --
        let dedup_row = event_id.as_ref().map(|eid| {
            TriggerDedupRow::new(
                self.trigger_id.as_str(),
                eid.as_str(),
                self.scope.clone(),
                chrono::Utc::now().to_rfc3339(),
            )
        });

        // --- step 4: atomic dedup ∧ execution-row ∧ Start-enqueue -----------
        //
        // `claim_and_materialize_start` runs all three writes in one transaction:
        //   Dispatched — dedup guard + Created execution row + Start job
        //   Duplicate  — guard already present; returns winner's execution id
        //
        // The returned `outcome.execution_id` is the EFFECTIVE id — on
        // Dispatched it equals `candidate_id`; on Duplicate it is the original
        // winner's id.
        let outcome = self
            .dedup
            .claim_and_materialize_start(dedup_row.as_ref(), &start, &new_execution)
            .await
            .map_err(|e: StorageError| {
                ActionError::retryable(format!("dedup inbox storage error: {e}"))
            })?;

        tracing::debug!(
            outcome_kind = ?outcome.kind,
            effective_execution_id = %outcome.execution_id,
            candidate_id = %candidate_id,
            trigger_id   = %self.trigger_id,
            workflow_id  = %workflow_id,
            event_id     = event_id.as_ref().map(IdempotencyKey::as_str),
            "durable_emitter: claim_and_materialize_start"
        );

        match outcome.kind {
            DispatchKind::Dispatched => {
                tracing::info!(
                    execution_id = %outcome.execution_id,
                    trigger_id   = %self.trigger_id,
                    workflow_id  = %workflow_id,
                    event_id     = event_id.as_ref().map(IdempotencyKey::as_str),
                    "durable_emitter: dispatched"
                );
            },
            DispatchKind::Duplicate => {
                tracing::debug!(
                    winner_execution_id = %outcome.execution_id,
                    candidate_id = %candidate_id,
                    trigger_id   = %self.trigger_id,
                    workflow_id  = %workflow_id,
                    event_id     = event_id.as_ref().map(IdempotencyKey::as_str),
                    "durable_emitter: duplicate — returning winner id"
                );
            },
            // `DispatchKind` is #[non_exhaustive] — a future variant whose
            // semantics are unknown MUST be rejected fail-closed.
            _ => {
                return Err(ActionError::fatal(format!(
                    "unknown DispatchKind variant {:?}; refusing to return an execution id",
                    outcome.kind
                )));
            },
        }

        // Parse the effective id back to the typed wrapper.
        outcome.execution_id.parse::<ExecutionId>().map_err(|e| {
            ActionError::fatal(format!("effective execution id is not a valid ULID: {e}"))
        })
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
