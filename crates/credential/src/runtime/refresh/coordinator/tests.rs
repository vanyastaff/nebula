use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use crate::RefreshNotAppliedContext;
use chrono::Utc;
use nebula_storage_port::store::{
    ExpiredClaim, ReauthEscalation, RefreshClaimError as RepoError, RefreshClaimReclaimer,
    SentinelEscalationPolicy,
};
use nebula_storage_port::{CredentialOwner, CredentialSelector};
use tokio::sync::Notify;

use super::*;

const HEARTBEAT_OK: u8 = 0;
const HEARTBEAT_LOST: u8 = 1;

fn test_selector(credential_id: CredentialId) -> CredentialSelector {
    CredentialSelector::new(CredentialOwner::from_canonical("test-owner"), credential_id)
}

struct ScriptedClaimRepo {
    active: AtomicBool,
    try_claim_count: AtomicUsize,
    release_count: AtomicUsize,
    heartbeat_count: AtomicUsize,
    heartbeat_mode: AtomicU8,
    block_try_claim: AtomicBool,
    block_sentinel: AtomicBool,
    block_release: AtomicBool,
    try_claim_entered: Notify,
    try_claim_continue: Notify,
    sentinel_entered: Notify,
    sentinel_continue: Notify,
    release_entered: Notify,
    release_continue: Notify,
    release_completed: Notify,
}

struct PoisonClaimRepo {
    credential_id: CredentialId,
    try_claim_count: AtomicUsize,
    release_count: AtomicUsize,
    reclaim_count: AtomicUsize,
    evidence_count: AtomicUsize,
    evidence_recorded: AtomicBool,
}

impl PoisonClaimRepo {
    fn new(credential_id: CredentialId) -> Self {
        Self {
            credential_id,
            try_claim_count: AtomicUsize::new(0),
            release_count: AtomicUsize::new(0),
            reclaim_count: AtomicUsize::new(0),
            evidence_count: AtomicUsize::new(0),
            evidence_recorded: AtomicBool::new(false),
        }
    }
}

impl ScriptedClaimRepo {
    fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            try_claim_count: AtomicUsize::new(0),
            release_count: AtomicUsize::new(0),
            heartbeat_count: AtomicUsize::new(0),
            heartbeat_mode: AtomicU8::new(HEARTBEAT_OK),
            block_try_claim: AtomicBool::new(false),
            block_sentinel: AtomicBool::new(false),
            block_release: AtomicBool::new(false),
            try_claim_entered: Notify::new(),
            try_claim_continue: Notify::new(),
            sentinel_entered: Notify::new(),
            sentinel_continue: Notify::new(),
            release_entered: Notify::new(),
            release_continue: Notify::new(),
            release_completed: Notify::new(),
        }
    }

    async fn wait_for_release(&self) {
        if self.release_count.load(Ordering::SeqCst) == 0 {
            self.release_completed.notified().await;
        }
    }

    async fn wait_for_try_claim_count(&self, target: usize) {
        while self.try_claim_count.load(Ordering::SeqCst) < target {
            self.try_claim_entered.notified().await;
        }
    }
}

#[async_trait::async_trait]
impl RefreshClaimRepo for ScriptedClaimRepo {
    async fn try_claim(
        &self,
        selector: &CredentialSelector,
        _holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        self.try_claim_count.fetch_add(1, Ordering::SeqCst);
        self.try_claim_entered.notify_one();
        if self.block_try_claim.load(Ordering::SeqCst) {
            self.try_claim_continue.notified().await;
        }
        let now = Utc::now();
        let ttl = chrono::Duration::from_std(ttl).map_err(|_| RepoError::InvalidState)?;
        if self.active.swap(true, Ordering::SeqCst) {
            return Ok(ClaimAttempt::Contended {
                existing_expires_at: now + ttl,
            });
        }
        Ok(ClaimAttempt::Acquired(RefreshClaim {
            selector: selector.clone(),
            token: ClaimToken {
                selector: selector.clone(),
                claim_id: "00000000-0000-0000-0000-000000000001"
                    .parse()
                    .expect("test claim id is a UUID"),
                generation: 1,
            },
            acquired_at: now,
            expires_at: now + ttl,
        }))
    }

    async fn heartbeat(&self, _token: &ClaimToken, _ttl: Duration) -> Result<(), HeartbeatError> {
        self.heartbeat_count.fetch_add(1, Ordering::SeqCst);
        if self.heartbeat_mode.load(Ordering::SeqCst) == HEARTBEAT_LOST {
            Err(HeartbeatError::ClaimLost)
        } else {
            Ok(())
        }
    }

    async fn release(&self, _token: ClaimToken) -> Result<(), RepoError> {
        self.release_entered.notify_one();
        if self.block_release.load(Ordering::SeqCst) {
            self.release_continue.notified().await;
        }
        self.active.store(false, Ordering::SeqCst);
        self.release_count.fetch_add(1, Ordering::SeqCst);
        self.release_completed.notify_one();
        Ok(())
    }

    async fn mark_sentinel(&self, _token: &ClaimToken) -> Result<(), RepoError> {
        self.sentinel_entered.notify_one();
        if self.block_sentinel.load(Ordering::SeqCst) {
            self.sentinel_continue.notified().await;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl RefreshClaimRepo for PoisonClaimRepo {
    async fn try_claim(
        &self,
        selector: &CredentialSelector,
        _holder: &ReplicaId,
        _ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        assert_eq!(selector.credential_id(), self.credential_id);
        self.try_claim_count.fetch_add(1, Ordering::SeqCst);
        Ok(ClaimAttempt::OutcomeUnknown {
            expired_at: Utc::now() - chrono::Duration::seconds(1),
        })
    }

    async fn heartbeat(&self, _token: &ClaimToken, _ttl: Duration) -> Result<(), HeartbeatError> {
        Err(HeartbeatError::ClaimLost)
    }

    async fn release(&self, _token: ClaimToken) -> Result<(), RepoError> {
        self.release_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn mark_sentinel(&self, _token: &ClaimToken) -> Result<(), RepoError> {
        Err(RepoError::InvalidState)
    }
}

#[async_trait::async_trait]
impl RefreshClaimReclaimer for PoisonClaimRepo {
    async fn reclaim_stuck(
        &self,
        _policy: SentinelEscalationPolicy,
    ) -> Result<Vec<ExpiredClaim>, RepoError> {
        self.reclaim_count.fetch_add(1, Ordering::SeqCst);
        let newly_accounted = !self.evidence_recorded.swap(true, Ordering::SeqCst);
        let result = if newly_accounted {
            self.evidence_count.fetch_add(1, Ordering::SeqCst);
            vec![ExpiredClaim::OutcomeUnknownAccounted {
                selector: test_selector(self.credential_id),
                previous_holder: ReplicaId::new("crashed-provider-holder"),
                previous_generation: 7,
                event_count: u32::try_from(self.evidence_count.load(Ordering::SeqCst))
                    .unwrap_or(u32::MAX),
                escalation: ReauthEscalation::ReauthRequired {
                    changed: true,
                    version: nebula_storage_port::CredentialVersion::MIN,
                    material_epoch: nebula_storage_port::CredentialMaterialEpoch::MIN,
                },
            }]
        } else {
            Vec::new()
        };
        Ok(result)
    }
}

fn coordinator(
    repo: Arc<ScriptedClaimRepo>,
    config: RefreshCoordConfig,
) -> Arc<RefreshCoordinator> {
    Arc::new(
        RefreshCoordinator::new_with(repo, ReplicaId::new("test-replica"), config)
            .expect("test coordinator config is valid"),
    )
}

fn paused_config() -> RefreshCoordConfig {
    RefreshCoordConfig {
        claim_ttl: Duration::from_millis(60),
        heartbeat_interval: Duration::from_millis(10),
        refresh_timeout: Duration::from_millis(30),
        reclaim_sweep_interval: Duration::from_millis(60),
        sentinel_threshold: 3,
        sentinel_window: Duration::from_mins(1),
    }
}

async fn advance_until_heartbeat(repo: &ScriptedClaimRepo, interval: Duration) {
    for _ in 0..3 {
        tokio::time::advance(interval).await;
        tokio::task::yield_now().await;
        if repo.heartbeat_count.load(Ordering::SeqCst) > 0 {
            return;
        }
    }
    panic!("heartbeat task did not reach the scripted repository");
}

async fn wait_until_l1_empty(coordinator: &RefreshCoordinator) {
    for _ in 0..8 {
        if coordinator.l1.in_flight_count() == 0 {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("L1 completion was not released after exact disposition");
}

#[test]
fn zero_durations_are_rejected_before_provider_work() {
    for (field, config) in [
        (
            "claim_ttl",
            RefreshCoordConfig {
                claim_ttl: Duration::ZERO,
                ..RefreshCoordConfig::default()
            },
        ),
        (
            "heartbeat_interval",
            RefreshCoordConfig {
                heartbeat_interval: Duration::ZERO,
                ..RefreshCoordConfig::default()
            },
        ),
        (
            "refresh_timeout",
            RefreshCoordConfig {
                refresh_timeout: Duration::ZERO,
                ..RefreshCoordConfig::default()
            },
        ),
        (
            "reclaim_sweep_interval",
            RefreshCoordConfig {
                reclaim_sweep_interval: Duration::ZERO,
                ..RefreshCoordConfig::default()
            },
        ),
        (
            "sentinel_window",
            RefreshCoordConfig {
                sentinel_window: Duration::ZERO,
                ..RefreshCoordConfig::default()
            },
        ),
    ] {
        let error = RefreshCoordinator::new_with(
            Arc::new(ScriptedClaimRepo::new()),
            ReplicaId::new("zero-config-test"),
            config,
        )
        .expect_err("zero duration must fail at construction");
        assert!(matches!(
            error,
            ConfigError::ZeroDuration {
                field: actual
            } if actual == field
        ));
    }
}

#[test]
fn zero_sentinel_threshold_is_rejected_before_provider_work() {
    let error = RefreshCoordinator::new_with(
        Arc::new(ScriptedClaimRepo::new()),
        ReplicaId::new("test-replica"),
        RefreshCoordConfig {
            sentinel_threshold: 0,
            ..RefreshCoordConfig::default()
        },
    )
    .expect_err("zero sentinel threshold must fail construction");

    assert!(matches!(error, ConfigError::ZeroSentinelThreshold));
}

#[tokio::test]
async fn repeated_poison_denials_leave_threshold_observation_to_periodic_owner() {
    use nebula_eventbus::EventBus;

    use crate::{CredentialEvent, contract::resolve::ReauthReason};

    use super::super::reclaim::run_one_sweep;

    let credential_id = CredentialId::new();
    let repo = Arc::new(PoisonClaimRepo::new(credential_id));
    let repo_port: Arc<dyn RefreshClaimRepo> = repo.clone();
    let coordinator = Arc::new(
        RefreshCoordinator::new_with(
            Arc::clone(&repo_port),
            ReplicaId::new("test-replica"),
            RefreshCoordConfig {
                sentinel_threshold: 1,
                ..RefreshCoordConfig::default()
            },
        )
        .expect("test coordinator config is valid"),
    );
    let provider_calls = Arc::new(AtomicUsize::new(0));

    for _ in 0..2 {
        let calls = Arc::clone(&provider_calls);
        let outcome = coordinator
            .refresh_coalesced(
                &test_selector(credential_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(())
                },
            )
            .await;
        assert!(matches!(outcome, Err(RefreshError::CriticalOutcomePending)));
    }

    assert_eq!(
        repo.reclaim_count.load(Ordering::SeqCst),
        0,
        "request paths must not consume periodic accounting work"
    );
    assert_eq!(repo.evidence_count.load(Ordering::SeqCst), 0);

    let event_bus = Arc::new(EventBus::new(8));
    let mut events = event_bus.subscribe();
    let policy = SentinelEscalationPolicy::new(
        coordinator.config.sentinel_threshold,
        coordinator.config.sentinel_window,
    )
    .expect("coordinator already validated sentinel policy");
    run_one_sweep(
        repo.as_ref(),
        policy,
        Some(&event_bus),
        coordinator.metrics(),
        None,
    )
    .await
    .expect("periodic owner should account poison");

    let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("threshold observation should be emitted")
        .expect("event bus should remain open");
    assert!(matches!(
        event,
        CredentialEvent::ReauthRequired {
            credential_id: observed,
            reason: ReauthReason::SentinelRepeated {
                event_count: 1,
                ..
            },
        } if observed == credential_id
    ));

    run_one_sweep(
        repo.as_ref(),
        policy,
        Some(&event_bus),
        coordinator.metrics(),
        None,
    )
    .await
    .expect("repeated periodic sweep should be idempotent");

    assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
    assert_eq!(repo.try_claim_count.load(Ordering::SeqCst), 2);
    assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
    assert_eq!(repo.reclaim_count.load(Ordering::SeqCst), 2);
    assert_eq!(
        repo.evidence_count.load(Ordering::SeqCst),
        1,
        "periodic reclaim must record one durable event per poisoned generation"
    );
    assert_eq!(coordinator.metrics.claims_outcome_unknown.get(), 2);
    assert_eq!(
        coordinator.metrics.reclaim_outcome_unknown_accounted.get(),
        1
    );
    assert_eq!(coordinator.metrics.reclaim_reclaimed.get(), 0);
    assert_eq!(coordinator.metrics.reclaim_no_work.get(), 1);
    assert!(
        events.try_recv().is_none(),
        "already-accounted poison must not emit a duplicate threshold observation"
    );
}

#[tokio::test]
async fn dropped_reauth_observation_cannot_undo_durable_escalation() {
    use nebula_eventbus::EventBus;

    use crate::CredentialEvent;

    use super::super::reclaim::run_one_sweep;

    let credential_id = CredentialId::new();
    let repo = PoisonClaimRepo::new(credential_id);
    let event_bus = Arc::new(EventBus::<CredentialEvent>::new(1));
    let metrics = RefreshCoordMetrics::for_tests().expect("test metrics registry is valid");
    let policy = SentinelEscalationPolicy::new(1, Duration::from_mins(1))
        .expect("test sentinel policy is valid");

    run_one_sweep(&repo, policy, Some(&event_bus), &metrics, None)
        .await
        .expect("best-effort observation loss must not fail an already-committed escalation");

    assert_eq!(
        repo.evidence_count.load(Ordering::SeqCst),
        1,
        "the reclaimer's durable escalation result must remain authoritative"
    );
    assert_eq!(event_bus.stats().dropped_count, 1);
    assert_eq!(metrics.sentinel_reauth_triggered.get(), 1);

    run_one_sweep(&repo, policy, Some(&event_bus), &metrics, None)
        .await
        .expect("retry after observation loss must remain idempotent");

    assert_eq!(repo.evidence_count.load(Ordering::SeqCst), 1);
    assert_eq!(event_bus.stats().dropped_count, 1);
}

#[tokio::test]
async fn caller_drop_cannot_release_or_duplicate_owned_provider_commit() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
    let credential_id = CredentialId::new();
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let provider_entered = Arc::new(Notify::new());
    let commit_continue = Arc::new(Notify::new());

    let winner_coordinator = Arc::clone(&coordinator);
    let winner_provider_calls = Arc::clone(&provider_calls);
    let winner_writes = Arc::clone(&writes);
    let winner_provider_entered = Arc::clone(&provider_entered);
    let winner_commit_continue = Arc::clone(&commit_continue);
    let winner = tokio::spawn(async move {
        winner_coordinator
            .refresh_coalesced(
                &test_selector(credential_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    winner_provider_calls.fetch_add(1, Ordering::SeqCst);
                    winner_provider_entered.notify_one();
                    winner_commit_continue.notified().await;
                    winner_writes.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(7_u8)
                },
            )
            .await
    });

    provider_entered.notified().await;
    winner.abort();
    assert!(winner.await.is_err(), "the outer waiter was cancelled");
    assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
    assert!(repo.active.load(Ordering::SeqCst));
    assert_eq!(coordinator.l1.in_flight_count(), 1);
    assert_eq!(writes.load(Ordering::SeqCst), 0);

    let duplicate_calls = Arc::new(AtomicUsize::new(0));
    let waiter_coordinator = Arc::clone(&coordinator);
    let waiter_duplicate_calls = Arc::clone(&duplicate_calls);
    let waiter_writes = Arc::clone(&writes);
    let waiter = tokio::spawn(async move {
        waiter_coordinator
            .refresh_coalesced(
                &test_selector(credential_id),
                move |_| {
                    let writes = Arc::clone(&waiter_writes);
                    async move {
                        Ok(if writes.load(Ordering::SeqCst) == 0 {
                            RefreshRecheck::Needed
                        } else {
                            RefreshRecheck::Satisfied
                        })
                    }
                },
                move || async move {
                    waiter_duplicate_calls.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(9_u8)
                },
            )
            .await
    });
    tokio::task::yield_now().await;
    assert!(
        !waiter.is_finished(),
        "L1 waiter woke before commit disposition"
    );

    commit_continue.notify_one();
    repo.wait_for_release().await;
    let waiter_result = waiter.await.expect("waiter task joins");
    assert!(matches!(
        waiter_result,
        Err(RefreshError::CoalescedByOtherReplica)
    ));
    wait_until_l1_empty(&coordinator).await;
    assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert_eq!(duplicate_calls.load(Ordering::SeqCst), 0);
    assert_eq!(repo.release_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn state_advanced_completion_can_elect_one_winner_for_a_newer_epoch() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
    let credential_id = CredentialId::new();
    let first_entered = Arc::new(Notify::new());
    let first_continue = Arc::new(Notify::new());

    let winner_coordinator = Arc::clone(&coordinator);
    let winner_entered = Arc::clone(&first_entered);
    let winner_continue = Arc::clone(&first_continue);
    let winner_id = credential_id;
    let winner = tokio::spawn(async move {
        winner_coordinator
            .refresh_coalesced(
                &test_selector(winner_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    winner_entered.notify_one();
                    winner_continue.notified().await;
                    RefreshDisposition::state_advanced(1_u8)
                },
            )
            .await
    });
    first_entered.notified().await;

    let recheck_entered = Arc::new(Notify::new());
    let recheck_continue = Arc::new(Notify::new());
    let recheck_calls = Arc::new(AtomicUsize::new(0));
    let second_provider_calls = Arc::new(AtomicUsize::new(0));
    let waiter_coordinator = Arc::clone(&coordinator);
    let waiter_recheck_entered = Arc::clone(&recheck_entered);
    let waiter_recheck_continue = Arc::clone(&recheck_continue);
    let waiter_recheck_calls = Arc::clone(&recheck_calls);
    let waiter_provider_calls = Arc::clone(&second_provider_calls);
    let waiter_id = credential_id;
    let waiter = tokio::spawn(async move {
        waiter_coordinator
            .refresh_coalesced(
                &test_selector(waiter_id),
                move |_| {
                    let entered = Arc::clone(&waiter_recheck_entered);
                    let continue_recheck = Arc::clone(&waiter_recheck_continue);
                    let call = waiter_recheck_calls.fetch_add(1, Ordering::SeqCst);
                    async move {
                        if call == 0 {
                            entered.notify_one();
                            continue_recheck.notified().await;
                        }
                        // The first winner advanced its epoch, but newer
                        // authoritative work arrived before this waiter
                        // rechecked state. The post-acquire recheck must
                        // observe that work as still needed.
                        Ok(RefreshRecheck::Needed)
                    }
                },
                move || async move {
                    waiter_provider_calls.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(2_u8)
                },
            )
            .await
    });

    while coordinator
        .l1
        .waiter_count_for_test(&credential_id.to_string())
        == 0
    {
        tokio::task::yield_now().await;
    }
    first_continue.notify_one();
    recheck_entered.notified().await;
    repo.wait_for_release().await;
    recheck_continue.notify_one();

    assert_eq!(
        winner
            .await
            .expect("first winner task must join")
            .expect("first epoch must complete"),
        1
    );
    assert_eq!(
        waiter
            .await
            .expect("new-epoch waiter task must join")
            .expect("newer epoch must elect a winner"),
        2
    );
    assert_eq!(second_provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(recheck_calls.load(Ordering::SeqCst), 2);
    assert_eq!(repo.try_claim_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn exact_no_progress_completion_does_not_turn_waiters_into_a_retry_herd() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
    let credential_id = CredentialId::new();
    let first_entered = Arc::new(Notify::new());
    let first_continue = Arc::new(Notify::new());

    let winner_coordinator = Arc::clone(&coordinator);
    let winner_entered = Arc::clone(&first_entered);
    let winner_continue = Arc::clone(&first_continue);
    let winner_id = credential_id;
    let winner = tokio::spawn(async move {
        winner_coordinator
            .refresh_coalesced(
                &test_selector(winner_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    winner_entered.notify_one();
                    winner_continue.notified().await;
                    RefreshDisposition::no_state_change(1_u8)
                },
            )
            .await
    });
    first_entered.notified().await;

    let duplicate_provider_calls = Arc::new(AtomicUsize::new(0));
    let waiter_coordinator = Arc::clone(&coordinator);
    let waiter_provider_calls = Arc::clone(&duplicate_provider_calls);
    let waiter_id = credential_id;
    let waiter = tokio::spawn(async move {
        waiter_coordinator
            .refresh_coalesced(
                &test_selector(waiter_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    waiter_provider_calls.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(2_u8)
                },
            )
            .await
    });

    while coordinator
        .l1
        .waiter_count_for_test(&credential_id.to_string())
        == 0
    {
        tokio::task::yield_now().await;
    }
    first_continue.notify_one();

    assert_eq!(
        winner
            .await
            .expect("first winner task must join")
            .expect("exact first outcome must be returned"),
        1
    );
    assert!(matches!(
        waiter.await.expect("waiter task must join"),
        Err(RefreshError::PriorAttemptNoProgress)
    ));
    assert_eq!(duplicate_provider_calls.load(Ordering::SeqCst), 0);
    assert_eq!(repo.try_claim_count.load(Ordering::SeqCst), 1);
    repo.wait_for_release().await;
}

#[tokio::test]
async fn exact_local_finalization_failure_requires_waiter_reconciliation() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
    let credential_id = CredentialId::new();
    let first_entered = Arc::new(Notify::new());
    let first_continue = Arc::new(Notify::new());

    let winner_coordinator = Arc::clone(&coordinator);
    let winner_entered = Arc::clone(&first_entered);
    let winner_continue = Arc::clone(&first_continue);
    let winner_id = credential_id;
    let winner = tokio::spawn(async move {
        winner_coordinator
            .refresh_coalesced(
                &test_selector(winner_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    winner_entered.notify_one();
                    winner_continue.notified().await;
                    // Exact local finalization failures use RetryUnsafe:
                    // the winner keeps its concrete error while L1 remains
                    // deliberately payload-free.
                    RefreshDisposition::retry_unsafe(1_u8)
                },
            )
            .await
    });
    first_entered.notified().await;

    let duplicate_provider_calls = Arc::new(AtomicUsize::new(0));
    let waiter_coordinator = Arc::clone(&coordinator);
    let waiter_provider_calls = Arc::clone(&duplicate_provider_calls);
    let waiter_id = credential_id;
    let waiter = tokio::spawn(async move {
        waiter_coordinator
            .refresh_coalesced(
                &test_selector(waiter_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    waiter_provider_calls.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(2_u8)
                },
            )
            .await
    });

    while coordinator
        .l1
        .waiter_count_for_test(&credential_id.to_string())
        == 0
    {
        tokio::task::yield_now().await;
    }
    first_continue.notify_one();

    assert_eq!(
        winner
            .await
            .expect("first winner task must join")
            .expect("exact unsafe outcome must reach its owner"),
        1
    );
    assert!(matches!(
        waiter.await.expect("waiter task must join"),
        Err(RefreshError::ReconciliationRequired)
    ));
    assert_eq!(duplicate_provider_calls.load(Ordering::SeqCst), 0);
    assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
    assert!(
        repo.active.load(Ordering::SeqCst),
        "retry-unsafe completion must retain the sentinel claim"
    );
}

#[tokio::test]
async fn outcome_unknown_completion_keeps_waiters_unknown_and_fail_closed() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
    let credential_id = CredentialId::new();
    let first_entered = Arc::new(Notify::new());
    let first_continue = Arc::new(Notify::new());

    let winner_coordinator = Arc::clone(&coordinator);
    let winner_entered = Arc::clone(&first_entered);
    let winner_continue = Arc::clone(&first_continue);
    let winner_id = credential_id;
    let winner = tokio::spawn(async move {
        winner_coordinator
            .refresh_coalesced(
                &test_selector(winner_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    winner_entered.notify_one();
                    winner_continue.notified().await;
                    RefreshDisposition::outcome_unknown(1_u8)
                },
            )
            .await
    });
    first_entered.notified().await;

    let duplicate_provider_calls = Arc::new(AtomicUsize::new(0));
    let waiter_coordinator = Arc::clone(&coordinator);
    let waiter_provider_calls = Arc::clone(&duplicate_provider_calls);
    let waiter_id = credential_id;
    let waiter = tokio::spawn(async move {
        waiter_coordinator
            .refresh_coalesced(
                &test_selector(waiter_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    waiter_provider_calls.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(2_u8)
                },
            )
            .await
    });

    while coordinator
        .l1
        .waiter_count_for_test(&credential_id.to_string())
        == 0
    {
        tokio::task::yield_now().await;
    }
    first_continue.notify_one();

    assert_eq!(
        winner
            .await
            .expect("first winner task must join")
            .expect("unknown outcome value must reach its owner"),
        1
    );
    assert!(matches!(
        waiter.await.expect("waiter task must join"),
        Err(RefreshError::CriticalOutcomePending)
    ));
    assert_eq!(duplicate_provider_calls.load(Ordering::SeqCst), 0);
    assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
    assert!(
        repo.active.load(Ordering::SeqCst),
        "outcome-unknown completion must retain the sentinel claim"
    );
}

#[tokio::test(start_paused = true)]
async fn released_contended_claim_is_polled_well_before_its_ttl() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    repo.active.store(true, Ordering::SeqCst);
    let config = RefreshCoordConfig {
        claim_ttl: Duration::from_millis(300),
        heartbeat_interval: Duration::from_millis(25),
        refresh_timeout: Duration::from_millis(100),
        reclaim_sweep_interval: Duration::from_millis(300),
        sentinel_threshold: 3,
        sentinel_window: Duration::from_mins(1),
    };
    let coordinator = coordinator(Arc::clone(&repo), config);
    let credential_id = CredentialId::new();
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let task_provider_calls = Arc::clone(&provider_calls);
    let task_coordinator = Arc::clone(&coordinator);

    let task = tokio::spawn(async move {
        task_coordinator
            .refresh_coalesced(
                &test_selector(credential_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    task_provider_calls.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(())
                },
            )
            .await
    });

    repo.wait_for_try_claim_count(1).await;
    repo.active.store(false, Ordering::SeqCst);
    tokio::time::advance(Duration::from_millis(40)).await;
    tokio::task::yield_now().await;

    task.await
        .expect("coordinator task must join")
        .expect("released claim must be acquired on the next adaptive poll");
    assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        repo.try_claim_count.load(Ordering::SeqCst),
        2,
        "claim release must be observed without sleeping to the 300 ms TTL"
    );
}

#[tokio::test(start_paused = true)]
async fn post_contention_recheck_failure_denies_provider_dispatch() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    repo.active.store(true, Ordering::SeqCst);
    let coordinator = coordinator(Arc::clone(&repo), paused_config());
    let credential_id = CredentialId::new();
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let task_provider_calls = Arc::clone(&provider_calls);
    let task_coordinator = Arc::clone(&coordinator);

    let task = tokio::spawn(async move {
        task_coordinator
            .refresh_coalesced(
                &test_selector(credential_id),
                |_| async { Err(RefreshRecheckError::Unavailable) },
                move || async move {
                    task_provider_calls.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(())
                },
            )
            .await
    });

    repo.wait_for_try_claim_count(1).await;
    tokio::time::advance(Duration::from_millis(200)).await;
    let result = task.await.expect("coordinator task joins");

    assert!(matches!(
        result,
        Err(RefreshError::StateRecheck(RefreshRecheckError::Unavailable))
    ));
    assert_eq!(
        provider_calls.load(Ordering::SeqCst),
        0,
        "an unavailable authoritative recheck must never authorize provider egress"
    );
    assert_eq!(repo.try_claim_count.load(Ordering::SeqCst), 1);
    assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
    assert_eq!(coordinator.l1.in_flight_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn l1_waiter_timeout_is_bounded_without_false_coalesced_success() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    let config = paused_config();
    let coordinator = coordinator(Arc::clone(&repo), config.clone());
    let credential_id = CredentialId::new();
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let duplicate_calls = Arc::new(AtomicUsize::new(0));
    let provider_entered = Arc::new(Notify::new());
    let provider_continue = Arc::new(Notify::new());

    let winner_coordinator = Arc::clone(&coordinator);
    let winner_calls = Arc::clone(&provider_calls);
    let winner_entered = Arc::clone(&provider_entered);
    let winner_continue = Arc::clone(&provider_continue);
    let winner_id = credential_id;
    let winner = tokio::spawn(async move {
        winner_coordinator
            .refresh_coalesced(
                &test_selector(winner_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    winner_calls.fetch_add(1, Ordering::SeqCst);
                    winner_entered.notify_one();
                    winner_continue.notified().await;
                    RefreshDisposition::state_advanced(())
                },
            )
            .await
    });

    provider_entered.notified().await;
    let waiter_coordinator = Arc::clone(&coordinator);
    let waiter_duplicate_calls = Arc::clone(&duplicate_calls);
    let waiter_id = credential_id;
    let waiter = tokio::spawn(async move {
        waiter_coordinator
            .refresh_coalesced(
                &test_selector(waiter_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    waiter_duplicate_calls.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(())
                },
            )
            .await
    });

    for _ in 0..8 {
        if coordinator
            .l1
            .waiter_count_for_test(&credential_id.to_string())
            == 1
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        coordinator
            .l1
            .waiter_count_for_test(&credential_id.to_string()),
        1,
        "the second caller must register as an L1 waiter"
    );

    tokio::time::advance(config.refresh_timeout).await;
    let winner_outcome = winner.await.expect("winner caller joins");
    let waiter_outcome = waiter.await.expect("L1 waiter joins");
    assert!(matches!(
        winner_outcome,
        Err(RefreshError::CriticalOutcomePending)
    ));
    assert!(matches!(
        waiter_outcome,
        Err(RefreshError::CriticalOutcomePending)
    ));
    assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(duplicate_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        coordinator
            .l1
            .waiter_count_for_test(&credential_id.to_string()),
        0,
        "timed-out waiters must not accumulate behind a stuck winner"
    );
    assert_eq!(
        coordinator.metrics.coalesced_l1.get(),
        0,
        "an unresolved waiter timeout is not a coalesced success"
    );
    assert_eq!(
        coordinator.l1.in_flight_count(),
        1,
        "the owned critical task must retain the L1 winner entry"
    );

    provider_continue.notify_one();
    repo.wait_for_release().await;
    wait_until_l1_empty(&coordinator).await;
}

#[tokio::test(start_paused = true)]
async fn caller_timeout_detaches_and_heartbeat_keeps_lease_past_original_ttl() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    let config = paused_config();
    let coordinator = coordinator(Arc::clone(&repo), config.clone());
    let credential_id = CredentialId::new();
    let provider_entered = Arc::new(Notify::new());
    let commit_continue = Arc::new(Notify::new());
    let writes = Arc::new(AtomicUsize::new(0));

    let task_coordinator = Arc::clone(&coordinator);
    let task_entered = Arc::clone(&provider_entered);
    let task_continue = Arc::clone(&commit_continue);
    let task_writes = Arc::clone(&writes);
    let waiter = tokio::spawn(async move {
        task_coordinator
            .refresh_coalesced(
                &test_selector(credential_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    task_entered.notify_one();
                    task_continue.notified().await;
                    task_writes.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(())
                },
            )
            .await
    });

    provider_entered.notified().await;
    tokio::time::advance(config.refresh_timeout).await;
    let outcome = waiter.await.expect("timeout waiter joins");
    assert!(matches!(outcome, Err(RefreshError::CriticalOutcomePending)));
    assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
    assert_eq!(coordinator.l1.in_flight_count(), 1);
    assert_eq!(writes.load(Ordering::SeqCst), 0);

    // The caller has already gone away, but the owned critical task still
    // holds and heartbeats L2. Advancing past the acquisition's original
    // TTL must not turn elapsed time into replay authority.
    tokio::time::advance(config.claim_ttl).await;
    tokio::task::yield_now().await;
    assert!(
        repo.heartbeat_count.load(Ordering::SeqCst) > 0,
        "the detached exact-outcome task must renew its lease past the original TTL"
    );
    assert!(repo.active.load(Ordering::SeqCst));

    commit_continue.notify_one();
    repo.wait_for_release().await;
    wait_until_l1_empty(&coordinator).await;
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert_eq!(repo.release_count.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn heartbeat_loss_after_provider_boundary_cannot_cancel_commit() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    let config = paused_config();
    let coordinator = coordinator(Arc::clone(&repo), config.clone());
    let credential_id = CredentialId::new();
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let provider_entered = Arc::new(Notify::new());
    let commit_continue = Arc::new(Notify::new());

    let task_coordinator = Arc::clone(&coordinator);
    let task_provider_calls = Arc::clone(&provider_calls);
    let task_writes = Arc::clone(&writes);
    let task_entered = Arc::clone(&provider_entered);
    let task_continue = Arc::clone(&commit_continue);
    let waiter = tokio::spawn(async move {
        task_coordinator
            .refresh_coalesced(
                &test_selector(credential_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    task_provider_calls.fetch_add(1, Ordering::SeqCst);
                    task_entered.notify_one();
                    task_continue.notified().await;
                    task_writes.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(13_u8)
                },
            )
            .await
    });

    provider_entered.notified().await;
    repo.heartbeat_mode.store(HEARTBEAT_LOST, Ordering::SeqCst);
    advance_until_heartbeat(&repo, config.heartbeat_interval).await;
    assert!(!waiter.is_finished());
    assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
    assert_eq!(writes.load(Ordering::SeqCst), 0);

    commit_continue.notify_one();
    assert_eq!(waiter.await.expect("waiter joins").expect("confirmed"), 13);
    repo.wait_for_release().await;
    assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert_eq!(repo.release_count.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn heartbeat_loss_before_provider_boundary_starts_no_provider_work() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    repo.block_sentinel.store(true, Ordering::SeqCst);
    repo.heartbeat_mode.store(HEARTBEAT_LOST, Ordering::SeqCst);
    let config = paused_config();
    let coordinator = coordinator(Arc::clone(&repo), config.clone());
    let credential_id = CredentialId::new();
    let provider_calls = Arc::new(AtomicUsize::new(0));

    let task_coordinator = Arc::clone(&coordinator);
    let task_provider_calls = Arc::clone(&provider_calls);
    let waiter = tokio::spawn(async move {
        task_coordinator
            .refresh_coalesced(
                &test_selector(credential_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    task_provider_calls.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(())
                },
            )
            .await
    });

    repo.sentinel_entered.notified().await;
    advance_until_heartbeat(&repo, config.heartbeat_interval).await;
    let outcome = waiter.await.expect("waiter joins");
    assert!(matches!(
        outcome,
        Err(RefreshError::ClaimLostBeforeProvider)
    ));
    repo.wait_for_release().await;
    assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
    assert_eq!(repo.release_count.load(Ordering::SeqCst), 1);
    assert!(!repo.active.load(Ordering::SeqCst));
}

#[tokio::test]
async fn post_acquire_recheck_closes_stale_open_preflight_window() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    repo.block_try_claim.store(true, Ordering::SeqCst);
    let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
    let credential_id = CredentialId::new();
    let gate_installed = Arc::new(AtomicBool::new(false));
    let provider_calls = Arc::new(AtomicUsize::new(0));

    let task = tokio::spawn({
        let coordinator = Arc::clone(&coordinator);
        let gate_installed = Arc::clone(&gate_installed);
        let provider_calls = Arc::clone(&provider_calls);
        async move {
            coordinator
                .refresh_coalesced(
                    &test_selector(credential_id),
                    move |_| {
                        let gate_installed = Arc::clone(&gate_installed);
                        async move {
                            if gate_installed.load(Ordering::SeqCst) {
                                let context = RefreshNotAppliedContext::from_spec(
                                    crate::RefreshNotAppliedPhase::BeforeDispatch,
                                    crate::RefreshFailureSpec::new(
                                        crate::RefreshErrorKind::ProtocolError,
                                        crate::RetryAdvice::Never,
                                    ),
                                );
                                Ok(RefreshRecheck::Suppressed(Box::new(context)))
                            } else {
                                Ok(RefreshRecheck::Needed)
                            }
                        }
                    },
                    move || async move {
                        provider_calls.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(())
                    },
                )
                .await
        }
    });

    repo.wait_for_try_claim_count(1).await;
    // Another replica installs the gate and releases its claim after this
    // caller's stale outer preflight but before immediate L2 acquisition
    // completes.
    gate_installed.store(true, Ordering::SeqCst);
    repo.try_claim_continue.notify_one();

    let outcome = task.await.expect("contender task joins");
    let Err(RefreshError::RetrySuppressed(context)) = outcome else {
        panic!("post-acquire recheck must return the typed durable block");
    };
    assert_eq!(context.retry(), crate::RetryAdvice::Never);
    assert_eq!(
        provider_calls.load(Ordering::SeqCst),
        0,
        "a stale Open observation must never cross the provider boundary"
    );
    repo.wait_for_release().await;
    assert!(!repo.active.load(Ordering::SeqCst));
}

#[tokio::test]
async fn l1_waiter_receives_typed_gate_written_by_winner() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
    let credential_id = CredentialId::new();
    let gate_installed = Arc::new(AtomicBool::new(false));
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let provider_entered = Arc::new(Notify::new());
    let provider_continue = Arc::new(Notify::new());

    let winner = tokio::spawn({
        let coordinator = Arc::clone(&coordinator);
        let gate_for_recheck = Arc::clone(&gate_installed);
        let gate_for_commit = Arc::clone(&gate_installed);
        let provider_calls = Arc::clone(&provider_calls);
        let provider_entered = Arc::clone(&provider_entered);
        let provider_continue = Arc::clone(&provider_continue);
        async move {
            coordinator
                .refresh_coalesced(
                    &test_selector(credential_id),
                    move |_| {
                        let gate = Arc::clone(&gate_for_recheck);
                        async move {
                            if gate.load(Ordering::SeqCst) {
                                Ok(RefreshRecheck::Suppressed(Box::new(
                                    RefreshNotAppliedContext::from_spec(
                                        crate::RefreshNotAppliedPhase::BeforeDispatch,
                                        crate::RefreshFailureSpec::new(
                                            crate::RefreshErrorKind::ProtocolError,
                                            crate::RetryAdvice::Never,
                                        ),
                                    ),
                                )))
                            } else {
                                Ok(RefreshRecheck::Needed)
                            }
                        }
                    },
                    move || async move {
                        provider_calls.fetch_add(1, Ordering::SeqCst);
                        provider_entered.notify_one();
                        provider_continue.notified().await;
                        gate_for_commit.store(true, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(())
                    },
                )
                .await
        }
    });

    provider_entered.notified().await;
    let duplicate_calls = Arc::new(AtomicUsize::new(0));
    let waiter = tokio::spawn({
        let coordinator = Arc::clone(&coordinator);
        let gate_installed = Arc::clone(&gate_installed);
        let duplicate_calls = Arc::clone(&duplicate_calls);
        async move {
            coordinator
                .refresh_coalesced(
                    &test_selector(credential_id),
                    move |_| {
                        let gate = Arc::clone(&gate_installed);
                        async move {
                            if gate.load(Ordering::SeqCst) {
                                Ok(RefreshRecheck::Suppressed(Box::new(
                                    RefreshNotAppliedContext::from_spec(
                                        crate::RefreshNotAppliedPhase::BeforeDispatch,
                                        crate::RefreshFailureSpec::new(
                                            crate::RefreshErrorKind::ProtocolError,
                                            crate::RetryAdvice::Never,
                                        ),
                                    ),
                                )))
                            } else {
                                Ok(RefreshRecheck::Needed)
                            }
                        }
                    },
                    move || async move {
                        duplicate_calls.fetch_add(1, Ordering::SeqCst);
                        RefreshDisposition::state_advanced(())
                    },
                )
                .await
        }
    });
    while coordinator
        .l1
        .waiter_count_for_test(&credential_id.to_string())
        == 0
    {
        tokio::task::yield_now().await;
    }

    provider_continue.notify_one();
    winner
        .await
        .expect("winner task joins")
        .expect("winner writes the gate");
    let waiter_outcome = waiter.await.expect("waiter task joins");
    let Err(RefreshError::RetrySuppressed(context)) = waiter_outcome else {
        panic!("L1 waiter must receive the exact durable retry block");
    };
    assert_eq!(context.retry(), crate::RetryAdvice::Never);
    assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(duplicate_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn hung_l2_release_cannot_wedge_l1_or_global_permit() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    repo.block_release.store(true, Ordering::SeqCst);
    let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
    let credential_id = CredentialId::new();
    let baseline_permits = coordinator.l1.available_permits();

    let result = coordinator
        .refresh_coalesced(
            &test_selector(credential_id),
            |_| async { Ok(RefreshRecheck::Needed) },
            || async { RefreshDisposition::state_advanced(55_u8) },
        )
        .await
        .expect("exact result must not wait on L2 release");
    assert_eq!(result, 55);
    assert_eq!(
        coordinator.l1.in_flight_count(),
        0,
        "exact disposition must wake local waiters before L2 release"
    );
    assert_eq!(
        coordinator.l1.available_permits(),
        baseline_permits,
        "exact disposition must return the global permit before L2 release"
    );
    repo.release_entered.notified().await;

    let duplicate_calls = Arc::new(AtomicUsize::new(0));
    let waiter_coordinator = Arc::clone(&coordinator);
    let waiter_duplicate_calls = Arc::clone(&duplicate_calls);
    let waiter = tokio::spawn(async move {
        waiter_coordinator
            .refresh_coalesced(
                &test_selector(credential_id),
                |_| async { Ok(RefreshRecheck::Needed) },
                move || async move {
                    waiter_duplicate_calls.fetch_add(1, Ordering::SeqCst);
                    RefreshDisposition::state_advanced(())
                },
            )
            .await
    });
    repo.wait_for_try_claim_count(2).await;
    assert_eq!(
        duplicate_calls.load(Ordering::SeqCst),
        0,
        "the still-live L2 row must coalesce the local waiter"
    );

    waiter.abort();
    assert!(waiter.await.is_err());
    wait_until_l1_empty(&coordinator).await;
    assert_eq!(coordinator.l1.available_permits(), baseline_permits);

    repo.release_continue.notify_one();
    repo.wait_for_release().await;
    assert!(!repo.active.load(Ordering::SeqCst));
}

#[tokio::test]
async fn unknown_commit_ack_retains_claim_as_durable_poison() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
    let credential_id = CredentialId::new();

    let result = coordinator
        .refresh_coalesced(
            &test_selector(credential_id),
            |_| async { Ok(RefreshRecheck::Needed) },
            || async { RefreshDisposition::outcome_unknown(21_u8) },
        )
        .await
        .expect("the enclosed typed outcome is returned");

    assert_eq!(result, 21);
    assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
    assert!(repo.active.load(Ordering::SeqCst));
    assert_eq!(coordinator.l1.in_flight_count(), 0);
}

#[tokio::test]
async fn definite_post_provider_failure_also_blocks_immediate_replay() {
    let repo = Arc::new(ScriptedClaimRepo::new());
    let coordinator = coordinator(Arc::clone(&repo), RefreshCoordConfig::default());
    let credential_id = CredentialId::new();

    let result = coordinator
        .refresh_coalesced(
            &test_selector(credential_id),
            |_| async { Ok(RefreshRecheck::Needed) },
            || async { RefreshDisposition::retry_unsafe(34_u8) },
        )
        .await
        .expect("the exact enclosed failure is returned");

    assert_eq!(result, 34);
    assert_eq!(repo.release_count.load(Ordering::SeqCst), 0);
    assert!(
        repo.active.load(Ordering::SeqCst),
        "another replica must not immediately re-POST the persisted stale grant"
    );
    assert_eq!(coordinator.l1.in_flight_count(), 0);
}
