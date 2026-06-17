//! [`EngineExecutionSink`] — orchestrator→engine hand-off for the
//! trigger-dispatch slice.
//!
//! Implements [`nebula_orchestrator::ExecutionSink`] by mirroring the logic
//! of [`EngineControlDispatch`]: read the persisted status for idempotency,
//! then call `resume_execution` for `Created`/`Paused` rows.
//!
//! ## Idempotency contract (mirrors `EngineControlDispatch`)
//!
//! The orchestrator's reclaim sweep can redeliver a `JobDispatchMsg` whose
//! `mark_dispatched` failed after a successful dispatch.  This sink handles
//! re-delivery via the same guard as `EngineControlDispatch::dispatch_start`:
//!
//! - **Already terminal / Running / Cancelling** → `Ok(())` (idempotent no-op).
//! - **Created / Paused** → drive via `resume_execution`.
//! - **Row not found** → `ExecutionSinkError::Rejected` (orphaned Start).
//! - **`EngineError::Leased`** → `Ok(())` (sibling runner already holds the
//!   lease; same reasoning as `EngineControlDispatch`).
//!
//! [`EngineControlDispatch`]: crate::control_dispatch::EngineControlDispatch
//! [`nebula_orchestrator::ExecutionSink`]: nebula_orchestrator::ExecutionSink

use std::sync::Arc;

use nebula_core::id::ExecutionId;
use nebula_execution::ExecutionStatus;
use nebula_orchestrator::{ExecutionSink, ExecutionSinkError};
use nebula_storage_port::{Scope, dto::JobDispatchMsg, store::ExecutionStore};

use crate::{WorkflowEngine, error::EngineError};

/// Orchestrator → engine hand-off.
///
/// Holds a shared [`WorkflowEngine`] and a scoped [`ExecutionStore`] handle
/// (the same stores the engine was configured with) so that:
///
/// 1. Status reads and engine lease CAS observe the same row.
/// 2. Re-delivery of a claimed job is safe (idempotent via status read).
///
/// Construction mirrors [`EngineControlDispatch::new`].
///
/// [`EngineControlDispatch::new`]: crate::control_dispatch::EngineControlDispatch::new
#[derive(Clone)]
pub struct EngineExecutionSink {
    engine: Arc<WorkflowEngine>,
    execution: Arc<dyn ExecutionStore>,
}

impl std::fmt::Debug for EngineExecutionSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineExecutionSink")
            .field("execution", &"Arc<dyn ExecutionStore>")
            .finish_non_exhaustive()
    }
}

impl EngineExecutionSink {
    /// Build a new sink.
    ///
    /// `execution` MUST be the same scoped store the engine was configured
    /// with via [`WorkflowEngine::with_execution_stores`] so the idempotency
    /// read and the engine's internal CAS observe the same row.
    ///
    /// [`WorkflowEngine::with_execution_stores`]: crate::WorkflowEngine::with_execution_stores
    #[must_use]
    pub fn new(engine: Arc<WorkflowEngine>, execution: Arc<dyn ExecutionStore>) -> Self {
        Self { engine, execution }
    }

    /// Read the persisted [`ExecutionStatus`] for idempotency under the given
    /// tenant `scope`, returning `None` if the row does not exist.
    ///
    /// `scope` MUST be the per-message scope from `JobDispatchMsg.scope` so
    /// that a row persisted under a different tenant is never visible here
    /// (cross-tenant isolation invariant #7).
    async fn read_status(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<Option<ExecutionStatus>, ExecutionSinkError> {
        let json = self
            .execution
            .get(scope, &execution_id.to_string())
            .await
            .map_err(|e| {
                ExecutionSinkError::Internal(format!(
                    "read execution state for idempotency guard: {e}"
                ))
            })?
            .map(|record| record.state);
        match json {
            None => Ok(None),
            Some(json) => match json.get("status") {
                Some(s) => serde_json::from_value::<ExecutionStatus>(s.clone())
                    .map(Some)
                    .map_err(|e| {
                        ExecutionSinkError::Internal(format!(
                            "execution {execution_id}: status field did not deserialize: {e}"
                        ))
                    }),
                None => Err(ExecutionSinkError::Internal(format!(
                    "execution {execution_id}: persisted state has no `status` field"
                ))),
            },
        }
    }

    /// Drive an execution through `resume_execution` under the given tenant scope.
    ///
    /// Maps [`EngineError::Leased`] to `Ok(())` (sibling runner already owns
    /// the dispatch; we should not mark the row Failed) and re-checks terminal
    /// status before surfacing other engine errors as `Internal`.
    async fn drive(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ExecutionSinkError> {
        match self.engine.resume_execution(scope, execution_id).await {
            Ok(_) => Ok(()),
            // Concurrent dispatcher already holds the lease — the canonical
            // idempotency outcome. Returning Ok prevents the orchestrator from
            // marking the row Failed; the lease holder owns the transition.
            Err(EngineError::Leased { .. }) => Ok(()),
            Err(e) => {
                // Last-ditch idempotency guard: re-read in case a sibling
                // driver beat us to terminal state between our initial read and
                // the engine's own `get_state` inside `resume_execution`.
                if let Ok(Some(status)) = self.read_status(scope, execution_id).await
                    && (status.is_terminal() || matches!(status, ExecutionStatus::Cancelling))
                {
                    return Ok(());
                }
                Err(ExecutionSinkError::Internal(format!(
                    "engine dispatch failed for {execution_id}: {e}"
                )))
            },
        }
    }
}

#[async_trait::async_trait]
impl ExecutionSink for EngineExecutionSink {
    #[tracing::instrument(
        level = "debug",
        skip(self, msg),
        fields(
            execution_id = %msg.execution_id,
            command      = msg.command.as_str(),
            reclaim      = msg.reclaim_count,
        )
    )]
    async fn dispatch(&self, msg: &JobDispatchMsg) -> Result<(), ExecutionSinkError> {
        let execution_id = msg.execution_id.parse::<ExecutionId>().map_err(|e| {
            ExecutionSinkError::Rejected(format!(
                "invalid execution_id `{}` in job dispatch msg: {e}",
                msg.execution_id
            ))
        })?;

        // msg.scope is the per-message tenant scope derived from the dispatch
        // row. read_status uses it so a row persisted under a different tenant
        // is not visible here — cross-tenant isolation invariant #7.
        match self.read_status(&msg.scope, execution_id).await? {
            None => Err(ExecutionSinkError::Rejected(format!(
                "execution {execution_id} not found — start command orphaned"
            ))),
            // Already past Created gate: running, cancelling, or terminal.
            // Re-delivered Start is a safe no-op (idempotency contract).
            Some(
                ExecutionStatus::Running
                | ExecutionStatus::Cancelling
                | ExecutionStatus::Completed
                | ExecutionStatus::Failed
                | ExecutionStatus::Cancelled
                | ExecutionStatus::TimedOut,
            ) => Ok(()),
            Some(ExecutionStatus::Created | ExecutionStatus::Paused) => {
                self.drive(&msg.scope, execution_id).await
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_core::{id::ExecutionId, plugin_key};
    use nebula_execution::ExecutionState;
    use nebula_orchestrator::{ExecutionSink, ExecutionSinkError};
    use nebula_storage::InMemoryExecutionStore;
    use nebula_storage_port::{
        Scope,
        dto::{ControlCommand, JobDispatchMsg},
        store::ExecutionStore,
    };

    use nebula_action::ActionResult;

    use super::EngineExecutionSink;
    use crate::{
        ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessRunner,
        WorkflowEngine,
    };

    /// Build the minimal `ActionRuntime` needed to construct a `WorkflowEngine`.
    /// The engine is never asked to actually run a workflow in this test — the
    /// cross-tenant check short-circuits in `read_status` before `resume_execution`
    /// is ever called.
    fn minimal_runtime() -> Arc<ActionRuntime> {
        let registry = Arc::new(ActionRegistry::new());
        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = nebula_metrics::MetricsRegistry::new();
        Arc::new(
            ActionRuntime::try_new(registry, runner, DataPassingPolicy::default(), metrics)
                .expect("minimal runtime"),
        )
    }

    /// Invariant #7 — fail-closed cross-tenant isolation.
    ///
    /// The row is seeded under `single_tenant_scope()` — `("nebula","nebula")`,
    /// the exact value the old `engine_scope()` constant returned. A
    /// `JobDispatchMsg` carrying `scope_b = ("wsB","orgB")` is then dispatched.
    ///
    /// **Why this is RED on the old code:**
    /// The old `read_status` called `self.execution.get(engine_scope(), id)` — i.e.
    /// `get(("nebula","nebula"), id)` — which FINDS the seeded row, so the `None`
    /// arm is NOT taken. Control falls through to `drive`, which calls
    /// `resume_execution(engine_scope(), id)`. The engine has no workflow store
    /// attached (minimal construction), so `resume_execution` returns
    /// `EngineError::StoreNotConfigured` which maps to `ExecutionSinkError::Internal`
    /// — NOT `Rejected`. The `assert!(matches!(Rejected))` therefore fails → RED.
    ///
    /// **Why this is GREEN on the new code:**
    /// `read_status` now reads under `msg.scope = scope_b = ("wsB","orgB")`.
    /// The store holds no row under that scope → `None` → the function returns
    /// `Err(Rejected("not found …"))` immediately, never reaching `drive` → GREEN.
    #[tokio::test]
    async fn cross_tenant_dispatch_is_rejected() {
        // The row is intentionally seeded under single_tenant_scope() — the same
        // ("nebula","nebula") the OLD engine_scope() constant always returned.
        // This makes the test RED when read_status is reverted to the constant.
        let single_tenant = crate::store_seam::single_tenant_scope();
        let scope_b = Scope::new("wsB", "orgB");

        let execution_store: Arc<dyn ExecutionStore> = Arc::new(InMemoryExecutionStore::new());
        let execution_id = ExecutionId::new();
        let exec_state = ExecutionState::new(execution_id, nebula_core::id::WorkflowId::new(), &[]);
        let state_json = serde_json::to_value(&exec_state).unwrap();
        execution_store
            .create(
                &single_tenant, // row lives under ("nebula","nebula") = old constant
                &execution_id.to_string(),
                &nebula_core::id::WorkflowId::new().to_string(),
                state_json,
            )
            .await
            .unwrap();

        // Minimal engine (no stores) — the test must short-circuit in read_status,
        // never reaching resume_execution.
        let metrics = nebula_metrics::MetricsRegistry::new();
        let engine = Arc::new(WorkflowEngine::new(minimal_runtime(), metrics).expect("engine"));
        let sink = EngineExecutionSink::new(Arc::clone(&engine), Arc::clone(&execution_store));

        // Dispatch under scope_b — a different tenant than the row's scope.
        let plugin_key = plugin_key!("test.cross_tenant");
        let msg = JobDispatchMsg::new(
            [0u8; 16],
            execution_id.to_string(),
            ControlCommand::Start,
            scope_b, // wrong tenant: row lives under single_tenant, not scope_b
            serde_json::Value::Null,
            None::<String>,
            "sha",
            plugin_key.clone(),
            vec![plugin_key],
            None::<String>,
            0,
        );

        let result = sink.dispatch(&msg).await;

        // New code: read_status reads under scope_b → None → Rejected.
        // Old code: read_status reads under ("nebula","nebula") → finds the row →
        //   drive → resume_execution → Internal (no store) — NOT Rejected → RED.
        match result {
            Err(ExecutionSinkError::Rejected(msg)) => {
                assert!(
                    msg.contains("not found"),
                    "expected 'not found' in rejection message, got: {msg}"
                );
            },
            other => panic!("expected Rejected, got: {other:?}"),
        }
    }
}
