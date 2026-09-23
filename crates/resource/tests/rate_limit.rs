//! Rate limiting through the manager: the limiter is consumed on acquire,
//! waits within the caller's deadline, fails fast past it, never trips the
//! recovery gate, and is reachable per call through the guard.

mod common;

use std::{sync::Arc, time::Duration};

use common::{ResidentTestResource, test_config, test_ctx};
use nebula_resource::{
    AcquireOptions, ErrorKind, GateState, Manager, RecoveryGate, RecoveryGateConfig,
    RegistrationSpec, Resident, ResidentConfig, ScopeLevel, SlotIdentity,
    rate_limit::{RateLimitSettings, RateLimiter},
};
use tokio::time::Instant;

fn register(
    manager: &Manager,
    settings: RateLimitSettings,
    recovery_gate: Option<Arc<RecoveryGate>>,
) {
    manager
        .register(RegistrationSpec {
            resource: ResidentTestResource::new(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate,
            rate_limit: Some(Arc::new(
                RateLimiter::new(settings).expect("valid settings"),
            )),
        })
        .expect("registration succeeds");
}

fn deadline_in(after: Duration) -> AcquireOptions {
    AcquireOptions::default().with_deadline(std::time::Instant::now() + after)
}

#[tokio::test(start_paused = true)]
async fn acquire_waits_for_its_slot_within_the_deadline() {
    let manager = Manager::new();
    register(&manager, RateLimitSettings::new(10, 1_000), None);

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
    register(&manager, RateLimitSettings::new(1, 10_000), None);

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
    register(
        &manager,
        RateLimitSettings::new(1, 10_000),
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
        RateLimitSettings::new(10, 1_000).with_burst(2),
        None,
    );

    let guard = manager
        .acquire::<ResidentTestResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .expect("acquire consumes one permit of the burst");
    let limiter = guard.rate_limiter().expect("the row has a rate limiter");
    limiter
        .until_ready(1, None)
        .await
        .expect("the second burst permit is free");
    let started = Instant::now();
    limiter
        .until_ready(1, None)
        .await
        .expect("the third call waits one emission interval");
    assert_eq!(started.elapsed(), Duration::from_millis(100));
}
