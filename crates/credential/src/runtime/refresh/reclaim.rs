//! Background reclaim-sweep task. Parallel to control-queue reclaim.
//!
//! Per sub-spec
//! `docs/INTEGRATION_MODEL.md`
//! +.
//!
//! On a fixed cadence the task calls `RefreshClaimRepo::reclaim_stuck`,
//! atomically accounting every claim row whose `expires_at` is in the past:
//!
//! - `Normal` -- the holder timed out before provider egress. The row is
//!   deleted and may be safely acquired again.
//! - `RefreshInFlight` -- provider outcome is unknown. The repository records
//!   one durable sentinel event atomically but retains the claim row as
//!   fail-closed poison. The sweep only evaluates the already-recorded count;
//!   it never authorizes provider replay. Explicit reconciliation is K3.
//!
//! The storage boundary makes repeated and concurrent sweeps idempotent:
//! normal rows are deleted at most once, while each poisoned
//! credential/generation is returned only when its evidence is newly recorded.

use std::{sync::Arc, time::Duration};

use crate::{CredentialEvent, contract::resolve::ReauthReason};
use nebula_eventbus::EventBus;
use nebula_storage_port::store::{
    ExpiredClaim, RefreshClaimError as RepoError, RefreshClaimStore as RefreshClaimRepo,
};
use tracing::Instrument;

use crate::audit::AuditSink;

use super::{
    audit::{emit_reauth_threshold_reached, emit_sentinel_triggered},
    coordinator::RefreshCoordinator,
    metrics::RefreshCoordMetrics,
    sentinel::{SentinelDecision, SentinelTrigger},
};

/// Handle for the background reclaim sweep task.
///
/// Holding the handle keeps the task alive; dropping or calling
/// [`Self::abort`] cancels it. The task itself loops forever --
/// the handle is the only path to graceful shutdown.
pub struct ReclaimSweepHandle {
    handle: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for ReclaimSweepHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReclaimSweepHandle")
            .field("is_finished", &self.handle.is_finished())
            .finish()
    }
}

impl ReclaimSweepHandle {
    /// Spawn the reclaim sweep task wired to a [`RefreshCoordinator`].
    ///
    /// The cadence and the underlying [`RefreshClaimRepo`] are derived
    /// from `coord` so the invariant
    /// `reclaim_sweep_interval <= claim_ttl` (validated at coordinator
    /// construction) reaches the sweep task by construction -- no
    /// freestanding `Duration` argument that could drift from the
    /// validated config (review feedback I3).
    ///
    /// The task wakes every `coord.config().reclaim_sweep_interval`,
    /// calls `repo.reclaim_stuck()`, routes newly-accounted
    /// `RefreshInFlight` poison to
    /// `sentinel.decision_for_accounted_event`, and emits
    /// a lossy `CredentialEvent::ReauthRequired` observation on the
    /// supplied event bus when the threshold is exceeded. It does not
    /// mutate the durable credential aggregate.
    ///
    /// `event_bus` is optional: in tests / desktop mode without an
    /// event bus the threshold-exceed path still records the event in
    /// `credential_sentinel_events` and logs a `tracing::warn!` line --
    /// only the cross-replica fan-out is skipped.
    pub fn spawn(
        coord: Arc<RefreshCoordinator>,
        sentinel: Arc<SentinelTrigger>,
        event_bus: Option<Arc<EventBus<CredentialEvent>>>,
    ) -> Self {
        let cadence = coord.config().reclaim_sweep_interval;
        let repo = Arc::clone(coord.repo());
        // Sub-spec -- sweep emits metrics + audit events through the
        // same handles wired into the coordinator so a single PromQL /
        // sink view aggregates both refresh sites.
        let metrics = coord.metrics().clone();
        let audit_sink = coord.audit_sink().cloned();
        let handle = tokio::spawn(async move {
            sweep_loop(repo, sentinel, cadence, event_bus, metrics, audit_sink).await;
        });
        Self { handle }
    }

    /// Abort the running sweep task. Safe to call multiple times.
    pub fn abort(&self) {
        self.handle.abort();
    }

    /// Whether the underlying task has finished (e.g. via abort).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

impl Drop for ReclaimSweepHandle {
    fn drop(&mut self) {
        // Cancel the spawned task so the sweep does not outlive the
        // engine that started it.
        self.handle.abort();
    }
}

async fn sweep_loop(
    repo: Arc<dyn RefreshClaimRepo>,
    sentinel: Arc<SentinelTrigger>,
    cadence: Duration,
    event_bus: Option<Arc<EventBus<CredentialEvent>>>,
    metrics: RefreshCoordMetrics,
    audit_sink: Option<Arc<dyn AuditSink>>,
) {
    let mut ticker = tokio::time::interval(cadence);
    // Avoid back-to-back sweeps after a long storage stall.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Burn the leading immediate tick so the first real sweep happens
    // after one cadence -- mirrors the heartbeat task pattern in
    // `coordinator.rs`.
    ticker.tick().await;
    #[expect(
        clippy::infinite_loop,
        reason = "reclaim daemon sweeps until its task is aborted at shutdown"
    )]
    loop {
        ticker.tick().await;
        if let Err(e) = run_one_sweep(
            &repo,
            &sentinel,
            event_bus.as_ref(),
            &metrics,
            audit_sink.as_deref(),
        )
        .await
        {
            // run_one_sweep already logs per-row errors; this catches
            // the top-level reclaim_stuck failure.
            tracing::warn!(?e, "credential refresh reclaim sweep failed");
        }
    }
}

pub(super) async fn run_one_sweep(
    repo: &Arc<dyn RefreshClaimRepo>,
    sentinel: &Arc<SentinelTrigger>,
    event_bus: Option<&Arc<EventBus<CredentialEvent>>>,
    metrics: &RefreshCoordMetrics,
    audit_sink: Option<&dyn AuditSink>,
) -> Result<(), RepoError> {
    let stuck = repo.reclaim_stuck().await?;
    // Sub-spec -- increment reclaim sweep counter once per sweep,
    // labeled by whether work was found. `no_work` is the steady state
    // for healthy systems; `reclaimed` rising is a crashed-runner signal.
    if stuck.is_empty() {
        metrics.reclaim_no_work.inc();
    } else if stuck
        .iter()
        .any(|claim| matches!(claim, ExpiredClaim::OutcomeUnknownAccounted { .. }))
    {
        // A mixed sweep is classified by its highest-severity result so every
        // sweep increments exactly one outcome and poison never masquerades as
        // an ordinary reclaim.
        metrics.reclaim_outcome_unknown_accounted.inc();
    } else {
        metrics.reclaim_reclaimed.inc();
    }
    for reclaimed in stuck {
        let ExpiredClaim::OutcomeUnknownAccounted {
            credential_id,
            previous_holder,
            previous_generation,
        } = reclaimed
        else {
            // Normal-path expiry was safely deleted; no provider-side effect
            // or sentinel observation exists.
            continue;
        };
        // Sub-spec -- per-row span; an operator can grep
        // `credential.refresh.sentinel.detected` for crashed-mid-refresh
        // events without wading through normal-expiry rows. Use
        // `.instrument(...)` rather than `.entered()` so the span
        // boundary respects `Send` for the spawned sweep task.
        let detect_span = tracing::info_span!(
            "credential.refresh.sentinel.detected",
            credential_id = %credential_id,
            crashed_holder = %previous_holder,
            generation = previous_generation,
        );
        let decision = match async { sentinel.decision_for_accounted_event(&credential_id).await }
            .instrument(detect_span)
            .await
        {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(
                    cred = %credential_id,
                    ?e,
                    "accounted sentinel decision failed; poison remains fail-closed"
                );
                continue;
            },
        };

        match decision {
            SentinelDecision::BelowThreshold { event_count } => {
                tracing::info!(
                    cred = %credential_id,
                    event_count,
                    "sentinel recorded; credential remains poisoned pending reconciliation"
                );
                // The event was already recorded atomically by reclaim_stuck.
                metrics.sentinel_recorded.inc();
                emit_sentinel_triggered(audit_sink, &credential_id, event_count);
            },
            SentinelDecision::EscalateToReauth {
                event_count,
                window_secs,
            } => {
                tracing::warn!(
                    cred = %credential_id,
                    event_count,
                    window_secs,
                    "sentinel threshold exceeded -- emitting reauth-required observation"
                );
                // Sub-spec -- bump both the recorded counter (every
                // detection counts) and the reauth_triggered counter
                // (the threshold-crossing observation itself).
                metrics.sentinel_recorded.inc();
                metrics.sentinel_reauth_triggered.inc();
                emit_sentinel_triggered(audit_sink, &credential_id, event_count);
                emit_reauth_threshold_reached(audit_sink, &credential_id, "sentinel_repeated");
                if let Some(bus) = event_bus {
                    let event = CredentialEvent::ReauthRequired {
                        credential_id,
                        reason: ReauthReason::SentinelRepeated {
                            event_count,
                            window_secs,
                        },
                    };
                    let outcome = bus.emit(event);
                    if !matches!(outcome, nebula_eventbus::PublishOutcome::Sent) {
                        tracing::warn!(
                            cred = %credential_id,
                            ?outcome,
                            "CredentialEvent::ReauthRequired publish dropped"
                        );
                    }
                }
            },
        }
    }
    Ok(())
}
