//! Event-bus and metrics integration tests for nebula-resource v2: emitted
//! `ResourceEvent`s (register/remove/acquire/release/recovery-gate
//! transition) and `ResourceOpsMetrics` snapshots recorded through a wired
//! `nebula_metrics::MetricsRegistry` (both registry-backed and none-wired).
//!
//! Split out of the former monolithic `basic_integration.rs` (pure move, no
//! test-body changes) — shared mocks/helpers live in `tests/common/mod.rs`.

mod common;

use std::sync::Arc;

use common::{PoolTestResource, ResidentTestResource, poll_until, test_config, test_ctx};
use nebula_core::{ResourceKey, resource_key};
use nebula_resource::{
    AcquireOptions, Error, ErrorKind, Manager, ManagerConfig, Pooled, Provider, RegistrationSpec,
    Resident, ResidentConfig, ResidentProvider, ResourceContext, ResourceEvent,
    RetirementFailureStage, RetirementOrigin, ScopeLevel, ShutdownConfig, ShutdownError,
    SlotIdentity, TeardownCx,
    guard::ResourceGuard,
    recovery::{RecoveryGate, RecoveryGateConfig},
};

#[derive(Clone)]
struct FailingRetirementResource<const ID: u8>;

impl<const ID: u8> nebula_resource::HasCredentialSlots for FailingRetirementResource<ID> {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    fn declares_credential_slots() -> bool {
        false
    }
}
impl<const ID: u8> ResidentProvider for FailingRetirementResource<ID> {}

#[async_trait::async_trait]
impl<const ID: u8> Provider for FailingRetirementResource<ID> {
    type Config = common::TestConfig;
    type Instance = ();
    type Topology = Resident<Self>;

    fn metadata() -> nebula_resource::ResourceMetadataDraft {
        nebula_resource::ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("FailingRetirementResource"),
            "",
        )
    }

    fn key() -> ResourceKey {
        ResourceKey::try_from(format!("test-failing-retirement-{ID}"))
            .expect("static test resource key is valid")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _context: &ResourceContext,
    ) -> Result<Self::Instance, Error> {
        Ok(())
    }

    async fn destroy(&self, (): (), _context: TeardownCx) -> Result<(), Error> {
        Err(Error::permanent(
            "provider supplied text must not reach lifecycle events",
        ))
    }
}

async fn instantiate<const ID: u8>(manager: &Manager) {
    let guard = manager
        .acquire_resident::<FailingRetirementResource<ID>>(&test_ctx(), &AcquireOptions::default())
        .await
        .expect("test resource acquires");
    assert_eq!(
        guard.release().await.expect("resident lease releases"),
        nebula_resource::ReleaseOutcome::Completed
    );
}

fn register_failing<const ID: u8>(manager: &Manager) {
    manager
        .register(RegistrationSpec {
            resource: FailingRetirementResource::<ID>,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .expect("test resource registers");
}

// ---------------------------------------------------------------------------
// Event emission tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_emits_registered_event() {
    let manager = Manager::new();
    let mut rx = manager.subscribe_events();

    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let event = rx.try_recv().expect("should have received an event");
    assert!(
        matches!(&event, ResourceEvent::Registered { key } if key == &resource_key!("test-resident")),
        "expected Registered event, got {event:?}"
    );
}

#[tokio::test]
async fn remove_emits_removed_event() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .unwrap();

    let mut rx = manager.subscribe_events();
    let key = resource_key!("test-resident");
    manager.remove(&key).expect("remove should succeed");

    let event = rx.try_recv().expect("should have received an event");
    assert!(
        matches!(&event, ResourceEvent::Removed { key } if key == &resource_key!("test-resident")),
        "expected Removed event, got {event:?}"
    );
}

#[tokio::test]
async fn shutdown_emits_one_redacted_failure_event_for_each_failing_row() {
    let registry = Arc::new(nebula_metrics::MetricsRegistry::new());
    let manager = Manager::with_config(
        ManagerConfig::default()
            .with_release_queue_workers(2)
            .with_metrics_registry(registry),
    );
    register_failing::<1>(&manager);
    register_failing::<2>(&manager);
    instantiate::<1>(&manager).await;
    instantiate::<2>(&manager).await;
    let mut events = manager.subscribe_events();

    let error = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect_err("terminal failures fail graceful shutdown");
    let ShutdownError::ResourceTeardownFailed { key, source } = error else {
        panic!("expected first typed row failure, got {error:?}");
    };
    assert!(
        key == FailingRetirementResource::<1>::key()
            || key == FailingRetirementResource::<2>::key()
    );
    assert_eq!(source.kind(), &ErrorKind::Permanent);
    assert_eq!(
        manager
            .metrics()
            .expect("configured metrics are available")
            .snapshot()
            .release_errors,
        2,
        "each failing row increments the lifecycle failure counter"
    );

    let mut failed_keys = Vec::new();
    while let Some(event) = events.try_recv() {
        if let ResourceEvent::ResourceTeardownFailed {
            key,
            origin,
            stage,
            kind,
            message,
            ..
        } = event
        {
            assert_eq!(origin, RetirementOrigin::Shutdown);
            assert_eq!(stage, RetirementFailureStage::TerminalCleanup);
            assert_eq!(kind, ErrorKind::Permanent);
            assert_eq!(message.as_str(), "resource terminal cleanup failed");
            assert!(!message.as_str().contains("provider supplied"));
            failed_keys.push(key);
        }
    }
    failed_keys.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    assert_eq!(
        failed_keys,
        vec![
            FailingRetirementResource::<1>::key(),
            FailingRetirementResource::<2>::key(),
        ]
    );
}

#[tokio::test]
async fn replacement_and_removal_failures_keep_their_retirement_origin() {
    let manager = Manager::new();
    register_failing::<3>(&manager);
    instantiate::<3>(&manager).await;
    let mut events = manager.subscribe_events();

    register_failing::<3>(&manager);
    let replacement = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if let Some(ResourceEvent::ResourceTeardownFailed { key, origin, .. }) =
                events.recv().await
                && origin == RetirementOrigin::Replacement
            {
                break key;
            }
        }
    })
    .await
    .expect("replacement cleanup settles");
    assert_eq!(replacement, FailingRetirementResource::<3>::key());

    instantiate::<3>(&manager).await;
    manager
        .remove(&FailingRetirementResource::<3>::key())
        .expect("registered row removes");
    let removal = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if let Some(ResourceEvent::ResourceTeardownFailed { key, origin, .. }) =
                events.recv().await
                && origin == RetirementOrigin::Removal
            {
                break key;
            }
        }
    })
    .await
    .expect("removal cleanup settles");
    assert_eq!(removal, FailingRetirementResource::<3>::key());
}

#[tokio::test]
async fn manager_drop_reports_each_row_abandoned_without_enqueuing_terminal_work() {
    let manager = Manager::new();
    register_failing::<4>(&manager);
    instantiate::<4>(&manager).await;
    let mut events = manager.subscribe_events();

    drop(manager);

    std::assert_matches!(
        events.try_recv(),
        Some(ResourceEvent::ResourceTeardownFailed {
            key,
            origin: RetirementOrigin::ManagerDrop,
            stage: RetirementFailureStage::TerminalCleanup,
            kind: ErrorKind::Cancelled,
            ..
        }) if key == FailingRetirementResource::<4>::key()
    );
    assert!(
        events.try_recv().is_none(),
        "manager drop must classify a row exactly once"
    );
}

#[tokio::test]
async fn acquire_emits_success_event() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .unwrap();

    let mut rx = manager.subscribe_events();
    // Drain the Registered event.
    let _ = rx.try_recv();

    let ctx = test_ctx();
    let _handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    let event = rx.try_recv().expect("should have received an event");
    assert!(
        matches!(&event, ResourceEvent::AcquireSuccess { key, .. } if key == &resource_key!("test-resident")),
        "expected AcquireSuccess event, got {event:?}"
    );
}

/// Dropping a manager-minted guard must emit `ResourceEvent::Released`.
///
/// Regression guard for the EventBus migration: the guard's release sink is
/// wired by `Manager::run_acquire` (`with_event_bus`). If that wiring is
/// dropped, acquires still succeed and every other test stays green — only
/// this assertion fails — so the `Released` lifecycle signal is pinned here.
#[tokio::test]
async fn drop_guard_emits_released_event() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .unwrap();

    let mut rx = manager.subscribe_events();

    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    // Drop the guard — its `Drop` impl runs the release pathway and emits
    // `Released` after the recycle/destroy effect.
    drop(handle);

    // Drop queues cleanup; Released is emitted after settlement, not on the
    // dropping caller's stack. Await the manager's lifecycle checkpoint.
    manager.graceful_shutdown(Default::default()).await.unwrap();

    let mut saw_released = false;
    while let Some(event) = rx.try_recv() {
        if matches!(
            &event,
            ResourceEvent::Released { key, .. }
                if key == &resource_key!("test-resident")
        ) {
            saw_released = true;
            break;
        }
    }
    assert!(
        saw_released,
        "expected a Released event after the guard was dropped",
    );
}

/// Registering a resource with a recovery gate must wire the manager's event
/// bus into that gate, so its state transitions surface as
/// `ResourceEvent::RecoveryGateChanged`.
///
/// Regression guard for the EventBus migration: the sink is attached in
/// `Manager::register` (`gate.set_event_sink`). If that wiring is dropped the
/// gate still functions but goes silent — only this assertion catches it.
#[tokio::test]
async fn recovery_gate_transition_emits_event_via_manager_bus() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());
    let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig::default()));

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: Some(Arc::clone(&gate)),
        })
        .unwrap();

    let mut rx = manager.subscribe_events();

    // Drive the gate Idle -> InProgress and assert *that* transition is
    // observed before resolving — pinning the `try_begin` emission to the
    // manager-wired sink specifically, not merely the later resolve-side
    // InProgress -> Idle event (which would pass even with broken wiring of
    // the begin path).
    let ticket = gate.try_begin().expect("gate starts idle");

    let mut saw_in_progress = false;
    while let Some(event) = rx.try_recv() {
        if let ResourceEvent::RecoveryGateChanged { key, state } = &event
            && key == &resource_key!("test-resident")
            && state.contains("in_progress")
        {
            saw_in_progress = true;
            break;
        }
    }
    assert!(
        saw_in_progress,
        "expected a RecoveryGateChanged(in_progress) event after gate.try_begin()",
    );

    // Resolve to leave the gate idle for any later reuse; not asserted here.
    ticket.resolve();
}

// ---------------------------------------------------------------------------
// Metrics verification
// ---------------------------------------------------------------------------

#[tokio::test]
async fn metrics_track_acquire_release_create_destroy() {
    let registry = Arc::new(nebula_metrics::MetricsRegistry::new());
    let manager = Manager::with_config(
        ManagerConfig::default()
            .with_release_queue_workers(2)
            .with_metrics_registry(registry.clone()),
    );
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    // register calls record_create
    let snap = manager.metrics().expect("metrics present").snapshot();
    assert_eq!(snap.create_total, 1, "register should record create");

    // Acquire.
    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    let snap = manager.metrics().expect("metrics present").snapshot();
    assert_eq!(snap.acquire_total, 1, "acquire should be counted");
    assert_eq!(snap.acquire_errors, 0, "no errors expected");

    drop(handle);

    // Remove — calls record_destroy.
    let key = resource_key!("test-resident");
    manager.remove(&key).expect("remove should succeed");

    let snap = manager.metrics().expect("metrics present").snapshot();
    assert_eq!(snap.destroy_total, 1, "remove should record destroy");
}

// ---------------------------------------------------------------------------
// Registry-backed metrics tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn registry_backed_metrics_record_operations() {
    let registry = Arc::new(nebula_metrics::MetricsRegistry::new());
    let manager = Manager::with_config(
        ManagerConfig::default()
            .with_release_queue_workers(1)
            .with_metrics_registry(registry.clone()),
    );

    // Register two resources.
    let pool_resource = PoolTestResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool_rt = Pooled::<PoolTestResource>::new(pool_config, 1);

    manager
        .register(RegistrationSpec {
            resource: pool_resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("pool registration should succeed");

    let resident_resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource: resident_resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("resident registration should succeed");

    // Acquire the pooled resource once.
    let ctx = test_ctx();
    let handle: ResourceGuard<PoolTestResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
        .await
        .expect("pool acquire should succeed");
    drop(handle);
    // `acquire_total` / `create_total` are recorded synchronously during
    // the acquire (not by the release worker). Poll the precondition the
    // asserts depend on rather than guess a wall-clock delay.
    poll_until(std::time::Duration::from_secs(2), || {
        manager
            .metrics()
            .map(nebula_resource::ResourceOpsMetrics::snapshot)
            .is_some_and(|s| s.acquire_total >= 1 && s.create_total >= 2)
    })
    .await;

    // Aggregate metrics via snapshot.
    let snap = manager
        .metrics()
        .expect("metrics should be present")
        .snapshot();
    assert_eq!(snap.acquire_total, 1, "should have 1 acquire");
    assert_eq!(
        snap.create_total, 2,
        "should have 2 creates (pool + resident)"
    );

    // Same counters visible via registry directly.
    let create_counter = registry
        .counter(nebula_metrics::naming::NEBULA_RESOURCE_CREATE_TOTAL)
        .unwrap();
    assert_eq!(create_counter.get(), 2);

    manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(50)),
        )
        .await
        .expect("graceful_shutdown must succeed");
}

#[tokio::test]
async fn metrics_none_when_no_registry() {
    let manager = Manager::new();
    assert!(
        manager.metrics().is_none(),
        "manager without registry should have no metrics"
    );
}
