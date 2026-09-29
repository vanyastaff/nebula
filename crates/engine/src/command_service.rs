//! Execution control commands: the one owner of the §12.2 contract.
//!
//! Cancel and terminate are **intents**, not writes. The execution aggregate has
//! exactly one writer — the runtime, holding the lease and the fencing token that
//! proves it — so this service reads the durable state, decides whether the
//! command is admissible, and records durable intent on the control queue. The
//! runtime performs `Running → Cancelling → Cancelled` under its own authority
//! once it has honored the command.
//!
//! Every surface (the HTTP handlers today; the embedded façade next) calls this
//! service instead of copying the contract.

use std::sync::Arc;

use nebula_core::{ExecutionId, W3cTraceContext};
use nebula_execution::ExecutionStatus;
use nebula_metrics::{
    MetricsRegistry,
    naming::{NEBULA_ENGINE_EXECUTION_COMMAND_TOTAL, execution_command_outcome},
};
use nebula_storage_port::{
    Scope, StorageError,
    dto::{ControlCommand, ControlMsg},
    store::{ControlQueue, ExecutionStore},
};
use nebula_tenancy::{ScopedControlQueue, ScopedExecutionStore};

/// Statuses from which no control command can change the outcome.
const TERMINAL_STATUSES: [&str; 4] = ["completed", "failed", "cancelled", "timed_out"];

/// Why a control command was not accepted.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExecutionCommandError {
    /// No execution with that id exists in the caller's tenant.
    #[error("execution {0} not found")]
    NotFound(ExecutionId),
    /// The execution already reached a terminal state.
    #[error("Cannot {verb} execution in '{status}' state")]
    Terminal {
        /// The command that was refused (`cancel` / `terminate`).
        verb: &'static str,
        /// The terminal status the execution is in.
        status: String,
    },
    /// The execution store could not be read.
    #[error("failed to read execution: {0}")]
    Store(String),
    /// The control-queue backend is absent or unreachable (infra down, not a
    /// logic bug); the command was **not** recorded.
    #[error("control-queue backend unavailable: {0}")]
    QueueUnavailable(String),
    /// The control-queue write failed; the command was **not** recorded.
    #[error("failed to enqueue control command: {0}")]
    Enqueue(String),
}

/// What the service accepted.
#[derive(Debug, Clone)]
pub struct CommandReceipt {
    /// The execution state exactly as it is durably stored right now.
    pub execution_state: serde_json::Value,
    /// `false` when the command was already in flight (duplicate cancel), so no
    /// second control row was written.
    pub enqueued: bool,
}

/// Submits typed control commands for executions of one tenant at a time.
#[derive(Debug, Clone)]
pub struct ExecutionCommandService {
    execution_store: Arc<dyn ExecutionStore>,
    control_queue: Arc<dyn ControlQueue>,
    metrics: MetricsRegistry,
}

impl ExecutionCommandService {
    /// Compose the service over the shared (unscoped) storage handles; every
    /// call binds them to the caller's tenant `Scope`.
    #[must_use]
    pub fn new(
        execution_store: Arc<dyn ExecutionStore>,
        control_queue: Arc<dyn ControlQueue>,
    ) -> Self {
        Self {
            execution_store,
            control_queue,
            metrics: MetricsRegistry::new(),
        }
    }

    /// Record command outcomes on the shared registry (default: a private one).
    #[must_use]
    pub fn with_metrics(mut self, metrics: MetricsRegistry) -> Self {
        self.metrics = metrics;
        self
    }

    fn record(&self, verb: &'static str, outcome: &'static str) {
        let labels = self
            .metrics
            .interner()
            .label_set(&[("command", verb), ("outcome", outcome)]);
        if let Ok(counter) = self
            .metrics
            .counter_labeled(NEBULA_ENGINE_EXECUTION_COMMAND_TOTAL, &labels)
        {
            counter.inc();
        }
    }

    /// Request cooperative cancellation. A duplicate request while the
    /// execution is already `Cancelling` is idempotent: nothing is enqueued.
    ///
    /// # Errors
    ///
    /// See [`ExecutionCommandError`].
    #[tracing::instrument(
        name = "execution.command.cancel",
        skip(self, scope, w3c),
        fields(execution_id = %execution_id),
    )]
    pub async fn cancel(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        w3c: Option<W3cTraceContext>,
    ) -> Result<CommandReceipt, ExecutionCommandError> {
        self.submit(
            scope,
            execution_id,
            ControlCommand::Cancel,
            "cancel",
            true,
            w3c,
        )
        .await
    }

    /// Request termination. The engine has no forced-shutdown path, so this is
    /// a cooperative-cancel synonym on the queue; unlike [`Self::cancel`] a
    /// repeated request is enqueued again.
    ///
    /// # Errors
    ///
    /// See [`ExecutionCommandError`].
    #[tracing::instrument(
        name = "execution.command.terminate",
        skip(self, scope, w3c),
        fields(execution_id = %execution_id),
    )]
    pub async fn terminate(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        w3c: Option<W3cTraceContext>,
    ) -> Result<CommandReceipt, ExecutionCommandError> {
        self.submit(
            scope,
            execution_id,
            ControlCommand::Terminate,
            "terminate",
            false,
            w3c,
        )
        .await
    }

    async fn submit(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        command: ControlCommand,
        verb: &'static str,
        idempotent_while_cancelling: bool,
        w3c: Option<W3cTraceContext>,
    ) -> Result<CommandReceipt, ExecutionCommandError> {
        let result = self
            .submit_inner(
                scope,
                execution_id,
                command,
                verb,
                idempotent_while_cancelling,
                w3c,
            )
            .await;
        self.record(
            verb,
            match &result {
                Ok(receipt) if receipt.enqueued => execution_command_outcome::ENQUEUED,
                Ok(_) => execution_command_outcome::DUPLICATE,
                Err(ExecutionCommandError::Terminal { .. }) => execution_command_outcome::TERMINAL,
                Err(ExecutionCommandError::NotFound(_)) => execution_command_outcome::NOT_FOUND,
                Err(ExecutionCommandError::QueueUnavailable(_)) => {
                    execution_command_outcome::UNAVAILABLE
                },
                Err(_) => execution_command_outcome::FAILED,
            },
        );
        result
    }

    async fn submit_inner(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        command: ControlCommand,
        verb: &'static str,
        idempotent_while_cancelling: bool,
        w3c: Option<W3cTraceContext>,
    ) -> Result<CommandReceipt, ExecutionCommandError> {
        let store = ScopedExecutionStore::new(Arc::clone(&self.execution_store), scope.clone());
        let record = store
            .get(scope, &execution_id.to_string())
            .await
            .map_err(|e| ExecutionCommandError::Store(e.to_string()))?
            .ok_or(ExecutionCommandError::NotFound(execution_id))?;
        let execution_state = record.state;

        let status = execution_state
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if TERMINAL_STATUSES.contains(&status) {
            return Err(ExecutionCommandError::Terminal {
                verb,
                status: status.to_owned(),
            });
        }

        // The command is already in flight and runtime control owns the
        // outcome, so re-requesting it must not enqueue a second row.
        if idempotent_while_cancelling && status == ExecutionStatus::Cancelling.to_string() {
            tracing::debug!(
                execution_id = %execution_id,
                command = command.as_str(),
                "execution: command already in flight; returning the stored state"
            );
            return Ok(CommandReceipt {
                execution_state,
                enqueued: false,
            });
        }

        tracing::debug!(
            execution_id = %execution_id,
            command = command.as_str(),
            has_trace_context = w3c.is_some(),
            "execution: enqueue control command"
        );
        let queue = ScopedControlQueue::new(Arc::clone(&self.control_queue), scope.clone());
        let msg = ControlMsg {
            id: *uuid::Uuid::new_v4().as_bytes(),
            execution_id: execution_id.to_string(),
            command,
            scope: scope.clone(),
            w3c_traceparent: w3c.as_ref().map(|c| c.traceparent().to_owned()),
            reclaim_count: 0,
            resume_target: None,
        };
        queue.enqueue(&msg).await.map_err(|e| {
            let detail = e.to_string();
            if matches!(e, StorageError::Internal(_) | StorageError::Connection(_)) {
                ExecutionCommandError::QueueUnavailable(detail)
            } else {
                ExecutionCommandError::Enqueue(detail)
            }
        })?;

        Ok(CommandReceipt {
            execution_state,
            enqueued: true,
        })
    }
}
