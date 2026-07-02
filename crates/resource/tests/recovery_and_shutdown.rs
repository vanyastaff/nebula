//! `RecoveryGate` and graceful-shutdown integration tests for nebula-resource
//! v2: gate state transitions (begin/resolve/backoff), error-kind
//! classification, the no-manager-side-retry invariant (Mythos v2 — exactly
//! one attempt per `acquire_*` call regardless of failure kind), the
//! probe-boundary herd-serialization regression (#322), and
//! `Manager::graceful_shutdown` (happy path, drain-timeout abort, double-call
//! rejection, force-clear on timeout — #302).
//!
//! Split out of the former monolithic `basic_integration.rs` (pure move, no
//! test-body changes) — shared mocks/helpers live in `tests/common/mod.rs`.

mod common;

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use common::{ResidentTestResource, TestConfig, test_config, test_ctx};
use nebula_core::{ResourceKey, resource_key};
use nebula_resource::{
    AcquireOptions, Manager, RegistrationSpec, Resident, ResidentConfig, ResourceContext,
    ScopeLevel, ShutdownConfig, SlotIdentity,
    error::{Error, ErrorKind},
    guard::ResourceGuard,
    recovery::{GateState, RecoveryGate, RecoveryGateConfig},
    resource::{Provider, ResourceMetadata},
    topology::resident::ResidentProvider,
};

// ---------------------------------------------------------------------------
// RecoveryGate tests
// ---------------------------------------------------------------------------

#[test]
fn recovery_gate_begin_resolve_cycle() {
    let gate = RecoveryGate::new(RecoveryGateConfig::default());
    assert!(matches!(gate.state(), GateState::Idle));

    // Begin recovery.
    let ticket = gate.try_begin().expect("should get ticket");
    assert_eq!(ticket.attempt(), 1);
    assert!(matches!(gate.state(), GateState::InProgress { .. }));

    // Resolve.
    ticket.resolve();
    assert!(matches!(gate.state(), GateState::Idle));

    // Can begin again after resolve.
    let ticket2 = gate.try_begin().expect("should get second ticket");
    assert_eq!(ticket2.attempt(), 1); // resets to 1 after resolve
    ticket2.resolve();
}

#[test]
fn recovery_gate_fail_transient_and_retry() {
    let config = RecoveryGateConfig {
        max_attempts: 5,
        base_backoff: std::time::Duration::from_millis(0),
    };
    let gate = RecoveryGate::new(config);

    let ticket = gate.try_begin().unwrap();
    ticket.fail_transient("connection refused");

    assert!(matches!(gate.state(), GateState::Failed { .. }));

    // Zero backoff means we can retry immediately.
    let ticket2 = gate.try_begin().expect("should allow retry");
    assert_eq!(ticket2.attempt(), 2);
    ticket2.resolve();
}

#[test]
fn recovery_gate_permanent_failure() {
    let gate = RecoveryGate::new(RecoveryGateConfig::default());
    let ticket = gate.try_begin().unwrap();
    ticket.fail_permanent("certificate expired");

    assert!(matches!(gate.state(), GateState::PermanentlyFailed { .. }));

    // Further attempts fail.
    assert!(gate.try_begin().is_err());

    // Admin reset clears it.
    gate.reset();
    assert!(matches!(gate.state(), GateState::Idle));
    assert!(gate.try_begin().is_ok());
}

// ---------------------------------------------------------------------------
// Error classification tests
// ---------------------------------------------------------------------------

#[test]
fn error_retryability() {
    assert!(Error::transient("timeout").is_retryable());
    assert!(Error::exhausted("rate limited", None).is_retryable());
    assert!(!Error::permanent("bad config").is_retryable());
    assert!(!Error::cancelled().is_retryable());
    assert!(Error::backpressure("pool full").is_retryable());
}

// ---------------------------------------------------------------------------
// Graceful shutdown tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graceful_shutdown_stops_new_acquires() {
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

    manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(10)),
        )
        .await
        .expect("graceful_shutdown must succeed");

    assert!(manager.is_shutdown());

    // Acquire should fail with Cancelled.
    let ctx = test_ctx();
    match manager
        .acquire_resident::<ResidentTestResource>(&ctx, &AcquireOptions::default())
        .await
    {
        Err(e) => assert!(
            matches!(e.kind(), ErrorKind::Cancelled),
            "expected Cancelled, got {e:?}"
        ),
        Ok(_) => panic!("acquire after graceful shutdown should fail"),
    }
}

#[tokio::test]
async fn graceful_shutdown_clears_registry() {
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

    // Graceful shutdown now clears the registry to allow release queue
    // workers to drain.
    let report = manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(10)),
        )
        .await
        .expect("graceful_shutdown must succeed");
    assert!(
        report.registry_cleared,
        "ShutdownReport should confirm registry was cleared"
    );

    assert!(manager.is_shutdown());
    assert!(
        !manager.contains(&resource_key!("test-resident")),
        "registry should be cleared after graceful shutdown"
    );
}

#[tokio::test]
async fn graceful_shutdown_default_config() {
    let config = ShutdownConfig::default();
    assert_eq!(config.drain_timeout, std::time::Duration::from_secs(30));
}

// ---------------------------------------------------------------------------
// No-manager-side-retry invariant (Mythos v2)
//
// Manager-side `AcquireResilience` deleted. Retry policy at the resource
// layer is the caller's concern (peer pattern: sqlx/deadpool/bb8). These
// tests pin the contract that `Manager::acquire_*` performs exactly one
// attempt regardless of failure kind — retry composes one layer up
// (engine activity, or caller-supplied `nebula-resilience` pipeline).
// ---------------------------------------------------------------------------

/// A resident resource that fails with a transient error for the first N
/// `create` calls, then succeeds.
#[derive(Clone)]
struct FailingResidentResource {
    create_count: Arc<AtomicU64>,
    /// Number of initial creates that return a transient error.
    failures_before_success: u64,
    alive: Arc<AtomicBool>,
}

impl FailingResidentResource {
    fn new(failures_before_success: u64) -> Self {
        Self {
            create_count: Arc::new(AtomicU64::new(0)),
            failures_before_success,
            alive: Arc::new(AtomicBool::new(true)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for FailingResidentResource {
    type Config = TestConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test-failing-resident")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, Error> {
        let count = self.create_count.fetch_add(1, Ordering::Relaxed);
        if count < self.failures_before_success {
            Err(Error::transient("transient failure"))
        } else {
            Ok(Arc::new(AtomicU64::new(count)))
        }
    }

    async fn destroy(
        &self,
        _runtime: Arc<AtomicU64>,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

nebula_resource::no_credential_slots!(FailingResidentResource);

#[async_trait::async_trait]
impl ResidentProvider for FailingResidentResource {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

/// Resident resource whose `create` blocks on a [`tokio::sync::Notify`]
/// until [`Self::unblock`] is woken. Exists to prove "manager imposes no
/// wall-clock timeout on `create`" — `FailingResidentResource::create`
/// completes immediately (success or error), so it cannot distinguish
/// "no manager timeout" from "fast manager timeout".
#[derive(Clone)]
struct BlockingResidentResource {
    unblock: Arc<tokio::sync::Notify>,
}

impl BlockingResidentResource {
    fn new() -> Self {
        Self {
            unblock: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

#[async_trait::async_trait]
impl Provider for BlockingResidentResource {
    type Config = TestConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test-blocking-resident")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, Error> {
        self.unblock.notified().await;
        Err(Error::transient("unblocked but never satisfied"))
    }

    async fn destroy(
        &self,
        _runtime: Arc<AtomicU64>,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

nebula_resource::no_credential_slots!(BlockingResidentResource);

#[async_trait::async_trait]
impl ResidentProvider for BlockingResidentResource {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        true
    }
}

/// A resident resource that always fails with a permanent error.
#[derive(Clone)]
struct PermanentFailResource {
    create_count: Arc<AtomicU64>,
}

impl PermanentFailResource {
    fn new() -> Self {
        Self {
            create_count: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for PermanentFailResource {
    type Config = TestConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test-permanent-fail")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, Error> {
        self.create_count.fetch_add(1, Ordering::Relaxed);
        Err(Error::permanent("permanent failure"))
    }

    async fn destroy(
        &self,
        _runtime: Arc<AtomicU64>,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

nebula_resource::no_credential_slots!(PermanentFailResource);

#[async_trait::async_trait]
impl ResidentProvider for PermanentFailResource {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        true
    }
}

#[tokio::test]
async fn acquire_does_not_retry_transient_at_manager_layer() {
    // Mythos v2: manager performs exactly one acquire attempt. A transient
    // failure surfaces immediately to the caller; retry is composed above.
    let manager = Manager::new();
    let resource = FailingResidentResource::new(1);
    let resident_rt = Resident::<FailingResidentResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let result = manager
        .acquire_resident::<FailingResidentResource>(&ctx, &AcquireOptions::default())
        .await;

    assert!(result.is_err(), "transient failure must surface (no retry)");
    assert_eq!(
        resource.create_count.load(Ordering::Relaxed),
        1,
        "exactly one acquire attempt at manager layer"
    );
}

#[tokio::test]
async fn acquire_does_not_retry_permanent_at_manager_layer() {
    let manager = Manager::new();
    let resource = PermanentFailResource::new();
    let resident_rt = Resident::<PermanentFailResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let result = manager
        .acquire_resident::<PermanentFailResource>(&ctx, &AcquireOptions::default())
        .await;

    // Assert both the attempt count AND the typed error kind. The count
    // alone pins "no manager-layer retry"; pinning `ErrorKind::Permanent`
    // pins the orthogonal "no error-kind normalization at the manager
    // layer" invariant — if the manager ever started re-wrapping
    // permanent into transient (or vice versa), the count assertion
    // would still pass but classification would be silently broken.
    let err = result.expect_err("permanent failure surfaces immediately");
    assert_eq!(*err.kind(), ErrorKind::Permanent);
    assert_eq!(
        resource.create_count.load(Ordering::Relaxed),
        1,
        "exactly one acquire attempt at manager layer"
    );
}

#[tokio::test]
async fn acquire_succeeds_without_resilience() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire without resilience should succeed");

    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    drop(handle);
}

#[tokio::test(start_paused = true)]
async fn acquire_has_no_manager_layer_timeout() {
    // Mythos v2: manager applies no wall-clock timeout. Acquire-timeout
    // belongs to the topology runtime (`create_timeout` on resident /
    // pool config) or to a caller-composed `nebula-resilience` pipeline.
    //
    // To prove the manager imposes no wall-clock bound on a slow
    // `create`, we use a resource whose `create` blocks indefinitely on
    // a `Notify` and wrap the acquire in a caller-side
    // `tokio::time::timeout`. The outer timeout MUST be the path that
    // resolves first — if the manager ever started imposing its own
    // bound, this test would fail by either succeeding (acquire returns
    // before the outer timeout) or returning a typed manager-side error
    // (acquire returns an `Error` before the outer `Elapsed`).
    //
    // `start_paused = true` gives us deterministic time control: the
    // outer 1-second timeout advances via `tokio::time::timeout`'s own
    // internal clock manipulation without sleeping wall-time.
    let manager = Manager::new();
    let resource = BlockingResidentResource::new();
    let resident_rt = Resident::<BlockingResidentResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        manager.acquire_resident::<BlockingResidentResource>(&ctx, &AcquireOptions::default()),
    )
    .await;

    assert!(
        outcome.is_err(),
        "outer caller-side timeout must fire (manager imposes no wall-clock bound) — outcome: {outcome:?}",
    );
    // Release the blocked `create` so the spawned future does not leak
    // beyond the test (`tokio::time::timeout` drops the future, which
    // should cancel-drop the in-progress create — `notify_one` here is
    // belt-and-suspenders for any internal create-detach path that may
    // outlive the caller cancellation in future refactors).
    resource.unblock.notify_waiters();
}

// ---------------------------------------------------------------------------
// graceful_shutdown edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graceful_shutdown_second_call_errors_already_shutting_down() {
    use nebula_resource::manager::ShutdownError;

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

    let short_drain =
        ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(10));

    // First shutdown wins the CAS and proceeds.
    manager
        .graceful_shutdown(short_drain.clone())
        .await
        .expect("first graceful_shutdown must succeed");
    assert!(manager.is_shutdown());

    // Second shutdown must fail-fast with AlreadyShuttingDown (#302).
    // Before the CAS guard, a second call would re-enter the phases and
    // race against a half-torn manager.
    let err = manager
        .graceful_shutdown(short_drain)
        .await
        .expect_err("second graceful_shutdown must error");
    assert!(
        matches!(err, ShutdownError::AlreadyShuttingDown),
        "expected AlreadyShuttingDown, got {err:?}"
    );
    assert!(manager.is_shutdown());
}

// ---------------------------------------------------------------------------
// Error kind preserved across manager (no rewrap)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn acquire_surfaces_underlying_transient_error_kind() {
    // Mythos v2: with no retry/timeout wrapping at this layer, the
    // resource's typed error reaches the caller unchanged — preserving
    // `Classify` for any upstream pipeline composed by the caller.
    let manager = Manager::new();
    let resource = FailingResidentResource::new(100);
    let resident_rt = Resident::<FailingResidentResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .unwrap();

    let ctx = test_ctx();
    let result = manager
        .acquire_resident::<FailingResidentResource>(&ctx, &AcquireOptions::default())
        .await;

    assert_eq!(
        resource.create_count.load(Ordering::Relaxed),
        1,
        "exactly one acquire attempt"
    );

    match result {
        Err(e) => assert!(
            matches!(e.kind(), ErrorKind::Transient),
            "transient error kind preserved across manager, got {:?}",
            e.kind()
        ),
        Ok(_) => panic!("acquire must fail when create fails"),
    }
}

// ---------------------------------------------------------------------------
// Acquire failure triggers RecoveryGate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn acquire_failure_passively_triggers_recovery_gate() {
    let manager = Manager::new();
    // Always fails with transient error.
    let resource = FailingResidentResource::new(100);
    let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig {
        max_attempts: 5,
        base_backoff: std::time::Duration::from_mins(5),
    }));
    let resident_rt = Resident::<FailingResidentResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: Some(gate.clone()),
        })
        .unwrap();

    let ctx = test_ctx();

    // First acquire fails — should trigger the gate.
    let _ = manager
        .acquire_resident::<FailingResidentResource>(&ctx, &AcquireOptions::default())
        .await;

    // Gate should no longer be Idle.
    assert!(
        !matches!(gate.state(), GateState::Idle),
        "gate should have been triggered by transient acquire failure, got {:?}",
        gate.state()
    );
}

// ---------------------------------------------------------------------------
// Recovery gate integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn recovery_gate_blocks_acquire_when_permanently_failed() {
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    let gate = RecoveryGate::new(RecoveryGateConfig::default());
    // Force permanent failure.
    let ticket = gate.try_begin().expect("gate starts idle");
    ticket.fail_permanent("backend certificate expired");

    let manager = Manager::new();
    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: Some(Arc::new(gate)),
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let result: Result<ResourceGuard<ResidentTestResource>, _> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await;

    let err = result.expect_err("acquire should fail when gate is permanently failed");
    assert_eq!(
        *err.kind(),
        ErrorKind::Permanent,
        "should be a permanent error"
    );
}

#[tokio::test]
async fn recovery_gate_blocks_acquire_when_in_progress() {
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    let gate = RecoveryGate::new(RecoveryGateConfig::default());
    // Hold the ticket — gate is InProgress.
    let _ticket = gate.try_begin().expect("gate starts idle");

    let manager = Manager::new();
    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: Some(Arc::new(gate)),
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let result: Result<ResourceGuard<ResidentTestResource>, _> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await;

    let err = result.expect_err("acquire should fail when gate is in progress");
    assert_eq!(
        *err.kind(),
        ErrorKind::Transient,
        "should be a transient error"
    );
}

// ---------------------------------------------------------------------------
// GateState point-in-time read via `Manager::health_check`
// ---------------------------------------------------------------------------

/// `ResourceHealthSnapshot::gate_state` is the point-in-time dashboard read
/// of a resource's `RecoveryGate` — `RecoveryGateChanged` is the event
/// stream a subscriber follows live, this is the poll a health-check
/// endpoint or admin API uses instead.
#[tokio::test]
async fn health_check_surfaces_recovery_gate_state() {
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    let gate = RecoveryGate::new(RecoveryGateConfig::default());
    // Hold the ticket — gate is InProgress.
    let _ticket = gate.try_begin().expect("gate starts idle");

    let manager = Manager::new();
    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: Some(Arc::new(gate)),
        })
        .expect("registration should succeed");

    let health = manager
        .health_check::<ResidentTestResource>(&ScopeLevel::Global)
        .expect("resource is registered");
    assert!(
        matches!(health.gate_state, Some(GateState::InProgress { .. })),
        "expected InProgress, got {:?}",
        health.gate_state
    );
}

/// A resource registered without a `RecoveryGate` reports `gate_state:
/// None` — the snapshot does not fabricate a state for a gate that was
/// never attached.
#[tokio::test]
async fn health_check_gate_state_is_none_without_a_gate() {
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    let manager = Manager::new();
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

    let health = manager
        .health_check::<ResidentTestResource>(&ScopeLevel::Global)
        .expect("resource is registered");
    assert!(
        health.gate_state.is_none(),
        "no gate attached ⇒ no gate_state"
    );
}

#[tokio::test]
async fn recovery_gate_allows_acquire_when_idle() {
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    let gate = RecoveryGate::new(RecoveryGateConfig::default());

    let manager = Manager::new();
    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: Some(Arc::new(gate)),
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed when gate is idle");
    drop(handle);
}

#[tokio::test]
async fn recovery_gate_allows_acquire_after_backoff_expires() {
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    let gate = RecoveryGate::new(RecoveryGateConfig {
        max_attempts: 5,
        base_backoff: std::time::Duration::from_millis(0), // instant expiry
    });
    // Fail transiently — backoff is 0ms, so retry_at is already in the past.
    let ticket = gate.try_begin().expect("gate starts idle");
    ticket.fail_transient("connection refused");

    let manager = Manager::new();
    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: Some(Arc::new(gate)),
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    // Backoff expired, so acquire should proceed (caller acts as probe).
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed after backoff expires");
    drop(handle);
}

#[tokio::test]
async fn recovery_gate_none_does_not_affect_acquire() {
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    let manager = Manager::new();
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

    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed without recovery gate");
    drop(handle);
}

// ---------------------------------------------------------------------------
// #302 regression — DrainTimeoutPolicy / ShutdownReport / ShutdownError
// ---------------------------------------------------------------------------

/// #302: with the default `DrainTimeoutPolicy::Abort`, a drain timeout must
/// return `Err(ShutdownError::DrainTimeout { outstanding })` and **must not**
/// clear the registry. Any outstanding handle remains valid.
#[tokio::test]
async fn graceful_shutdown_abort_on_drain_timeout_preserves_registry() {
    use nebula_resource::manager::ShutdownError;

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

    // Hold a handle across the shutdown so drain cannot complete.
    let ctx = test_ctx();
    let _handle = manager
        .acquire_resident::<ResidentTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire must succeed");

    let err = manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(20)),
        )
        .await
        .expect_err("Abort policy must surface drain timeout as Err");

    match err {
        ShutdownError::DrainTimeout { outstanding } => {
            assert!(
                outstanding >= 1,
                "expected at least one outstanding handle, got {outstanding}"
            );
        },
        other => panic!("expected DrainTimeout, got {other:?}"),
    }

    // Registry must still contain the resource — the whole point of #302.
    assert!(
        manager.contains(&resource_key!("test-resident")),
        "Abort policy must preserve the registry on drain timeout"
    );
}

/// Regression: `DrainTimeoutPolicy::Abort` must transition every
/// registered resource to `ResourcePhase::Failed`, **not** restore
/// `Ready`. Pre-fix the manager would set the phase back to `Ready` to
/// "keep the resource acquirable", but the cancel token already rejects
/// new acquires and `health_check` then lied about lifecycle state.
///
/// Also asserts the per-resource `HealthChanged{healthy:false}` event is
/// emitted on the broadcast channel so external observers see the
/// failure signal even if they only subscribe to events.
#[tokio::test]
async fn graceful_shutdown_abort_marks_resources_failed_not_ready() {
    use nebula_resource::{ResourceEvent, ResourcePhase, manager::ShutdownError};

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

    // Subscribe BEFORE the shutdown so we can capture the
    // HealthChanged{healthy:false} broadcast emitted by
    // set_phase_all_failed.
    let mut events = manager.subscribe_events();

    // Hold a handle across the shutdown so drain cannot complete.
    let ctx = test_ctx();
    let _handle = manager
        .acquire_resident::<ResidentTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire must succeed");

    let err = manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(20)),
        )
        .await
        .expect_err("Abort policy must surface drain timeout as Err");

    assert!(
        matches!(err, ShutdownError::DrainTimeout { .. }),
        "expected DrainTimeout, got {err:?}"
    );

    // Assertion: phase is `Failed`, NOT `Ready`. Pre-fix this
    // would be `Ready` (the bug). We bypass `health_check` here because
    // it goes through `lookup` which short-circuits on the cancel token
    // post-shutdown; `get_any` reads the type-erased registry entry
    // directly so we can observe the phase the abort branch wrote.
    let phase = manager
        .get_any(&resource_key!("test-resident"), &ScopeLevel::Global)
        .expect("registry preserved (Abort policy)")
        .phase();
    assert_eq!(
        phase,
        ResourcePhase::Failed,
        "drain-abort must transition phase to Failed, got {phase:?} \
         (Ready would be the pre-fix bug)",
    );

    // Assertion: per-resource HealthChanged{healthy:false} was
    // emitted. Drain the event subscriber until we find it (other
    // events like `Registered` and `AcquireSuccess` were also emitted
    // earlier).
    let mut saw_health_change = false;
    while let Some(event) = events.try_recv() {
        if let ResourceEvent::HealthChanged {
            key,
            healthy: false,
        } = event
            && key == resource_key!("test-resident")
        {
            saw_health_change = true;
            break;
        }
    }
    assert!(
        saw_health_change,
        "drain-abort must emit HealthChanged{{healthy:false}} per resource"
    );
}

/// #302: `DrainTimeoutPolicy::Force` is the opt-in escape hatch. It clears
/// the registry anyway and reports the outstanding-handle count so a
/// supervisor with a hard deadline can still exit.
#[tokio::test]
async fn graceful_shutdown_force_clears_registry_on_timeout() {
    use nebula_resource::manager::DrainTimeoutPolicy;

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

    let ctx = test_ctx();
    let _handle = manager
        .acquire_resident::<ResidentTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire must succeed");

    let report = manager
        .graceful_shutdown(
            ShutdownConfig::default()
                .with_drain_timeout(std::time::Duration::from_millis(20))
                .with_drain_timeout_policy(DrainTimeoutPolicy::Force),
        )
        .await
        .expect("Force policy must yield Ok(ShutdownReport)");

    assert!(report.registry_cleared);
    assert!(
        report.outstanding_handles_after_drain >= 1,
        "report must surface the outstanding count"
    );
    assert!(
        !manager.contains(&resource_key!("test-resident")),
        "Force policy must clear the registry"
    );
}

/// Happy path: no outstanding handles, shutdown returns `Ok` with zero
/// outstanding and `registry_cleared: true`.
#[tokio::test]
async fn graceful_shutdown_happy_path_returns_zero_outstanding() {
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

    let report = manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(50)),
        )
        .await
        .expect("happy path must succeed");

    assert_eq!(report.outstanding_handles_after_drain, 0);
    assert!(report.registry_cleared);
    assert!(report.release_queue_drained);
}

// ---------------------------------------------------------------------------
// #322 regression — probe-boundary serialization under concurrent acquires
// ---------------------------------------------------------------------------

/// #322: before the fix, `check_recovery_gate` inspected `gate.state()`
/// read-only and, on expired `Failed`, returned `Ok(())` so every caller
/// proceeded. A herd of N concurrent acquires after backoff expiry would
/// all hit the backend. The new `admit_through_gate` CAS-claims the probe
/// slot up front, so exactly one caller becomes the probe and the others
/// receive an admission error.
#[tokio::test]
async fn probe_boundary_serializes_callers_under_herd() {
    let manager = Arc::new(Manager::new());

    // Resource always fails transiently — this lets us count how many
    // acquires actually reached `create`.
    let resource = FailingResidentResource::new(u64::MAX);
    let create_counter = resource.create_count.clone();

    let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig {
        max_attempts: 100,
        // Very short backoff so we can unblock quickly, but long enough
        // that the second herd all arrives before any retry.
        base_backoff: std::time::Duration::from_millis(15),
    }));

    let resident_rt = Resident::<FailingResidentResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: Some(gate.clone()),
        })
        .unwrap();

    // First acquire becomes the probe and fails — gate transitions to Failed.
    let ctx = test_ctx();
    let first = manager
        .acquire_resident::<FailingResidentResource>(&ctx, &AcquireOptions::default())
        .await;
    assert!(first.is_err(), "first acquire must fail");
    assert_eq!(
        create_counter.load(Ordering::Relaxed),
        1,
        "first acquire should have called create exactly once"
    );

    // Wait until the gate's backoff expires so the next try_begin from
    // Failed transitions to InProgress.
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;

    // Fire a herd of 64 concurrent acquires. Exactly one must be admitted
    // as the probe (calling `create`); the rest must receive an admission
    // error from the gate without touching the backend. Pre-fix, every
    // caller in the herd saw the same `Failed` snapshot with an expired
    // `retry_at` and proceeded to `create`.
    let before = create_counter.load(Ordering::Relaxed);

    let mut handles = Vec::new();
    for _ in 0..64 {
        let mgr = Arc::clone(&manager);
        handles.push(tokio::spawn(async move {
            let ctx = test_ctx();
            mgr.acquire_resident::<FailingResidentResource>(&ctx, &AcquireOptions::default())
                .await
        }));
    }

    for h in handles {
        let _ = h.await.expect("task must not panic");
    }

    let after = create_counter.load(Ordering::Relaxed);
    let probes = after - before;
    assert_eq!(
        probes, 1,
        "#322: exactly one caller should have been admitted as the probe, got {probes}"
    );
}
