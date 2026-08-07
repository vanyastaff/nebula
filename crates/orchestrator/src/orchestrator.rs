//! Capability-routed job-dispatch pull loop.
//!
//! [`Orchestrator`] mirrors the shape of `ControlConsumer` in `nebula-engine`:
//! biased `tokio::select!`, `MissedTickBehavior::Delay` with first-tick skip,
//! claim-deadline held across the select so reclaim ticks don't reset backoff,
//! per-row failure isolation, and optional [`MetricsRegistry`] injection via
//! the builder.
//!
//! ## Claim lifecycle — the handoff (#976)
//!
//! A dispatch claim protects **delivery only**, never work. Each claimed row
//! is processed as `validate → handoff → dispatch`:
//!
//! 1. **Validate** the claim locally (the execution id parses). A row that
//!    can never be dispatched is terminalised with `mark_failed` instead of
//!    being redelivered forever.
//! 2. **Hand off** durably via [`ExecutionTurnHandoff::accept_turn`]: the
//!    execution lease is acquired and the queue row acknowledged in one
//!    transaction. The claim ends at this point — how long the action
//!    subsequently runs cannot extend it.
//! 3. **Dispatch** the accepted turn to the [`ExecutionSink`], which drives
//!    the execution under the handoff's fence. Post-handoff failures never
//!    touch queue state: recovery proceeds from the execution lease and
//!    persisted aggregate truth.
//!
//! A crash before the handoff commits leaves the row `Processing` for the
//! reclaim sweep; a crash after leaves a terminal row and a leased execution
//! that recovery drives. There is no state in between.
//!
//! ## Shutdown contract
//!
//! When [`CancellationToken`] is cancelled the orchestrator flushes the
//! in-flight batch already being processed, then returns. It does **not** begin
//! a fresh [`JobDispatchQueue::claim_pending`] once shutdown is requested.
//! Rows claimed but not yet handed off remain in `Processing` and are
//! recovered by the next runner's reclaim sweep; rows already handed off are
//! governed by their execution lease.
//!
//! Worst-case shutdown observability latency is bounded by
//! `max(one reclaim_stuck() sweep, batch_size × one sink.dispatch() latency)`,
//! because `tick()` / `sweep_reclaim()` run in `select!` arm bodies and
//! shutdown is only observed on the next loop iteration.
//!
//! [`CancellationToken`]: tokio_util::sync::CancellationToken
//! [`MetricsRegistry`]: nebula_metrics::MetricsRegistry
//! [`JobDispatchQueue::claim_pending`]: nebula_storage_port::store::JobDispatchQueue::claim_pending
//! [`ExecutionTurnHandoff::accept_turn`]: nebula_storage_port::store::ExecutionTurnHandoff::accept_turn

use std::{sync::Arc, time::Duration};

use nebula_core::PluginKey;
use nebula_core::id::ExecutionId;
use nebula_metrics::{
    MetricsRegistry,
    naming::{
        NEBULA_ORCHESTRATOR_DISPATCH_TOTAL, NEBULA_ORCHESTRATOR_HANDOFF_TOTAL,
        NEBULA_ORCHESTRATOR_RECLAIM_TOTAL, orchestrator_dispatch_outcome,
        orchestrator_handoff_outcome, orchestrator_reclaim_outcome,
    },
};
use nebula_storage_port::StorageError;
use nebula_storage_port::store::{
    ExecutionTurnHandoff, JobClaim, JobClaimToken, JobDispatchQueue, ReclaimOutcome,
    TurnAcceptance, TurnHandoff,
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::sink::{DispatchedTurn, ExecutionSink};

/// Default claim batch size.
///
/// Mirrors `ControlConsumer::DEFAULT_BATCH_SIZE` in `nebula-engine`: small
/// enough that a slow sink does not block many rows from operator visibility;
/// large enough to avoid per-row round-trips on a busy queue.
pub const DEFAULT_BATCH_SIZE: u32 = 32;

/// Default idle poll interval (queue empty).
///
/// Matches `ControlConsumer` — short enough for interactive latency on the
/// local path; the Postgres path may shorten further once `LISTEN/NOTIFY` is
/// wired as an optimisation.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How long an in-flight dispatch may keep running after shutdown is
/// requested.
///
/// Long enough that ordinary work finishes and is acknowledged (the graceful
/// shutdown contract), short enough that a dispatch parked on a durable timer
/// or an external wait cannot hold the process open indefinitely.
const SHUTDOWN_DISPATCH_GRACE: Duration = Duration::from_secs(5);

/// Maximum backoff between `claim_pending` retries after repeated storage
/// errors. Prevents a high-frequency error-log flood when the backend is down.
pub const MAX_CLAIM_ERROR_BACKOFF: Duration = Duration::from_secs(30);

/// Default staleness window before a `Processing` row is reclaimable.
///
/// Matches `ControlConsumer` — 5× a 30-second lease TTL so a runner that
/// has missed 15 heartbeats is presumed dead.
///
/// ## Relationship between the job-dispatch claim and execution liveness
///
/// The job-dispatch claim protects **delivery only** (#976): the orchestrator
/// claims the row, ends the claim at the durable handoff
/// ([`ExecutionTurnHandoff::accept_turn`]), and only then dispatches the
/// accepted turn. The row is terminal before the action runs, so how long the
/// action takes can neither extend the claim nor trip this window — a
/// deliberately slow action and a crashed runner are no longer the same
/// signal.
///
/// Execution liveness is tracked separately: the handoff acquires the
/// per-execution lease, and the engine renews it on a heartbeat loop whose
/// TTL is the authoritative signal that a runner has crashed. The
/// job-dispatch `reclaim_stuck` sweep is a **routing-layer** recovery
/// mechanism for rows whose runner crashed before the handoff committed —
/// not an execution-failure detector.
///
/// See ADR-0017 (execution lease contract) and ADR-0095 (job-dispatch
/// routing) for the full context.
///
/// [`ExecutionTurnHandoff::accept_turn`]: nebula_storage_port::store::ExecutionTurnHandoff::accept_turn
pub const DEFAULT_RECLAIM_AFTER: Duration = Duration::from_secs(150);

/// Default lease TTL the durable handoff mints for the accepted turn.
///
/// Matches the engine's `DEFAULT_EXECUTION_LEASE_TTL`: long enough that the
/// engine's heartbeat loop takes over renewal with margin, short enough to
/// bound recovery latency when the accepting runner crashes right after the
/// commit. Every backend clamps it to `[1s, 24h]`, the same window
/// `ExecutionStore::acquire_lease` enforces.
pub const DEFAULT_HANDOFF_LEASE_TTL: Duration = Duration::from_secs(30);

/// Default cadence of the reclaim sweep.
pub const DEFAULT_RECLAIM_INTERVAL: Duration = Duration::from_secs(30);

/// Default retry budget before a reclaim-eligible row moves to `Failed`.
///
/// A row that has been reclaimed this many times transitions to `Failed` on
/// the next sweep, preventing unbounded redelivery of permanently stuck jobs.
/// Since the handoff (#976) ends the claim before the action runs, only rows
/// whose runner repeatedly crashes **before** the handoff commits consume
/// this budget.
pub const DEFAULT_MAX_RECLAIM_COUNT: u32 = 3;

/// Capability-routed job-dispatch pull loop (ADR-0095).
///
/// Claims [`JobDispatchQueue`] rows whose `required_plugins ⊆ available_plugins`,
/// ends each claim at the durable handoff, and dispatches the accepted turn
/// to an [`ExecutionSink`]. A periodic sweep reclaims rows stuck in
/// `Processing` after a runner crashed before its handoff committed.
///
/// Construct with [`Orchestrator::new`] and optional builder methods, then
/// call [`Orchestrator::run`] (or [`Orchestrator::spawn`]).
///
/// [`JobDispatchQueue`]: nebula_storage_port::store::JobDispatchQueue
#[must_use = "call .spawn() or .run() to start the pull loop"]
pub struct Orchestrator {
    queue: Arc<dyn JobDispatchQueue>,
    sink: Arc<dyn ExecutionSink>,
    /// Durable owner of the dispatch-claim → execution-turn handoff. MUST be
    /// backed by the same store the queue and the engine's execution store
    /// use: the handoff commits the lease write and the queue acknowledgement
    /// in one transaction, and two backends would give them two boundaries.
    handoff: Arc<dyn ExecutionTurnHandoff>,
    /// Fixed 16-byte fence token recorded in `processed_by` and matched on
    /// `mark_failed`. Typed `[u8; 16]` end-to-end — no truncate/pad of an
    /// arbitrary-length id, which would let two distinct workers collapse to
    /// the same token and ack each other's rows.
    processor_id: [u8; 16],
    /// Identity recorded as the accepted turn's lease holder. Derived from
    /// `processor_id` so contention diagnostics name the processor, but the
    /// lease authority is the fence, not this string.
    lease_holder: String,
    available_plugins: Vec<PluginKey>,
    batch_size: u32,
    poll_interval: Duration,
    reclaim_after: Duration,
    reclaim_interval: Duration,
    max_reclaim_count: u32,
    /// Lease TTL the handoff mints for each accepted turn. The engine's
    /// heartbeat loop takes over renewal once the turn is dispatched.
    handoff_lease_ttl: Duration,
    /// Shared metrics registry. Defaults to a private fresh registry so the
    /// orchestrator is always emit-safe without injection; production
    /// composition roots inject the shared registry via [`with_metrics`] so
    /// counters reach the Prometheus scrape endpoint.
    ///
    /// [`with_metrics`]: Self::with_metrics
    metrics: MetricsRegistry,
}

impl Orchestrator {
    /// Construct an orchestrator.
    ///
    /// `handoff` is the durable owner of the dispatch-claim → execution-turn
    /// handoff and MUST share the backend the queue and the engine's
    /// execution store use — see the field docs. An orchestrator without a
    /// handoff would hold claims for whole action durations (NS05), so none
    /// can be constructed.
    ///
    /// `processor_id` is the fixed 16-byte fence token recorded in the row's
    /// `processed_by`. Supply the full id bytes — no truncation or padding is
    /// done, which would let two distinct workers collapse to the same token.
    pub fn new(
        queue: Arc<dyn JobDispatchQueue>,
        sink: Arc<dyn ExecutionSink>,
        handoff: Arc<dyn ExecutionTurnHandoff>,
        processor_id: [u8; 16],
        available_plugins: Vec<PluginKey>,
    ) -> Self {
        let lease_holder = format!("orchestrator:{}", hex_display(&processor_id));
        Self {
            queue,
            sink,
            handoff,
            processor_id,
            lease_holder,
            available_plugins,
            batch_size: DEFAULT_BATCH_SIZE,
            poll_interval: DEFAULT_POLL_INTERVAL,
            reclaim_after: DEFAULT_RECLAIM_AFTER,
            reclaim_interval: DEFAULT_RECLAIM_INTERVAL,
            max_reclaim_count: DEFAULT_MAX_RECLAIM_COUNT,
            handoff_lease_ttl: DEFAULT_HANDOFF_LEASE_TTL,
            metrics: MetricsRegistry::new(),
        }
    }

    /// Override the claim batch size. Default: [`DEFAULT_BATCH_SIZE`].
    pub fn with_batch_size(mut self, n: u32) -> Self {
        self.batch_size = n;
        self
    }

    /// Override the idle poll interval. Default: [`DEFAULT_POLL_INTERVAL`].
    pub fn with_poll_interval(mut self, d: Duration) -> Self {
        self.poll_interval = d;
        self
    }

    /// Override the staleness window before a row is reclaimable.
    /// Default: [`DEFAULT_RECLAIM_AFTER`].
    pub fn with_reclaim_after(mut self, d: Duration) -> Self {
        self.reclaim_after = d;
        self
    }

    /// Override the cadence of the reclaim sweep tick.
    /// Default: [`DEFAULT_RECLAIM_INTERVAL`].
    pub fn with_reclaim_interval(mut self, d: Duration) -> Self {
        self.reclaim_interval = d;
        self
    }

    /// Override the max retry budget before a reclaim-eligible row moves to
    /// `Failed`. Default: [`DEFAULT_MAX_RECLAIM_COUNT`].
    pub fn with_max_reclaim_count(mut self, n: u32) -> Self {
        self.max_reclaim_count = n;
        self
    }

    /// Override the lease TTL the handoff mints for each accepted turn.
    /// Default: [`DEFAULT_HANDOFF_LEASE_TTL`].
    ///
    /// Backends clamp it to `[1s, 24h]`. Shorter values bound recovery
    /// latency after a post-handoff crash at the cost of less margin before
    /// the engine's heartbeat loop takes over renewal.
    pub fn with_handoff_lease_ttl(mut self, d: Duration) -> Self {
        self.handoff_lease_ttl = d;
        self
    }

    /// Inject the shared [`MetricsRegistry`] the orchestrator emits counters
    /// against. Without this the counters increment against a private registry
    /// no scraper sees.
    pub fn with_metrics(mut self, m: MetricsRegistry) -> Self {
        self.metrics = m;
        self
    }

    /// Spawn the orchestrator as a Tokio task. Returns a [`JoinHandle`] that
    /// completes when `shutdown` is cancelled.
    ///
    /// ## Shutdown contract
    ///
    /// The orchestrator flushes the in-flight batch already being processed,
    /// then returns; it does not begin a fresh claim once shutdown is
    /// requested. Rows claimed but not yet marked remain in `Processing` and
    /// are recovered by the next runner's reclaim sweep.
    pub fn spawn(self, shutdown: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move { self.run(shutdown).await })
    }

    /// Run the pull loop on the current task. Exits when `shutdown` is
    /// cancelled. Prefer [`spawn`](Self::spawn) unless integrating into a
    /// custom task structure.
    ///
    /// ## Shutdown contract
    ///
    /// The orchestrator flushes the in-flight batch already being processed,
    /// then returns; it does not begin a fresh claim once shutdown is
    /// requested. Rows claimed but not yet marked remain in `Processing` and
    /// are recovered by the next runner's reclaim sweep.
    pub async fn run(self, shutdown: CancellationToken) {
        tracing::info!(
            processor = %hex_display(&self.processor_id),
            batch_size = self.batch_size,
            poll_ms = self.poll_interval.as_millis() as u64,
            reclaim_after_ms = self.reclaim_after.as_millis() as u64,
            reclaim_interval_ms = self.reclaim_interval.as_millis() as u64,
            max_reclaim_count = self.max_reclaim_count,
            available_plugins = ?self.available_plugins.iter().map(PluginKey::as_str).collect::<Vec<_>>(),
            "orchestrator started (ADR-0095)"
        );

        let mut consecutive_errors: u32 = 0;
        let mut reclaim_ticker = tokio::time::interval(self.reclaim_interval);
        // Skip the immediate first tick — nothing is stuck yet and the first
        // `claim_pending` call has priority. Mirrors ControlConsumer.
        reclaim_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let _ = reclaim_ticker.tick().await;

        // Hold `claim_deadline` across the select so a reclaim tick does not
        // reset the backoff / idle-poll clock.
        let mut claim_deadline = tokio::time::Instant::now();

        loop {
            let claim_sleep = tokio::time::sleep_until(claim_deadline);
            tokio::pin!(claim_sleep);

            tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    tracing::info!(
                        processor = %hex_display(&self.processor_id),
                        "orchestrator shutting down"
                    );
                    return;
                }
                _ = reclaim_ticker.tick() => {
                    self.sweep_reclaim().await;
                    // `claim_deadline` is preserved — reclaim does not reset
                    // the backoff or idle poll delay.
                }
                () = &mut claim_sleep => {
                    let next_delay = self.tick(&mut consecutive_errors, &shutdown).await;
                    claim_deadline = tokio::time::Instant::now()
                        + next_delay.unwrap_or(Duration::ZERO);
                }
            }
        }
    }

    /// Run a single reclaim sweep.
    ///
    /// Storage errors are logged and swallowed — a transient failure on one
    /// sweep should not abort the loop; the next tick will retry.
    async fn sweep_reclaim(&self) {
        let swept: Result<ReclaimOutcome, String> = self
            .queue
            .reclaim_stuck(self.reclaim_after, self.max_reclaim_count)
            .await
            .map_err(|e| e.to_string());

        match swept {
            Ok(outcome) => {
                if outcome.reclaimed > 0 {
                    let labels = self
                        .metrics
                        .interner()
                        .single("outcome", orchestrator_reclaim_outcome::RECLAIMED);
                    if let Ok(c) = self
                        .metrics
                        .counter_labeled(NEBULA_ORCHESTRATOR_RECLAIM_TOTAL, &labels)
                    {
                        c.inc_by(outcome.reclaimed);
                    }
                }
                if outcome.exhausted > 0 {
                    let labels = self
                        .metrics
                        .interner()
                        .single("outcome", orchestrator_reclaim_outcome::EXHAUSTED);
                    if let Ok(c) = self
                        .metrics
                        .counter_labeled(NEBULA_ORCHESTRATOR_RECLAIM_TOTAL, &labels)
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
                        "orchestrator reclaim sweep recovered stuck rows (ADR-0095)"
                    );
                } else {
                    tracing::debug!(
                        processor = %hex_display(&self.processor_id),
                        "orchestrator reclaim sweep: no stuck rows"
                    );
                }
            },
            Err(e) => {
                tracing::error!(
                    processor = %hex_display(&self.processor_id),
                    error = %e,
                    "orchestrator reclaim sweep failed; will retry next tick"
                );
            },
        }
    }

    /// Drain a single batch. Returns the duration to wait before the next
    /// claim attempt, or `None` for immediate re-claim.
    async fn tick(
        &self,
        consecutive_errors: &mut u32,
        shutdown: &CancellationToken,
    ) -> Option<Duration> {
        let claimed: Result<Vec<JobClaim>, String> = self
            .queue
            .claim_pending(&self.processor_id, self.batch_size, &self.available_plugins)
            .await
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
                    "orchestrator claim_pending failed; backing off"
                );
                return Some(backoff);
            },
        };

        if claimed.is_empty() {
            return Some(self.poll_interval);
        }

        for claim in claimed {
            self.handle_entry(claim, shutdown).await;
        }
        None
    }

    async fn handle_entry(&self, claim: JobClaim, shutdown: &CancellationToken) {
        let JobClaim { msg, token } = claim;
        // The queue routing predicate (`required_plugins ⊆ available_plugins`)
        // is enforced at claim time. This assert checks the implied single-key
        // condition (`required_plugin_key ∈ available_plugins`, since
        // `required_plugin_key ∈ required_plugins` by the DTO invariant) in
        // debug builds only — it is not a release guard.
        debug_assert!(
            self.available_plugins.contains(&msg.required_plugin_key),
            "claim routing invariant violated: required_plugin_key {:?} not in available_plugins {:?}",
            msg.required_plugin_key,
            self.available_plugins
        );

        let row_id = msg.id;
        let has_w3c = msg.w3c_traceparent.is_some();

        let span = tracing::info_span!(
            "orchestrator.dispatch",
            execution_id = %msg.execution_id,
            required_plugin_key = %msg.required_plugin_key,
            command = msg.command.as_str(),
            reclaim_count = msg.reclaim_count,
            row_has_w3c = has_w3c,
        );

        // Attach W3C parent if present — same non-fatal policy as
        // ControlConsumer: malformed carrier → warn and dispatch as root span.
        if let Some(ref tp) = msg.w3c_traceparent {
            match nebula_core::W3cTraceContext::from_traceparent_str(tp) {
                Ok(w3c) => attach_w3c_parent(&span, &w3c),
                Err(e) => {
                    tracing::warn!(
                        target: "nebula_orchestrator",
                        row_id = %hex_display(&row_id),
                        error = %e,
                        "orchestrator row has a malformed w3c traceparent; \
                         dispatching without trace linkage"
                    );
                },
            }
        }

        // ── 1. validate ─────────────────────────────────────────────────────
        //
        // Check everything decidable from the claim alone before the handoff
        // writes anything. A row that fails here can never be dispatched, so
        // it is terminalised instead of being redelivered until the reclaim
        // budget exhausts. The claim is still ours at this point — the
        // handoff has not run — so `mark_failed` is the honest write.
        if msg.execution_id.parse::<ExecutionId>().is_err() {
            let reason = format!("invalid execution_id `{}`", msg.execution_id);
            tracing::error!(
                row_id = %hex_display(&row_id),
                execution_id = %msg.execution_id,
                "orchestrator claim failed validation; marking row failed (#976)"
            );
            self.mark_failed(&token, &reason).await;
            self.inc_dispatch(orchestrator_dispatch_outcome::FAILED);
            return;
        }

        // ── 2. handoff ──────────────────────────────────────────────────────
        //
        // End the claim at a definite point: `accept_turn` acquires the
        // execution lease and acknowledges the queue row in one transaction.
        // Past this commit the action's duration is governed by the lease,
        // and the queue can never redeliver this row.
        let turn_handoff = TurnHandoff {
            scope: &msg.scope,
            execution_id: &msg.execution_id,
            claim: token,
            holder: &self.lease_holder,
            lease_ttl: self.handoff_lease_ttl,
        };
        let acceptance = self.handoff.accept_turn(&turn_handoff).await;
        let fence = match acceptance {
            Ok(TurnAcceptance::Accepted { fence }) => {
                self.inc_handoff(orchestrator_handoff_outcome::ACCEPTED);
                fence
            },
            // Another attempt now owns the row — nothing was written, and the
            // current owner drives the turn. There is nothing to do.
            Ok(TurnAcceptance::ClaimSuperseded) => {
                self.inc_handoff(orchestrator_handoff_outcome::CLAIM_SUPERSEDED);
                tracing::debug!(
                    row_id = %hex_display(&row_id),
                    execution_id = %msg.execution_id,
                    claim_generation = %token.generation(),
                    "handoff found the claim superseded; the current owner drives the turn (#976)"
                );
                return;
            },
            // A live lease owned by someone else: the row is deliberately left
            // claimed so the sweep redelivers it once that lease expires.
            // Acknowledging here would drop the turn on the floor.
            Ok(TurnAcceptance::TurnHeldByAnotherOwner) => {
                self.inc_handoff(orchestrator_handoff_outcome::TURN_HELD);
                tracing::debug!(
                    row_id = %hex_display(&row_id),
                    execution_id = %msg.execution_id,
                    "turn already owned by another holder; leaving the row claimable for redelivery (#976)"
                );
                return;
            },
            // The execution (or the row itself) is gone: an orphaned dispatch.
            // The emitter materialises the execution row and the queue row in
            // one atomic write, so an execution missing here will not appear
            // later — terminalise rather than redeliver forever.
            Err(StorageError::NotFound { .. }) => {
                self.inc_handoff(orchestrator_handoff_outcome::ERROR);
                let reason = format!("execution {} not found at handoff", msg.execution_id);
                tracing::error!(
                    row_id = %hex_display(&row_id),
                    execution_id = %msg.execution_id,
                    "handoff found no execution; marking row failed (#976)"
                );
                self.mark_failed(&token, &reason).await;
                self.inc_dispatch(orchestrator_dispatch_outcome::FAILED);
                return;
            },
            // Backend error: nothing was written, the row stays `Processing`,
            // and the reclaim sweep redelivers it. No terminal write — the
            // failure may be transient.
            Err(ref e) => {
                self.inc_handoff(orchestrator_handoff_outcome::ERROR);
                tracing::error!(
                    row_id = %hex_display(&row_id),
                    execution_id = %msg.execution_id,
                    claim_generation = %token.generation(),
                    error = %e,
                    "handoff failed; row left in Processing for reclaim (#976)"
                );
                return;
            },
        };

        // ── 3. dispatch the accepted turn ───────────────────────────────────
        //
        // Bounded drain on shutdown.
        //
        // Graceful shutdown flushes the in-flight batch — a dispatch that is
        // nearly done should finish rather than be abandoned. But `dispatch`
        // drives a whole execution step, which can park on a durable timer or
        // an external wait, so awaiting it unconditionally means the loop never
        // returns to its `select!` and a process asked to stop hangs for as
        // long as the workflow does.
        //
        // So: keep flushing after cancellation, but only within
        // [`SHUTDOWN_DISPATCH_GRACE`]. Past that the turn is abandoned to the
        // documented recovery path — the queue row is already acknowledged by
        // the handoff, and the execution lease this handoff minted expires so
        // recovery drives from persisted aggregate truth. Nothing here can
        // lose the turn: a crash at any point past the handoff commit is the
        // same case.
        let sink = Arc::clone(&self.sink);
        let turn = DispatchedTurn { msg: &msg, fence };
        let dispatch = sink.dispatch(&turn).instrument(span);
        tokio::pin!(dispatch);
        let dispatch_result = tokio::select! {
            biased;
            result = &mut dispatch => result,
            () = shutdown.cancelled() => {
                let Ok(result) =
                    tokio::time::timeout(SHUTDOWN_DISPATCH_GRACE, &mut dispatch).await
                else {
                    tracing::warn!(
                        row_id = %hex_display(&row_id),
                        execution_id = %msg.execution_id,
                        claim_generation = %token.generation(),
                        grace_ms = SHUTDOWN_DISPATCH_GRACE.as_millis() as u64,
                        "orchestrator dispatch did not drain within the shutdown grace; \
                         the acknowledged row's execution lease governs recovery (#976)"
                    );
                    return;
                };
                result
            }
        };

        match dispatch_result {
            Ok(()) => {
                self.inc_dispatch(orchestrator_dispatch_outcome::DISPATCHED);
            },
            Err(ref e) => {
                // The queue row was already acknowledged by the handoff, so
                // there is no queue state to fix: the execution lease and
                // persisted recovery state govern what happens next (lease
                // expiry → recovery redrive). Record the failure for
                // operators and move on.
                tracing::error!(
                    row_id = %hex_display(&row_id),
                    execution_id = %msg.execution_id,
                    command = msg.command.as_str(),
                    fence_generation = %fence.generation(),
                    error = %e,
                    "orchestrator dispatch failed after handoff; recovery drives from the execution lease (#976)"
                );
                self.inc_dispatch(orchestrator_dispatch_outcome::FAILED);
            },
        }
    }

    /// Increment the handoff-outcome counter. Counter construction failure is
    /// swallowed (same policy as the dispatch/reclaim counters): metrics must
    /// never take down the pull loop.
    fn inc_handoff(&self, outcome: &'static str) {
        let labels = self.metrics.interner().single("outcome", outcome);
        if let Ok(c) = self
            .metrics
            .counter_labeled(NEBULA_ORCHESTRATOR_HANDOFF_TOTAL, &labels)
        {
            c.inc();
        }
    }

    /// Increment the dispatch-outcome counter. Counter construction failure is
    /// swallowed (same policy as the reclaim counter): metrics must never take
    /// down the pull loop.
    fn inc_dispatch(&self, outcome: &'static str) {
        let labels = self.metrics.interner().single("outcome", outcome);
        if let Ok(c) = self
            .metrics
            .counter_labeled(NEBULA_ORCHESTRATOR_DISPATCH_TOTAL, &labels)
        {
            c.inc();
        }
    }

    async fn mark_failed(&self, token: &JobClaimToken, reason: &str) {
        if let Err(e) = self
            .queue
            .mark_failed(token, reason)
            .await
            .map_err(|e| e.to_string())
        {
            tracing::error!(
                row_id = %hex_display(token.row_id()),
                claim_generation = %token.generation(),
                error = %e,
                "orchestrator mark_failed failed; row left in Processing for reclaim"
            );
        }
    }
}

/// Attach the remote OTel parent from `w3c` to `span`.
///
/// Mirrors `control_trace::attach_control_queue_w3c_parent` from `nebula-engine`
/// without importing that crate (layer boundary). Invalid carriers leave the
/// span as a root — same non-fatal policy as the HTTP edge.
fn attach_w3c_parent(span: &tracing::Span, w3c: &nebula_core::W3cTraceContext) {
    use opentelemetry::global;
    use opentelemetry::propagation::Extractor;
    use opentelemetry::trace::TraceContextExt;
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    struct W3cExtractor<'a> {
        traceparent: &'a str,
        tracestate: Option<&'a str>,
    }

    impl Extractor for W3cExtractor<'_> {
        fn get(&self, key: &str) -> Option<&str> {
            if key.eq_ignore_ascii_case("traceparent") {
                return Some(self.traceparent);
            }
            if key.eq_ignore_ascii_case("tracestate") {
                return self.tracestate;
            }
            None
        }

        fn keys(&self) -> Vec<&str> {
            if self.tracestate.is_some() {
                vec!["traceparent", "tracestate"]
            } else {
                vec!["traceparent"]
            }
        }
    }

    let parent_ctx = global::get_text_map_propagator(|prop| {
        prop.extract(&W3cExtractor {
            traceparent: w3c.traceparent(),
            tracestate: w3c.tracestate(),
        })
    });

    // Borrow parent_ctx via .span()/.span_context(), then clone the SpanContext
    // (a Clone-cheap struct) to get an owned value. The borrow ends at the
    // semicolon, so parent_ctx can be moved into set_parent below.
    let span_ctx = parent_ctx.span().span_context().clone();
    if span_ctx.is_valid() {
        match span.set_parent(parent_ctx) {
            Ok(()) => tracing::debug!(
                trace_id = %span_ctx.trace_id(),
                "orchestrator: linked dispatch span to W3C parent from job-dispatch row"
            ),
            Err(err) => tracing::warn!(
                trace_id = %span_ctx.trace_id(),
                error = ?err,
                "orchestrator: span.set_parent failed after carrier validation; dispatch span stays root"
            ),
        }
    } else {
        tracing::debug!(
            "orchestrator: W3C carrier on row did not yield valid OTel parent; dispatch span stays root"
        );
    }
}

/// Exponential backoff for repeated `claim_pending` storage errors.
///
/// Starts at `base` and doubles per consecutive error, capped at
/// [`MAX_CLAIM_ERROR_BACKOFF`]. `consecutive_errors` is 1-indexed.
///
/// Average case O(1); worst case O(1) (saturating arithmetic, fixed cap).
fn claim_error_backoff(base: Duration, consecutive_errors: u32) -> Duration {
    let multiplier = 1u64
        .checked_shl(consecutive_errors.saturating_sub(1).min(30))
        .unwrap_or(u64::MAX);
    let scaled = base
        .checked_mul(u32::try_from(multiplier.min(u64::from(u32::MAX))).unwrap_or(u32::MAX))
        .unwrap_or(MAX_CLAIM_ERROR_BACKOFF);
    scaled.min(MAX_CLAIM_ERROR_BACKOFF)
}

/// Hex-render opaque byte fields for structured logs.
fn hex_display(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
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
        // Cap kicks in before overflow (100ms * 2^29 > 30s).
        assert_eq!(claim_error_backoff(base, 15), MAX_CLAIM_ERROR_BACKOFF);
        assert_eq!(claim_error_backoff(base, u32::MAX), MAX_CLAIM_ERROR_BACKOFF);
    }

    #[test]
    fn claim_error_backoff_zero_is_base() {
        // consecutive_errors == 0 never reached in practice (saturating_add
        // before call), but must be safe and return base.
        let base = Duration::from_millis(50);
        assert_eq!(claim_error_backoff(base, 0), base);
    }
}
