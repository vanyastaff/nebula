//! Basic integration tests for nebula-resource v2.
//!
//! These tests exercise the public API surface across topologies without
//! involving real network resources. Mock resources use simple counters
//! to verify lifecycle semantics.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use nebula_core::{ExecutionId, ResourceKey, resource_key};
use nebula_credential::{Credential, NoCredential};
use nebula_resource::{
    AcquireOptions, Manager, ResourceContext, ScopeLevel, ShutdownConfig,
    error::{Error, ErrorKind},
    guard::ResourceGuard,
    recovery::{GateState, RecoveryGate, RecoveryGateConfig},
    release_queue::ReleaseQueue,
    resource::{Resource, ResourceConfig, ResourceMetadata},
    runtime::{
        TopologyRuntime, exclusive::ExclusiveRuntime, pool::PoolRuntime, resident::ResidentRuntime,
        service::ServiceRuntime, transport::TransportRuntime,
    },
    topology::{
        exclusive,
        exclusive::Exclusive,
        pooled::{BrokenCheck, Pooled, RecycleDecision},
        resident,
        resident::Resident,
        service,
        service::{Service, TokenMode},
        transport,
        transport::Transport,
    },
};

// ---------------------------------------------------------------------------
// Mock resource error
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TestError(String);

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TestError {}

impl From<TestError> for Error {
    fn from(e: TestError) -> Self {
        Error::transient(e.0)
    }
}

// ---------------------------------------------------------------------------
// Mock config
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct TestConfig {
    name: String,
}

nebula_schema::impl_empty_has_schema!(TestConfig);

impl ResourceConfig for TestConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.name.is_empty() {
            return Err(Error::permanent("name must not be empty"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.name.hash(&mut h);
        h.finish()
    }
}

// ---------------------------------------------------------------------------
// Pooled mock resource
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct PoolTestResource {
    create_counter: Arc<AtomicU64>,
    break_flag: Arc<AtomicBool>,
}

impl PoolTestResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            break_flag: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Resource for PoolTestResource {
    type Config = TestConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("test-pool")
    }

    fn create(
        &self,
        _config: &TestConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
        let counter = self.create_counter.clone();
        async move {
            let id = counter.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(AtomicU64::new(id)))
        }
    }

    async fn destroy(&self, _runtime: Arc<AtomicU64>) -> Result<(), TestError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Pooled for PoolTestResource {
    fn is_broken(&self, _runtime: &Arc<AtomicU64>) -> BrokenCheck {
        if self.break_flag.load(Ordering::Relaxed) {
            BrokenCheck::Broken("forced".into())
        } else {
            BrokenCheck::Healthy
        }
    }

    async fn recycle(
        &self,
        _runtime: &Arc<AtomicU64>,
        _metrics: &nebula_resource::topology::pooled::InstanceMetrics,
    ) -> Result<RecycleDecision, TestError> {
        Ok(RecycleDecision::Keep)
    }
}

// Also impl Exclusive so we can test topology mismatch
// (register as Pool, call acquire_exclusive).
impl Exclusive for PoolTestResource {}

// ---------------------------------------------------------------------------
// Resident mock resource
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ResidentTestResource {
    create_counter: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
}

impl ResidentTestResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            alive: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl Resource for ResidentTestResource {
    type Config = TestConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("test-resident")
    }

    fn create(
        &self,
        _config: &TestConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
        let counter = self.create_counter.clone();
        async move {
            let id = counter.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(AtomicU64::new(id)))
        }
    }

    async fn destroy(&self, _runtime: Arc<AtomicU64>) -> Result<(), TestError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for ResidentTestResource {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_ctx() -> ResourceContext {
    use nebula_core::scope::Scope;
    use tokio_util::sync::CancellationToken;
    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

fn test_config() -> TestConfig {
    TestConfig {
        name: "test".into(),
    }
}

// ---------------------------------------------------------------------------
// Pool tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_acquire_use_release_reacquire() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool = PoolRuntime::<PoolTestResource>::new(config, 1);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    // First acquire creates a new instance.
    let handle = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("first acquire should succeed");

    assert_eq!(handle.topology_tag(), nebula_resource::TopologyTag::Pool);
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    // Use the lease.
    let _val = handle.load(Ordering::Relaxed);

    // Release by dropping.
    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Pool should have one idle instance now.
    assert_eq!(pool.idle_count().await, 1);

    // Second acquire reuses the idle instance (no new creation).
    let handle2 = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("second acquire should succeed");

    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        1,
        "should reuse, not create"
    );
    drop(handle2);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

#[tokio::test]
async fn pool_broken_instance_gets_replaced() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 2,
        ..Default::default()
    };
    let pool = PoolRuntime::<PoolTestResource>::new(config, 1);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    // Acquire and release to populate idle queue.
    let handle = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .unwrap();
    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(pool.idle_count().await, 1);

    // Mark as broken.
    resource.break_flag.store(true, Ordering::Relaxed);

    // Next acquire should destroy the broken instance and create new.
    let handle2 = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("should create a fresh instance");

    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        2,
        "broken instance was replaced"
    );

    drop(handle2);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Broken flag still set, so the released instance was destroyed.
    assert_eq!(pool.idle_count().await, 0);

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// Resident tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resident_acquire_creates_then_clones() {
    let resource = ResidentTestResource::new();
    let rt = ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());
    let ctx = test_ctx();

    // First acquire creates.
    let h1 = rt
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &AcquireOptions::default(),
        )
        .await
        .expect("first acquire");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    assert_eq!(h1.topology_tag(), nebula_resource::TopologyTag::Resident);

    // Second acquire clones (no new creation).
    let h2 = rt
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &AcquireOptions::default(),
        )
        .await
        .expect("second acquire");
    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        1,
        "should clone, not create"
    );

    // Both handles reference the same logical value.
    assert_eq!(h1.load(Ordering::Relaxed), h2.load(Ordering::Relaxed));
}

#[tokio::test]
async fn resident_recreates_when_not_alive() {
    let resource = ResidentTestResource::new();
    let config = resident::config::Config {
        recreate_on_failure: true,
        ..Default::default()
    };
    let rt = ResidentRuntime::<ResidentTestResource>::new(config);
    let ctx = test_ctx();

    let _h1 = rt
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &AcquireOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    // Mark not alive.
    resource.alive.store(false, Ordering::Relaxed);

    // Next acquire should recreate.
    let _h2 = rt
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &AcquireOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        2,
        "should have recreated"
    );
}

// ---------------------------------------------------------------------------
// Manager tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manager_register_and_acquire_pooled() {
    let manager = Manager::new();
    let resource = PoolTestResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool_rt = PoolRuntime::<PoolTestResource>::new(pool_config, 1);

    manager
        .register(
            resource.clone(),
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Pool(pool_rt),
            None,
            None,
        )
        .expect("registration should succeed");

    assert!(manager.contains(&resource_key!("test-pool")));

    let ctx = test_ctx();
    let handle: ResourceGuard<PoolTestResource> = manager
        .acquire_pooled(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    assert_eq!(handle.topology_tag(), nebula_resource::TopologyTag::Pool);
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    drop(handle);

    manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(50)),
        )
        .await
        .expect("graceful_shutdown must succeed");
}

#[tokio::test]
async fn manager_register_and_acquire_resident() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource.clone(),
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    assert_eq!(
        handle.topology_tag(),
        nebula_resource::TopologyTag::Resident
    );
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn manager_shutdown_rejects_acquire() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .unwrap();

    manager.shutdown();
    assert!(manager.is_shutdown());

    let ctx = test_ctx();
    let result = manager
        .acquire_resident::<ResidentTestResource>(&(), &ctx, &AcquireOptions::default())
        .await;

    assert!(result.is_err());
    let err = result.expect_err("should be an error");
    assert_eq!(*err.kind(), ErrorKind::Cancelled);
}

// ---------------------------------------------------------------------------
// #390 — pool config validation + max_concurrent_creates enforcement
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_pooled_rejects_min_greater_than_max() {
    let manager = Manager::new();
    let resource = PoolTestResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        min_size: 5,
        max_size: 2,
        ..Default::default()
    };

    let err = manager
        .register_pooled::<PoolTestResource>(resource, test_config(), pool_config)
        .expect_err("min > max must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("min_size") && msg.contains("max_size"),
        "error message must mention min_size and max_size, got: {msg}",
    );
}

#[tokio::test]
async fn register_pooled_rejects_max_size_zero() {
    let manager = Manager::new();
    let resource = PoolTestResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        min_size: 0,
        max_size: 0,
        ..Default::default()
    };

    let err = manager
        .register_pooled::<PoolTestResource>(resource, test_config(), pool_config)
        .expect_err("max_size == 0 must be rejected");
    assert!(err.to_string().contains("max_size"));
}

#[derive(Clone)]
struct SlowCreatePoolResource {
    in_flight: Arc<AtomicU64>,
    peak: Arc<AtomicU64>,
}

impl Resource for SlowCreatePoolResource {
    type Config = TestConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("slow-create-pool")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, TestError> {
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        // Update peak = max(peak, now) via `AtomicU64::update` (Rust 1.95).
        // Load and store orderings both SeqCst — match the prior CAS loop.
        let _ = self
            .peak
            .update(Ordering::SeqCst, Ordering::SeqCst, |cur| cur.max(now));
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        Ok(Arc::new(AtomicU64::new(0)))
    }

    async fn destroy(&self, _runtime: Arc<AtomicU64>) -> Result<(), TestError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Pooled for SlowCreatePoolResource {
    fn is_broken(&self, _runtime: &Arc<AtomicU64>) -> BrokenCheck {
        BrokenCheck::Healthy
    }

    async fn recycle(
        &self,
        _runtime: &Arc<AtomicU64>,
        _metrics: &nebula_resource::topology::pooled::InstanceMetrics,
    ) -> Result<RecycleDecision, TestError> {
        Ok(RecycleDecision::Keep)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pool_create_path_respects_max_concurrent_creates() {
    use nebula_resource::topology::pooled::config::{Config as PoolCfg, WarmupStrategy};

    let resource = SlowCreatePoolResource {
        in_flight: Arc::new(AtomicU64::new(0)),
        peak: Arc::new(AtomicU64::new(0)),
    };
    let peak = resource.peak.clone();

    let manager = Arc::new(Manager::new());
    let pool_config = PoolCfg {
        min_size: 0,
        max_size: 10,
        max_concurrent_creates: 2,
        warmup: WarmupStrategy::None,
        ..Default::default()
    };
    manager
        .register_pooled::<SlowCreatePoolResource>(resource, test_config(), pool_config)
        .expect("register");

    // Fire 10 concurrent acquires so they all hit the create path.
    let mut handles = Vec::new();
    for _ in 0..10 {
        let mgr = Arc::clone(&manager);
        handles.push(tokio::spawn(async move {
            let ctx = test_ctx();
            mgr.acquire_pooled::<SlowCreatePoolResource>(&(), &ctx, &AcquireOptions::default())
                .await
                .expect("acquire")
        }));
    }
    let mut leases = Vec::with_capacity(10);
    for h in handles {
        leases.push(h.await.expect("spawn"));
    }
    drop(leases);

    let observed = peak.load(Ordering::SeqCst);
    assert!(
        observed <= 2,
        "max_concurrent_creates=2 violated — observed peak={observed} (#390)",
    );
    assert!(
        observed > 0,
        "create path never ran — test fixture is broken",
    );
}

// ---------------------------------------------------------------------------
// #387 — ResourceStatus.phase lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_transitions_phase_to_ready() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .expect("register");

    let snap = manager
        .health_check::<ResidentTestResource>(&ScopeLevel::Global)
        .expect("health");
    assert_eq!(snap.phase, nebula_resource::state::ResourcePhase::Ready);
    assert_eq!(snap.generation, 0);
}

#[tokio::test]
async fn reload_config_bumps_status_generation() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .expect("register");

    let updated_config = TestConfig {
        name: "test-v2".into(),
    };
    manager
        .reload_config::<ResidentTestResource>(updated_config, &ScopeLevel::Global)
        .expect("reload");

    let snap = manager
        .health_check::<ResidentTestResource>(&ScopeLevel::Global)
        .expect("health");
    assert_eq!(snap.phase, nebula_resource::state::ResourcePhase::Ready);
    assert_eq!(
        snap.generation, 1,
        "reload_config must bake the new generation into ResourceStatus (#387)",
    );
}

#[tokio::test]
async fn graceful_shutdown_report_marks_registry_cleared() {
    use nebula_resource::manager::ShutdownConfig;

    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .expect("register");

    let report = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect("graceful");
    assert!(report.registry_cleared);
}

#[tokio::test]
async fn remove_nonexistent_returns_not_found() {
    let manager = Manager::new();
    let key = resource_key!("does-not-exist");

    let result = manager.remove(&key);

    assert!(result.is_err());
    let err = result.expect_err("should be an error");
    assert_eq!(*err.kind(), ErrorKind::NotFound);
}

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
// Handle RAII semantics
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tainted_handle_not_recycled() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 2,
        ..Default::default()
    };
    let pool = PoolRuntime::<PoolTestResource>::new(config, 1);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    let mut handle = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .unwrap();

    handle.taint();
    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Tainted handle should NOT be recycled.
    assert_eq!(pool.idle_count().await, 0);

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// Event emission tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_emits_registered_event() {
    let manager = Manager::new();
    let mut rx = manager.subscribe_events();

    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .expect("registration should succeed");

    let event = rx.try_recv().expect("should have received an event");
    assert!(
        matches!(&event, nebula_resource::ResourceEvent::Registered { key } if key == &resource_key!("test-resident")),
        "expected Registered event, got {event:?}"
    );
}

#[tokio::test]
async fn remove_emits_removed_event() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .unwrap();

    let mut rx = manager.subscribe_events();
    let key = resource_key!("test-resident");
    manager.remove(&key).expect("remove should succeed");

    let event = rx.try_recv().expect("should have received an event");
    assert!(
        matches!(&event, nebula_resource::ResourceEvent::Removed { key } if key == &resource_key!("test-resident")),
        "expected Removed event, got {event:?}"
    );
}

#[tokio::test]
async fn acquire_emits_success_event() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .unwrap();

    let mut rx = manager.subscribe_events();
    // Drain the Registered event.
    let _ = rx.try_recv();

    let ctx = test_ctx();
    let _handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    let event = rx.try_recv().expect("should have received an event");
    assert!(
        matches!(&event, nebula_resource::ResourceEvent::AcquireSuccess { key, .. } if key == &resource_key!("test-resident")),
        "expected AcquireSuccess event, got {event:?}"
    );
}

// ---------------------------------------------------------------------------
// Pool concurrency scenarios
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_concurrent_acquire_respects_max_size() {
    let resource = PoolTestResource::new();
    let max_size = 3;
    let config = nebula_resource::topology::pooled::config::Config {
        max_size,
        create_timeout: std::time::Duration::from_millis(200),
        ..Default::default()
    };
    let pool = PoolRuntime::<PoolTestResource>::new(config, 1);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    // Acquire max_size handles concurrently — all should succeed.
    let mut handles = Vec::new();
    for _ in 0..max_size {
        let handle = pool
            .acquire(
                &resource,
                &test_config(),
                &(),
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                None,
            )
            .await
            .expect("acquire within max_size should succeed");
        handles.push(handle);
    }
    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        u64::from(max_size),
    );

    // One more acquire should time out (pool full, short timeout via deadline).
    let opts = AcquireOptions::default()
        .with_deadline(std::time::Instant::now() + std::time::Duration::from_millis(100));
    let result = pool
        .acquire(&resource, &test_config(), &(), &ctx, &rq, 0, &opts, None)
        .await;
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected backpressure error when pool is full"),
    };
    assert_eq!(*err.kind(), ErrorKind::Backpressure);

    drop(handles);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

#[tokio::test]
async fn pool_backpressure_when_full() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 1,
        create_timeout: std::time::Duration::from_millis(200),
        ..Default::default()
    };
    let pool = PoolRuntime::<PoolTestResource>::new(config, 1);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    // Acquire the single slot.
    let _held = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("first acquire should succeed");

    // Short deadline — should get backpressure quickly.
    let opts = AcquireOptions::default()
        .with_deadline(std::time::Instant::now() + std::time::Duration::from_millis(50));
    let result = pool
        .acquire(&resource, &test_config(), &(), &ctx, &rq, 0, &opts, None)
        .await;

    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected backpressure error when pool is full"),
    };
    assert_eq!(*err.kind(), ErrorKind::Backpressure);

    drop(_held);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// Scope-aware lookup
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manager_scope_exact_match() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    let org_id = nebula_core::OrgId::new();
    let scope = ScopeLevel::Organization(org_id);
    manager
        .register(
            resource.clone(),
            test_config(),
            scope.clone(),
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .expect("registration should succeed");

    // Acquire with the same org scope.
    use nebula_core::scope::Scope;
    use tokio_util::sync::CancellationToken;
    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(org_id),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("acquire with matching scope should succeed");

    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    drop(handle);
}

#[tokio::test]
async fn manager_scope_fallback_to_global() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    // Register at Global scope.
    manager
        .register(
            resource.clone(),
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .expect("registration should succeed");

    // Acquire with Organization scope — should fall back to Global.
    use nebula_core::scope::Scope;
    use tokio_util::sync::CancellationToken;
    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(nebula_core::OrgId::new()),
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("acquire should fall back to Global");

    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    drop(handle);
}

#[tokio::test]
async fn manager_scope_mismatch_not_found() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    // Register at Organization(org_id) — no Global fallback.
    let org_id = nebula_core::OrgId::new();
    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Organization(org_id),
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .expect("registration should succeed");

    // Acquire with a different org scope — no match, no Global fallback.
    use nebula_core::scope::Scope;
    use tokio_util::sync::CancellationToken;
    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(nebula_core::OrgId::new()),
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let result = manager
        .acquire_resident::<ResidentTestResource>(&(), &ctx, &AcquireOptions::default())
        .await;

    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected NotFound error for mismatched scope"),
    };
    assert_eq!(*err.kind(), ErrorKind::NotFound);
}

// ---------------------------------------------------------------------------
// Metrics verification
// ---------------------------------------------------------------------------

#[tokio::test]
async fn metrics_track_acquire_release_create_destroy() {
    let registry = Arc::new(nebula_telemetry::metrics::MetricsRegistry::new());
    let manager = Manager::with_config(nebula_resource::ManagerConfig {
        release_queue_workers: 2,
        metrics_registry: Some(registry.clone()),
        ..Default::default()
    });
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .expect("registration should succeed");

    // register calls record_create
    let snap = manager.metrics().expect("metrics present").snapshot();
    assert_eq!(snap.create_total, 1, "register should record create");

    // Acquire.
    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
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
// Multiple resources coexist
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manager_multiple_resources_coexist() {
    let manager = Manager::new();

    // Register a pool resource.
    let pool_resource = PoolTestResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 2,
        ..Default::default()
    };
    let pool_rt = PoolRuntime::<PoolTestResource>::new(pool_config, 1);

    manager
        .register(
            pool_resource.clone(),
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Pool(pool_rt),
            None,
            None,
        )
        .expect("pool registration should succeed");

    // Register a resident resource.
    let resident_resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resident_resource.clone(),
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .expect("resident registration should succeed");

    assert!(manager.contains(&resource_key!("test-pool")));
    assert!(manager.contains(&resource_key!("test-resident")));
    assert_eq!(manager.keys().len(), 2);

    // Acquire each independently.
    let ctx = test_ctx();
    let pool_handle: ResourceGuard<PoolTestResource> = manager
        .acquire_pooled(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("pool acquire should succeed");

    let resident_handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("resident acquire should succeed");

    assert_eq!(
        pool_handle.topology_tag(),
        nebula_resource::TopologyTag::Pool
    );
    assert_eq!(
        resident_handle.topology_tag(),
        nebula_resource::TopologyTag::Resident
    );
    assert_eq!(pool_resource.create_counter.load(Ordering::Relaxed), 1);
    assert_eq!(resident_resource.create_counter.load(Ordering::Relaxed), 1);

    drop(pool_handle);
    drop(resident_handle);

    manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(50)),
        )
        .await
        .expect("graceful_shutdown must succeed");
}

// ---------------------------------------------------------------------------
// AcquireOptions deadline
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_acquire_with_deadline() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 1,
        // Long default timeout — the deadline should override this.
        create_timeout: std::time::Duration::from_secs(30),
        ..Default::default()
    };
    let pool = PoolRuntime::<PoolTestResource>::new(config, 1);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    // Acquire the single slot.
    let _held = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("first acquire should succeed");

    // Very short deadline should override the 30s default timeout.
    let opts = AcquireOptions::default()
        .with_deadline(std::time::Instant::now() + std::time::Duration::from_millis(100));
    let start = std::time::Instant::now();
    let result = pool
        .acquire(&resource, &test_config(), &(), &ctx, &rq, 0, &opts, None)
        .await;

    let elapsed = start.elapsed();
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected backpressure error with short deadline"),
    };
    assert_eq!(*err.kind(), ErrorKind::Backpressure);
    // Should have timed out quickly, not waited 30s.
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "deadline should override default timeout, elapsed: {elapsed:?}"
    );

    drop(_held);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// Handle detach
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_detach_removes_from_pool() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 2,
        ..Default::default()
    };
    let pool = PoolRuntime::<PoolTestResource>::new(config, 1);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    let handle = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("acquire should succeed");

    // Detach — the lease is extracted, on_release callback is disarmed.
    let lease = handle.detach();
    assert!(lease.is_some(), "guarded handle detach should return Some");

    // Give time for any potential (but should-not-happen) release processing.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Pool should NOT get the instance back — idle_count stays 0.
    assert_eq!(
        pool.idle_count().await,
        0,
        "detached handle should not return to pool"
    );

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// Service mock resource
// ---------------------------------------------------------------------------

/// Inner state for the service runtime.
#[derive(Debug)]
struct ServiceInner {
    data: String,
}

/// Token handed out by the service.
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct ServiceToken {
    data: String,
}

#[derive(Clone)]
struct ServiceTestResource {
    create_counter: Arc<AtomicU64>,
    token_counter: Arc<AtomicU64>,
}

impl ServiceTestResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            token_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Resource for ServiceTestResource {
    type Config = TestConfig;
    type Runtime = Arc<ServiceInner>;
    type Lease = ServiceToken;
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("test-service")
    }

    fn create(
        &self,
        _config: &TestConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<ServiceInner>, TestError>> + Send {
        let counter = self.create_counter.clone();
        async move {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(ServiceInner {
                data: "svc-data".into(),
            }))
        }
    }

    async fn destroy(&self, _runtime: Arc<ServiceInner>) -> Result<(), TestError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Service for ServiceTestResource {
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;

    fn acquire_token(
        &self,
        runtime: &Arc<ServiceInner>,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<ServiceToken, TestError>> + Send {
        let token_id = self.token_counter.fetch_add(1, Ordering::Relaxed);
        let data = format!("{}-token-{token_id}", runtime.data);
        async move { Ok(ServiceToken { data }) }
    }
}

// ---------------------------------------------------------------------------
// Transport mock resource
// ---------------------------------------------------------------------------

/// Inner state for the transport runtime.
#[derive(Debug)]
#[allow(dead_code)]
struct TransportInner {
    name: String,
}

/// Session handle returned by the transport.
#[derive(Debug)]
#[allow(dead_code)]
struct SessionHandle {
    id: u64,
}

#[derive(Clone)]
struct TransportTestResource {
    create_counter: Arc<AtomicU64>,
    session_counter: Arc<AtomicU64>,
    close_counter: Arc<AtomicU64>,
}

impl TransportTestResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            session_counter: Arc::new(AtomicU64::new(0)),
            close_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Resource for TransportTestResource {
    type Config = TestConfig;
    type Runtime = Arc<TransportInner>;
    type Lease = SessionHandle;
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("test-transport")
    }

    fn create(
        &self,
        _config: &TestConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<TransportInner>, TestError>> + Send {
        let counter = self.create_counter.clone();
        async move {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(TransportInner {
                name: "transport".into(),
            }))
        }
    }

    async fn destroy(&self, _runtime: Arc<TransportInner>) -> Result<(), TestError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Transport for TransportTestResource {
    fn open_session(
        &self,
        _transport: &Arc<TransportInner>,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<SessionHandle, TestError>> + Send {
        let id = self.session_counter.fetch_add(1, Ordering::Relaxed);
        async move { Ok(SessionHandle { id }) }
    }

    fn close_session(
        &self,
        _transport: &Arc<TransportInner>,
        _session: SessionHandle,
        _healthy: bool,
    ) -> impl Future<Output = Result<(), TestError>> + Send {
        let close_counter = self.close_counter.clone();
        async move {
            close_counter.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Exclusive mock resource
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ExclusiveTestResource {
    create_counter: Arc<AtomicU64>,
    reset_counter: Arc<AtomicU64>,
}

impl ExclusiveTestResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            reset_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Resource for ExclusiveTestResource {
    type Config = TestConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("test-exclusive")
    }

    fn create(
        &self,
        _config: &TestConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
        let counter = self.create_counter.clone();
        async move {
            let id = counter.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(AtomicU64::new(id)))
        }
    }

    async fn destroy(&self, _runtime: Arc<AtomicU64>) -> Result<(), TestError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Exclusive for ExclusiveTestResource {
    fn reset(
        &self,
        _runtime: &Arc<AtomicU64>,
    ) -> impl Future<Output = Result<(), TestError>> + Send {
        self.reset_counter.fetch_add(1, Ordering::Relaxed);
        async { Ok(()) }
    }
}

// ---------------------------------------------------------------------------
// Service tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn service_acquire_cloned_token() {
    let resource = ServiceTestResource::new();
    let runtime = Arc::new(ServiceInner {
        data: "svc-data".into(),
    });
    let rt =
        ServiceRuntime::<ServiceTestResource>::new(runtime, service::config::Config::default());
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    // Acquire first token.
    let h1 = rt
        .acquire(&resource, &ctx, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first acquire should succeed");

    assert_eq!(h1.topology_tag(), nebula_resource::TopologyTag::Service);
    // Owned handle (Cloned mode) — generation is None.
    assert!(h1.generation().is_none());

    // Acquire second token concurrently — both should succeed.
    let h2 = rt
        .acquire(&resource, &ctx, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("second acquire should succeed");

    // Tokens are distinct (different token_counter values).
    assert_eq!(resource.token_counter.load(Ordering::Relaxed), 2);

    drop(h1);
    drop(h2);

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

#[tokio::test]
async fn service_acquire_via_manager() {
    let manager = Manager::new();
    let resource = ServiceTestResource::new();
    let runtime = Arc::new(ServiceInner {
        data: "managed-svc".into(),
    });
    let svc_rt =
        ServiceRuntime::<ServiceTestResource>::new(runtime, service::config::Config::default());

    manager
        .register(
            resource.clone(),
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Service(svc_rt),
            None,
            None,
        )
        .expect("registration should succeed");

    assert!(manager.contains(&resource_key!("test-service")));

    let ctx = test_ctx();
    let handle: ResourceGuard<ServiceTestResource> = manager
        .acquire_service(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    assert_eq!(handle.topology_tag(), nebula_resource::TopologyTag::Service);
    assert_eq!(resource.token_counter.load(Ordering::Relaxed), 1);
}

// ---------------------------------------------------------------------------
// Transport tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn transport_acquire_opens_session() {
    let resource = TransportTestResource::new();
    let runtime = Arc::new(TransportInner {
        name: "test-conn".into(),
    });
    let config = transport::config::Config {
        max_sessions: 10,
        ..Default::default()
    };
    let rt = TransportRuntime::<TransportTestResource>::new(runtime, config);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    let handle = rt
        .acquire(&resource, &ctx, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("acquire should succeed");

    assert_eq!(
        handle.topology_tag(),
        nebula_resource::TopologyTag::Transport
    );
    assert_eq!(resource.session_counter.load(Ordering::Relaxed), 1);

    // Drop triggers close_session via release queue.
    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert_eq!(resource.close_counter.load(Ordering::Relaxed), 1);

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

#[tokio::test]
async fn transport_session_bounded_by_semaphore() {
    let resource = TransportTestResource::new();
    let runtime = Arc::new(TransportInner {
        name: "bounded-conn".into(),
    });
    let config = transport::config::Config {
        max_sessions: 2,
        keepalive_interval: None,
        ..Default::default()
    };
    let rt = TransportRuntime::<TransportTestResource>::new(runtime, config);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    // Acquire two sessions — should both succeed (max_sessions = 2).
    let h1 = rt
        .acquire(&resource, &ctx, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first session");

    let h2 = rt
        .acquire(&resource, &ctx, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("second session");

    assert_eq!(resource.session_counter.load(Ordering::Relaxed), 2);

    // Third acquire should block because semaphore is exhausted.
    let rt_ref = &rt;
    let resource_ref = &resource;
    let rq_ref = &rq;
    let ctx_ref = &ctx;

    let result = tokio::time::timeout(std::time::Duration::from_millis(100), async {
        rt_ref
            .acquire(
                resource_ref,
                ctx_ref,
                rq_ref,
                0,
                &AcquireOptions::default(),
                None,
            )
            .await
    })
    .await;

    assert!(result.is_err(), "third acquire should have timed out");

    // Release one session — frees a semaphore permit.
    drop(h1);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Now third acquire should succeed.
    let h3 = rt
        .acquire(&resource, &ctx, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("third session after release");

    assert_eq!(resource.session_counter.load(Ordering::Relaxed), 3);

    drop(h2);
    drop(h3);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

#[tokio::test]
async fn transport_acquire_timeout_when_sessions_exhausted() {
    let resource = TransportTestResource::new();
    let runtime = Arc::new(TransportInner {
        name: "timeout-conn".into(),
    });
    let config = transport::config::Config {
        max_sessions: 1,
        keepalive_interval: None,
        acquire_timeout: std::time::Duration::from_millis(50),
    };
    let rt = TransportRuntime::<TransportTestResource>::new(runtime, config);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    // Hold the only session.
    let _held = rt
        .acquire(&resource, &ctx, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first acquire should succeed");

    // Second acquire must time out (no deadline override — uses config timeout).
    let result = rt
        .acquire(&resource, &ctx, &rq, 0, &AcquireOptions::default(), None)
        .await;
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("second acquire should time out"),
    };

    assert!(
        matches!(err.kind(), ErrorKind::Backpressure),
        "expected Backpressure, got {err:?}",
    );

    // With an explicit deadline the caller-supplied timeout is respected.
    let short_deadline = std::time::Instant::now() + std::time::Duration::from_millis(25);
    let opts = AcquireOptions::default().with_deadline(short_deadline);
    let result2 = rt.acquire(&resource, &ctx, &rq, 0, &opts, None).await;
    let err2 = match result2 {
        Err(e) => e,
        Ok(_) => panic!("deadline acquire should time out"),
    };

    assert!(
        matches!(err2.kind(), ErrorKind::Backpressure),
        "expected Backpressure with deadline, got {err2:?}",
    );

    drop(_held);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// Exclusive tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exclusive_acquire_one_at_a_time() {
    let resource = ExclusiveTestResource::new();
    let runtime = Arc::new(AtomicU64::new(42));
    let rt = ExclusiveRuntime::<ExclusiveTestResource>::new(
        runtime,
        exclusive::config::Config::default(),
    );
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    // First acquire succeeds.
    let h1 = rt
        .acquire(&resource, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first acquire should succeed");

    assert_eq!(h1.topology_tag(), nebula_resource::TopologyTag::Exclusive);

    // Second acquire should block (semaphore has 1 permit).
    let rt_ref = &rt;
    let resource_ref = &resource;
    let rq_ref = &rq;

    let result = tokio::time::timeout(std::time::Duration::from_millis(100), async {
        rt_ref
            .acquire(resource_ref, rq_ref, 0, &AcquireOptions::default(), None)
            .await
    })
    .await;

    assert!(result.is_err(), "second acquire should have timed out");

    // Release the first handle.
    drop(h1);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Now second acquire should succeed.
    let h2 = rt
        .acquire(&resource, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("second acquire after release");

    assert_eq!(h2.topology_tag(), nebula_resource::TopologyTag::Exclusive);

    drop(h2);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

#[tokio::test]
async fn exclusive_reset_called_on_release() {
    let resource = ExclusiveTestResource::new();
    let runtime = Arc::new(AtomicU64::new(0));
    let rt = ExclusiveRuntime::<ExclusiveTestResource>::new(
        runtime,
        exclusive::config::Config::default(),
    );
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let handle = rt
        .acquire(&resource, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("acquire should succeed");

    assert_eq!(resource.reset_counter.load(Ordering::Relaxed), 0);

    // Drop triggers reset via release queue.
    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert_eq!(
        resource.reset_counter.load(Ordering::Relaxed),
        1,
        "reset should have been called once after release"
    );

    // Acquire and release again to confirm reset increments.
    let handle2 = rt
        .acquire(&resource, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("second acquire");

    drop(handle2);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert_eq!(
        resource.reset_counter.load(Ordering::Relaxed),
        2,
        "reset should have been called twice"
    );

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

#[tokio::test]
async fn exclusive_acquire_timeout_when_locked() {
    let resource = ExclusiveTestResource::new();
    let runtime = Arc::new(AtomicU64::new(0));
    let config = exclusive::config::Config {
        acquire_timeout: std::time::Duration::from_millis(50),
    };
    let rt = ExclusiveRuntime::<ExclusiveTestResource>::new(runtime, config);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    // Hold the exclusive lock.
    let _h1 = rt
        .acquire(&resource, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first acquire should succeed");

    // Second acquire should time out via config timeout.
    let result = rt
        .acquire(&resource, &rq, 0, &AcquireOptions::default(), None)
        .await;
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("second acquire should time out"),
    };
    assert!(
        matches!(err.kind(), ErrorKind::Backpressure),
        "expected Backpressure, got {:?}",
        err.kind(),
    );

    // Also test deadline-based timeout via AcquireOptions.
    let short_deadline = AcquireOptions::default()
        .with_deadline(std::time::Instant::now() + std::time::Duration::from_millis(10));
    let result2 = rt.acquire(&resource, &rq, 0, &short_deadline, None).await;
    let err2 = match result2 {
        Err(e) => e,
        Ok(_) => panic!("deadline acquire should time out"),
    };
    assert!(
        matches!(err2.kind(), ErrorKind::Backpressure),
        "expected Backpressure, got {:?}",
        err2.kind(),
    );

    drop(_h1);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// #384 — exclusive permit held until reset() completes
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct SlowResetExclusive {
    in_progress: Arc<AtomicBool>,
    overlap_observed: Arc<AtomicBool>,
}

impl Resource for SlowResetExclusive {
    type Config = TestConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("slow-reset-exclusive")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, TestError> {
        Ok(Arc::new(AtomicU64::new(0)))
    }

    async fn destroy(&self, _runtime: Arc<AtomicU64>) -> Result<(), TestError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Exclusive for SlowResetExclusive {
    fn reset(
        &self,
        _runtime: &Arc<AtomicU64>,
    ) -> impl Future<Output = Result<(), TestError>> + Send {
        let in_progress = self.in_progress.clone();
        async move {
            in_progress.store(true, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            in_progress.store(false, Ordering::SeqCst);
            Ok(())
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exclusive_next_acquire_waits_until_reset_completes() {
    // #384: verify a second acquire cannot enter the critical section
    // while a previous reset() is still running asynchronously. The
    // resource's reset() holds `in_progress = true` for ~150ms, so under
    // the fix the next acquire must block for at least that long.
    let resource = SlowResetExclusive {
        in_progress: Arc::new(AtomicBool::new(false)),
        overlap_observed: Arc::new(AtomicBool::new(false)),
    };
    let runtime = Arc::new(AtomicU64::new(0));
    let rt =
        ExclusiveRuntime::<SlowResetExclusive>::new(runtime, exclusive::config::Config::default());
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let h1 = rt
        .acquire(&resource, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first acquire");
    drop(h1);

    // Give the release queue worker time to pick up the reset task and
    // start its sleep. Without the fix the permit was already dropped by
    // GuardInner::Guarded's Drop; with the fix the permit moved into the
    // reset future and is still held.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let start = std::time::Instant::now();
    let h2 = rt
        .acquire(&resource, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("second acquire");
    let elapsed = start.elapsed();

    // Any overlap with the in-flight reset is a bug.
    if resource.in_progress.load(Ordering::SeqCst) {
        resource.overlap_observed.store(true, Ordering::SeqCst);
    }
    assert!(
        !resource.overlap_observed.load(Ordering::SeqCst),
        "second acquire raced against an in-flight reset() — permit was \
         returned before reset completed (#384)",
    );
    assert!(
        elapsed >= std::time::Duration::from_millis(80),
        "second acquire must block until reset() finishes (~150ms), \
         actually blocked {elapsed:?} — the fix is missing (#384)",
    );

    drop(h2);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// Pool permit leak regression test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_permit_not_leaked_after_release() {
    // Pool with max_size=1. Acquire, drop, acquire again.
    // If the permit leaked, the second acquire would block forever.
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 1,
        ..Default::default()
    };
    let pool = PoolRuntime::<PoolTestResource>::new(config, 1);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();
    let handle = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("first acquire should succeed");
    drop(handle);
    // Permit should be returned immediately on handle drop (not after async
    // recycle). A short sleep ensures the drop has executed.
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    // Second acquire must succeed — permit was returned.
    let handle2 = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("second acquire must not block — permit should be available");
    drop(handle2);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// Registry-backed metrics tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn registry_backed_metrics_record_operations() {
    let registry = Arc::new(nebula_telemetry::metrics::MetricsRegistry::new());
    let manager = Manager::with_config(nebula_resource::ManagerConfig {
        release_queue_workers: 1,
        metrics_registry: Some(registry.clone()),
        ..Default::default()
    });

    // Register two resources.
    let pool_resource = PoolTestResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool_rt = PoolRuntime::<PoolTestResource>::new(pool_config, 1);

    manager
        .register(
            pool_resource.clone(),
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Pool(pool_rt),
            None,
            None,
        )
        .expect("pool registration should succeed");

    let resident_resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resident_resource.clone(),
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .expect("resident registration should succeed");

    // Acquire the pooled resource once.
    let ctx = test_ctx();
    let handle: ResourceGuard<PoolTestResource> = manager
        .acquire_pooled(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("pool acquire should succeed");
    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

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
    let create_counter = registry.counter(nebula_metrics::naming::NEBULA_RESOURCE_CREATE_TOTAL);
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

// ---------------------------------------------------------------------------
// Graceful shutdown tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graceful_shutdown_stops_new_acquires() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
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
        .acquire_resident::<ResidentTestResource>(&(), &ctx, &AcquireOptions::default())
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
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
// AcquireResilience tests
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

impl Resource for FailingResidentResource {
    type Config = TestConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("test-failing-resident")
    }

    fn create(
        &self,
        _config: &TestConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
        let count = self.create_count.fetch_add(1, Ordering::Relaxed);
        let threshold = self.failures_before_success;
        async move {
            if count < threshold {
                Err(TestError("transient failure".into()))
            } else {
                Ok(Arc::new(AtomicU64::new(count)))
            }
        }
    }

    async fn destroy(&self, _runtime: Arc<AtomicU64>) -> Result<(), TestError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for FailingResidentResource {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

/// Error that maps to a permanent (non-retryable) resource error.
#[derive(Debug, Clone)]
struct PermanentTestError(String);

impl std::fmt::Display for PermanentTestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PermanentTestError {}

impl From<PermanentTestError> for Error {
    fn from(e: PermanentTestError) -> Self {
        Error::permanent(e.0)
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

impl Resource for PermanentFailResource {
    type Config = TestConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = PermanentTestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("test-permanent-fail")
    }

    fn create(
        &self,
        _config: &TestConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, PermanentTestError>> + Send {
        self.create_count.fetch_add(1, Ordering::Relaxed);
        async { Err(PermanentTestError("permanent failure".into())) }
    }

    async fn destroy(&self, _runtime: Arc<AtomicU64>) -> Result<(), PermanentTestError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for PermanentFailResource {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        true
    }
}

#[tokio::test]
async fn acquire_retries_on_transient_failure() {
    use nebula_resource::integration::{AcquireResilience, AcquireRetryConfig};

    let manager = Manager::new();
    // Fails on first create, succeeds on second.
    let resource = FailingResidentResource::new(1);
    let resident_rt =
        ResidentRuntime::<FailingResidentResource>::new(resident::config::Config::default());

    let resilience = AcquireResilience {
        timeout: None,
        retry: Some(AcquireRetryConfig {
            max_attempts: 3,
            initial_backoff: std::time::Duration::from_millis(1),
            max_backoff: std::time::Duration::from_millis(10),
        }),
    };

    manager
        .register(
            resource.clone(),
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            Some(resilience),
            None,
        )
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceGuard<FailingResidentResource> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed after retry");

    // The first create failed, the second succeeded (value = 1).
    assert_eq!(handle.load(Ordering::Relaxed), 1);
    // Two creates total: one failure + one success.
    assert_eq!(resource.create_count.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn acquire_no_retry_on_permanent_failure() {
    use nebula_resource::integration::{AcquireResilience, AcquireRetryConfig};

    let manager = Manager::new();
    let resource = PermanentFailResource::new();
    let resident_rt =
        ResidentRuntime::<PermanentFailResource>::new(resident::config::Config::default());

    let resilience = AcquireResilience {
        timeout: None,
        retry: Some(AcquireRetryConfig {
            max_attempts: 3,
            initial_backoff: std::time::Duration::from_millis(1),
            max_backoff: std::time::Duration::from_millis(10),
        }),
    };

    manager
        .register(
            resource.clone(),
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            Some(resilience),
            None,
        )
        .expect("registration should succeed");

    let ctx = test_ctx();
    let result = manager
        .acquire_resident::<PermanentFailResource>(&(), &ctx, &AcquireOptions::default())
        .await;

    assert!(result.is_err(), "acquire should fail on permanent error");
    // Permanent error is NOT retryable — only 1 attempt should have been made.
    assert_eq!(
        resource.create_count.load(Ordering::Relaxed),
        1,
        "permanent error should not trigger retries"
    );
}

#[tokio::test]
async fn acquire_succeeds_without_resilience() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource.clone(),
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("acquire without resilience should succeed");

    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    drop(handle);
}

#[tokio::test]
async fn acquire_timeout_fires() {
    use nebula_resource::integration::AcquireResilience;

    let manager = Manager::new();
    // Resource that always fails (so the resident runtime create will fail),
    // but we set a very short timeout.
    let resource = FailingResidentResource::new(100);
    let resident_rt = ResidentRuntime::<FailingResidentResource>::new(resident::config::Config {
        // Set create_timeout long so the resilience timeout fires first.
        create_timeout: std::time::Duration::from_mins(1),
        ..Default::default()
    });

    let resilience = AcquireResilience {
        timeout: Some(std::time::Duration::from_millis(1)),
        retry: None,
    };

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            Some(resilience),
            None,
        )
        .expect("registration should succeed");

    let ctx = test_ctx();
    let result = manager
        .acquire_resident::<FailingResidentResource>(&(), &ctx, &AcquireOptions::default())
        .await;

    // Should fail — either from timeout or from the transient error.
    assert!(result.is_err(), "acquire should fail with timeout or error");
}

// ---------------------------------------------------------------------------
// graceful_shutdown edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graceful_shutdown_second_call_errors_already_shutting_down() {
    use nebula_resource::manager::ShutdownError;

    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
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
// Topology mismatch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn topology_mismatch_returns_permanent_error() {
    let manager = Manager::new();
    let resource = PoolTestResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 2,
        ..Default::default()
    };
    let pool_rt = PoolRuntime::<PoolTestResource>::new(pool_config, 1);

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Pool(pool_rt),
            None,
            None,
        )
        .unwrap();

    let ctx = test_ctx();

    // Pool resource, but we call acquire_exclusive — wrong topology.
    let result = manager
        .acquire_exclusive::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await;

    match result {
        Err(e) => assert!(
            matches!(e.kind(), ErrorKind::Permanent),
            "topology mismatch should be a permanent error, got {:?}",
            e.kind()
        ),
        Ok(_) => panic!("wrong topology should fail"),
    }
}

// ---------------------------------------------------------------------------
// Retry exhaustion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn retry_exhaustion_returns_last_transient_error() {
    use nebula_resource::integration::{AcquireResilience, AcquireRetryConfig};

    let manager = Manager::new();
    // Always fails — failures_before_success > max_attempts.
    let resource = FailingResidentResource::new(100);
    let resident_rt =
        ResidentRuntime::<FailingResidentResource>::new(resident::config::Config::default());

    let resilience = AcquireResilience {
        timeout: None,
        retry: Some(AcquireRetryConfig {
            max_attempts: 3,
            initial_backoff: std::time::Duration::from_millis(1),
            max_backoff: std::time::Duration::from_millis(5),
        }),
    };

    manager
        .register(
            resource.clone(),
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            Some(resilience),
            None,
        )
        .unwrap();

    let ctx = test_ctx();
    let result = manager
        .acquire_resident::<FailingResidentResource>(&(), &ctx, &AcquireOptions::default())
        .await;

    // All 3 attempts should have been made.
    assert_eq!(
        resource.create_count.load(Ordering::Relaxed),
        3,
        "should exhaust all max_attempts"
    );

    // The error should be the transient failure, not a generic message.
    match result {
        Err(e) => assert!(
            matches!(e.kind(), ErrorKind::Transient),
            "exhausted retries should return last error kind, got {:?}",
            e.kind()
        ),
        Ok(_) => panic!("all attempts should fail"),
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
    let resident_rt =
        ResidentRuntime::<FailingResidentResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            Some(gate.clone()),
        )
        .unwrap();

    let ctx = test_ctx();

    // First acquire fails — should trigger the gate.
    let _ = manager
        .acquire_resident::<FailingResidentResource>(&(), &ctx, &AcquireOptions::default())
        .await;

    // Gate should no longer be Idle.
    assert!(
        !matches!(gate.state(), GateState::Idle),
        "gate should have been triggered by transient acquire failure, got {:?}",
        gate.state()
    );
}

// ---------------------------------------------------------------------------
// 1. Panic in release callback doesn't abort
// ---------------------------------------------------------------------------

/// A minimal resource for handle-level tests that don't need a pool.
#[derive(Clone)]
struct HandleDummyResource;

impl Resource for HandleDummyResource {
    type Config = TestConfig;
    type Runtime = u32;
    type Lease = u32;
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("handle-dummy")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> Result<u32, TestError> {
        Ok(1)
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

#[test]
fn panic_in_release_callback_does_not_abort() {
    // Create a guarded handle with a callback that panics.
    // Drop the handle. Process must not abort.
    // catch_unwind in Drop should catch it.
    use std::sync::atomic::{AtomicBool, Ordering};

    let callback_entered = Arc::new(AtomicBool::new(false));
    let entered = callback_entered.clone();

    {
        let _handle = ResourceGuard::<HandleDummyResource>::guarded(
            42,
            resource_key!("handle-dummy"),
            nebula_resource::TopologyTag::Pool,
            1,
            move |_lease, _tainted| {
                entered.store(true, Ordering::Relaxed);
                panic!("intentional panic in release callback");
            },
        );
    }
    // If we get here, the process didn't abort.
    assert!(
        callback_entered.load(Ordering::Relaxed),
        "callback should have been invoked before the panic was caught"
    );
}

// ---------------------------------------------------------------------------
// 2. Pool stale fingerprint evicts idle entry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_stale_fingerprint_evicts_idle_entry() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool = PoolRuntime::<PoolTestResource>::new(config, 1);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    // Acquire + release to populate idle.
    let handle = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("first acquire should succeed");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(pool.idle_count().await, 1);

    // Change fingerprint — makes the idle entry stale.
    pool.set_fingerprint(999);

    // Next acquire should destroy stale entry and create fresh.
    let handle2 = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("second acquire should succeed after fingerprint change");

    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        2,
        "stale fingerprint should have forced a fresh creation"
    );

    drop(handle2);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// 3. Pool max_lifetime evicts expired entry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_max_lifetime_evicts_expired_entry() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        max_lifetime: Some(std::time::Duration::from_millis(50)),
        ..Default::default()
    };
    let pool = PoolRuntime::<PoolTestResource>::new(config, 1);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    // Acquire + release to populate idle.
    let handle = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("first acquire should succeed");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(pool.idle_count().await, 1);

    // Sleep past max_lifetime.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Next acquire should destroy expired entry and create fresh.
    let handle2 = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("second acquire should succeed after max_lifetime expiry");

    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        2,
        "expired entry should have forced a fresh creation"
    );

    drop(handle2);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// 4. Pool recycle Drop decision destroys entry
// ---------------------------------------------------------------------------

/// A pool resource whose `recycle()` always returns `RecycleDecision::Drop`.
#[derive(Clone)]
struct DropOnRecycleResource {
    create_counter: Arc<AtomicU64>,
}

impl DropOnRecycleResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Resource for DropOnRecycleResource {
    type Config = TestConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("drop-on-recycle")
    }

    fn create(
        &self,
        _config: &TestConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
        let counter = self.create_counter.clone();
        async move {
            let id = counter.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(AtomicU64::new(id)))
        }
    }

    async fn destroy(&self, _runtime: Arc<AtomicU64>) -> Result<(), TestError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Pooled for DropOnRecycleResource {
    async fn recycle(
        &self,
        _runtime: &Arc<AtomicU64>,
        _metrics: &nebula_resource::topology::pooled::InstanceMetrics,
    ) -> Result<RecycleDecision, TestError> {
        Ok(RecycleDecision::Drop)
    }
}

#[tokio::test]
async fn pool_recycle_drop_destroys_entry() {
    let resource = DropOnRecycleResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool = PoolRuntime::<DropOnRecycleResource>::new(config, 1);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    // Acquire + release. Entry should NOT return to idle because recycle
    // returns Drop.
    let handle = pool
        .acquire(
            &resource,
            &test_config(),
            &(),
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("acquire should succeed");

    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert_eq!(
        pool.idle_count().await,
        0,
        "recycle=Drop should not return entry to idle"
    );

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// 5. Transport: open_session failure frees permit
// ---------------------------------------------------------------------------

/// A transport resource whose `open_session` always fails.
#[derive(Clone)]
struct FailingSessionTransport;

impl Resource for FailingSessionTransport {
    type Config = TestConfig;
    type Runtime = u32;
    type Lease = u32;
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("failing-session")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> Result<u32, TestError> {
        Ok(1)
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Transport for FailingSessionTransport {
    async fn open_session(
        &self,
        _transport: &u32,
        _ctx: &ResourceContext,
    ) -> Result<u32, TestError> {
        Err(TestError("session open failed".into()))
    }
}

#[tokio::test]
async fn transport_open_session_failure_frees_permit() {
    let resource = FailingSessionTransport;
    let config = transport::config::Config {
        max_sessions: 1,
        ..Default::default()
    };
    let transport_rt = TransportRuntime::<FailingSessionTransport>::new(1u32, config);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    // First acquire should fail (open_session errors).
    let result = transport_rt
        .acquire(&resource, &ctx, &rq, 0, &AcquireOptions::default(), None)
        .await;
    assert!(result.is_err(), "open_session should fail");

    // Second acquire should also fail — but NOT hang waiting for the permit.
    // If the permit was leaked by the first failure, this would timeout.
    let result2 = transport_rt
        .acquire(
            &resource,
            &ctx,
            &rq,
            0,
            &AcquireOptions::default()
                .with_deadline(std::time::Instant::now() + std::time::Duration::from_millis(200)),
            None,
        )
        .await;
    assert!(
        result2.is_err(),
        "should fail again (still a bad resource), but not timeout waiting for permit"
    );
    // Verify it's a transient error (from open_session), not backpressure (from semaphore timeout).
    let err = result2.expect_err("already asserted is_err above");
    assert_ne!(
        *err.kind(),
        ErrorKind::Backpressure,
        "should be a transient error from open_session, not a backpressure/timeout — permit was freed"
    );

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// 6. Exclusive: reset failure is silent, doesn't block next acquire
// ---------------------------------------------------------------------------

/// An exclusive resource whose `reset()` always fails.
#[derive(Clone)]
struct FailingResetExclusive;

impl Resource for FailingResetExclusive {
    type Config = TestConfig;
    type Runtime = u32;
    type Lease = u32;
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("failing-reset")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> Result<u32, TestError> {
        Ok(1)
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Exclusive for FailingResetExclusive {
    async fn reset(&self, _runtime: &u32) -> Result<(), TestError> {
        Err(TestError("reset failed".into()))
    }
}

#[tokio::test]
async fn exclusive_reset_failure_does_not_block_next_acquire() {
    let resource = FailingResetExclusive;
    let config = exclusive::config::Config {
        acquire_timeout: std::time::Duration::from_secs(5),
    };
    let exclusive_rt = ExclusiveRuntime::<FailingResetExclusive>::new(1u32, config);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    // First acquire should succeed.
    let handle = exclusive_rt
        .acquire(&resource, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first acquire should succeed");

    // Drop the handle — this triggers reset() which fails.
    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Second acquire should still succeed — the permit was returned despite
    // the reset failure.
    let handle2 = exclusive_rt
        .acquire(
            &resource,
            &rq,
            0,
            &AcquireOptions::default()
                .with_deadline(std::time::Instant::now() + std::time::Duration::from_millis(500)),
            None,
        )
        .await;
    assert!(
        handle2.is_ok(),
        "second acquire should succeed despite reset failure: {:?}",
        handle2.err()
    );

    drop(handle2);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// Recovery gate integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn recovery_gate_blocks_acquire_when_permanently_failed() {
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    let gate = RecoveryGate::new(RecoveryGateConfig::default());
    // Force permanent failure.
    let ticket = gate.try_begin().expect("gate starts idle");
    ticket.fail_permanent("backend certificate expired");

    let manager = Manager::new();
    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            Some(Arc::new(gate)),
        )
        .expect("registration should succeed");

    let ctx = test_ctx();
    let result: Result<ResourceGuard<ResidentTestResource>, _> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    let gate = RecoveryGate::new(RecoveryGateConfig::default());
    // Hold the ticket — gate is InProgress.
    let _ticket = gate.try_begin().expect("gate starts idle");

    let manager = Manager::new();
    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            Some(Arc::new(gate)),
        )
        .expect("registration should succeed");

    let ctx = test_ctx();
    let result: Result<ResourceGuard<ResidentTestResource>, _> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
        .await;

    let err = result.expect_err("acquire should fail when gate is in progress");
    assert_eq!(
        *err.kind(),
        ErrorKind::Transient,
        "should be a transient error"
    );
}

#[tokio::test]
async fn recovery_gate_allows_acquire_when_idle() {
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    let gate = RecoveryGate::new(RecoveryGateConfig::default());

    let manager = Manager::new();
    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            Some(Arc::new(gate)),
        )
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed when gate is idle");
    drop(handle);
}

#[tokio::test]
async fn recovery_gate_allows_acquire_after_backoff_expires() {
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    let gate = RecoveryGate::new(RecoveryGateConfig {
        max_attempts: 5,
        base_backoff: std::time::Duration::from_millis(0), // instant expiry
    });
    // Fail transiently — backoff is 0ms, so retry_at is already in the past.
    let ticket = gate.try_begin().expect("gate starts idle");
    ticket.fail_transient("connection refused");

    let manager = Manager::new();
    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            Some(Arc::new(gate)),
        )
        .expect("registration should succeed");

    let ctx = test_ctx();
    // Backoff expired, so acquire should proceed (caller acts as probe).
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed after backoff expires");
    drop(handle);
}

#[tokio::test]
async fn recovery_gate_none_does_not_affect_acquire() {
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    let manager = Manager::new();
    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None, // no recovery gate
        )
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed without recovery gate");
    drop(handle);
}

// ---------------------------------------------------------------------------
// Config hot-reload tests
// ---------------------------------------------------------------------------

/// Config with a controllable fingerprint for reload tests.
#[derive(Clone, Debug)]
struct ReloadConfig {
    fingerprint: u64,
    valid: bool,
}

nebula_schema::impl_empty_has_schema!(ReloadConfig);

impl ReloadConfig {
    fn new(fingerprint: u64) -> Self {
        Self {
            fingerprint,
            valid: true,
        }
    }

    fn invalid() -> Self {
        Self {
            fingerprint: 0,
            valid: false,
        }
    }
}

impl ResourceConfig for ReloadConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.valid {
            Ok(())
        } else {
            Err(Error::permanent("invalid config"))
        }
    }

    fn fingerprint(&self) -> u64 {
        self.fingerprint
    }
}

/// Minimal pooled resource for reload tests.
#[derive(Clone)]
struct ReloadPoolResource {
    create_counter: Arc<AtomicU64>,
}

impl ReloadPoolResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Resource for ReloadPoolResource {
    type Config = ReloadConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = TestError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        resource_key!("test-reload-pool")
    }

    fn create(
        &self,
        _config: &ReloadConfig,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
        let counter = self.create_counter.clone();
        async move {
            let id = counter.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(AtomicU64::new(id)))
        }
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Pooled for ReloadPoolResource {
    fn is_broken(&self, _runtime: &Arc<AtomicU64>) -> BrokenCheck {
        BrokenCheck::Healthy
    }
}

#[tokio::test]
async fn reload_config_swaps_config_and_bumps_generation() {
    let manager = Manager::new();
    let resource = ReloadPoolResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool_rt = PoolRuntime::<ReloadPoolResource>::new(pool_config, 1);

    manager
        .register(
            resource,
            ReloadConfig::new(1),
            ScopeLevel::Global,
            TopologyRuntime::Pool(pool_rt),
            None,
            None,
        )
        .expect("register should succeed");

    // Check initial generation.
    let managed = manager
        .lookup::<ReloadPoolResource>(&ScopeLevel::Global)
        .expect("lookup should succeed");
    assert_eq!(managed.generation(), 0);
    assert_eq!(managed.config().fingerprint, 1);

    // Reload with new config.
    manager
        .reload_config::<ReloadPoolResource>(ReloadConfig::new(42), &ScopeLevel::Global)
        .expect("reload should succeed");

    assert_eq!(managed.generation(), 1);
    assert_eq!(managed.config().fingerprint, 42);
}

#[tokio::test]
async fn reload_config_rejects_invalid_config() {
    let manager = Manager::new();
    let resource = ReloadPoolResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool_rt = PoolRuntime::<ReloadPoolResource>::new(pool_config, 1);

    manager
        .register(
            resource,
            ReloadConfig::new(1),
            ScopeLevel::Global,
            TopologyRuntime::Pool(pool_rt),
            None,
            None,
        )
        .expect("register should succeed");

    // Reload with invalid config — should fail.
    let result =
        manager.reload_config::<ReloadPoolResource>(ReloadConfig::invalid(), &ScopeLevel::Global);
    assert!(result.is_err());
    assert_eq!(*result.unwrap_err().kind(), ErrorKind::Permanent);

    // Original config still intact.
    let managed = manager
        .lookup::<ReloadPoolResource>(&ScopeLevel::Global)
        .expect("lookup should succeed");
    assert_eq!(
        managed.generation(),
        0,
        "generation should not change on failure"
    );
    assert_eq!(
        managed.config().fingerprint,
        1,
        "config should not change on failure"
    );
}

#[tokio::test]
async fn reload_config_emits_event() {
    let manager = Manager::new();
    let mut rx = manager.subscribe_events();
    let resource = ReloadPoolResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool_rt = PoolRuntime::<ReloadPoolResource>::new(pool_config, 1);

    manager
        .register(
            resource,
            ReloadConfig::new(1),
            ScopeLevel::Global,
            TopologyRuntime::Pool(pool_rt),
            None,
            None,
        )
        .expect("register should succeed");

    // Drain the Registered event.
    let _ = rx.recv().await.expect("should receive Registered event");

    manager
        .reload_config::<ReloadPoolResource>(ReloadConfig::new(99), &ScopeLevel::Global)
        .expect("reload should succeed");

    let event = rx.recv().await.expect("should receive event");
    assert!(
        matches!(event, nebula_resource::ResourceEvent::ConfigReloaded { ref key } if key == &resource_key!("test-reload-pool")),
        "expected ConfigReloaded event, got {event:?}"
    );
}

#[tokio::test]
async fn reload_config_evicts_stale_pool_instances() {
    let manager = Manager::new();
    let resource = ReloadPoolResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool_rt = PoolRuntime::<ReloadPoolResource>::new(pool_config, 1);

    manager
        .register(
            resource.clone(),
            ReloadConfig::new(1),
            ScopeLevel::Global,
            TopologyRuntime::Pool(pool_rt),
            None,
            None,
        )
        .expect("register should succeed");

    let ctx = test_ctx();

    // Acquire and release to populate idle queue with fingerprint=1.
    let handle: ResourceGuard<ReloadPoolResource> = manager
        .acquire_pooled(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Reload with new fingerprint — stale instances should be evicted.
    manager
        .reload_config::<ReloadPoolResource>(ReloadConfig::new(2), &ScopeLevel::Global)
        .expect("reload should succeed");

    // Next acquire should create a fresh instance (stale one evicted).
    let handle2: ResourceGuard<ReloadPoolResource> = manager
        .acquire_pooled(&(), &ctx, &AcquireOptions::default())
        .await
        .expect("second acquire should succeed");
    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        2,
        "stale instance should have been evicted, forcing new creation"
    );

    drop(handle2);

    manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(50)),
        )
        .await
        .expect("graceful_shutdown must succeed");
}

#[tokio::test]
async fn reload_config_not_found_returns_error() {
    let manager = Manager::new();

    let result =
        manager.reload_config::<ReloadPoolResource>(ReloadConfig::new(1), &ScopeLevel::Global);
    assert!(result.is_err());
    assert_eq!(*result.unwrap_err().kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn reload_config_rejected_when_shutdown() {
    let manager = Manager::new();
    let resource = ReloadPoolResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config::default();
    let pool_rt = PoolRuntime::<ReloadPoolResource>::new(pool_config, 1);

    manager
        .register(
            resource,
            ReloadConfig::new(1),
            ScopeLevel::Global,
            TopologyRuntime::Pool(pool_rt),
            None,
            None,
        )
        .expect("register should succeed");

    manager.shutdown();

    let result =
        manager.reload_config::<ReloadPoolResource>(ReloadConfig::new(2), &ScopeLevel::Global);
    assert!(result.is_err());
    assert_eq!(*result.unwrap_err().kind(), ErrorKind::Cancelled);
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .unwrap();

    // Hold a handle across the shutdown so drain cannot complete.
    let ctx = test_ctx();
    let _handle = manager
        .acquire_resident::<ResidentTestResource>(&(), &ctx, &AcquireOptions::default())
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

/// #302: `DrainTimeoutPolicy::Force` is the opt-in escape hatch. It clears
/// the registry anyway and reports the outstanding-handle count so a
/// supervisor with a hard deadline can still exit.
#[tokio::test]
async fn graceful_shutdown_force_clears_registry_on_timeout() {
    use nebula_resource::manager::DrainTimeoutPolicy;

    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
        .unwrap();

    let ctx = test_ctx();
    let _handle = manager
        .acquire_resident::<ResidentTestResource>(&(), &ctx, &AcquireOptions::default())
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
        )
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

    let resident_rt =
        ResidentRuntime::<FailingResidentResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            test_config(),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            Some(gate.clone()),
        )
        .unwrap();

    // First acquire becomes the probe and fails — gate transitions to Failed.
    let ctx = test_ctx();
    let first = manager
        .acquire_resident::<FailingResidentResource>(&(), &ctx, &AcquireOptions::default())
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
            mgr.acquire_resident::<FailingResidentResource>(&(), &ctx, &AcquireOptions::default())
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
