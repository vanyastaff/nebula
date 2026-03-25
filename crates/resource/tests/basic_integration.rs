//! Basic integration tests for nebula-resource v2.
//!
//! These tests exercise the public API surface across topologies without
//! involving real network resources. Mock resources use simple counters
//! to verify lifecycle semantics.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use nebula_resource::Manager;
use nebula_resource::ctx::{BasicCtx, Ctx, ScopeLevel};
use nebula_resource::error::{Error, ErrorKind};
use nebula_resource::handle::ResourceHandle;
use nebula_resource::recovery::{GateState, RecoveryGate, RecoveryGateConfig};
use nebula_resource::release_queue::ReleaseQueue;
use nebula_resource::resource::{Resource, ResourceConfig, ResourceMetadata};
use nebula_resource::runtime::TopologyRuntime;
use nebula_resource::runtime::pool::PoolRuntime;
use nebula_resource::runtime::resident::ResidentRuntime;
use nebula_resource::topology::pooled::{BrokenCheck, Pooled, RecycleDecision};
use nebula_resource::topology::resident;
use nebula_resource::topology::resident::Resident;

use nebula_core::{ExecutionId, ResourceKey, resource_key};

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

impl ResourceConfig for TestConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.name.is_empty() {
            return Err(Error::permanent("name must not be empty"));
        }
        Ok(())
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
    type Credential = ();

    fn key() -> ResourceKey {
        resource_key!("test-pool")
    }

    fn create(
        &self,
        _config: &TestConfig,
        _credential: &(),
        _ctx: &dyn Ctx,
    ) -> impl std::future::Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
        let counter = self.create_counter.clone();
        async move {
            let id = counter.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(AtomicU64::new(id)))
        }
    }

    fn destroy(
        &self,
        _runtime: Arc<AtomicU64>,
    ) -> impl std::future::Future<Output = Result<(), TestError>> + Send {
        async { Ok(()) }
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

    fn recycle(
        &self,
        _runtime: &Arc<AtomicU64>,
        _metrics: &nebula_resource::topology::pooled::InstanceMetrics,
    ) -> impl std::future::Future<Output = Result<RecycleDecision, TestError>> + Send {
        async { Ok(RecycleDecision::Keep) }
    }
}

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
    type Credential = ();

    fn key() -> ResourceKey {
        resource_key!("test-resident")
    }

    fn create(
        &self,
        _config: &TestConfig,
        _credential: &(),
        _ctx: &dyn Ctx,
    ) -> impl std::future::Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
        let counter = self.create_counter.clone();
        async move {
            let id = counter.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(AtomicU64::new(id)))
        }
    }

    fn destroy(
        &self,
        _runtime: Arc<AtomicU64>,
    ) -> impl std::future::Future<Output = Result<(), TestError>> + Send {
        async { Ok(()) }
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

fn test_ctx() -> BasicCtx {
    BasicCtx::new(ExecutionId::new())
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
        .acquire(&resource, &test_config(), &(), &ctx, &rq, 0)
        .await
        .expect("first acquire should succeed");

    assert_eq!(handle.topology_tag(), "pool");
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
        .acquire(&resource, &test_config(), &(), &ctx, &rq, 0)
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
        .acquire(&resource, &test_config(), &(), &ctx, &rq, 0)
        .await
        .unwrap();
    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(pool.idle_count().await, 1);

    // Mark as broken.
    resource.break_flag.store(true, Ordering::Relaxed);

    // Next acquire should destroy the broken instance and create new.
    let handle2 = pool
        .acquire(&resource, &test_config(), &(), &ctx, &rq, 0)
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
        .acquire(&resource, &test_config(), &(), &ctx)
        .await
        .expect("first acquire");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    assert_eq!(h1.topology_tag(), "resident");

    // Second acquire clones (no new creation).
    let h2 = rt
        .acquire(&resource, &test_config(), &(), &ctx)
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
    };
    let rt = ResidentRuntime::<ResidentTestResource>::new(config);
    let ctx = test_ctx();

    let _h1 = rt
        .acquire(&resource, &test_config(), &(), &ctx)
        .await
        .unwrap();
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    // Mark not alive.
    resource.alive.store(false, Ordering::Relaxed);

    // Next acquire should recreate.
    let _h2 = rt
        .acquire(&resource, &test_config(), &(), &ctx)
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
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    manager
        .register(
            resource.clone(),
            test_config(),
            (),
            ScopeLevel::Global,
            TopologyRuntime::Pool(pool_rt),
            rq.clone(),
        )
        .expect("registration should succeed");

    assert!(manager.contains(&resource_key!("test-pool")));

    let ctx = test_ctx();
    let handle: ResourceHandle<PoolTestResource> = manager
        .acquire_pooled(&(), &ctx)
        .await
        .expect("acquire should succeed");

    assert_eq!(handle.topology_tag(), "pool");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Drop manager first (it holds Arc<ReleaseQueue> via ManagedResource),
    // then the local Arc, so the ReleaseQueue workers can shut down.
    drop(manager);
    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

#[tokio::test]
async fn manager_register_and_acquire_resident() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    manager
        .register(
            resource.clone(),
            test_config(),
            (),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            rq.clone(),
        )
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceHandle<ResidentTestResource> = manager
        .acquire_resident(&(), &ctx)
        .await
        .expect("acquire should succeed");

    assert_eq!(handle.topology_tag(), "resident");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    drop(handle);
    drop(manager);
    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

#[tokio::test]
async fn manager_shutdown_rejects_acquire() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    manager
        .register(
            resource,
            test_config(),
            (),
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            rq.clone(),
        )
        .unwrap();

    manager.shutdown();
    assert!(manager.is_shutdown());

    let ctx = test_ctx();
    let result = manager
        .acquire_resident::<ResidentTestResource>(&(), &ctx)
        .await;

    assert!(result.is_err());
    let err = result.err().expect("should be an error");
    assert_eq!(*err.kind(), ErrorKind::Cancelled);

    drop(manager);
    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
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
    assert!(!Error::backpressure("pool full").is_retryable());
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
        .acquire(&resource, &test_config(), &(), &ctx, &rq, 0)
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
