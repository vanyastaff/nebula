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
    /// The idempotency reads performed by this sink use the `scope` carried in
    /// each [`JobDispatchMsg`] — the same scope the emitter used when it
    /// persisted the `Created` row — rather than a global placeholder.
    ///
    /// [`WorkflowEngine::with_execution_stores`]: crate::WorkflowEngine::with_execution_stores
    /// [`JobDispatchMsg`]: nebula_storage_port::dto::JobDispatchMsg
    #[must_use]
    pub fn new(engine: Arc<WorkflowEngine>, execution: Arc<dyn ExecutionStore>) -> Self {
        Self { engine, execution }
    }

    /// Read the persisted [`ExecutionStatus`] for idempotency, returning
    /// `None` if the row does not exist.
    ///
    /// `scope` must be the scope the emitter used when it wrote the `Created`
    /// row — i.e. the [`Scope`] carried in the [`JobDispatchMsg`] — so the
    /// read addresses the same tenant partition as the original write.
    ///
    /// [`JobDispatchMsg`]: nebula_storage_port::dto::JobDispatchMsg
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

    /// Drive an execution through `resume_execution`.
    ///
    /// Maps [`EngineError::Leased`] to `Ok(())` (sibling runner already owns
    /// the dispatch; we should not mark the row Failed) and re-checks terminal
    /// status before surfacing other engine errors as `Internal`.
    ///
    /// `scope` is threaded to [`Self::read_status`] for the last-ditch
    /// idempotency re-read so it addresses the same tenant partition as the
    /// initial read.
    async fn drive(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ExecutionSinkError> {
        match self.engine.resume_execution(execution_id).await {
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
