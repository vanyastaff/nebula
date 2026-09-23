//! Background refresh-claim reclaim task.
//!
//! The injected [`RefreshClaimReclaimer`] owns the atomic storage boundary:
//! incident accounting, rolling-window evaluation, and any credential
//! reauthentication transition commit together. This task consumes that
//! authoritative result and emits observations only after commit.

use std::{sync::Arc, time::Duration};

use nebula_eventbus::EventBus;
use nebula_storage_port::store::{
    ExpiredClaim, ReauthEscalation, RefreshClaimError, RefreshClaimReclaimer,
    SentinelEscalationPolicy,
};

use crate::{CredentialEvent, audit::AuditSink, contract::resolve::ReauthReason};

use super::{
    audit::{emit_reauth_threshold_reached, emit_sentinel_triggered},
    coordinator::RefreshCoordinator,
    metrics::RefreshCoordMetrics,
};

/// Handle for the background reclaim sweep task.
pub struct ReclaimSweepHandle {
    handle: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for ReclaimSweepHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReclaimSweepHandle")
            .field("is_finished", &self.handle.is_finished())
            .finish()
    }
}

impl ReclaimSweepHandle {
    /// Spawn the sole periodic reclaim authority.
    pub fn spawn(
        coord: Arc<RefreshCoordinator>,
        reclaimer: Arc<dyn RefreshClaimReclaimer>,
        policy: SentinelEscalationPolicy,
        event_bus: Option<Arc<EventBus<CredentialEvent>>>,
    ) -> Self {
        let cadence = coord.config().reclaim_sweep_interval;
        let metrics = coord.metrics().clone();
        let audit_sink = coord.audit_sink().cloned();
        let handle = tokio::spawn(async move {
            sweep_loop(reclaimer, policy, cadence, event_bus, metrics, audit_sink).await;
        });
        Self { handle }
    }

    /// Abort the running sweep task. Safe to call multiple times.
    pub fn abort(&self) {
        self.handle.abort();
    }

    /// Whether the underlying task has finished.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

impl Drop for ReclaimSweepHandle {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn sweep_loop(
    reclaimer: Arc<dyn RefreshClaimReclaimer>,
    policy: SentinelEscalationPolicy,
    cadence: Duration,
    event_bus: Option<Arc<EventBus<CredentialEvent>>>,
    metrics: RefreshCoordMetrics,
    audit_sink: Option<Arc<dyn AuditSink>>,
) {
    let mut ticker = tokio::time::interval(cadence);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticker.tick().await;
    #[expect(
        clippy::infinite_loop,
        reason = "reclaim daemon sweeps until its task is aborted at shutdown"
    )]
    loop {
        ticker.tick().await;
        if let Err(error) = run_one_sweep(
            reclaimer.as_ref(),
            policy,
            event_bus.as_ref(),
            &metrics,
            audit_sink.as_deref(),
        )
        .await
        {
            tracing::warn!(?error, "credential refresh reclaim sweep failed");
        }
    }
}

pub(super) async fn run_one_sweep(
    reclaimer: &dyn RefreshClaimReclaimer,
    policy: SentinelEscalationPolicy,
    event_bus: Option<&Arc<EventBus<CredentialEvent>>>,
    metrics: &RefreshCoordMetrics,
    audit_sink: Option<&dyn AuditSink>,
) -> Result<(), RefreshClaimError> {
    let stuck = reclaimer.reclaim_stuck(policy).await?;
    if stuck.is_empty() {
        metrics.reclaim_no_work.inc();
    } else if stuck
        .iter()
        .any(|claim| matches!(claim, ExpiredClaim::OutcomeUnknownAccounted { .. }))
    {
        metrics.reclaim_outcome_unknown_accounted.inc();
    } else {
        metrics.reclaim_reclaimed.inc();
    }

    for reclaimed in stuck {
        let ExpiredClaim::OutcomeUnknownAccounted {
            selector,
            previous_holder,
            previous_generation,
            event_count,
            escalation,
        } = reclaimed
        else {
            continue;
        };
        let credential_id = selector.credential_id();
        let span = tracing::info_span!(
            "credential.refresh.sentinel.detected",
            credential_id = %credential_id,
            crashed_holder = %previous_holder,
            generation = previous_generation,
            event_count,
        );
        let _entered = span.enter();
        metrics.sentinel_recorded.inc();
        emit_sentinel_triggered(audit_sink, &credential_id, event_count);

        match escalation {
            ReauthEscalation::BelowThreshold => {
                tracing::info!(
                    "sentinel recorded; credential remains poisoned pending reconciliation"
                );
            },
            ReauthEscalation::AggregateTerminal => {
                tracing::info!(
                    "sentinel recorded after credential became terminal; no reauth transition needed"
                );
            },
            ReauthEscalation::ReauthRequired {
                changed,
                version,
                material_epoch,
            } => {
                metrics.sentinel_reauth_triggered.inc();
                emit_reauth_threshold_reached(audit_sink, &credential_id, "sentinel_repeated");
                tracing::warn!(
                    changed,
                    credential_version = %version,
                    material_epoch = %material_epoch,
                    "sentinel threshold committed durable reauthentication state"
                );
                if let Some(bus) = event_bus {
                    let event = CredentialEvent::ReauthRequired {
                        credential_id,
                        reason: ReauthReason::SentinelRepeated {
                            event_count,
                            window_secs: policy.window().as_secs().max(1),
                        },
                    };
                    let outcome = bus.emit(event);
                    if !matches!(outcome, nebula_eventbus::PublishOutcome::Sent) {
                        tracing::warn!(?outcome, "CredentialEvent::ReauthRequired publish dropped");
                    }
                }
            },
        }
    }
    Ok(())
}
