//! Durable-timer wake scanner — the cross-execution liveness safety net for
//! parked timer waits (ADR-0099 pre-1.0 durability requirement).
//!
//! A timer-`Waiting` node fires in-process while its owning runner is alive (the
//! frontier loop blocks in `select!` on the wake deadline). If that runner
//! **crashes**, nothing re-drives the timer: a re-delivered `Start` no-ops on
//! the `Running` status ([`control_dispatch`](crate::control_dispatch)), and
//! `Resume` recovery only arms *signal* waits — so the overdue timer is
//! stranded until this scanner picks it up. The durable-timer guarantee is
//! otherwise *conditional on the parking runner surviving until the deadline*,
//! which is not durable.
//!
//! The sweep lists `Running` executions, finds any with a `Waiting` node whose
//! persisted `next_attempt_at` is now overdue, and re-drives each through
//! [`WorkflowEngine::resume_execution`] — the same path a fresh runner takes,
//! which re-seeds the `wait_heap` from the persisted deadline and fires the
//! overdue wake in the frontier's Phase 0b.
//!
//! The **execution lease is the liveness oracle** (canon: `acquire_lease` is
//! the sole liveness authority — no second liveness store): a live owner still
//! holds the lease, so `resume_execution` returns [`EngineError::Leased`] and
//! the scanner skips (no double-drive); a crashed owner's lease has expired, so
//! `resume_execution` acquires it and drives the wake. The scanner therefore
//! adds no new persisted state and no new claim protocol — it only re-invokes,
//! under the existing lease discipline, the wake path that already works.

use std::sync::Arc;
use std::time::Duration;

use nebula_execution::ExecutionState;
use tokio::task::JoinHandle;

use super::*;

/// Default cadence of the durable-timer wake scanner sweep.
pub const DEFAULT_TIMER_SCAN_INTERVAL: Duration = Duration::from_secs(30);

impl WorkflowEngine {
    /// Re-drive every `Running` execution that has an overdue parked timer but
    /// no live owner, firing the wake the crashed runner would have fired.
    ///
    /// The execution lease is the dead-vs-live oracle: a live owner holds it
    /// (`resume_execution` → [`EngineError::Leased`] → skipped, no double-drive);
    /// a crashed owner's lease has expired (acquired → wake driven). Per-node
    /// signal-only waits (`next_attempt_at == None`) are never touched — they
    /// are not timers. A signal wait parked *with* a timeout (`wait_wake ==
    /// Timeout`) whose deadline is overdue is correctly fired down its timeout
    /// (fail) branch, exactly as a live runner would.
    ///
    /// Returns the number of executions re-driven. Per-execution errors are
    /// logged and never abort the sweep.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::PlanningFailed`] only if the initial
    /// `list_running` storage call fails; individual execution failures are
    /// absorbed so one bad row cannot wedge the sweep.
    pub async fn sweep_overdue_timers(&self, scope: &Scope) -> Result<usize, EngineError> {
        let Some(stores) = self.stores.clone() else {
            // No durable store wired (library mode) — nothing to scan.
            return Ok(0);
        };
        let now = self.clock.now();
        let running =
            stores.execution.list_running(scope).await.map_err(|e| {
                EngineError::PlanningFailed(format!("timer-scan list_running: {e}"))
            })?;

        let mut redriven = 0usize;
        for id in running {
            let Ok(Some(record)) = stores.execution.get(scope, &id).await else {
                // Storage blip or the row vanished between list and get — the
                // next sweep retries.
                continue;
            };
            // `from_str` (not `from_value`): `ExecutionState` carries a borrowing
            // field that `serde_json::from_value` cannot deserialize from an owned
            // `Value`, so round-trip through the string form (the same convention
            // the engine's own state-load path uses).
            let Ok(state) = serde_json::from_str::<ExecutionState>(&record.state.to_string())
            else {
                continue;
            };
            let has_overdue_timer = state.node_states.values().any(|ns| {
                ns.state == NodeState::Waiting && ns.next_attempt_at.is_some_and(|when| when <= now)
            });
            if !has_overdue_timer {
                continue;
            }
            let Ok(execution_id) = id.parse::<ExecutionId>() else {
                tracing::warn!(
                    target = "engine::timer_scan",
                    execution_id = %id,
                    "unparseable execution id in running set; skipping"
                );
                continue;
            };
            match self.resume_execution(scope, execution_id).await {
                Ok(_) => {
                    redriven += 1;
                    tracing::debug!(
                        target = "engine::timer_scan",
                        %execution_id,
                        "re-drove overdue timer wake for a no-live-owner execution"
                    );
                },
                // Live owner holds the lease — it will fire the wake in-process.
                Err(EngineError::Leased { .. }) => {},
                Err(e) => {
                    tracing::warn!(
                        target = "engine::timer_scan",
                        %execution_id,
                        error = %e,
                        "overdue-timer re-drive failed; will retry next sweep"
                    );
                },
            }
        }
        Ok(redriven)
    }

    /// Spawn the durable-timer wake scanner as a background task.
    ///
    /// Ticks every `interval`, calling [`sweep_overdue_timers`](Self::sweep_overdue_timers)
    /// for `scope`, until `shutdown` is cancelled. Mirrors the control-queue
    /// reclaim sweep's plain-task model (a missed tick is delayed, not bursted).
    /// The composition root owns the cadence; [`DEFAULT_TIMER_SCAN_INTERVAL`] is
    /// a sane default.
    #[must_use = "the returned JoinHandle owns the scanner task; dropping it detaches the loop"]
    pub fn spawn_timer_scanner(
        self: Arc<Self>,
        scope: Scope,
        interval: Duration,
        shutdown: CancellationToken,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // Skip the immediate first tick: nothing is overdue the instant we
            // start, and a freshly-restarted process should let live runners
            // re-seed before sweeping.
            let _ = ticker.tick().await;
            loop {
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => {
                        tracing::info!(
                            target = "engine::timer_scan",
                            "durable-timer scanner shutting down"
                        );
                        return;
                    }
                    _ = ticker.tick() => {
                        match self.sweep_overdue_timers(&scope).await {
                            Ok(n) if n > 0 => tracing::info!(
                                target = "engine::timer_scan",
                                redriven = n,
                                "durable-timer scanner re-drove overdue wakes"
                            ),
                            Ok(_) => {},
                            Err(e) => tracing::warn!(
                                target = "engine::timer_scan",
                                error = %e,
                                "durable-timer sweep failed; will retry next tick"
                            ),
                        }
                    }
                }
            }
        })
    }
}
