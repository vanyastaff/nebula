//! Execution control commands: the one owner of the §12.2 contract.
//!
//! Cancel, terminate, typed signals and webhook resumes are **intents**, not writes.
//! The execution aggregate has exactly one writer — the runtime, holding the
//! lease and the fencing token that proves it — so this service reads the
//! durable state, decides whether the command is admissible, and records
//! durable intent on the control queue. The runtime performs
//! `Running → Cancelling → Cancelled` under its own authority once it has
//! honored the command. A webhook resume is admitted from its bearer token
//! instead, and the token burn commits atomically with the `Resume` row.
//!
//! Every surface (the HTTP handlers today; the embedded façade next) calls this
//! service instead of copying the contract.

use std::sync::Arc;

use nebula_core::{ExecutionId, Principal, W3cTraceContext};
use nebula_execution::ExecutionStatus;
use nebula_metrics::{
    MetricsRegistry,
    naming::{NEBULA_ENGINE_EXECUTION_COMMAND_TOTAL, execution_command_outcome},
};
use nebula_storage_port::{
    Scope, StorageError,
    dto::{
        ControlCommand, ControlMsg, ResumeTarget,
        resume_token::{ResumeTokenWaitKind, TokenHash},
    },
    store::{ControlQueue, ExecutionStore, ResumeProducer},
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
        /// The command that was refused (`cancel`, `terminate` or `signal`).
        verb: &'static str,
        /// The terminal status the execution is in.
        status: String,
    },
    /// The durable command state (the execution row or the resume-token row)
    /// could not be read; the cause is the [`source`](std::error::Error::source).
    #[error("failed to read command state from storage")]
    Store(#[source] StorageError),
    /// The control-queue backend is absent or unreachable (infra down, not a
    /// logic bug); the command was **not** recorded.
    #[error("control-queue backend unavailable")]
    QueueUnavailable(#[source] StorageError),
    /// The control-queue write failed; the command was **not** recorded.
    #[error("failed to enqueue control command")]
    Enqueue(#[source] StorageError),
    /// Only a human user can approve: the principal is not a user.
    #[error("only a user principal can deliver an approval signal")]
    ApproverNotUser,
    /// An `ExecutionCompleted` signal named an execution that has not reached
    /// a terminal state; the completion is a durable fact, not a claim.
    #[error("awaited execution {execution_id} has not completed (status '{status}')")]
    AwaitedNotTerminal {
        /// The execution the signal claimed had completed.
        execution_id: ExecutionId,
        /// Its current durable status.
        status: String,
    },
    /// The bearer is absent, expired, consumed or not a webhook token. One
    /// variant for every case, so the refusal never reveals which applied.
    #[error("resume token not found")]
    ResumeTokenNotFound,
    /// No resume producer was composed into this service, so a webhook resume
    /// cannot be recorded (a composition-root fault).
    #[error("webhook resume is not wired: no resume producer")]
    ResumeUnwired,
}

impl ExecutionCommandError {
    /// Classify a failed control-queue write: an unreachable backend is
    /// `QueueUnavailable`, anything else is `Enqueue`. Both keep the cause.
    fn from_enqueue(error: StorageError) -> Self {
        if matches!(
            error,
            StorageError::Internal(_) | StorageError::Connection(_)
        ) {
            Self::QueueUnavailable(error)
        } else {
            Self::Enqueue(error)
        }
    }

    /// The `outcome` label this refusal is counted under.
    fn outcome(&self) -> &'static str {
        match self {
            Self::Terminal { .. } => execution_command_outcome::TERMINAL,
            Self::ApproverNotUser | Self::AwaitedNotTerminal { .. } => {
                execution_command_outcome::FORBIDDEN
            },
            Self::NotFound(_) | Self::ResumeTokenNotFound => execution_command_outcome::NOT_FOUND,
            Self::QueueUnavailable(_) | Self::ResumeUnwired => {
                execution_command_outcome::UNAVAILABLE
            },
            Self::Store(_) | Self::Enqueue(_) => execution_command_outcome::FAILED,
        }
    }
}

/// A signal an authenticated caller may deliver through
/// [`ExecutionCommandService::signal`]. Its identity is never a free-form
/// payload: the approver comes from the caller's principal, and webhook waits
/// are not representable (they resume only with their bearer).
///
/// ```no_run
/// # use nebula_engine::{ExecutionCommandService, Signal};
/// # async fn deliver(svc: &ExecutionCommandService, scope: &nebula_storage_port::Scope,
/// #     id: nebula_core::ExecutionId, caller: &nebula_core::Principal) {
/// let _ = svc.signal(scope, id, caller, Signal::Approval, None).await;
/// # }
/// ```
///
/// A webhook target cannot be signalled by an authenticated caller:
///
/// ```compile_fail
/// # use nebula_engine::{ExecutionCommandService, Signal};
/// # async fn deliver(svc: &ExecutionCommandService, scope: &nebula_storage_port::Scope,
/// #     id: nebula_core::ExecutionId, caller: &nebula_core::Principal) {
/// let webhook = Signal::Webhook { callback_id: "cb".to_owned() };
/// let _ = svc.signal(scope, id, caller, webhook, None).await;
/// # }
/// ```
///
/// Nor can a caller name the approver:
///
/// ```compile_fail
/// # use nebula_engine::{ExecutionCommandService, Signal};
/// # async fn deliver(svc: &ExecutionCommandService, scope: &nebula_storage_port::Scope,
/// #     id: nebula_core::ExecutionId, caller: &nebula_core::Principal) {
/// let approval = Signal::Approval { approver: "anyone".to_owned() };
/// let _ = svc.signal(scope, id, caller, approval, None).await;
/// # }
/// ```
///
/// And there is no untargeted resume, which would arm every signal wait:
///
/// ```compile_fail
/// # use nebula_engine::ExecutionCommandService;
/// # async fn deliver(svc: &ExecutionCommandService, scope: &nebula_storage_port::Scope,
/// #     id: nebula_core::ExecutionId) {
/// let _ = svc.resume(scope, id, None).await;
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Signal {
    /// Approve the gate parked for the calling user.
    Approval,
    /// Report that the awaited execution completed.
    ExecutionCompleted {
        /// The execution the parked wait is gated on.
        execution_id: ExecutionId,
    },
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
    resume_producer: Option<Arc<dyn ResumeProducer>>,
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
            resume_producer: None,
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
            (ControlCommand::Cancel, None),
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
            (ControlCommand::Terminate, None),
            "terminate",
            false,
            w3c,
        )
        .await
    }

    /// Install the atomic bearer-resume producer used by
    /// [`Self::resume_webhook`]. It must be the undecorated producer of the
    /// same backend as the control queue, so the token burn and the `Resume`
    /// row commit in one transaction.
    #[must_use]
    pub fn with_resume_producer(mut self, producer: Arc<dyn ResumeProducer>) -> Self {
        self.resume_producer = Some(producer);
        self
    }

    /// Deliver a typed signal to one parked wait of the execution.
    ///
    /// Authority comes from the caller, never from a payload:
    ///
    /// - [`Signal::Approval`] targets the approval gate whose approver is the
    ///   authenticated `principal`'s user id (`usr_…`); any other principal
    ///   kind is refused with [`ExecutionCommandError::ApproverNotUser`].
    /// - [`Signal::ExecutionCompleted`] is admitted only if the awaited execution
    ///   exists in `scope` and has durably reached a terminal state; otherwise it
    ///   is refused (`NotFound` / [`ExecutionCommandError::AwaitedNotTerminal`]).
    /// - A webhook wait cannot be signalled here — it resumes only through
    ///   [`Self::resume_webhook`] with its verified bearer.
    /// - There is deliberately no untargeted resume: this service only ever
    ///   enqueues identity-targeted signals. The runtime independently refuses
    ///   to let an untargeted Resume satisfy approval or webhook gates, so
    ///   neither layer alone is trusted with that authority.
    ///
    /// The signal is acknowledged only once it is durably on the control
    /// queue; it never travels over the event bus.
    ///
    /// # Errors
    ///
    /// See [`ExecutionCommandError`]; delivery remains execution-owner fenced.
    #[tracing::instrument(
        name = "execution.command.signal",
        skip(self, scope, principal, signal, w3c),
        fields(execution_id = %execution_id),
    )]
    pub async fn signal(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        principal: &Principal,
        signal: Signal,
        w3c: Option<W3cTraceContext>,
    ) -> Result<CommandReceipt, ExecutionCommandError> {
        let target = match signal {
            Signal::Approval => {
                let Principal::User(user) = principal else {
                    let refusal = ExecutionCommandError::ApproverNotUser;
                    self.record("signal", refusal.outcome());
                    return Err(refusal);
                };
                ResumeTarget::Approval {
                    approver: user.to_string(),
                }
            },
            Signal::ExecutionCompleted {
                execution_id: awaited,
            } => {
                // The completion must be a durable fact in the caller's own
                // scope, never the caller's claim.
                if let Err(refusal) = self.require_terminal(scope, awaited).await {
                    self.record("signal", refusal.outcome());
                    return Err(refusal);
                }
                ResumeTarget::Execution {
                    execution_id: awaited.to_string(),
                }
            },
        };
        self.submit(
            scope,
            execution_id,
            (ControlCommand::Resume, Some(target)),
            "signal",
            false,
            w3c,
        )
        .await
    }

    /// Atomically consume a webhook bearer and enqueue its exact Resume.
    ///
    /// Scope and target come exclusively from the token row. Wrong kind,
    /// expired, malformed and replayed tokens all have the same refusal.
    /// Transport rate limiting stays outside this owner; no token is burned
    /// independently of its durable command.
    ///
    /// # Errors
    ///
    /// [`ExecutionCommandError::ResumeTokenNotFound`] for every refused token,
    /// [`ExecutionCommandError::ResumeUnwired`] without a producer, or the
    /// backend failure; on any error the token stays live.
    #[tracing::instrument(
        name = "execution.command.resume_webhook",
        skip_all,
        fields(outcome = tracing::field::Empty),
    )]
    pub async fn resume_webhook(
        &self,
        hash: &TokenHash,
        now: std::time::SystemTime,
        w3c: Option<W3cTraceContext>,
    ) -> Result<Scope, ExecutionCommandError> {
        let result = self.resume_webhook_inner(hash, now, w3c).await;
        let outcome = match &result {
            Ok(_) => execution_command_outcome::ENQUEUED,
            Err(error) => error.outcome(),
        };
        tracing::Span::current().record("outcome", outcome);
        // Its own label: unauthenticated bearer traffic (forged tokens
        // included) must not pollute the authenticated command series.
        self.record("resume_webhook", outcome);
        result
    }

    async fn resume_webhook_inner(
        &self,
        hash: &TokenHash,
        now: std::time::SystemTime,
        w3c: Option<W3cTraceContext>,
    ) -> Result<Scope, ExecutionCommandError> {
        let producer = self
            .resume_producer
            .as_ref()
            .ok_or(ExecutionCommandError::ResumeUnwired)?;
        // Read-only peek: every refusal below leaves the token unburned.
        let row = producer
            .peek(hash)
            .await
            .map_err(ExecutionCommandError::Store)?
            .ok_or(ExecutionCommandError::ResumeTokenNotFound)?;
        // `ResumeTokenWaitKind` is non-exhaustive: only `Webhook` is admissible.
        if row.wait_kind != ResumeTokenWaitKind::Webhook {
            tracing::debug!(
                execution_id = %row.execution_id,
                wait_kind = ?row.wait_kind,
                "webhook resume: token is not a webhook wait (no burn)"
            );
            return Err(ExecutionCommandError::ResumeTokenNotFound);
        }
        if let Some(expires_at) = row.expires_at.as_deref() {
            let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(expires_at) else {
                tracing::warn!(
                    execution_id = %row.execution_id,
                    expires_at,
                    "webhook resume: malformed expires_at, failing closed (no burn)"
                );
                return Err(ExecutionCommandError::ResumeTokenNotFound);
            };
            // Pre-epoch expiries fail closed, as the transport always did.
            if expiry.timestamp() < 0 || now >= std::time::SystemTime::from(expiry) {
                tracing::debug!(
                    execution_id = %row.execution_id,
                    expires_at,
                    "webhook resume: token expired (no burn)"
                );
                return Err(ExecutionCommandError::ResumeTokenNotFound);
            }
        }
        let msg = ControlMsg {
            id: *uuid::Uuid::new_v4().as_bytes(),
            execution_id: row.execution_id,
            command: ControlCommand::Resume,
            scope: row.scope.clone(),
            w3c_traceparent: w3c.map(|context| context.traceparent().to_owned()),
            reclaim_count: 0,
            resume_target: Some(ResumeTarget::Webhook {
                callback_id: row.callback_label,
            }),
        };
        // One transaction burns the token and records the Resume: a failure
        // rolls both back, and losing a concurrent race returns `false`.
        if !producer
            .consume_and_enqueue_resume(hash, &msg)
            .await
            .map_err(ExecutionCommandError::from_enqueue)?
        {
            return Err(ExecutionCommandError::ResumeTokenNotFound);
        }
        tracing::debug!(execution_id = %msg.execution_id, "webhook resume intent recorded atomically");
        Ok(row.scope)
    }

    async fn submit(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        intent: (ControlCommand, Option<ResumeTarget>),
        verb: &'static str,
        idempotent_while_cancelling: bool,
        w3c: Option<W3cTraceContext>,
    ) -> Result<CommandReceipt, ExecutionCommandError> {
        let result = self
            .submit_inner(
                scope,
                execution_id,
                intent,
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
                Err(error) => error.outcome(),
            },
        );
        result
    }

    /// Admit an `ExecutionCompleted` claim only if the awaited execution exists
    /// in `scope` and has durably reached a terminal state. A foreign-scope or
    /// absent id is `NotFound`, so the refusal reveals nothing across tenants.
    async fn require_terminal(
        &self,
        scope: &Scope,
        awaited: ExecutionId,
    ) -> Result<(), ExecutionCommandError> {
        let store = ScopedExecutionStore::new(Arc::clone(&self.execution_store), scope.clone());
        let record = store
            .get(scope, &awaited.to_string())
            .await
            .map_err(ExecutionCommandError::Store)?
            .ok_or(ExecutionCommandError::NotFound(awaited))?;
        let status = record
            .state
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if TERMINAL_STATUSES.contains(&status) {
            Ok(())
        } else {
            Err(ExecutionCommandError::AwaitedNotTerminal {
                execution_id: awaited,
                status: status.to_owned(),
            })
        }
    }

    async fn submit_inner(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        intent: (ControlCommand, Option<ResumeTarget>),
        verb: &'static str,
        idempotent_while_cancelling: bool,
        w3c: Option<W3cTraceContext>,
    ) -> Result<CommandReceipt, ExecutionCommandError> {
        let (command, target) = intent;
        let store = ScopedExecutionStore::new(Arc::clone(&self.execution_store), scope.clone());
        let record = store
            .get(scope, &execution_id.to_string())
            .await
            .map_err(ExecutionCommandError::Store)?
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
            resume_target: target,
        };
        queue
            .enqueue(&msg)
            .await
            .map_err(ExecutionCommandError::from_enqueue)?;

        Ok(CommandReceipt {
            execution_state,
            enqueued: true,
        })
    }
}
