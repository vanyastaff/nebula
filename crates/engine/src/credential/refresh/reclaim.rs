//! Background reclaim-sweep task. Parallel to control-queue reclaim.
//!
//! Per sub-spec
//! `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`
//! §3.3 + §3.4.
//!
//! On a fixed cadence the task calls `RefreshClaimRepo::reclaim_stuck`,
//! atomically sweeping every claim row whose `expires_at` is in the
//! past. For each row the sweep returns paired with its observed
//! sentinel state:
//!
//! - `SentinelState::Normal` — the holder simply timed out without ever entering the IdP POST. The
//!   row is already deleted by `reclaim_stuck`; nothing more to do.
//! - `SentinelState::RefreshInFlight` — the holder crashed mid-refresh. The sweep routes the event
//!   to [`SentinelTrigger::on_sentinel_detected`] which records it in `credential_sentinel_events`
//!   and consults the rolling-window count. When the threshold is reached the sweep publishes
//!   `CredentialEvent::ReauthRequired { credential_id, reason: SentinelRepeated }` so downstream
//!   consumers (UI, monitoring, resource pools) react immediately.
//!
//! Two sweeps observing the same expired row are prevented at the
//! storage layer — `reclaim_stuck` issues one atomic `DELETE ... RETURNING`
//! so each row appears in exactly one sweeper's result.

use std::{sync::Arc, time::Duration};

use nebula_credential::{CredentialEvent, resolve::ReauthReason};
use nebula_eventbus::EventBus;
use nebula_storage::credential::{RefreshClaimRepo, SentinelState};

use super::sentinel::{SentinelDecision, SentinelTrigger};

/// Handle for the background reclaim sweep task.
///
/// Holding the handle keeps the task alive; dropping or calling
/// [`Self::abort`] cancels it. The task itself loops forever — the
/// handle is the only path to graceful shutdown.
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
    /// Spawn the reclaim sweep task. The task wakes every `cadence`,
    /// calls `repo.reclaim_stuck()`, routes any `RefreshInFlight`
    /// claims to `sentinel.on_sentinel_detected`, and emits
    /// `CredentialEvent::ReauthRequired` on the supplied event bus
    /// when the threshold is exceeded.
    ///
    /// `event_bus` is optional: in tests / desktop mode without an
    /// event bus the threshold-exceed path still records the event in
    /// `credential_sentinel_events` and logs a `tracing::warn!` line —
    /// only the cross-replica fan-out is skipped.
    pub fn spawn(
        repo: Arc<dyn RefreshClaimRepo>,
        sentinel: Arc<SentinelTrigger>,
        cadence: Duration,
        event_bus: Option<Arc<EventBus<CredentialEvent>>>,
    ) -> Self {
        let handle = tokio::spawn(async move {
            sweep_loop(repo, sentinel, cadence, event_bus).await;
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
) {
    let mut ticker = tokio::time::interval(cadence);
    // Avoid back-to-back sweeps after a long storage stall.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Burn the leading immediate tick so the first real sweep happens
    // after one cadence — mirrors the heartbeat task pattern in
    // `coordinator.rs`.
    ticker.tick().await;
    loop {
        ticker.tick().await;
        if let Err(e) = run_one_sweep(&repo, &sentinel, event_bus.as_ref()).await {
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
) -> Result<(), nebula_storage::credential::RepoError> {
    let stuck = repo.reclaim_stuck().await?;
    for reclaimed in stuck {
        if reclaimed.sentinel != SentinelState::RefreshInFlight {
            // Normal-path expiry — claim was reclaimed, nothing more to
            // do (no mid-refresh crash to record).
            continue;
        }
        let decision = match sentinel
            .on_sentinel_detected(
                &reclaimed.credential_id,
                &reclaimed.previous_holder,
                reclaimed.previous_generation,
            )
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
                    "sentinel recoverable — credential refresh will retry"
                );
            },
            SentinelDecision::EscalateToReauth {
                event_count,
                window_secs,
            } => {
                tracing::warn!(
                    cred = %reclaimed.credential_id,
                    event_count,
                    window_secs,
                    "sentinel threshold exceeded — escalating to ReauthRequired"
                );
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

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
        time::Duration,
    };

    use nebula_core::CredentialId;
    use nebula_eventbus::EventBus;
    use nebula_storage::credential::{
        ClaimAttempt, InMemoryRefreshClaimRepo, RefreshClaimRepo, ReplicaId,
    };

    use super::*;
    use crate::credential::refresh::sentinel::{SentinelThresholdConfig, SentinelTrigger};

    /// Set up an expired RefreshInFlight claim so the next reclaim_stuck
    /// returns it as a sentinel event.
    async fn seed_stuck_inflight_claim(repo: &Arc<dyn RefreshClaimRepo>, cid: CredentialId) {
        let claim = match repo
            .try_claim(&cid, &ReplicaId::new("crashed"), Duration::from_millis(20))
            .await
            .expect("try_claim ok")
        {
            ClaimAttempt::Acquired(c) => c,
            ClaimAttempt::Contended { .. } => panic!("setup must always acquire"),
        };
        repo.mark_sentinel(&claim.token)
            .await
            .expect("mark sentinel");
        // Wait past the TTL.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn run_one_sweep_below_threshold_does_not_publish_reauth() {
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let sentinel = Arc::new(SentinelTrigger::new(
            Arc::clone(&repo),
            SentinelThresholdConfig::default(),
        ));
        let bus = Arc::new(EventBus::<CredentialEvent>::new(16));
        let mut subscriber = bus.subscribe();

        // Seed one stuck RefreshInFlight claim. With the default
        // threshold of 3, one event must NOT trigger ReauthRequired.
        let cid = CredentialId::new();
        seed_stuck_inflight_claim(&repo, cid).await;

        run_one_sweep(&repo, &sentinel, Some(&bus))
            .await
            .expect("sweep ok");

        // Subscriber should observe no events (we published none).
        let recv = tokio::time::timeout(Duration::from_millis(50), subscriber.recv()).await;
        assert!(
            recv.is_err(),
            "below-threshold sentinel must NOT publish ReauthRequired; got {recv:?}"
        );
    }

    #[tokio::test]
    async fn run_one_sweep_at_threshold_publishes_reauth_with_sentinel_reason() {
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let sentinel = Arc::new(SentinelTrigger::new(
            Arc::clone(&repo),
            SentinelThresholdConfig::default(),
        ));
        let bus = Arc::new(EventBus::<CredentialEvent>::new(16));
        let mut subscriber = bus.subscribe();

        let cid = CredentialId::new();
        // Three sweeps, each seeded with a stuck RefreshInFlight claim
        // for the same credential. The third call to
        // `on_sentinel_detected` reaches threshold (default 3-in-1h)
        // and the sweep must publish CredentialEvent::ReauthRequired.
        for _ in 0..3 {
            seed_stuck_inflight_claim(&repo, cid).await;
            run_one_sweep(&repo, &sentinel, Some(&bus))
                .await
                .expect("sweep ok");
        }

        // Drain the subscriber: there must be exactly one
        // ReauthRequired event published on the third sweep.
        let mut reauth_count = 0u32;
        let mut last_reason: Option<ReauthReason> = None;
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), subscriber.recv()).await
        {
            if let CredentialEvent::ReauthRequired {
                credential_id,
                reason,
            } = event
            {
                assert_eq!(credential_id, cid);
                reauth_count += 1;
                last_reason = Some(reason);
            }
        }
        assert_eq!(
            reauth_count, 1,
            "exactly one ReauthRequired event at threshold"
        );
        assert!(
            matches!(
                last_reason,
                Some(ReauthReason::SentinelRepeated {
                    event_count: 3,
                    window_secs: 3600,
                })
            ),
            "reason must be SentinelRepeated; got {last_reason:?}"
        );
    }

    #[tokio::test]
    async fn run_one_sweep_two_in_window_one_outside_does_not_escalate() {
        // Custom config: threshold 2 with a tight 100ms window so we
        // can stage "1 outside the window, 2 inside" deterministically.
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let sentinel = Arc::new(SentinelTrigger::new(
            Arc::clone(&repo),
            SentinelThresholdConfig {
                threshold: 2,
                window: Duration::from_millis(100),
            },
        ));
        let bus = Arc::new(EventBus::<CredentialEvent>::new(16));
        let mut subscriber = bus.subscribe();

        let cid = CredentialId::new();

        // Event #1.
        seed_stuck_inflight_claim(&repo, cid).await;
        run_one_sweep(&repo, &sentinel, Some(&bus))
            .await
            .expect("sweep 1 ok");

        // Wait past the rolling window so event #1 ages out.
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Event #2 — first inside the new window.
        seed_stuck_inflight_claim(&repo, cid).await;
        run_one_sweep(&repo, &sentinel, Some(&bus))
            .await
            .expect("sweep 2 ok");

        // Drain the subscriber. With 1 event aged-out and 1 event
        // inside (count=1, threshold=2), no ReauthRequired must have
        // been published.
        let mut reauth_count = 0u32;
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), subscriber.recv()).await
        {
            if let CredentialEvent::ReauthRequired { .. } = event {
                reauth_count += 1;
            }
        }
        assert_eq!(
            reauth_count, 0,
            "events outside the rolling window must not push us over the threshold"
        );
    }

    #[tokio::test]
    async fn spawn_emits_reauth_after_threshold_at_cadence() {
        // Black-box smoke: spawn the actual task with a 30ms cadence,
        // seed three stuck rows over time, and observe the
        // ReauthRequired emission on the bus.
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let sentinel = Arc::new(SentinelTrigger::new(
            Arc::clone(&repo),
            SentinelThresholdConfig::default(),
        ));
        let bus = Arc::new(EventBus::<CredentialEvent>::new(16));
        let mut subscriber = bus.subscribe();

        let _handle = ReclaimSweepHandle::spawn(
            Arc::clone(&repo),
            Arc::clone(&sentinel),
            Duration::from_millis(30),
            Some(Arc::clone(&bus)),
        );

        let cid = CredentialId::new();
        // Seed three stuck claims with brief gaps to let the sweep
        // process each in turn.
        let observed = Arc::new(AtomicU32::new(0));
        let observed_clone = Arc::clone(&observed);
        let cid_for_listener = cid;
        let listener = tokio::spawn(async move {
            while let Some(event) = subscriber.recv().await {
                if matches!(
                    event,
                    CredentialEvent::ReauthRequired { credential_id, .. }
                    if credential_id == cid_for_listener
                ) {
                    observed_clone.fetch_add(1, Ordering::SeqCst);
                    break;
                }
            }
        });

        for _ in 0..3 {
            seed_stuck_inflight_claim(&repo, cid).await;
            // Give the sweep loop one cadence to pick this up before
            // we seed the next.
            tokio::time::sleep(Duration::from_millis(80)).await;
        }

        // Wait up to 1s for the listener to observe the event.
        let _ = tokio::time::timeout(Duration::from_secs(1), listener).await;
        assert_eq!(
            observed.load(Ordering::SeqCst),
            1,
            "spawned reclaim sweep must publish exactly one ReauthRequired at threshold"
        );
    }
}
