//! Durable control-queue consumer — control-queue wiring.
//!
//! The `ControlConsumer` drains `execution_control_queue` (the spec-16
//! [`nebula_storage_port::store::ControlQueue`] port) and hands typed
//! commands to an engine-owned [`ControlDispatch`] implementation.
//! Wiring decisions: polling loop + claim/ack, engine-owned dispatch trait
//! (no `nebula-api` / `nebula-storage` row types leak into the public
//! surface), at-least-once delivery with idempotent consumer semantics.
//!
//! ## Status
//!
//! - construction, spawning, graceful shutdown, polling, claim/ack plumbing — **implemented**;
//! - dispatch of `Start` / `Resume` / `Restart` to the engine start/resume path — **implemented**
//!   (A2, closes #332 / #327). The engine-owned implementation lives in
//!   [`crate::control_dispatch::EngineControlDispatch`];
//! - dispatch of `Cancel` / `Terminate` to the engine cancel path — **implemented** (A3, closes
//!   #330). The `Cancel` command now reaches the live frontier loop via
//!   [`crate::WorkflowEngine::cancel_execution`]; `Terminate` shares the cooperative-cancel body
//!   until a distinct forced-shutdown path is wired.
//! - reclaim sweep for stuck `Processing` rows after a crashed runner — **implemented** (B1).
//!   A periodic `tokio::time::interval` arm calls `ControlQueue::reclaim_stuck`
//!   every [`DEFAULT_RECLAIM_INTERVAL`]; rows whose `processed_at` is older than
//!   [`DEFAULT_RECLAIM_AFTER`] are moved back to `Pending` (retry budget
//!   [`DEFAULT_MAX_RECLAIM_COUNT`]) or to `Failed` once the budget is exhausted. Each sweep emits
//!   the `nebula_engine_control_reclaim_total{outcome}` counter — wire the shared
//!   registry via [`ControlConsumer::with_metrics`].
//! - M3.5 — before each dispatch, optional `w3c_trace_context` on the row is attached as the
//!   OpenTelemetry parent of an `engine.control_queue.dispatch` span (`.instrument` across await).
//!   Redelivery reuses the **same** carrier from the row — no nested synthetic roots.
//!
//! [`EngineControlDispatch`]: crate::control_dispatch::EngineControlDispatch

use std::{sync::Arc, time::Duration};

use nebula_core::id::ExecutionId;
use nebula_metrics::{
    MetricsRegistry,
    naming::{NEBULA_ENGINE_CONTROL_RECLAIM_TOTAL, control_reclaim_outcome},
};
use nebula_storage_port::Scope;
use nebula_storage_port::dto::ControlCommand;
use nebula_storage_port::store::ControlQueue;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

/// Default batch size for each `claim_pending` call.
///
/// Tuned small enough that a slow dispatch does not block a large batch of
/// rows from being visible to operators, large enough that a busy queue does
/// not round-trip to storage per command.
pub const DEFAULT_BATCH_SIZE: u32 = 32;

/// Default poll interval when the queue is empty.
///
/// Short enough that a cancel feels interactive in the in-memory / SQLite
/// local path (local control-queue path); the Postgres path may shorten this further
/// once `LISTEN / NOTIFY` wake-up is wired as an optimisation over the
/// authoritative polling loop.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Maximum backoff between `claim_pending` retries after repeated storage
/// errors. Prevents a 10Hz error-log flood when the backend is down.
pub const MAX_CLAIM_ERROR_BACKOFF: Duration = Duration::from_secs(30);

/// Default staleness window before a `Processing` row is considered
/// reclaimable.
///
/// Set to 5× the lease TTL (30s) so a runner that has missed 15
/// heartbeats is presumed dead. Intentionally wider than any plausible GC
/// pause.
pub const DEFAULT_RECLAIM_AFTER: Duration = Duration::from_secs(150);

/// Default cadence of the reclaim sweep.
///
/// Matches the lease TTL shape — a runner that died less than 30s ago still
/// has a valid lease from another observer's perspective, so sweeping more
/// often buys nothing.
pub const DEFAULT_RECLAIM_INTERVAL: Duration = Duration::from_secs(30);

/// Default retry budget before a reclaim-eligible row moves to `Failed`.
///
/// Three crashed runners in a row on the same command makes the command
/// itself the suspect, not the runners.
pub const DEFAULT_MAX_RECLAIM_COUNT: u32 = 3;

/// Errors returned from [`ControlDispatch`] methods.
///
/// Kept dedicated (rather than reusing [`crate::EngineError`]) so the
/// dispatch surface can evolve independently of the engine's per-node
/// execution errors. A2 and A3 may extend this with typed variants for
/// "execution not found", "execution already terminal", etc. — for A1
/// only the catch-all exists because no dispatch yet happens.
#[derive(Debug, thiserror::Error)]
pub enum ControlDispatchError {
    /// A dispatch path rejected the command. The attached message is
    /// recorded on the control-queue row via `mark_failed`.
    #[error("control dispatch rejected command: {0}")]
    Rejected(String),

    /// An engine-internal failure prevented dispatch. Distinct from
    /// `Rejected` so operators can distinguish an engine bug from a
    /// legitimate domain-level reject. Also recorded via `mark_failed`.
    #[error("control dispatch failed: {0}")]
    Internal(String),

    /// A transient contention condition prevented dispatch — the control-queue
    /// row should be left in `Processing` so the B1 reclaim sweep redelivers
    /// it once the contention clears.
    ///
    /// Unlike `Rejected` and `Internal` (which call `mark_failed`), this
    /// variant causes the consumer to call neither `mark_completed` nor
    /// `mark_failed`, relying on the reclaim sweep to move the row back to
    /// `Pending`. Use only for conditions that are guaranteed to resolve
    /// (e.g. lease contention), not for permanent failures.
    #[error("control dispatch deferred (transient contention): {0}")]
    Deferred(String),
}

/// Engine-owned dispatch surface for control commands.
///
/// Implementors translate a typed command + `ExecutionId` into engine
/// action: activating a suspended execution (`Resume` / `Restart`),
/// signalling a running execution to stop (`Cancel` / `Terminate`), etc.
///
/// Implementations must be **idempotent per `(execution_id, command)`
/// pair**: receiving the same command twice (e.g. after an at-least-once
/// redelivery) for a terminal execution must return `Ok(())`, not an
/// error. This is a load-bearing contract for decision 5.
///
/// ## Status
///
/// Method stubs return `Ok(())` in A1 because no real dispatch happens
/// yet; A2 / A3 replace each method's body with a call into the engine's
/// start / cancel path. The trait shape (typed `ExecutionId` argument,
/// no storage / api types) is stabilised by A1's public-surface test.
#[async_trait::async_trait]
pub trait ControlDispatch: Send + Sync {
    /// Deliver a `Start` command to a newly-created execution (control-queue wiring,
    /// , #332).
    ///
    /// `scope` is the per-message tenant scope sourced from `ControlMsg.scope`;
    /// it scopes the idempotency status read and the engine's resume path so
    /// that execution rows from a different tenant are never visible.
    ///
    /// Enqueued by the API `start_execution` / `execute_workflow` handlers
    /// once the `ExecutionState::Created` row has been persisted. A2 wired
    /// the canonical engine-side body in
    /// [`crate::control_dispatch::EngineControlDispatch`] — no default
    /// implementation is provided, so every `ControlDispatch` implementor
    /// must supply a real dispatch (the A2 merge-checklist
    /// requirement).
    ///
    /// **Idempotency (critical):** double-start re-runs the workflow twice.
    /// Implementations must guard via CAS on `ExecutionRepo::transition` —
    /// a `Start` arriving for an already-running or already-terminal
    /// execution must be `Ok()`, not a second run.
    async fn dispatch_start(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError>;

    /// Deliver a `Cancel` command to a running execution.
    ///
    /// `scope` is the per-message tenant scope from `ControlMsg.scope`.
    ///
    /// A3 wired this into the engine's cooperative-cancel path (closes
    /// #330). The canonical engine-owned body lives in
    /// [`crate::control_dispatch::EngineControlDispatch`] and signals
    /// [`crate::WorkflowEngine::cancel_execution`] on every non-orphan
    /// delivery, regardless of persisted status.
    ///
    /// **Idempotency (load-bearing, ):** The underlying
    /// `CancellationToken::cancel` is idempotent per token, and a missing
    /// registry entry (cross-runner case, or this runner has already
    /// cleaned up) is a no-op. The consumer's ack path (`mark_completed`)
    /// can fail after a successful dispatch, and the reclaim path (B1)
    /// will redeliver; because the signal itself is idempotent, re-delivery
    /// is safe without a short-circuit on persisted status.
    async fn dispatch_cancel(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError>;

    /// Deliver a `Terminate` command to a running execution.
    ///
    /// `scope` is the per-message tenant scope from `ControlMsg.scope`.
    ///
    /// calls this "forced termination", but there is no distinct
    /// forced-shutdown path in the engine today — cooperative cancel via
    /// the same [`tokio_util::sync::CancellationToken`] is the honest A3
    /// minimum . The canonical
    /// [`crate::control_dispatch::EngineControlDispatch`] body delegates
    /// to [`dispatch_cancel`](Self::dispatch_cancel) until a process-level
    /// kill / `JoinSet` abort is wired as a separate chip.
    ///
    /// **Idempotency:** same contract as [`dispatch_cancel`](Self::dispatch_cancel).
    async fn dispatch_terminate(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError>;

    /// Deliver a `Resume` command to a suspended execution.
    ///
    /// `scope` is the per-message tenant scope from `ControlMsg.scope`.
    ///
    /// A2 wired the canonical body in
    /// [`crate::control_dispatch::EngineControlDispatch`].
    ///
    /// **Idempotency (critical):** double-resume starts the workflow twice.
    /// Implementations must guard via CAS on `ExecutionRepo::transition` —
    /// a `Resume` arriving for an already-running or already-terminal
    /// execution must be `Ok()`, not a second start.
    async fn dispatch_resume(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError>;

    /// Deliver a `Restart` command to an execution.
    ///
    /// `scope` is the per-message tenant scope from `ControlMsg.scope`.
    ///
    /// A2 wired the canonical body in
    /// [`crate::control_dispatch::EngineControlDispatch`]. Full
    /// rewind-from-input semantics require durable output purge and a
    /// monotonic restart counter — both are tracked as follow-ups under
    /// ; the A2 body honors idempotency for non-terminal states
    /// and surfaces a typed [`ControlDispatchError::Rejected`] for
    /// already-terminal executions until the rewind path is wired.
    ///
    /// **Idempotency:** same `Resume` contract applies — double-restart
    /// rewinds twice. Guard with a monotonic restart counter or CAS.
    async fn dispatch_restart(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError>;
}

/// A raw claimed control message. The `execution_id` decode (and its
/// per-row failure isolation) happens in `handle_entry`, not at claim
/// time, so one malformed row never fails the whole batch.
struct RawClaimed(nebula_storage_port::dto::ControlMsg);

/// Backend-agnostic reclaim-sweep tally (the normalized form the
/// metrics/log path consumes).
struct ReclaimCounts {
    reclaimed: u64,
    exhausted: u64,
}

/// One claimed control row, normalized so the dispatch / ack / trace
/// path is uniform.
struct ClaimedRow {
    /// 16-byte ULID primary key (raw bytes).
    id: [u8; 16],
    /// Target execution.
    execution_id: ExecutionId,
    /// Command to deliver.
    command: ControlCommand,
    /// Tenant scope this message belongs to (from `ControlMsg.scope`).
    ///
    /// Threaded into every `dispatch_*` call so the engine reads and drives
    /// the execution under the correct tenant — cross-tenant isolation invariant #7.
    scope: Scope,
    /// W3C `traceparent` carrier captured at enqueue, if any.
    w3c: Option<nebula_core::W3cTraceContext>,
}

impl RawClaimed {
    /// The 16-byte row id (for failure logging before a successful
    /// decode).
    fn row_id(&self) -> [u8; 16] {
        self.0.id
    }

    /// Normalize into a [`ClaimedRow`], decoding `execution_id` (carried
    /// as the opaque string form — parsed directly, no "UTF-8 of the
    /// ULID string" decode).
    ///
    /// `scope` is taken directly from `ControlMsg.scope` — it is the
    /// tenant this message belongs to and must be threaded into every
    /// dispatch call (cross-tenant isolation invariant #7).
    fn normalize(self) -> Result<ClaimedRow, String> {
        let m = self.0;
        let execution_id = m.execution_id.parse::<ExecutionId>().map_err(|e| {
            format!(
                "execution_id {:?} not a valid ExecutionId: {e}",
                m.execution_id
            )
        })?;
        let w3c = match m.w3c_traceparent.as_deref() {
            None => None,
            Some(s) => match nebula_core::W3cTraceContext::from_traceparent_str(s) {
                Ok(ctx) => Some(ctx),
                Err(e) => {
                    // A malformed carrier means this command's span is
                    // orphaned from its enqueuing trace — operationally
                    // significant (distributed-trace gap), so surface it
                    // rather than silently dropping (observability DoD).
                    // The stable target makes it log-metric countable.
                    tracing::warn!(
                        target: "nebula_engine::control_consumer",
                        row_id = ?m.id,
                        error = %e,
                        "control queue row has a malformed w3c traceparent; \
                         dispatching without trace linkage"
                    );
                    None
                },
            },
        };
        Ok(ClaimedRow {
            id: m.id,
            execution_id,
            command: m.command,
            scope: m.scope,
            w3c,
        })
    }
}

/// Drains `execution_control_queue` and hands typed commands to a
/// [`ControlDispatch`] implementation.
///
/// See the module docs and for wiring, atomicity, and idempotency
/// rules.
pub struct ControlConsumer {
    /// Scoped spec-16 [`ControlQueue`] port the consumer drains. The
    /// execution id is carried as the opaque string form (no "UTF-8 of
    /// the ULID string" decode).
    queue: Arc<dyn ControlQueue>,
    dispatch: Arc<dyn ControlDispatch>,
    /// Fixed 16-byte fence token recorded in `processed_by` and matched
    /// on `mark_completed`/`mark_failed`. Stored as `[u8; 16]` end-to-end
    /// so two distinct workers can never silently collapse to the same
    /// token (the previous `Vec<u8>` + truncate/pad let workers sharing a
    /// 16-byte prefix ack each other's rows — a stale-worker fence
    /// collapse).
    processor_id: [u8; 16],
    batch_size: u32,
    poll_interval: Duration,
    reclaim_after: Duration,
    reclaim_interval: Duration,
    max_reclaim_count: u32,
    /// Registry the reclaim sweep increments
    /// `nebula_engine_control_reclaim_total{outcome}` against (
    /// Seam). Defaults to a private fresh [`MetricsRegistry`] so the
    /// consumer is always emit-safe; production composition roots inject
    /// the shared registry via [`Self::with_metrics`] so the counter
    /// reaches the Prometheus scrape endpoint.
    metrics: MetricsRegistry,
}

impl ControlConsumer {
    /// Construct a consumer draining the spec-16 [`ControlQueue`] port.
    ///
    /// `processor_id` is the fixed 16-byte fence token recorded in the
    /// row's `processed_by` (and matched on `mark_completed` /
    /// `mark_failed` for the stale-worker fence). It is `[u8; 16]` by
    /// type — the caller supplies the full id (e.g. a ULID/UUID's 16
    /// bytes); there is deliberately no truncate/pad of an arbitrary-
    /// length id, which would let two distinct workers collapse to the
    /// same token and ack each other's rows.
    pub fn new(
        queue: Arc<dyn ControlQueue>,
        dispatch: Arc<dyn ControlDispatch>,
        processor_id: [u8; 16],
    ) -> Self {
        Self {
            queue,
            dispatch,
            processor_id,
            batch_size: DEFAULT_BATCH_SIZE,
            poll_interval: DEFAULT_POLL_INTERVAL,
            reclaim_after: DEFAULT_RECLAIM_AFTER,
            reclaim_interval: DEFAULT_RECLAIM_INTERVAL,
            max_reclaim_count: DEFAULT_MAX_RECLAIM_COUNT,
            metrics: MetricsRegistry::new(),
        }
    }

    /// Override the claim batch size. Default: [`DEFAULT_BATCH_SIZE`].
    #[must_use]
    pub fn with_batch_size(mut self, batch_size: u32) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Override the poll interval used when the queue is empty.
    /// Default: [`DEFAULT_POLL_INTERVAL`].
    #[must_use]
    pub fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    /// Override the staleness window before a `Processing` row is eligible
    /// for reclaim. Default: [`DEFAULT_RECLAIM_AFTER`].
    #[must_use]
    pub fn with_reclaim_after(mut self, reclaim_after: Duration) -> Self {
        self.reclaim_after = reclaim_after;
        self
    }

    /// Override the cadence of the reclaim sweep tick. Default:
    /// [`DEFAULT_RECLAIM_INTERVAL`].
    #[must_use]
    pub fn with_reclaim_interval(mut self, reclaim_interval: Duration) -> Self {
        self.reclaim_interval = reclaim_interval;
        self
    }

    /// Override the max retry budget before a reclaim-eligible row moves to
    /// `Failed`. Default: [`DEFAULT_MAX_RECLAIM_COUNT`].
    #[must_use]
    pub fn with_max_reclaim_count(mut self, max_reclaim_count: u32) -> Self {
        self.max_reclaim_count = max_reclaim_count;
        self
    }

    /// Inject the shared [`MetricsRegistry`] the reclaim sweep should
    /// emit `nebula_engine_control_reclaim_total{outcome}` against.
    ///
    /// Without this builder the consumer still increments the counter, but
    /// against a private registry no scraper sees — composition roots
    /// (`apps/server`) must wire the runtime registry so operators can
    /// alert on `outcome="exhausted"`.
    #[must_use]
    pub fn with_metrics(mut self, metrics: MetricsRegistry) -> Self {
        self.metrics = metrics;
        self
    }

    /// Spawn the consumer as a Tokio task. The returned handle completes
    /// when the task observes `shutdown` being cancelled.
    ///
    /// The consumer flushes any already-claimed commands before returning;
    /// it does not begin a fresh `claim_pending` once shutdown is requested.
    /// Rows that were claimed but not acknowledged remain in the `Processing`
    /// state and are recovered by the next runner via the reclaim sweep
    ///.
    pub fn spawn(self, shutdown: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move { self.run(shutdown).await })
    }

    /// Run the polling loop on the current task. Exits when `shutdown` is
    /// cancelled. Prefer [`spawn`](Self::spawn) unless integrating into a
    /// custom task structure.
    pub async fn run(self, shutdown: CancellationToken) {
        tracing::info!(
                   processor = %hex_display(&self.processor_id),
                   batch_size = self.batch_size,
                   poll_ms = self.poll_interval.as_millis() as u64,
                   reclaim_after_ms = self.reclaim_after.as_millis() as u64,
                   reclaim_interval_ms = self.reclaim_interval.as_millis() as u64,
                   max_reclaim_count = self.max_reclaim_count,
        "control-queue consumer started (control-queue wiring, )"
               );

        let mut consecutive_errors: u32 = 0;
        let mut reclaim_ticker = tokio::time::interval(self.reclaim_interval);
        // Skip the immediate first tick — we just started, nothing is stuck
        // yet and the first `claim_pending` call has priority.
        reclaim_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let _ = reclaim_ticker.tick().await;

        // Deadline at which the next `claim_pending` is allowed to fire. Held
        // in scope across the `tokio::select!` below so that a reclaim
        // interruption does not reset the backoff / poll_interval clock —
        // see the review finding for PR #483 and.
        let mut claim_deadline = tokio::time::Instant::now();

        loop {
            let claim_sleep = tokio::time::sleep_until(claim_deadline);
            tokio::pin!(claim_sleep);

            tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    tracing::info!(
                        processor = %hex_display(&self.processor_id),
                        "control-queue consumer shutting down"
                    );
                    return;
                }
                _ = reclaim_ticker.tick() => {
                    self.sweep_reclaim().await;
                    // `claim_deadline` is preserved — reclaim does not
                    // short-circuit the backoff or the idle poll delay.
                }
                () = &mut claim_sleep => {
                    let next_delay = self.tick(&mut consecutive_errors).await;
                    claim_deadline = tokio::time::Instant::now()
                        + next_delay.unwrap_or(Duration::ZERO);
                }
            }
        }
    }

    /// Run a single reclaim sweep, logging the outcome. Does not propagate
    /// storage errors — a transient failure on one sweep should not abort
    /// the consumer; the next tick will retry.
    async fn sweep_reclaim(&self) {
        // Normalize the port's `ReclaimOutcome` to `(reclaimed,
        // exhausted)` for the metrics/log path.
        let swept: Result<(u64, u64), String> = self
            .queue
            .reclaim_stuck(self.reclaim_after, self.max_reclaim_count)
            .await
            .map(|o| (o.reclaimed, o.exhausted))
            .map_err(|e| e.to_string());
        match swept {
            Ok((reclaimed, exhausted)) => {
                let outcome = ReclaimCounts {
                    reclaimed,
                    exhausted,
                };
                // : bump the per-outcome counter by the row
                // count for this sweep (not by 1 per sweep) so operators
                // can alert on `outcome="exhausted" > 0` and watch
                // `outcome="reclaimed"` for crashed-runner load.
                // `processor_id` is intentionally NOT a label —
                // cardinality hygiene; the structured `tracing` log
                // below carries the per-runner correlation.
                if outcome.reclaimed > 0 {
                    let labels = self
                        .metrics
                        .interner()
                        .single("outcome", control_reclaim_outcome::RECLAIMED);
                    if let Ok(c) = self
                        .metrics
                        .counter_labeled(NEBULA_ENGINE_CONTROL_RECLAIM_TOTAL, &labels)
                    {
                        c.inc_by(outcome.reclaimed);
                    }
                }
                if outcome.exhausted > 0 {
                    let labels = self
                        .metrics
                        .interner()
                        .single("outcome", control_reclaim_outcome::EXHAUSTED);
                    if let Ok(c) = self
                        .metrics
                        .counter_labeled(NEBULA_ENGINE_CONTROL_RECLAIM_TOTAL, &labels)
                    {
                        c.inc_by(outcome.exhausted);
                    }
                }

                if outcome.reclaimed > 0 || outcome.exhausted > 0 {
                    tracing::warn!(
                                           processor = %hex_display(&self.processor_id),
                                           reclaimed = outcome.reclaimed,
                                           exhausted = outcome.exhausted,
                                           reclaim_after_ms = self.reclaim_after.as_millis() as u64,
                    "control-queue reclaim sweep recovered stuck rows "
                                       );
                } else {
                    tracing::debug!(
                        processor = %hex_display(&self.processor_id),
                        "control-queue reclaim sweep: no stuck rows"
                    );
                }
            },
            Err(e) => {
                // No counter emit on the Err arm — a transient storage
                // failure is not a reclaim outcome.
                tracing::error!(
                    processor = %hex_display(&self.processor_id),
                    error = %e,
                    "control-queue reclaim sweep failed; will retry next tick"
                );
            },
        }
    }

    /// Drain a single batch. Returns the duration the caller should wait
    /// before the next claim attempt (backoff on error, poll interval on
    /// empty queue) or `None` if a batch was just processed and the loop
    /// should claim again immediately.
    ///
    /// The outer loop persists this delay as a deadline so that a reclaim
    /// interruption does not cancel the backoff — see `run`.
    async fn tick(&self, consecutive_errors: &mut u32) -> Option<Duration> {
        // Claim raw rows. A malformed `execution_id` must fail only
        // *that* row (mark it failed, continue) — not the whole batch —
        // so the decode happens per row in `handle_entry`, not here.
        let claimed: Result<Vec<RawClaimed>, String> = self
            .queue
            .claim_pending(&self.processor_id, self.batch_size)
            .await
            .map(|msgs| msgs.into_iter().map(RawClaimed).collect())
            .map_err(|e| e.to_string());

        let claimed = match claimed {
            Ok(rows) => {
                *consecutive_errors = 0;
                rows
            },
            Err(e) => {
                *consecutive_errors = consecutive_errors.saturating_add(1);
                let backoff = claim_error_backoff(self.poll_interval, *consecutive_errors);
                tracing::error!(
                    error = %e,
                    consecutive_errors = *consecutive_errors,
                    backoff_ms = backoff.as_millis() as u64,
                    "control-queue claim_pending failed; backing off"
                );
                return Some(backoff);
            },
        };

        if claimed.is_empty() {
            return Some(self.poll_interval);
        }

        for row in claimed {
            self.handle_entry(row).await;
        }
        None
    }

    async fn handle_entry(&self, raw: RawClaimed) {
        let row_id = raw.row_id();
        let ClaimedRow {
            id: row_id,
            execution_id,
            command,
            scope,
            w3c: w3c_opt,
        } = match raw.normalize() {
            Ok(row) => row,
            Err(reason) => {
                tracing::error!(
                    id = %hex_display(&row_id),
                    reason = %reason,
                    "control-queue row has malformed execution_id; marking failed"
                );
                self.ack_failed(&row_id, &format!("malformed execution_id: {reason}"))
                    .await;
                return;
            },
        };

        let has_carrier = w3c_opt.is_some();
        let span = tracing::info_span!(
            "engine.control_queue.dispatch",
            execution_id = %execution_id,
            command = command.as_str(),
            queue_row_has_w3c = has_carrier,
        );
        if let Some(ref w3c) = w3c_opt {
            crate::control_trace::attach_control_queue_w3c_parent(&span, w3c);
        }

        let dispatch = Arc::clone(&self.dispatch);
        let dispatch_result = async move {
            tracing::info!(
                execution_id = %execution_id,
                command = command.as_str(),
                queue_row_has_w3c = has_carrier,
                "control-queue: dispatch command (span carries M3.5 parent when row had carrier)"
            );
            match command {
                ControlCommand::Start => {
                    tracing::debug!(%execution_id, "control-queue: dispatching Start (A2)");
                    dispatch.dispatch_start(&scope, execution_id).await
                },
                ControlCommand::Cancel => {
                    tracing::debug!(%execution_id, "control-queue: dispatching Cancel (A3)");
                    dispatch.dispatch_cancel(&scope, execution_id).await
                },
                ControlCommand::Terminate => {
                    tracing::debug!(%execution_id, "control-queue: dispatching Terminate (A3)");
                    dispatch.dispatch_terminate(&scope, execution_id).await
                },
                ControlCommand::Resume => {
                    tracing::debug!(%execution_id, "control-queue: dispatching Resume (A2)");
                    dispatch.dispatch_resume(&scope, execution_id).await
                },
                ControlCommand::Restart => {
                    tracing::debug!(%execution_id, "control-queue: dispatching Restart (A2)");
                    dispatch.dispatch_restart(&scope, execution_id).await
                },
            }
        }
        .instrument(span)
        .await;

        match dispatch_result {
            Ok(()) => self.ack_completed(&row_id).await,
            Err(ControlDispatchError::Deferred(ref reason)) => {
                // Transient contention: leave the row in `Processing` so the
                // B1 reclaim sweep moves it back to `Pending` for redelivery.
                // Calling `mark_failed` here would permanently record the row as
                // failed — redelivery under B1 is only for `Processing` rows.
                tracing::warn!(
                    id = %hex_display(&row_id),
                    %execution_id,
                    command = command.as_str(),
                    reason = %reason,
                    "control-queue dispatch deferred (transient contention); \
                     leaving row in Processing for B1 reclaim"
                );
            },
            Err(e) => {
                tracing::error!(
                                   id = %hex_display(&row_id),
                                   %execution_id,
                                   command = command.as_str(),
                                   error = %e,
                "control-queue dispatch failed; marking failed (no auto-retry — )"
                               );
                self.ack_failed(&row_id, &e.to_string()).await;
            },
        }
    }

    async fn ack_completed(&self, id: &[u8; 16]) {
        // NOTE: dispatch already ran successfully at this point. If
        // `mark_completed` fails, the row stays in `Processing` and the B1
        // reclaim path redelivers the
        // command. Correctness under redelivery depends entirely on
        // `ControlDispatch` impls being idempotent per `(execution_id, command)`
        // — see the trait-level docs and.
        let result: Result<(), String> = self
            .queue
            .mark_completed(id, &self.processor_id)
            .await
            .map_err(|e| e.to_string());
        if let Err(e) = result {
            tracing::error!(
                id = %hex_display(id),
                error = %e,
                "control-queue mark_completed failed; row left in Processing for reclaim"
            );
        }
    }

    async fn ack_failed(&self, id: &[u8; 16], reason: &str) {
        let result: Result<(), String> = self
            .queue
            .mark_failed(id, &self.processor_id, reason)
            .await
            .map_err(|e| e.to_string());
        if let Err(e) = result {
            tracing::error!(
                id = %hex_display(id),
                error = %e,
                "control-queue mark_failed failed; row left in Processing for reclaim"
            );
        }
    }
}

/// Exponential backoff for repeated `claim_pending` storage errors.
///
/// Starts at `base` (the idle poll interval) and doubles per consecutive
/// error, capped at [`MAX_CLAIM_ERROR_BACKOFF`]. `consecutive_errors` is
/// 1-indexed (first failure → `base`, second → `base*2`, …).
fn claim_error_backoff(base: Duration, consecutive_errors: u32) -> Duration {
    let multiplier = 1u64
        .checked_shl(consecutive_errors.saturating_sub(1).min(30))
        .unwrap_or(u64::MAX);
    let scaled = base
        .checked_mul(u32::try_from(multiplier.min(u64::from(u32::MAX))).unwrap_or(u32::MAX))
        .unwrap_or(MAX_CLAIM_ERROR_BACKOFF);
    scaled.min(MAX_CLAIM_ERROR_BACKOFF)
}

/// Hex-render opaque byte fields for structured logs, keeping tracing
/// output human-readable without dragging in a heavy dependency.
fn hex_display(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_display_renders_bytes() {
        assert_eq!(hex_display(&[0x0a, 0xff, 0x00]), "0aff00");
    }

    #[test]
    fn claim_error_backoff_doubles_then_caps() {
        let base = Duration::from_millis(100);
        assert_eq!(claim_error_backoff(base, 1), Duration::from_millis(100));
        assert_eq!(claim_error_backoff(base, 2), Duration::from_millis(200));
        assert_eq!(claim_error_backoff(base, 3), Duration::from_millis(400));
        assert_eq!(claim_error_backoff(base, 4), Duration::from_millis(800));
        // Cap kicks in well before any overflow (100ms * 2^29 > 30s cap).
        assert_eq!(claim_error_backoff(base, 15), MAX_CLAIM_ERROR_BACKOFF);
        assert_eq!(claim_error_backoff(base, u32::MAX), MAX_CLAIM_ERROR_BACKOFF);
    }

    #[test]
    fn claim_error_backoff_zero_is_base() {
        // consecutive_errors == 0 never reached in practice (we saturating_add
        // before calling), but must be total and safe.
        let base = Duration::from_millis(50);
        assert_eq!(claim_error_backoff(base, 0), base);
    }
}
