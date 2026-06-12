//! Background reclaim-sweep task. Parallel to control-queue reclaim.
//!
//! Per sub-spec
//! `docs/INTEGRATION_MODEL.md`
//! +.
//!
//! On a fixed cadence the task calls `RefreshClaimRepo::reclaim_stuck`,
//! atomically sweeping every claim row whose `expires_at` is in the
//! past. For each row the sweep returns paired with its observed
//! sentinel state:
//!
//! - `SentinelState::Normal` -- the holder simply timed out without ever entering the IdP POST. The
//!   row is already deleted by `reclaim_stuck`; nothing more to do.
//! - `SentinelState::RefreshInFlight` -- the holder crashed mid-refresh. The sweep routes the event
//!   to [`SentinelTrigger::on_sentinel_detected`] which records it in `credential_sentinel_events`
//!   and consults the rolling-window count. When the threshold is reached the sweep publishes
//!   `CredentialEvent::ReauthRequired { credential_id, reason: SentinelRepeated }` so downstream
//!   consumers (UI, monitoring, resource pools) react immediately.
//!
//! Two sweeps observing the same expired row are prevented at the
//! storage layer -- `reclaim_stuck` issues one atomic `DELETE ... RETURNING`
//! so each row appears in exactly one sweeper's result.

use std::{sync::Arc, time::Duration};

use crate::{CredentialEvent, contract::resolve::ReauthReason};
use nebula_eventbus::EventBus;
use nebula_storage_port::store::{
    RefreshClaimError as RepoError, RefreshClaimStore as RefreshClaimRepo, SentinelState,
};
use tracing::Instrument;

use crate::audit::AuditSink;

use super::{
    audit::{emit_reauth_flagged, emit_sentinel_triggered},
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
    /// calls `repo.reclaim_stuck()`, routes any `RefreshInFlight`
    /// claims to `sentinel.on_sentinel_detected`, and emits
    /// `CredentialEvent::ReauthRequired` on the supplied event bus
    /// when the threshold is exceeded.
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

async fn run_one_sweep(
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
    } else {
        metrics.reclaim_reclaimed.inc();
    }
    for reclaimed in stuck {
        if reclaimed.sentinel != SentinelState::RefreshInFlight {
            // Normal-path expiry -- claim was reclaimed, nothing more to
            // do (no mid-refresh crash to record).
            continue;
        }
        // Sub-spec -- per-row span; an operator can grep
        // `credential.refresh.sentinel.detected` for crashed-mid-refresh
        // events without wading through normal-expiry rows. Use
        // `.instrument(...)` rather than `.entered()` so the span
        // boundary respects `Send` for the spawned sweep task.
        let detect_span = tracing::info_span!(
            "credential.refresh.sentinel.detected",
            credential_id = %reclaimed.credential_id,
            crashed_holder = %reclaimed.previous_holder,
            generation = reclaimed.previous_generation,
        );
        let decision = match async {
            sentinel
                .on_sentinel_detected(
                    &reclaimed.credential_id,
                    &reclaimed.previous_holder,
                    reclaimed.previous_generation,
                )
                .await
        }
        .instrument(detect_span)
        .await
        {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(
                    cred = %reclaimed.credential_id,
                    ?e,
                    "sentinel decision failed; reclaim sweep continues"
                );
                continue;
            },
        };

        match decision {
            SentinelDecision::Recoverable { event_count } => {
                tracing::info!(
                    cred = %reclaimed.credential_id,
                    event_count,
                    "sentinel recoverable -- credential refresh will retry"
                );
                // Sub-spec -- record the event (below threshold).
                metrics.sentinel_recorded.inc();
                emit_sentinel_triggered(audit_sink, &reclaimed.credential_id, event_count);
            },
            SentinelDecision::EscalateToReauth {
                event_count,
                window_secs,
            } => {
                tracing::warn!(
                    cred = %reclaimed.credential_id,
                    event_count,
                    window_secs,
                    "sentinel threshold exceeded -- escalating to ReauthRequired"
                );
                // Sub-spec -- bump both the recorded counter (every
                // detection counts) and the reauth_triggered counter
                // (the escalation transition itself).
                metrics.sentinel_recorded.inc();
                metrics.sentinel_reauth_triggered.inc();
                emit_sentinel_triggered(audit_sink, &reclaimed.credential_id, event_count);
                emit_reauth_flagged(audit_sink, &reclaimed.credential_id, "sentinel_repeated");
                if let Some(bus) = event_bus {
                    let event = CredentialEvent::ReauthRequired {
                        credential_id: reclaimed.credential_id,
                        reason: ReauthReason::SentinelRepeated {
                            event_count,
                            window_secs,
                        },
                    };
                    let outcome = bus.emit(event);
                    if !matches!(outcome, nebula_eventbus::PublishOutcome::Sent) {
                        tracing::warn!(
                            cred = %reclaimed.credential_id,
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
