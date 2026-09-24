//! Rate limiting through the manager: the limiter is consumed on acquire,
//! waits within the caller's deadline, fails fast past it, never trips the
//! recovery gate, is reachable per call through the guard, shares a quota
//! across rows with one key, and publishes transitions only.

mod common;

use std::{num::NonZeroU32, sync::Arc, time::Duration};

use common::{ResidentTestResource, test_config, test_ctx};
use nebula_resource::{
    AcquireOptions, ErrorKind, GateState, Manager, RecoveryGate, RecoveryGateConfig,
    RegistrationSpec, Resident, ResidentConfig, ResourceEvent, ScopeLevel, SlotIdentity,
    rate_limit::{LimitKey, Rate, RowLimit},
};
use tokio::time::Instant;

fn per_second(requests: u32, burst: u32) -> Rate {
    Rate::per_second(NonZeroU32::new(requests).unwrap())
        .with_burst(NonZeroU32::new(burst).unwrap())
        .unwrap()
}

fn register(
    manager: &Manager,
    scope: ScopeLevel,
    limit: RowLimit,
    recovery_gate: Option<Arc<RecoveryGate>>,
) {
    manager
        .register(RegistrationSpec {
            resource: ResidentTestResource::new(),
            config: test_config(),
            scope,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate,
            rate_limit: Some(limit),
        })
        .expect("registration succeeds");
}

fn deadline_in(after: Duration) -> AcquireOptions {
    AcquireOptions::default().with_deadline(std::time::Instant::now() + after)
}

#[tokio::test(start_paused = true)]
async fn acquire_waits_for_its_slot_within_the_deadline() {
    let manager = Manager::new();
    register(
        &manager,
        ScopeLevel::Global,
        RowLimit::rate(per_second(10, 1)),
        None,
    );

    drop(
        manager
            .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
            .await
            .expect("first acquire is free"),
    );
    let started = Instant::now();
    drop(
        manager
            .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
            .await
            .expect("second acquire waits for its slot"),
    );
    assert_eq!(started.elapsed(), Duration::from_millis(100));
}

#[tokio::test(start_paused = true)]
async fn acquire_fails_fast_when_the_slot_is_past_the_deadline() {
    let manager = Manager::new();
    let slow = Rate::new(NonZeroU32::MIN, Duration::from_secs(10)).unwrap();
    register(&manager, ScopeLevel::Global, RowLimit::rate(slow), None);

    drop(
        manager
            .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
            .await
            .expect("first acquire is free"),
    );
    let started = Instant::now();
    let error = manager
        .acquire::<ResidentTestResource>(&test_ctx(), &deadline_in(Duration::from_secs(1)))
        .await
        .expect_err("the next slot is 10 s away, past the 1 s deadline");
    assert_eq!(started.elapsed(), Duration::ZERO, "no pointless wait");
    assert!(matches!(
        error.kind(),
        ErrorKind::Exhausted { retry_after: Some(after) } if *after == Duration::from_secs(10)
    ));
}

#[tokio::test(start_paused = true)]
async fn rate_limit_denial_does_not_trip_the_recovery_gate() {
    let manager = Manager::new();
    let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig::default()));
    let slow = Rate::new(NonZeroU32::MIN, Duration::from_secs(10)).unwrap();
    register(
        &manager,
        ScopeLevel::Global,
        RowLimit::rate(slow),
        Some(Arc::clone(&gate)),
    );

    drop(
        manager
            .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
            .await
            .expect("first acquire is free"),
    );
    manager
        .acquire::<ResidentTestResource>(&test_ctx(), &deadline_in(Duration::from_millis(1)))
        .await
        .expect_err("rate limited");
    assert!(
        matches!(gate.state(), GateState::Idle),
        "our own rate limit is not backend ill health"
    );
}

#[tokio::test(start_paused = true)]
async fn guard_paces_calls_made_within_one_lease() {
    let manager = Manager::new();
    register(
        &manager,
        ScopeLevel::Global,
        RowLimit::rate(per_second(10, 2)),
        None,
    );

    let guard = manager
        .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .expect("acquire consumes one permit of the burst");
    let limits = guard.limits().expect("the row has a rate limit");
    limits
        .ready(None)
        .await
        .expect("the second burst permit is free");
    let started = Instant::now();
    limits
        .ready(None)
        .await
        .expect("the third call waits one emission interval");
    assert_eq!(started.elapsed(), Duration::from_millis(100));
}

/// Two rows drawing on one provider account share its quota: the key, not
/// the row, owns the limit.
#[tokio::test(start_paused = true)]
async fn rows_with_one_key_share_one_quota() {
    let manager = Manager::new();
    let account = LimitKey::new("account:shared").unwrap();
    let org = nebula_core::OrgId::new();
    for scope in [ScopeLevel::Organization(org), ScopeLevel::Global] {
        register(
            &manager,
            scope,
            RowLimit::rate(per_second(1, 1)).with_key(account.clone()),
            None,
        );
    }
    // The global row spends the account's permit…
    drop(
        manager
            .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
            .await
            .expect("first acquire on the account is free"),
    );
    // …so the organization's row, a different registry row, has none left.
    let org_ctx = nebula_resource::ResourceContext::minimal(
        nebula_core::scope::Scope {
            org_id: Some(org),
            ..Default::default()
        },
        tokio_util::sync::CancellationToken::new(),
    );
    let error = manager
        .acquire::<ResidentTestResource>(&org_ctx, &deadline_in(Duration::ZERO))
        .await
        .expect_err("the account's one permit per second is spent");
    assert!(matches!(error.kind(), ErrorKind::Exhausted { .. }));
}

#[tokio::test(start_paused = true)]
async fn limit_events_mark_transitions_and_penalties_only() {
    let manager = Manager::new();
    let mut events = manager.subscribe_events();
    register(
        &manager,
        ScopeLevel::Global,
        RowLimit::rate(per_second(10, 1)),
        None,
    );
    let guard = manager
        .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    let limits = guard.limits().unwrap();
    for _ in 0..3 {
        limits.ready(None).await.unwrap();
    }
    limits.penalize(Duration::from_secs(2)).await.unwrap();
    let started = Instant::now();
    limits.ready(None).await.unwrap();
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "a penalty blocks the next call until the provider's Retry-After"
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    limits.ready(None).await.unwrap();

    let mut limit_events = Vec::new();
    while let Some(event) = events.try_recv() {
        match event {
            ResourceEvent::RateLimitEngaged { .. } => limit_events.push("engaged"),
            ResourceEvent::RateLimitCleared { .. } => limit_events.push("cleared"),
            ResourceEvent::RateLimitPenalized { .. } => limit_events.push("penalized"),
            _ => {},
        }
    }
    assert_eq!(
        limit_events,
        ["engaged", "penalized", "cleared"],
        "three waits publish one transition, not three events"
    );
}
