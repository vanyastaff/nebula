//! Durable control-queue consumer — canon §12.2.
//!
//! The `ControlConsumer` drains `execution_control_queue` (see
//! `nebula_storage::repos::ControlQueueRepo`) and hands typed commands to an
//! engine-owned [`ControlDispatch`] implementation. ADR-0008 records the
//! wiring decisions: polling loop + claim/ack, engine-owned dispatch trait
//! (no `nebula-api` / `nebula-storage` row types leak into the public
//! surface), at-least-once delivery with idempotent consumer semantics.
//!
//! ## Status
//!
//! - construction, spawning, graceful shutdown, polling, claim/ack plumbing — **implemented**
//!   (§11.6);
//! - dispatch of `Start` / `Resume` / `Restart` to the engine start/resume path — **implemented**
//!   (A2, closes #332 / #327). The engine-owned implementation lives in
//!   [`crate::control_dispatch::EngineControlDispatch`];
//! - dispatch of `Cancel` / `Terminate` to the engine cancel path — **planned**, lands with A3
//!   (closes #330).
//!
//! Until A3 lands, the default [`EngineControlDispatch`] body for
//! `Cancel` / `Terminate` returns a typed [`ControlDispatchError::Rejected`]
//! so the consumer marks the row `Failed` rather than silently acking it.
//!
//! [`EngineControlDispatch`]: crate::control_dispatch::EngineControlDispatch

use std::{sync::Arc, time::Duration};

use nebula_core::id::ExecutionId;
use nebula_storage::repos::{ControlCommand, ControlQueueEntry, ControlQueueRepo};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Default batch size for each `claim_pending` call.
///
/// Tuned small enough that a slow dispatch does not block a large batch of
/// rows from being visible to operators, large enough that a busy queue does
/// not round-trip to storage per command.
pub const DEFAULT_BATCH_SIZE: u32 = 32;

/// Default poll interval when the queue is empty.
///
/// Short enough that a cancel feels interactive in the in-memory / SQLite
/// local path (canon §12.3); the Postgres path may shorten this further
/// once `LISTEN / NOTIFY` wake-up is wired as an optimisation over the
/// authoritative polling loop.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Maximum backoff between `claim_pending` retries after repeated storage
/// errors. Prevents a 10Hz error-log flood when the backend is down.
pub const MAX_CLAIM_ERROR_BACKOFF: Duration = Duration::from_secs(30);

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
/// error. This is a load-bearing contract for ADR-0008 decision 5.
///
/// ## Status
///
/// Method stubs return `Ok(())` in A1 because no real dispatch happens
/// yet; A2 / A3 replace each method's body with a call into the engine's
/// start / cancel path. The trait shape (typed `ExecutionId` argument,
/// no storage / api types) is stabilised by A1's public-surface test.
#[async_trait::async_trait]
pub trait ControlDispatch: Send + Sync {
    /// Deliver a `Start` command to a newly-created execution (canon §12.2,
    /// §13 step 3, #332).
    ///
    /// Enqueued by the API `start_execution` / `execute_workflow` handlers
    /// once the `ExecutionState::Created` row has been persisted. A2 wired
    /// the canonical engine-side body in
    /// [`crate::control_dispatch::EngineControlDispatch`] — no default
    /// implementation is provided, so every `ControlDispatch` implementor
    /// must supply a real dispatch (the ADR-0008 A2 merge-checklist
    /// requirement).
    ///
    /// **Idempotency (critical):** double-start re-runs the workflow twice.
    /// Implementations must guard via CAS on `ExecutionRepo::transition` —
    /// a `Start` arriving for an already-running or already-terminal
    /// execution must be `Ok(())`, not a second run. See ADR-0008 §5.
    async fn dispatch_start(&self, execution_id: ExecutionId) -> Result<(), ControlDispatchError>;

    /// Deliver a `Cancel` command to a running execution.
    ///
    /// A3 wires this into the engine's cooperative-cancel path. A1 stub
    /// returns `Ok(())`.
    ///
    /// **Idempotency (load-bearing, ADR-0008 §5):** Must return `Ok(())`
    /// when the execution is already terminal or already being cancelled.
    /// The consumer's ack path (`mark_completed`) can fail after a
    /// successful dispatch, and the reclaim path (B1) will redeliver; a
    /// non-idempotent implementation double-cancels.
    async fn dispatch_cancel(&self, execution_id: ExecutionId) -> Result<(), ControlDispatchError>;

    /// Deliver a `Terminate` command (forced termination) to a running
    /// execution.
    ///
    /// A3 wires this into the engine's forced-shutdown path. A1 stub
    /// returns `Ok(())`.
    ///
    /// **Idempotency:** same contract as [`dispatch_cancel`](Self::dispatch_cancel) —
    /// a repeat for a terminal execution must be `Ok(())`.
    async fn dispatch_terminate(
        &self,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError>;

    /// Deliver a `Resume` command to a suspended execution.
    ///
    /// A2 wired the canonical body in
    /// [`crate::control_dispatch::EngineControlDispatch`].
    ///
    /// **Idempotency (critical):** double-resume starts the workflow twice.
    /// Implementations must guard via CAS on `ExecutionRepo::transition` —
    /// a `Resume` arriving for an already-running or already-terminal
    /// execution must be `Ok(())`, not a second start. See ADR-0008 §5.
    async fn dispatch_resume(&self, execution_id: ExecutionId) -> Result<(), ControlDispatchError>;

    /// Deliver a `Restart` command to an execution.
    ///
    /// A2 wired the canonical body in
    /// [`crate::control_dispatch::EngineControlDispatch`]. Full
    /// rewind-from-input semantics require durable output purge and a
    /// monotonic restart counter — both are tracked as follow-ups under
    /// ADR-0008; the A2 body honors idempotency for non-terminal states
    /// and surfaces a typed [`ControlDispatchError::Rejected`] for
    /// already-terminal executions until the rewind path is wired.
    ///
    /// **Idempotency:** same `Resume` contract applies — double-restart
    /// rewinds twice. Guard with a monotonic restart counter or CAS.
    async fn dispatch_restart(&self, execution_id: ExecutionId)
    -> Result<(), ControlDispatchError>;
}

/// Drains `execution_control_queue` and hands typed commands to a
/// [`ControlDispatch`] implementation.
///
/// See the module docs and ADR-0008 for wiring, atomicity, and idempotency
/// rules.
pub struct ControlConsumer {
    queue: Arc<dyn ControlQueueRepo>,
    dispatch: Arc<dyn ControlDispatch>,
    processor_id: Vec<u8>,
    batch_size: u32,
    poll_interval: Duration,
}

impl ControlConsumer {
    /// Construct a new consumer.
    ///
    /// `processor_id` is opaque bytes the storage layer records in
    /// `execution_control_queue.processed_by`; operators use it to identify
    /// which instance claimed a row. A hostname, a ULID, or a tuple of
    /// both are all reasonable choices.
    pub fn new(
        queue: Arc<dyn ControlQueueRepo>,
        dispatch: Arc<dyn ControlDispatch>,
        processor_id: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            queue,
            dispatch,
            processor_id: processor_id.into(),
            batch_size: DEFAULT_BATCH_SIZE,
            poll_interval: DEFAULT_POLL_INTERVAL,
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

    /// Spawn the consumer as a Tokio task. The returned handle completes
    /// when the task observes `shutdown` being cancelled.
    ///
    /// The consumer flushes any already-claimed commands before returning;
    /// it does not begin a fresh `claim_pending` once shutdown is requested.
    /// Rows that were claimed but not acknowledged remain in the `Processing`
    /// state for the reclaim path to pick up (tracked with B1; see
    /// ADR-0008 §5).
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
            "control-queue consumer started (canon §12.2, ADR-0008)"
        );

        let mut consecutive_errors: u32 = 0;
        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    tracing::info!(
                        processor = %hex_display(&self.processor_id),
                        "control-queue consumer shutting down"
                    );
                    return;
                }
                () = self.tick(&mut consecutive_errors) => {}
            }
        }
    }

    async fn tick(&self, consecutive_errors: &mut u32) {
        let claimed = match self
            .queue
            .claim_pending(&self.processor_id, self.batch_size)
            .await
        {
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
                tokio::time::sleep(backoff).await;
                return;
            },
        };

        if claimed.is_empty() {
            tokio::time::sleep(self.poll_interval).await;
            return;
        }

        for entry in claimed {
            self.handle_entry(entry).await;
        }
    }

    async fn handle_entry(&self, entry: ControlQueueEntry) {
        let execution_id = match decode_execution_id(&entry.execution_id) {
            Ok(id) => id,
            Err(reason) => {
                tracing::error!(
                    id = %hex_display(&entry.id),
                    reason = %reason,
                    "control-queue row has malformed execution_id; marking failed"
                );
                self.ack_failed(&entry.id, &format!("malformed execution_id: {reason}"))
                    .await;
                return;
            },
        };

        let dispatch_result = match entry.command {
            ControlCommand::Start => {
                tracing::debug!(%execution_id, "control-queue: dispatching Start (A2)");
                self.dispatch.dispatch_start(execution_id).await
            },
            ControlCommand::Cancel => {
                tracing::info!(
                    %execution_id,
                    "control-queue: observed Cancel (TODO(A3): wire to engine cancel path)"
                );
                self.dispatch.dispatch_cancel(execution_id).await
            },
            ControlCommand::Terminate => {
                tracing::info!(
                    %execution_id,
                    "control-queue: observed Terminate (TODO(A3): wire to engine terminate path)"
                );
                self.dispatch.dispatch_terminate(execution_id).await
            },
            ControlCommand::Resume => {
                tracing::debug!(%execution_id, "control-queue: dispatching Resume (A2)");
                self.dispatch.dispatch_resume(execution_id).await
            },
            ControlCommand::Restart => {
                tracing::debug!(%execution_id, "control-queue: dispatching Restart (A2)");
                self.dispatch.dispatch_restart(execution_id).await
            },
        };

        match dispatch_result {
            Ok(()) => self.ack_completed(&entry.id).await,
            Err(e) => {
                tracing::error!(
                    id = %hex_display(&entry.id),
                    %execution_id,
                    command = entry.command.as_str(),
                    error = %e,
                    "control-queue dispatch failed; marking failed (no auto-retry — ADR-0008 §5)"
                );
                self.ack_failed(&entry.id, &e.to_string()).await;
            },
        }
    }

    async fn ack_completed(&self, id: &[u8]) {
        // NOTE: dispatch already ran successfully at this point. If
        // `mark_completed` fails, the row stays in `Processing` and the B1
        // reclaim path will redeliver the command. Correctness under redelivery
        // depends entirely on `ControlDispatch` impls being idempotent per
        // `(execution_id, command)` — see the trait-level docs and ADR-0008 §5.
        if let Err(e) = self.queue.mark_completed(id).await {
            tracing::error!(
                id = %hex_display(id),
                error = %e,
                "control-queue mark_completed failed; row left in Processing for reclaim"
            );
        }
    }

    async fn ack_failed(&self, id: &[u8], reason: &str) {
        if let Err(e) = self.queue.mark_failed(id, reason).await {
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

/// Decode the UTF-8 ULID bytes stored in `ControlQueueEntry.execution_id`.
///
/// Canon note: `control_queue.rs` documents this encoding choice (UTF-8
/// string bytes, not raw 16-byte ULIDs) — the consumer honours it here so
/// the [`ControlDispatch`] surface sees a typed `ExecutionId`.
fn decode_execution_id(bytes: &[u8]) -> Result<ExecutionId, String> {
    let s = std::str::from_utf8(bytes).map_err(|e| format!("not valid UTF-8: {e}"))?;
    s.parse::<ExecutionId>()
        .map_err(|e| format!("not a valid ExecutionId ({s:?}): {e}"))
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

    #[test]
    fn decode_execution_id_rejects_non_utf8() {
        let invalid = vec![0xff, 0xfe, 0xfd];
        let err = decode_execution_id(&invalid).unwrap_err();
        assert!(err.contains("not valid UTF-8"), "got: {err}");
    }

    #[test]
    fn decode_execution_id_rejects_bad_prefix() {
        let wrong = b"not-a-ulid".to_vec();
        let err = decode_execution_id(&wrong).unwrap_err();
        assert!(err.contains("not a valid ExecutionId"), "got: {err}");
    }

    #[test]
    fn decode_execution_id_accepts_round_trip() {
        let id = ExecutionId::new();
        let bytes = id.to_string().into_bytes();
        let decoded = decode_execution_id(&bytes).expect("round trip");
        assert_eq!(decoded, id);
    }
}
