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
use nebula_resource::{
    AcquireOptions, BoundedConfig, BoundedRuntime, Manager, RegistrationSpec, ResourceContext,
    ScopeLevel, ShutdownConfig, SlotIdentity,
    error::{Error, ErrorKind},
    guard::ResourceGuard,
    recovery::{GateState, RecoveryGate, RecoveryGateConfig},
    release_queue::ReleaseQueue,
    resource::{Resource, ResourceConfig, ResourceMetadata},
    runtime::{TopologyRuntime, pool::PoolRuntime, resident::ResidentRuntime},
    topology::{
        bounded::{Bounded, BoundedRelease, Capped, Exclusive as ExclusiveCap, Unbounded},
        pooled::{BrokenCheck, Pooled, RecycleDecision},
        resident,
        resident::Resident,
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
    /// Incremented by `destroy`. The deterministic completion signal for a
    /// release that ends in destruction (tainted / broken / stale) — a
    /// release runs on the [`ReleaseQueue`] worker, so a test that asserts
    /// "the instance was NOT recycled" must wait for this rather than guess
    /// a wall-clock delay (idle stays `0` the whole time, so polling idle is
    /// not a usable settle signal for that case).
    destroy_counter: Arc<AtomicU64>,
}

impl PoolTestResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            break_flag: Arc::new(AtomicBool::new(false)),
            destroy_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Resource for PoolTestResource {
    type Config = TestConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = TestError;

    fn key() -> ResourceKey {
        resource_key!("test-pool")
    }

    fn create(
        &self,
        _config: &TestConfig,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
        let counter = self.create_counter.clone();
        async move {
            let id = counter.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(AtomicU64::new(id)))
        }
    }

    async fn destroy(&self, _runtime: Arc<AtomicU64>) -> Result<(), TestError> {
        self.destroy_counter.fetch_add(1, Ordering::Relaxed);
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

// Also impl `Bounded` (Exclusive cap) so we can test topology mismatch:
// register as Pool, then call the Bounded acquire path and assert it is
// rejected (the bodies never run — the pipeline errors on the topology
// mismatch before dispatch; this folds the old `impl Exclusive {}`).
impl Bounded for PoolTestResource {
    type Cap = ExclusiveCap;

    async fn acquire_one(
        &self,
        runtime: &Arc<AtomicU64>,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, TestError> {
        Ok(Arc::clone(runtime))
    }
}

impl BoundedRelease for PoolTestResource {
    async fn release_one(
        &self,
        _runtime: &Arc<AtomicU64>,
        _lease: Arc<AtomicU64>,
        _healthy: bool,
    ) -> Result<(), TestError> {
        Ok(())
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

    fn key() -> ResourceKey {
        resource_key!("test-resident")
    }

    fn create(
        &self,
        _config: &TestConfig,
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

/// Polls `cond` until it returns `true` or the deadline elapses, then
/// returns the final value of `cond`.
///
/// Replaces fixed `sleep(50ms)` "settle" points: release/recycle work runs
/// on the [`ReleaseQueue`] background worker, so the test must wait for the
/// *observable effect* (an idle count, a counter) rather than guess a
/// wall-clock delay. A short poll interval keeps fast cases fast; the
/// bounded deadline turns a real regression into a prompt failure instead of
/// a hang.
async fn poll_until(deadline: std::time::Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if cond() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    cond()
}

/// Waits until a pool's idle count equals `expected` (bounded), failing the
/// test with the observed count if it never does. The deterministic
/// replacement for `drop(handle); sleep(50ms); assert_eq!(idle_count, n)`.
async fn wait_idle_count<R>(pool: &PoolRuntime<R>, expected: usize)
where
    R: Pooled + Clone + Send + Sync + 'static,
    R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
    R::Lease: Into<R::Runtime> + Send + 'static,
{
    let deadline = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();
    loop {
        let idle = pool.idle_count().await;
        if idle == expected {
            return;
        }
        assert!(
            start.elapsed() < deadline,
            "pool idle count never reached {expected}; last observed {idle}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// Waits until `counter` reaches at least `expected` (bounded). Used as the
/// release-completion signal for the destroyed-not-recycled case, where the
/// idle count stays `0` throughout and is therefore not a usable settle
/// signal.
async fn wait_count_at_least(counter: &Arc<AtomicU64>, expected: u64) {
    let deadline = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();
    loop {
        let observed = counter.load(Ordering::Relaxed);
        if observed >= expected {
            return;
        }
        assert!(
            start.elapsed() < deadline,
            "counter never reached {expected}; last observed {observed}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
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
    // Deterministic settle: wait for the release worker to recycle the
    // instance back into idle instead of guessing a wall-clock delay.
    wait_idle_count(&pool, 1).await;

    // Pool should have one idle instance now.
    assert_eq!(pool.idle_count().await, 1);

    // Second acquire reuses the idle instance (no new creation).
    let handle2 = pool
        .acquire(
            &resource,
            &test_config(),
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
    // `ReleaseQueue::shutdown` drains buffered release tasks, so no
    // wall-clock settle is needed before tearing the queue down.
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
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .unwrap();
    drop(handle);
    wait_idle_count(&pool, 1).await;
    assert_eq!(pool.idle_count().await, 1);

    // Mark as broken.
    resource.break_flag.store(true, Ordering::Relaxed);

    // Next acquire should destroy the broken instance and create new.
    let handle2 = pool
        .acquire(
            &resource,
            &test_config(),
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
    // Broken flag still set: the released instance must be DESTROYED, not
    // recycled. `idle_count == 0` is not a settle signal here — the pool
    // never rises above idle 0 in this window, so waiting on it returns
    // before the release worker has even run. The deterministic signal is
    // the destroy counter. It is already 1 (the `acquire` above evicted +
    // destroyed the first broken instance inline), so the event under test
    // — releasing `handle2` destroys (not recycles) its instance — is the
    // 1 -> 2 transition: wait for >= 2.
    wait_count_at_least(&resource.destroy_counter, 2).await;
    assert_eq!(
        resource.destroy_counter.load(Ordering::Relaxed),
        2,
        "released broken instance must be destroyed, not recycled"
    );
    assert_eq!(
        pool.idle_count().await,
        0,
        "destroyed instance must not return to the pool"
    );

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
        .acquire(&resource, &test_config(), &ctx, &AcquireOptions::default())
        .await
        .expect("first acquire");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    assert_eq!(h1.topology_tag(), nebula_resource::TopologyTag::Resident);

    // Second acquire clones (no new creation).
    let h2 = rt
        .acquire(&resource, &test_config(), &ctx, &AcquireOptions::default())
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
        .acquire(&resource, &test_config(), &ctx, &AcquireOptions::default())
        .await
        .unwrap();
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    // Mark not alive.
    resource.alive.store(false, Ordering::Relaxed);

    // Next acquire should recreate.
    let _h2 = rt
        .acquire(&resource, &test_config(), &ctx, &AcquireOptions::default())
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
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(pool_rt),
            acquire: Manager::erased_acquire_pooled_for::<PoolTestResource>(),
            recovery_gate: None,
        })
        .expect("registration should succeed");

    assert!(manager.contains(&resource_key!("test-pool")));

    let ctx = test_ctx();
    let handle: ResourceGuard<PoolTestResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
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
async fn pool_maintenance_reaper_evicts_idle_timed_out_instance() {
    use nebula_resource::ResourceEvent;

    let manager = Manager::new();
    let resource = PoolTestResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        min_size: 0,
        max_size: 4,
        idle_timeout: Some(std::time::Duration::from_millis(100)),
        max_lifetime: None,
        maintenance_interval: std::time::Duration::from_millis(50),
        ..Default::default()
    };
    let pool_rt = PoolRuntime::<PoolTestResource>::new(pool_config, 1);

    let mut events = manager.subscribe_events();

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(pool_rt),
            acquire: Manager::erased_acquire_pooled_for::<PoolTestResource>(),
            recovery_gate: None,
        })
        .expect("registration should succeed");

    // Create exactly one idle instance: acquire then drop. The release runs
    // on the ReleaseQueue, so the instance lands in the idle queue
    // asynchronously with `returned_at ~= now`.
    let ctx = test_ctx();
    let handle: ResourceGuard<PoolTestResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");
    drop(handle);

    // Nobody calls run_maintenance: the ONLY way this instance is destroyed
    // is the background reaper sweeping it once it ages past idle_timeout.
    let evicted = poll_until(std::time::Duration::from_secs(3), || {
        resource.destroy_counter.load(Ordering::Relaxed) >= 1
    })
    .await;
    assert!(
        evicted,
        "background maintenance reaper should have evicted the idle-timed-out \
         instance without any manual run_maintenance call"
    );

    // And it surfaced a MaintenanceEvicted observability event.
    let mut saw_event = false;
    while let Some(evt) = events.try_recv() {
        if let ResourceEvent::MaintenanceEvicted { evicted, key } = evt {
            assert_eq!(key.as_str(), "test-pool");
            assert!(evicted >= 1, "evicted count must be positive");
            saw_event = true;
        }
    }
    assert!(
        saw_event,
        "expected a ResourceEvent::MaintenanceEvicted from the reaper"
    );

    manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(50)),
        )
        .await
        .expect("graceful_shutdown must succeed");
}

#[tokio::test]
async fn pool_maintenance_reaper_not_spawned_without_ttl() {
    // With neither idle_timeout nor max_lifetime set, no reaper is spawned,
    // so a healthy idle instance is never evicted in the background
    // (the zero-overhead guard). Assert the instance is NOT destroyed over a
    // window that comfortably exceeds the maintenance interval.
    let manager = Manager::new();
    let resource = PoolTestResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        min_size: 0,
        max_size: 4,
        idle_timeout: None,
        max_lifetime: None,
        maintenance_interval: std::time::Duration::from_millis(50),
        ..Default::default()
    };
    let pool_rt = PoolRuntime::<PoolTestResource>::new(pool_config, 1);

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(pool_rt),
            acquire: Manager::erased_acquire_pooled_for::<PoolTestResource>(),
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceGuard<PoolTestResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");
    drop(handle);

    // Poll past several maintenance intervals; with no TTL the instance must
    // remain idle and never be destroyed by a (non-existent) sweep.
    let destroyed = poll_until(std::time::Duration::from_millis(400), || {
        resource.destroy_counter.load(Ordering::Relaxed) >= 1
    })
    .await;
    assert!(
        !destroyed,
        "no TTL configured => no reaper => idle instance must not be evicted \
         (destroy_counter = {})",
        resource.destroy_counter.load(Ordering::Relaxed)
    );

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
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
            recovery_gate: None,
        })
        .unwrap();

    manager.shutdown();
    assert!(manager.is_shutdown());

    let ctx = test_ctx();
    let result = manager
        .acquire_resident::<ResidentTestResource>(&ctx, &AcquireOptions::default())
        .await;

    assert!(result.is_err());
    let err = result.expect_err("should be an error");
    assert_eq!(*err.kind(), ErrorKind::Cancelled);
}

// ---------------------------------------------------------------------------
// #390 — pool config validation + max_concurrent_creates enforcement
// ---------------------------------------------------------------------------

// #390 is now enforced *structurally* at `PoolRuntime` construction
// rather than re-validated at register time: a `TopologyRuntime::Pool`
// holding an invalid `(min_size, max_size)` is unrepresentable because
// `PoolRuntime::new` panics before such a runtime can be built (the
// deleted `register_pooled[_with]` shorthands surfaced a soft `Err` only
// because they took the raw config *before* constructing the runtime).
// These tests pin that the invariant still rejects a broken config — the
// signal moved from a registration `Error` to a construction panic, but
// "an invalid pool config cannot deadlock the pool" is preserved.

#[test]
fn pool_runtime_rejects_min_greater_than_max() {
    let pool_config = nebula_resource::topology::pooled::config::Config {
        min_size: 5,
        max_size: 2,
        ..Default::default()
    };
    let result = std::panic::catch_unwind(|| {
        PoolRuntime::<PoolTestResource>::new(pool_config, test_config().fingerprint())
    });
    let panic = match result {
        Ok(_) => panic!("min > max must be rejected at PoolRuntime construction"),
        Err(p) => p,
    };
    let msg = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("");
    assert!(
        msg.contains("min_size") && msg.contains("max_size"),
        "panic message must mention min_size and max_size, got: {msg}",
    );
}

#[test]
fn pool_runtime_rejects_max_size_zero() {
    let pool_config = nebula_resource::topology::pooled::config::Config {
        min_size: 0,
        max_size: 0,
        ..Default::default()
    };
    let result = std::panic::catch_unwind(|| {
        PoolRuntime::<PoolTestResource>::new(pool_config, test_config().fingerprint())
    });
    let panic = match result {
        Ok(_) => panic!("max_size == 0 must be rejected at PoolRuntime construction"),
        Err(p) => p,
    };
    let msg = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("");
    assert!(
        msg.contains("max_size"),
        "panic message must mention max_size, got: {msg}",
    );
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

    fn key() -> ResourceKey {
        resource_key!("slow-create-pool")
    }

    async fn create(
        &self,
        _config: &TestConfig,
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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(PoolRuntime::<SlowCreatePoolResource>::new(
                pool_config,
                test_config().fingerprint(),
            )),
            acquire: Manager::erased_acquire_pooled_for::<SlowCreatePoolResource>(),
            recovery_gate: None,
        })
        .expect("register");

    // Fire 10 concurrent acquires so they all hit the create path.
    let mut handles = Vec::new();
    for _ in 0..10 {
        let mgr = Arc::clone(&manager);
        handles.push(tokio::spawn(async move {
            let ctx = test_ctx();
            mgr.acquire_pooled::<SlowCreatePoolResource>(&ctx, &AcquireOptions::default())
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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
            recovery_gate: None,
        })
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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
            recovery_gate: None,
        })
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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
            recovery_gate: None,
        })
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
    // A tainted handle is destroyed, not recycled — wait for the release
    // worker to run `destroy` (idle stays 0 throughout, so the destroy
    // counter is the deterministic completion signal here).
    wait_count_at_least(&resource.destroy_counter, 1).await;

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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
            recovery_gate: None,
        })
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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
            recovery_gate: None,
        })
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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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
        matches!(&event, nebula_resource::ResourceEvent::AcquireSuccess { key, .. } if key == &resource_key!("test-resident")),
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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

    let mut saw_released = false;
    while let Some(event) = rx.try_recv() {
        if matches!(
            &event,
            nebula_resource::ResourceEvent::Released { key, .. }
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());
    let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig::default()));

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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
        if let nebula_resource::ResourceEvent::RecoveryGateChanged { key, state } = &event
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
        .acquire(&resource, &test_config(), &ctx, &rq, 0, &opts, None)
        .await;
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected backpressure error when pool is full"),
    };
    assert_eq!(*err.kind(), ErrorKind::Backpressure);

    drop(handles);
    // `ReleaseQueue::shutdown` drains buffered release tasks; no wall-clock
    // settle is needed before teardown.
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
        .acquire(&resource, &test_config(), &ctx, &rq, 0, &opts, None)
        .await;

    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected backpressure error when pool is full"),
    };
    assert_eq!(*err.kind(), ErrorKind::Backpressure);

    drop(_held);
    // `ReleaseQueue::shutdown` drains buffered release tasks; no wall-clock
    // settle is needed before teardown.
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
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: scope.clone(),
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
            recovery_gate: None,
        })
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
        .acquire_resident(&ctx, &AcquireOptions::default())
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
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
            recovery_gate: None,
        })
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
        .acquire_resident(&ctx, &AcquireOptions::default())
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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Organization(org_id),
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
            recovery_gate: None,
        })
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
        .acquire_resident::<ResidentTestResource>(&ctx, &AcquireOptions::default())
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
    let registry = Arc::new(nebula_metrics::MetricsRegistry::new());
    let manager = Manager::with_config(nebula_resource::ManagerConfig {
        release_queue_workers: 2,
        metrics_registry: Some(registry.clone()),
    });
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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
        .register(RegistrationSpec {
            resource: pool_resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(pool_rt),
            acquire: Manager::erased_acquire_pooled_for::<PoolTestResource>(),
            recovery_gate: None,
        })
        .expect("pool registration should succeed");

    // Register a resident resource.
    let resident_resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource: resident_resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
            recovery_gate: None,
        })
        .expect("resident registration should succeed");

    assert!(manager.contains(&resource_key!("test-pool")));
    assert!(manager.contains(&resource_key!("test-resident")));
    assert_eq!(manager.keys().len(), 2);

    // Acquire each independently.
    let ctx = test_ctx();
    let pool_handle: ResourceGuard<PoolTestResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
        .await
        .expect("pool acquire should succeed");

    let resident_handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
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
        .acquire(&resource, &test_config(), &ctx, &rq, 0, &opts, None)
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
    // `ReleaseQueue::shutdown` drains buffered release tasks; no wall-clock
    // settle is needed before teardown.
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

    // `detach` disarms the release callback synchronously, so nothing can
    // ever be submitted to the queue. Draining the release worker is the
    // deterministic proof: after `shutdown` has run every buffered release
    // task to completion, a (erroneously) enqueued return-to-pool would
    // already have executed — so `idle_count == 0` afterward means "never",
    // not merely "not yet" (a bare scheduler yield could only show the
    // latter).
    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;

    // Pool must NOT have gotten the instance back.
    assert_eq!(
        pool.idle_count().await,
        0,
        "detached handle should not return to pool"
    );
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

    fn key() -> ResourceKey {
        resource_key!("test-service")
    }

    fn create(
        &self,
        _config: &TestConfig,
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

// Folds the former `Service` in `TokenMode::Cloned`: `Cap = Unbounded`
// ⇒ owned handle, blanket no-op `BoundedRelease` (no release boilerplate),
// `acquire_token` is now `acquire_one`.
impl Bounded for ServiceTestResource {
    type Cap = Unbounded;

    fn acquire_one(
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

// Folds the former `Transport` onto `Bounded` with `Cap = Capped<N>`:
// `open_session` is `acquire_one`, `close_session` is `release_one`. The
// former `TransportConfig::max_sessions` (a runtime field) is the cap
// **typestate** const generic `N`; each test instantiates the `N` it set
// on the old `TransportConfig`.
#[derive(Clone)]
struct TransportTestResource<const N: usize> {
    create_counter: Arc<AtomicU64>,
    session_counter: Arc<AtomicU64>,
    close_counter: Arc<AtomicU64>,
}

impl<const N: usize> TransportTestResource<N> {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            session_counter: Arc::new(AtomicU64::new(0)),
            close_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl<const N: usize> Resource for TransportTestResource<N> {
    type Config = TestConfig;
    type Runtime = Arc<TransportInner>;
    type Lease = SessionHandle;
    type Error = TestError;

    fn key() -> ResourceKey {
        resource_key!("test-transport")
    }

    fn create(
        &self,
        _config: &TestConfig,
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

impl<const N: usize> Bounded for TransportTestResource<N> {
    type Cap = Capped<N>;

    fn acquire_one(
        &self,
        _transport: &Arc<TransportInner>,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<SessionHandle, TestError>> + Send {
        let id = self.session_counter.fetch_add(1, Ordering::Relaxed);
        async move { Ok(SessionHandle { id }) }
    }
}

impl<const N: usize> BoundedRelease for TransportTestResource<N> {
    fn release_one(
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

    fn key() -> ResourceKey {
        resource_key!("test-exclusive")
    }

    fn create(
        &self,
        _config: &TestConfig,
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

// Folds the former `Exclusive` onto `Bounded` with `Cap = Exclusive`:
// the old `ExclusiveRuntime` cloned the runtime into the lease, so
// `acquire_one` returns a clone of the runtime; `release_one` IS the
// reset (permit-held-until-`release_one`, #384).
impl Bounded for ExclusiveTestResource {
    type Cap = ExclusiveCap;

    fn acquire_one(
        &self,
        runtime: &Arc<AtomicU64>,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
        let lease = Arc::clone(runtime);
        async move { Ok(lease) }
    }
}

impl BoundedRelease for ExclusiveTestResource {
    fn release_one(
        &self,
        _runtime: &Arc<AtomicU64>,
        _lease: Arc<AtomicU64>,
        _healthy: bool,
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
        BoundedRuntime::<ServiceTestResource>::new(&resource, runtime, BoundedConfig::default());
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    // Acquire first token.
    let h1 = rt
        .acquire(&resource, &ctx, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first acquire should succeed");

    assert_eq!(h1.topology_tag(), nebula_resource::TopologyTag::Bounded);
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
        BoundedRuntime::<ServiceTestResource>::new(&resource, runtime, BoundedConfig::default());

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Bounded(svc_rt),
            acquire: Manager::erased_acquire_bounded_for::<ServiceTestResource>(),
            recovery_gate: None,
        })
        .expect("registration should succeed");

    assert!(manager.contains(&resource_key!("test-service")));

    let ctx = test_ctx();
    let handle: ResourceGuard<ServiceTestResource> = manager
        .acquire_bounded(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    assert_eq!(handle.topology_tag(), nebula_resource::TopologyTag::Bounded);
    assert_eq!(resource.token_counter.load(Ordering::Relaxed), 1);
}

// ---------------------------------------------------------------------------
// Transport tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn transport_acquire_opens_session() {
    let resource = TransportTestResource::<10>::new();
    let runtime = Arc::new(TransportInner {
        name: "test-conn".into(),
    });
    let config = BoundedConfig::default();
    let rt = BoundedRuntime::<TransportTestResource<10>>::new(&resource, runtime, config);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();

    let handle = rt
        .acquire(&resource, &ctx, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("acquire should succeed");

    assert_eq!(handle.topology_tag(), nebula_resource::TopologyTag::Bounded);
    assert_eq!(resource.session_counter.load(Ordering::Relaxed), 1);

    // Drop triggers close_session via the release queue — wait for it to
    // run rather than guess a delay.
    drop(handle);
    poll_until(std::time::Duration::from_secs(2), || {
        resource.close_counter.load(Ordering::Relaxed) >= 1
    })
    .await;

    assert_eq!(resource.close_counter.load(Ordering::Relaxed), 1);

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

#[tokio::test]
async fn transport_session_bounded_by_semaphore() {
    let resource = TransportTestResource::<2>::new();
    let runtime = Arc::new(TransportInner {
        name: "bounded-conn".into(),
    });
    // max_sessions=2 is now the `Capped<2>` cap typestate (the fixture
    // type param); only the non-cap fields remain in the config.
    let config = BoundedConfig {
        keepalive_interval: None,
        ..BoundedConfig::default()
    };
    let rt = BoundedRuntime::<TransportTestResource<2>>::new(&resource, runtime, config);
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

    // Release one session — `close_session` (release queue) frees a
    // semaphore permit. Wait for the close to complete so the permit is
    // genuinely available before the next acquire. Assert the wait
    // succeeded: a bug that frees the permit early (without running
    // `close_session`) would otherwise let the next acquire pass and hide
    // the regression.
    drop(h1);
    assert!(
        poll_until(std::time::Duration::from_secs(2), || {
            resource.close_counter.load(Ordering::Relaxed) >= 1
        })
        .await,
        "close_session never ran after releasing the first session"
    );

    // Now third acquire should succeed.
    let h3 = rt
        .acquire(&resource, &ctx, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("third session after release");

    assert_eq!(resource.session_counter.load(Ordering::Relaxed), 3);

    drop(h2);
    drop(h3);
    // `ReleaseQueue::shutdown` drains buffered release tasks; no wall-clock
    // settle is needed before teardown.
    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

#[tokio::test]
async fn transport_acquire_timeout_when_sessions_exhausted() {
    let resource = TransportTestResource::<1>::new();
    let runtime = Arc::new(TransportInner {
        name: "timeout-conn".into(),
    });
    // max_sessions=1 is now the `Capped<1>` cap typestate.
    let config = BoundedConfig {
        keepalive_interval: None,
        acquire_timeout: std::time::Duration::from_millis(50),
        drain_timeout: None,
    };
    let rt = BoundedRuntime::<TransportTestResource<1>>::new(&resource, runtime, config);
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
    // `ReleaseQueue::shutdown` drains buffered release tasks; no wall-clock
    // settle is needed before teardown.
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
    let rt =
        BoundedRuntime::<ExclusiveTestResource>::new(&resource, runtime, BoundedConfig::default());
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    // First acquire succeeds.
    let h1 = rt
        .acquire(
            &resource,
            &test_ctx(),
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("first acquire should succeed");

    assert_eq!(h1.topology_tag(), nebula_resource::TopologyTag::Bounded);

    // Second acquire should block (semaphore has 1 permit).
    let rt_ref = &rt;
    let resource_ref = &resource;
    let rq_ref = &rq;

    let result = tokio::time::timeout(std::time::Duration::from_millis(100), async {
        rt_ref
            .acquire(
                resource_ref,
                &test_ctx(),
                rq_ref,
                0,
                &AcquireOptions::default(),
                None,
            )
            .await
    })
    .await;

    assert!(result.is_err(), "second acquire should have timed out");

    // Release the first handle. The next acquire (no deadline) blocks on
    // the semaphore until `reset` completes and the permit is dropped —
    // that is itself the deterministic wait, no fixed sleep needed.
    drop(h1);

    // Now second acquire should succeed.
    let h2 = rt
        .acquire(
            &resource,
            &test_ctx(),
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("second acquire after release");

    assert_eq!(h2.topology_tag(), nebula_resource::TopologyTag::Bounded);

    drop(h2);
    // `ReleaseQueue::shutdown` drains buffered release tasks; no wall-clock
    // settle is needed before teardown.
    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

#[tokio::test]
async fn exclusive_reset_called_on_release() {
    let resource = ExclusiveTestResource::new();
    let runtime = Arc::new(AtomicU64::new(0));
    let rt =
        BoundedRuntime::<ExclusiveTestResource>::new(&resource, runtime, BoundedConfig::default());
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let handle = rt
        .acquire(
            &resource,
            &test_ctx(),
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("acquire should succeed");

    assert_eq!(resource.reset_counter.load(Ordering::Relaxed), 0);

    // Drop triggers reset via the release queue — wait for it to run.
    drop(handle);
    poll_until(std::time::Duration::from_secs(2), || {
        resource.reset_counter.load(Ordering::Relaxed) >= 1
    })
    .await;

    assert_eq!(
        resource.reset_counter.load(Ordering::Relaxed),
        1,
        "reset should have been called once after release"
    );

    // Acquire and release again to confirm reset increments.
    let handle2 = rt
        .acquire(
            &resource,
            &test_ctx(),
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("second acquire");

    drop(handle2);
    poll_until(std::time::Duration::from_secs(2), || {
        resource.reset_counter.load(Ordering::Relaxed) >= 2
    })
    .await;

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
    let config = BoundedConfig {
        acquire_timeout: std::time::Duration::from_millis(50),
        ..BoundedConfig::default()
    };
    let rt = BoundedRuntime::<ExclusiveTestResource>::new(&resource, runtime, config);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    // Hold the exclusive lock.
    let _h1 = rt
        .acquire(
            &resource,
            &test_ctx(),
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("first acquire should succeed");

    // Second acquire should time out via config timeout.
    let result = rt
        .acquire(
            &resource,
            &test_ctx(),
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
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
    let result2 = rt
        .acquire(&resource, &test_ctx(), &rq, 0, &short_deadline, None)
        .await;
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
    // `ReleaseQueue::shutdown` drains buffered release tasks; no wall-clock
    // settle is needed before teardown.
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

    fn key() -> ResourceKey {
        resource_key!("slow-reset-exclusive")
    }

    async fn create(
        &self,
        _config: &TestConfig,
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

// Folds the former `Exclusive`: `release_one` IS the (slow) reset; the
// permit is held until it resolves (#384), exactly as the old
// `ExclusiveRuntime` held it across `reset`.
impl Bounded for SlowResetExclusive {
    type Cap = ExclusiveCap;

    fn acquire_one(
        &self,
        runtime: &Arc<AtomicU64>,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
        let lease = Arc::clone(runtime);
        async move { Ok(lease) }
    }
}

impl BoundedRelease for SlowResetExclusive {
    fn release_one(
        &self,
        _runtime: &Arc<AtomicU64>,
        _lease: Arc<AtomicU64>,
        _healthy: bool,
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
        BoundedRuntime::<SlowResetExclusive>::new(&resource, runtime, BoundedConfig::default());
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let h1 = rt
        .acquire(
            &resource,
            &test_ctx(),
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
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
        .acquire(
            &resource,
            &test_ctx(),
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
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
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("second acquire must not block — permit should be available");
    drop(handle2);
    // `ReleaseQueue::shutdown` drains buffered release tasks; no wall-clock
    // settle is needed before teardown.
    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// Registry-backed metrics tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn registry_backed_metrics_record_operations() {
    let registry = Arc::new(nebula_metrics::MetricsRegistry::new());
    let manager = Manager::with_config(nebula_resource::ManagerConfig {
        release_queue_workers: 1,
        metrics_registry: Some(registry.clone()),
    });

    // Register two resources.
    let pool_resource = PoolTestResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool_rt = PoolRuntime::<PoolTestResource>::new(pool_config, 1);

    manager
        .register(RegistrationSpec {
            resource: pool_resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(pool_rt),
            acquire: Manager::erased_acquire_pooled_for::<PoolTestResource>(),
            recovery_gate: None,
        })
        .expect("pool registration should succeed");

    let resident_resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource: resident_resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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

impl Resource for FailingResidentResource {
    type Config = TestConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = TestError;

    fn key() -> ResourceKey {
        resource_key!("test-failing-resident")
    }

    fn create(
        &self,
        _config: &TestConfig,
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

impl Resource for BlockingResidentResource {
    type Config = TestConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = TestError;

    fn key() -> ResourceKey {
        resource_key!("test-blocking-resident")
    }

    fn create(
        &self,
        _config: &TestConfig,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
        let unblock = Arc::clone(&self.unblock);
        async move {
            unblock.notified().await;
            Err(TestError("unblocked but never satisfied".into()))
        }
    }

    async fn destroy(&self, _runtime: Arc<AtomicU64>) -> Result<(), TestError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for BlockingResidentResource {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        true
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

    fn key() -> ResourceKey {
        resource_key!("test-permanent-fail")
    }

    fn create(
        &self,
        _config: &TestConfig,
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
async fn acquire_does_not_retry_transient_at_manager_layer() {
    // Mythos v2: manager performs exactly one acquire attempt. A transient
    // failure surfaces immediately to the caller; retry is composed above.
    let manager = Manager::new();
    let resource = FailingResidentResource::new(1);
    let resident_rt =
        ResidentRuntime::<FailingResidentResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<FailingResidentResource>(),
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
    let resident_rt =
        ResidentRuntime::<PermanentFailResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<PermanentFailResource>(),
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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
    let resident_rt =
        ResidentRuntime::<BlockingResidentResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<BlockingResidentResource>(),
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(pool_rt),
            acquire: Manager::erased_acquire_pooled_for::<PoolTestResource>(),
            recovery_gate: None,
        })
        .unwrap();

    let ctx = test_ctx();

    // Pool resource, but we call acquire_exclusive — wrong topology.
    let result = manager
        .acquire_bounded::<PoolTestResource>(&ctx, &AcquireOptions::default())
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
// Error kind preserved across manager (no rewrap)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn acquire_surfaces_underlying_transient_error_kind() {
    // Mythos v2: with no retry/timeout wrapping at this layer, the
    // resource's typed error reaches the caller unchanged — preserving
    // `Classify` for any upstream pipeline composed by the caller.
    let manager = Manager::new();
    let resource = FailingResidentResource::new(100);
    let resident_rt =
        ResidentRuntime::<FailingResidentResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<FailingResidentResource>(),
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
    let resident_rt =
        ResidentRuntime::<FailingResidentResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<FailingResidentResource>(),
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

    fn key() -> ResourceKey {
        resource_key!("handle-dummy")
    }

    async fn create(&self, _config: &TestConfig, _ctx: &ResourceContext) -> Result<u32, TestError> {
        Ok(1)
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

#[test]
fn panic_in_release_callback_does_not_abort() {
    // Create a guarded handle with a callback that panics. The callback now
    // *builds* the teardown future; the build runs synchronously on `Drop`
    // inside `catch_unwind`, so a panic there must still be caught and the
    // process must not abort.
    use std::sync::atomic::{AtomicBool, Ordering};

    // The guard holds a `ReleaseQueue` for its `Drop` fallback. Building the
    // queue spawns workers, so it must happen inside a Tokio runtime
    // context; keep the runtime alive for the whole test via `enter()`.
    let rt = tokio::runtime::Runtime::new().expect("build a tokio runtime");
    let _rt_guard = rt.enter();
    // Drop the handle (detaches the workers) — this test asserts the panic is
    // caught at future-build time on `Drop`, not queued-future completion.
    let (queue, _queue_handle) = ReleaseQueue::new(1);
    let queue = Arc::new(queue);

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
            queue,
        );
    }
    // If we get here, the process didn't abort.
    assert!(
        callback_entered.load(Ordering::Relaxed),
        "callback should have been invoked before the panic was caught"
    );
}

#[tokio::test]
async fn release_shared_guard_runs_teardown_inline_and_returns_ok() {
    // The `Shared` (Arc-wrapped) arm of `release()` has no production topology
    // producer today, but `ResourceGuard::shared` is public API. Exercise that
    // match arm directly: `release()` must run the teardown future INLINE
    // (awaited, not queued) and surface its `Ok`.
    use nebula_resource::guard::ResourceGuard;

    let (queue, _queue_handle) = ReleaseQueue::new(1);
    let queue = Arc::new(queue);

    let ran = Arc::new(AtomicBool::new(false));
    let ran_cb = Arc::clone(&ran);

    let guard = ResourceGuard::<HandleDummyResource>::shared(
        Arc::new(42_u32),
        resource_key!("handle-dummy"),
        nebula_resource::TopologyTag::Resident,
        1,
        move |_tainted: bool| {
            let ran = Arc::clone(&ran_cb);
            Box::pin(async move {
                ran.store(true, Ordering::Relaxed);
                Ok::<(), Error>(())
            }) as std::pin::Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>
        },
        queue,
    );

    guard
        .release()
        .await
        .expect("release of a shared guard runs its teardown future and returns Ok");
    assert!(
        ran.load(Ordering::Relaxed),
        "the shared release future must have run inline (awaited by release(), not queued)"
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
    wait_idle_count(&pool, 1).await;
    assert_eq!(pool.idle_count().await, 1);

    // Change fingerprint — makes the idle entry stale.
    pool.set_fingerprint(999);

    // Next acquire should destroy stale entry and create fresh.
    let handle2 = pool
        .acquire(
            &resource,
            &test_config(),
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
    // `ReleaseQueue::shutdown` drains buffered release tasks; no wall-clock
    // settle is needed before teardown.
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
    wait_idle_count(&pool, 1).await;
    assert_eq!(pool.idle_count().await, 1);

    // Sleep past max_lifetime — a deliberate clock advance (the entry must
    // actually age beyond its lifetime), not a release-settle guess.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Next acquire should destroy expired entry and create fresh.
    let handle2 = pool
        .acquire(
            &resource,
            &test_config(),
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
    // `ReleaseQueue::shutdown` drains buffered release tasks; no wall-clock
    // settle is needed before teardown.
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
    /// Release-completion signal: the idle count stays `0` for this resource
    /// (every release ends in `destroy`), so the destroy counter — not the
    /// idle count — is the deterministic "release ran" signal.
    destroy_counter: Arc<AtomicU64>,
}

impl DropOnRecycleResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            destroy_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Resource for DropOnRecycleResource {
    type Config = TestConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = TestError;

    fn key() -> ResourceKey {
        resource_key!("drop-on-recycle")
    }

    fn create(
        &self,
        _config: &TestConfig,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
        let counter = self.create_counter.clone();
        async move {
            let id = counter.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(AtomicU64::new(id)))
        }
    }

    async fn destroy(&self, _runtime: Arc<AtomicU64>) -> Result<(), TestError> {
        self.destroy_counter.fetch_add(1, Ordering::Relaxed);
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
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("acquire should succeed");

    drop(handle);
    // recycle=Drop destroys the instance — wait for `destroy` to run (idle
    // stays 0 throughout, so the destroy counter is the settle signal).
    assert!(
        poll_until(std::time::Duration::from_secs(2), || {
            resource.destroy_counter.load(Ordering::Relaxed) >= 1
        })
        .await,
        "destroy never ran for recycle=Drop"
    );

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

    fn key() -> ResourceKey {
        resource_key!("failing-session")
    }

    async fn create(&self, _config: &TestConfig, _ctx: &ResourceContext) -> Result<u32, TestError> {
        Ok(1)
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

// Folds the former `Transport`: `open_session` (always failing) is
// `acquire_one`; `release_one` is the old no-op default close.
impl Bounded for FailingSessionTransport {
    type Cap = Capped<1>;

    async fn acquire_one(
        &self,
        _transport: &u32,
        _ctx: &ResourceContext,
    ) -> Result<u32, TestError> {
        Err(TestError("session open failed".into()))
    }
}

impl BoundedRelease for FailingSessionTransport {
    async fn release_one(
        &self,
        _transport: &u32,
        _session: u32,
        _healthy: bool,
    ) -> Result<(), TestError> {
        Ok(())
    }
}

#[tokio::test]
async fn transport_open_session_failure_frees_permit() {
    let resource = FailingSessionTransport;
    let transport_rt =
        BoundedRuntime::<FailingSessionTransport>::new(&resource, 1u32, BoundedConfig::default());
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

    fn key() -> ResourceKey {
        resource_key!("failing-reset")
    }

    async fn create(&self, _config: &TestConfig, _ctx: &ResourceContext) -> Result<u32, TestError> {
        Ok(1)
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

// Folds the former `Exclusive`: `release_one` IS the (failing) reset.
// The `Exclusive` cap's observed release (R17/S4) poisons the runtime on
// a failed reset but STILL returns the permit, so the next acquire is not
// deadlocked — it fails *closed* (a prompt `Permanent` error) rather than
// being served the half-reset instance. That fail-closed-but-not-wedged
// pair is exactly the invariant this test asserts.
impl Bounded for FailingResetExclusive {
    type Cap = ExclusiveCap;

    async fn acquire_one(&self, runtime: &u32, _ctx: &ResourceContext) -> Result<u32, TestError> {
        Ok(*runtime)
    }
}

impl BoundedRelease for FailingResetExclusive {
    async fn release_one(
        &self,
        _runtime: &u32,
        _lease: u32,
        _healthy: bool,
    ) -> Result<(), TestError> {
        Err(TestError("reset failed".into()))
    }
}

#[tokio::test]
async fn exclusive_reset_failure_does_not_block_next_acquire() {
    let resource = FailingResetExclusive;
    let config = BoundedConfig {
        acquire_timeout: std::time::Duration::from_secs(5),
        ..BoundedConfig::default()
    };
    let exclusive_rt = BoundedRuntime::<FailingResetExclusive>::new(&resource, 1u32, config);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    // First acquire should succeed.
    let handle = exclusive_rt
        .acquire(
            &resource,
            &test_ctx(),
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("first acquire should succeed");

    // Drop the handle — this triggers reset() which fails. The next acquire
    // (bounded by its own deadline) blocks on the semaphore until the permit
    // is dropped after `reset` resolves — that wait is the deterministic
    // synchronization, no fixed sleep needed. The poison latch is set
    // *before* the permit is dropped, so once the second acquire clears the
    // permit it deterministically observes the poisoned runtime.
    drop(handle);

    // Second acquire must fail *closed* (S4): the failed reset poisoned the
    // runtime, so it is NOT served the half-reset instance. But the permit
    // was returned, so the rejection is a prompt `Permanent` error within
    // the deadline — NOT a backpressure/timeout (which would mean the
    // semaphore wedged). A pre-fix `is_ok()` here encoded the Thread-1 bug
    // (half-reset instance handed onward).
    let started = std::time::Instant::now();
    let handle2 = exclusive_rt
        .acquire(
            &resource,
            &test_ctx(),
            &rq,
            0,
            &AcquireOptions::default()
                .with_deadline(std::time::Instant::now() + std::time::Duration::from_millis(500)),
            None,
        )
        .await;
    let err = handle2.expect_err(
        "second acquire must fail closed after a poisoning failed reset \
         (S4): the half-reset instance must NOT be handed onward",
    );
    assert_eq!(
        *err.kind(),
        ErrorKind::Permanent,
        "the poisoned-runtime rejection must be a `Permanent` error, NOT a \
         backpressure/timeout (the permit was returned — no deadlock; S4 \
         preserve): {err:?}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_millis(500),
        "the fail-closed rejection must be prompt (permit returned, no \
         deadlock — within the 500ms deadline); took {:?}",
        started.elapsed()
    );
    // `ReleaseQueue::shutdown` drains buffered release tasks; no wall-clock
    // settle is needed before teardown.
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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

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
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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

#[tokio::test]
async fn recovery_gate_allows_acquire_when_idle() {
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    let gate = RecoveryGate::new(RecoveryGateConfig::default());

    let manager = Manager::new();
    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    let manager = Manager::new();
    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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

    fn key() -> ResourceKey {
        resource_key!("test-reload-pool")
    }

    fn create(
        &self,
        _config: &ReloadConfig,
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
        .register(RegistrationSpec {
            resource,
            config: ReloadConfig::new(1),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(pool_rt),
            acquire: Manager::erased_acquire_pooled_for::<ReloadPoolResource>(),
            recovery_gate: None,
        })
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
        .register(RegistrationSpec {
            resource,
            config: ReloadConfig::new(1),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(pool_rt),
            acquire: Manager::erased_acquire_pooled_for::<ReloadPoolResource>(),
            recovery_gate: None,
        })
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
        .register(RegistrationSpec {
            resource,
            config: ReloadConfig::new(1),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(pool_rt),
            acquire: Manager::erased_acquire_pooled_for::<ReloadPoolResource>(),
            recovery_gate: None,
        })
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
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: ReloadConfig::new(1),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(pool_rt),
            acquire: Manager::erased_acquire_pooled_for::<ReloadPoolResource>(),
            recovery_gate: None,
        })
        .expect("register should succeed");

    let ctx = test_ctx();

    // Acquire and release to populate idle queue with fingerprint=1.
    let handle: ResourceGuard<ReloadPoolResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    drop(handle);
    // Wait for the release worker to recycle the instance back into idle so
    // there is a stale entry for the reload to evict (deterministic settle
    // via the observable idle count, not a wall-clock guess).
    {
        let deadline = std::time::Duration::from_secs(2);
        let start = std::time::Instant::now();
        loop {
            let idle = manager
                .pool_stats::<ReloadPoolResource>(&ScopeLevel::Global)
                .await
                .map_or(0, |s| s.idle);
            if idle >= 1 {
                break;
            }
            assert!(
                start.elapsed() < deadline,
                "released instance never recycled back into idle (idle={idle})"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    // Reload with new fingerprint — stale instances should be evicted.
    manager
        .reload_config::<ReloadPoolResource>(ReloadConfig::new(2), &ScopeLevel::Global)
        .expect("reload should succeed");

    // Next acquire should create a fresh instance (stale one evicted).
    let handle2: ResourceGuard<ReloadPoolResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
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
        .register(RegistrationSpec {
            resource,
            config: ReloadConfig::new(1),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Pool(pool_rt),
            acquire: Manager::erased_acquire_pooled_for::<ReloadPoolResource>(),
            recovery_gate: None,
        })
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
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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
        .phase_erased();
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
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

    let resident_rt =
        ResidentRuntime::<FailingResidentResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<FailingResidentResource>(),
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

// ---------------------------------------------------------------------------
// ResourceGuard::release() — explicit, awaited release checkpoint (canon §11.4)
// ---------------------------------------------------------------------------

/// `release()` on a Pooled guard returns `Ok(())` and recycles the instance:
/// the slot lands back in idle and a subsequent acquire reuses it (the
/// `create_counter` does not advance). This is the awaited-inline counterpart
/// to the drop-then-`wait_idle_count` recycle path.
#[tokio::test]
async fn release_pooled_guard_recycles_and_returns_ok() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
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
            &ctx,
            &rq,
            0,
            &AcquireOptions::default(),
            None,
        )
        .await
        .expect("first acquire should succeed");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    // Explicit awaited release: runs the recycle inline and returns its
    // outcome. No `ReleaseQueue` worker is involved on this path.
    handle
        .release()
        .await
        .expect("release of a healthy pooled guard recycles and returns Ok");

    // The instance is already back in idle by the time `release()` returned
    // (the recycle was awaited inline, not queued) — no settle needed.
    assert_eq!(
        pool.idle_count().await,
        1,
        "an awaited release must have recycled the instance back to idle"
    );

    // Reacquire reuses the recycled instance: no new creation.
    let handle2 = pool
        .acquire(
            &resource,
            &test_config(),
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
        "release() recycled, not destroyed — reacquire reuses the instance"
    );

    drop(handle2);
    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

/// `release()` surfaces the teardown error AND still completes the drain
/// accounting. Uses the `Exclusive` cap whose `release_one` always fails: the
/// awaited `release()` returns `Err`, and because `settle` ran regardless of
/// the error, a subsequent `graceful_shutdown` drains promptly (does not
/// hang on the now-released slot).
#[tokio::test]
async fn release_surfaces_error_but_still_completes_drain() {
    let manager = Manager::new();
    let resource = FailingResetExclusive;
    let exclusive_rt =
        BoundedRuntime::<FailingResetExclusive>::new(&resource, 1u32, BoundedConfig::default());

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Bounded(exclusive_rt),
            acquire: Manager::erased_acquire_bounded_for::<FailingResetExclusive>(),
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceGuard<FailingResetExclusive> = manager
        .acquire_bounded(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");

    // The Exclusive `release_one` is a reset that always fails. `release()`
    // surfaces that error — but the poison-latch, the matching-instance
    // destroy, the permit drop (#384), and the drain accounting all already
    // ran before the error reached us.
    let err = handle
        .release()
        .await
        .expect_err("a failing reset must surface as an Err from release()");
    assert_eq!(
        *err.kind(),
        ErrorKind::Transient,
        "the surfaced error is the original release_one error (TestError → transient): {err:?}"
    );

    // The slot was released despite the error, so graceful_shutdown drains
    // promptly rather than hanging on an outstanding in-flight count.
    let report = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        manager.graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(500)),
        ),
    )
    .await
    .expect("graceful_shutdown must not hang — the errored release still drained the slot")
    .expect("graceful_shutdown must succeed");
    let _ = report;
}

/// `release()` on an Owned (resident) guard returns `Ok(())` — there is no
/// recycle/destroy work, only the drain + event settle.
#[tokio::test]
async fn release_owned_resident_guard_returns_ok() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    handle
        .release()
        .await
        .expect("release of an owned resident guard is a no-op teardown — Ok");

    manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(50)),
        )
        .await
        .expect("graceful_shutdown must succeed");
}

/// Calling `release()` then dropping the (consumed) guard must emit **exactly
/// one** `Released` event and decrement the drain counters exactly once: the
/// `release()` consumes `self`, so the subsequent drop of the husk is inert.
#[tokio::test]
async fn release_then_drop_emits_exactly_one_released_event() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt =
        ResidentRuntime::<ResidentTestResource>::new(resident::config::Config::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(resident_rt),
            acquire: Manager::erased_acquire_resident_for::<ResidentTestResource>(),
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let mut rx = manager.subscribe_events();

    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    // The awaited checkpoint runs the settle (emits `Released`) and consumes
    // `self`; the husk's drop at end of statement is fully inert.
    handle.release().await.expect("release should succeed");

    let mut released_count = 0usize;
    while let Some(event) = rx.try_recv() {
        if matches!(
            &event,
            nebula_resource::ResourceEvent::Released { key, .. }
                if key == &resource_key!("test-resident")
        ) {
            released_count += 1;
        }
    }
    assert_eq!(
        released_count, 1,
        "release() then drop must emit EXACTLY one Released event — the \
         drop-after-release husk is inert (no double emit / double decrement)"
    );

    manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(50)),
        )
        .await
        .expect("graceful_shutdown must succeed");
}
