//! Frontier wake — the `run_frontier` loop's per-iteration sleep/notify
//! barrier.
//!
//! [`WorkflowEngine::await_frontier_wake`] builds the three sleep futures
//! (wall-clock budget deadline, next retry timer, next wait timer), pins
//! them, and races them in one `tokio::select!` against the in-flight task
//! set, the cancel token, and the live Resume channel. It reports which arm
//! won as a [`WakeReason`], carrying the arm's payload (the join result, or
//! the dequeued `ResumeRequest`). The post-wake control flow — every side
//! effect and loop transition — stays in the loop body in `super`, which
//! resolves through this module's explicit imports.

use std::{
    cmp::Reverse,
    future::Future,
    pin::Pin,
    time::{Duration, Instant},
};

use nebula_action::ActionResult;
use nebula_core::NodeKey;
use nebula_execution::context::ExecutionBudget;
use tokio_util::sync::CancellationToken;

use crate::engine::{ResumeRequest, WorkflowEngine};
use crate::error::EngineError;

use super::FrontierCtx;

/// Output of the `join_next` arm: one completed in-flight task, or `None`
/// when the `JoinSet` drained mid-iteration.
pub(super) type JoinedResult = Option<
    Result<
        (
            tokio::task::Id,
            (
                NodeKey,
                Result<ActionResult<serde_json::Value>, EngineError>,
            ),
        ),
        tokio::task::JoinError,
    >,
>;

// Joined variant is the wide one — the other arms are
// unit-like timer markers. The size asymmetry is intrinsic
// to a wake-reason discriminant and acceptable on a path
// that allocates one value per loop iteration.
pub(super) enum WakeReason {
    Joined(JoinedResult),
    RetryTimer,
    WaitTimer,
    WallClock,
    Cancel,
    ResumeSignalled(ResumeRequest),
    ResumeChannelClosed,
}

impl WorkflowEngine {
    /// Build this iteration's sleep futures and wait until exactly one
    /// frontier wake source fires.
    ///
    /// Returns the winning wake signal; the caller owns all post-wake
    /// behavior. No side effect happens here — every arm body only maps its
    /// future's output into a [`WakeReason`].
    pub(super) async fn await_frontier_wake(
        &self,
        ctx: &mut FrontierCtx<'_>,
        cancel_token: &CancellationToken,
        budget: &ExecutionBudget,
        started: &Instant,
        elapsed_before_turn: Duration,
    ) -> WakeReason {
        // Race join_set against the wall-clock deadline so a hung node
        // cannot starve budget enforcement. The Phase 1 check_budget call
        // only fires while ready_queue has work; once everything is in
        // flight, this select is the sole budget guard.
        let wall_clock_remaining: Option<Duration> = budget.max_duration.map(|max_dur| {
            max_dur.saturating_sub(elapsed_before_turn.saturating_add(started.elapsed()))
        });
        let sleep_fut = async {
            if let Some(d) = wall_clock_remaining {
                tokio::time::sleep(d).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::pin!(sleep_fut);

        // Compute the sleep until the next retry timer fires. If
        // `retry_heap` is empty, sleep forever (the join_set / cancel
        // / wall-clock arms still drive the select).
        let next_retry_in: Option<Duration> = ctx.retry_heap.peek().map(|Reverse((when, _))| {
            when.signed_duration_since(self.clock.now())
                .to_std()
                .unwrap_or(Duration::ZERO)
        });
        let retry_sleep_fut = async {
            if let Some(d) = next_retry_in {
                tokio::time::sleep(d).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::pin!(retry_sleep_fut);

        // Compute the sleep until the earliest parked-wait timer fires.
        // This drives Phase 0b drains when `join_set` is otherwise idle.
        let next_wait_in: Option<Duration> = ctx.wait_heap.peek().map(|Reverse((when, _))| {
            when.signed_duration_since(self.clock.now())
                .to_std()
                .unwrap_or(Duration::ZERO)
        });
        let wait_sleep_fut = async {
            if let Some(d) = next_wait_in {
                tokio::time::sleep(d).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::pin!(wait_sleep_fut);

        // If join_set is empty but retry_heap or wait_heap has work,
        // we still need to sleep until the timer (or cancel /
        // wall-clock). Pre-pin a boxed future per branch so `select!`
        // never has to enter an `unreachable!()` placeholder — library
        // code must not panic on hot paths (hot-path safety).
        let join_set_empty = ctx.join_set.is_empty();

        let join_next_fut: Pin<Box<dyn Future<Output = JoinedResult> + Send + '_>> =
            if join_set_empty {
                Box::pin(std::future::pending::<JoinedResult>())
            } else {
                Box::pin(ctx.join_set.join_next_with_id())
            };

        // `mpsc::Receiver::recv()` is cancellation-safe in this `select!`:
        // a `ResumeRequest` that is sent while another arm wins this
        // iteration is NOT consumed — it stays buffered in the channel and
        // the next iteration's fresh `recv()` delivers it. This is strictly
        // better than the prior `Notify` permit-latch: the request (and its
        // `ack` reply channel) is never dropped between iterations, so the
        // caller's ack-await always resolves to a durable outcome.
        //
        // The `if !resume_rx_closed` guard is REQUIRED: once the channel is
        // closed, `recv()` returns `Ready(None)` synchronously on every
        // poll. Without the guard the closed arm would win the select! on
        // every iteration regardless of wall-clock sleep — a busy-spin for
        // the full run duration. After exactly one `None` the flag is set
        // and the arm becomes permanently `Pending` (disabled), letting the
        // other arms run at their natural pace.
        tokio::select! {
            result = join_next_fut => WakeReason::Joined(result),
            () = &mut retry_sleep_fut, if next_retry_in.is_some() => WakeReason::RetryTimer,
            () = &mut wait_sleep_fut, if next_wait_in.is_some() => WakeReason::WaitTimer,
            () = &mut sleep_fut => WakeReason::WallClock,
            () = cancel_token.cancelled() => WakeReason::Cancel,
            maybe_req = ctx.resume_rx.recv(), if !ctx.resume_rx_closed => match maybe_req {
                Some(req) => WakeReason::ResumeSignalled(req),
                None => WakeReason::ResumeChannelClosed,
            },
        }
    }
}
