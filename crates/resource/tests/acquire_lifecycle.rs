//! Core acquire-path integration tests for nebula-resource v2: pool/resident
//! creation, the Manager register→acquire funnel, pool-config validation,
//! `max_concurrent_creates` admission, `AcquireOptions` deadlines, the pool
//! permit accounting invariant, and topology-tagged acquire dispatch.
//!
//! Split out of the former monolithic `basic_integration.rs` (pure move, no
//! test-body changes) — shared mocks/helpers live in `tests/common/mod.rs`.

mod common;

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use common::{
    PoolTestResource, ResidentTestResource, TestConfig, idle_count, poll_until,
    pool_manager_with_metrics, register_pool, register_resident, test_config, test_ctx,
    wait_count_at_least, wait_idle_count,
};
use nebula_core::{ResourceKey, resource_key};
use nebula_resource::{
    AcquireOptions, Manager, ManagerConfig, Pooled, RegistrationSpec, Resident, ResidentConfig,
    ResourceContext, ScopeLevel, ShutdownConfig, SlotIdentity, TopologyTag,
    error::{Error, ErrorKind},
    guard::ResourceGuard,
    resource::{Provider, ResourceConfig, ResourceMetadata},
    topology::pooled::{BrokenCheck, PoolProvider},
};

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
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    // First acquire creates a new instance.
    let handle = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");

    assert_eq!(handle.topology_tag(), TopologyTag::Pool);
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    // Use the lease.
    let _val = handle.load(Ordering::Relaxed);

    // Release by dropping.
    drop(handle);
    // Deterministic settle: wait for the release worker to recycle the
    // instance back into idle instead of guessing a wall-clock delay.
    wait_idle_count::<PoolTestResource>(&mgr, 1).await;

    // Pool should have one idle instance now.
    assert_eq!(idle_count::<PoolTestResource>(&mgr).await, 1);

    // Second acquire reuses the idle instance (no new creation).
    let handle2 = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("second acquire should succeed");

    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        1,
        "should reuse, not create"
    );
    drop(handle2);
}

#[tokio::test]
async fn pool_broken_instance_gets_replaced() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 2,
        ..Default::default()
    };
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    // Acquire and release to populate idle queue.
    let handle = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .unwrap();
    drop(handle);
    wait_idle_count::<PoolTestResource>(&mgr, 1).await;
    assert_eq!(idle_count::<PoolTestResource>(&mgr).await, 1);

    // Mark as broken.
    resource.break_flag.store(true, Ordering::Relaxed);

    // Next acquire should destroy the broken instance and create new.
    let handle2 = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
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
        idle_count::<PoolTestResource>(&mgr).await,
        0,
        "destroyed instance must not return to the pool"
    );
}

/// ADR-0093 Tier-4: a clean pooled release records `recycled`, never
/// `discarded`. The recycled counter is the operator's positive signal that
/// the pool is actually reusing instances.
#[tokio::test]
async fn pool_clean_release_records_recycled() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = pool_manager_with_metrics(resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    let handle = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");
    drop(handle);

    // The clean instance is recycled back into idle: that is the settle
    // signal, and it is the same event the recycled counter observes.
    wait_idle_count::<PoolTestResource>(&mgr, 1).await;

    let snap = mgr
        .metrics()
        .expect("manager was built with a metrics registry")
        .snapshot()
        .recycle_outcomes;
    assert_eq!(snap.recycled, 1, "clean release must record recycled");
    assert_eq!(snap.discarded, 0, "clean release must not discard");
    // Exactly one outcome per release: recycled XOR discarded.
    assert_eq!(snap.recycled + snap.discarded, 1, "one outcome per release");
}

/// ADR-0093 Tier-4: a release whose instance is not kept (here: broken, so
/// the recycle decision drops it) records `discarded`, never `recycled` —
/// the signature an operator watches to catch a silently-evicting pool.
#[tokio::test]
async fn pool_discarded_release_records_discarded() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = pool_manager_with_metrics(resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    let handle = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");
    // Force the on-release recycle decision to drop the instance.
    resource.break_flag.store(true, Ordering::Relaxed);
    drop(handle);

    // Destroyed-not-recycled: idle stays 0, so the destroy counter is the
    // settle signal — the same event the discarded counter observes.
    wait_count_at_least(&resource.destroy_counter, 1).await;

    let snap = mgr
        .metrics()
        .expect("manager was built with a metrics registry")
        .snapshot()
        .recycle_outcomes;
    assert_eq!(snap.discarded, 1, "dropped release must record discarded");
    assert_eq!(snap.recycled, 0, "dropped release must not recycle");
    // Exactly one outcome per release: recycled XOR discarded.
    assert_eq!(snap.recycled + snap.discarded, 1, "one outcome per release");
    assert_eq!(
        idle_count::<PoolTestResource>(&mgr).await,
        0,
        "discarded instance must not return to the pool"
    );
}

// ---------------------------------------------------------------------------
// Resident tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resident_acquire_creates_then_clones() {
    let resource = ResidentTestResource::new();
    let rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());
    let mgr = Manager::new();
    register_resident(&mgr, resource.clone(), test_config(), rt);
    let ctx = test_ctx();

    // First acquire creates.
    let h1 = mgr
        .acquire_resident::<ResidentTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    assert_eq!(h1.topology_tag(), TopologyTag::Resident);

    // Second acquire clones (no new creation).
    let h2 = mgr
        .acquire_resident::<ResidentTestResource>(&ctx, &AcquireOptions::default())
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
    let config = ResidentConfig {
        recreate_on_failure: true,
        ..Default::default()
    };
    let rt = Resident::<ResidentTestResource>::new(config);
    let mgr = Manager::new();
    register_resident(&mgr, resource.clone(), test_config(), rt);
    let ctx = test_ctx();

    let _h1 = mgr
        .acquire_resident::<ResidentTestResource>(&ctx, &AcquireOptions::default())
        .await
        .unwrap();
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    // Mark not alive.
    resource.alive.store(false, Ordering::Relaxed);

    // Next acquire should recreate.
    let _h2 = mgr
        .acquire_resident::<ResidentTestResource>(&ctx, &AcquireOptions::default())
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
    let pool_rt = Pooled::<PoolTestResource>::new(pool_config, 1);

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    assert!(manager.contains(&resource_key!("test-pool")));

    let ctx = test_ctx();
    let handle: ResourceGuard<PoolTestResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    assert_eq!(handle.topology_tag(), TopologyTag::Pool);
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
    let pool_rt = Pooled::<PoolTestResource>::new(pool_config, 1);

    let mut events = manager.subscribe_events();

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
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

    // Nobody calls run_maintenance: the ONLY way this instance is destroyed is
    // the background reaper sweeping it once it ages past idle_timeout. The
    // reaper emits `MaintenanceEvicted` *after* `run_maintenance` returns, so
    // poll for the EVENT — draining the subscriber on every tick — rather than
    // racing the destroy counter (which flips mid-sweep) against the later
    // emit. The deadline comfortably exceeds the (TTL-floored) sweep cadence.
    let mut saw_event = false;
    let got_event = poll_until(std::time::Duration::from_secs(8), || {
        while let Some(evt) = events.try_recv() {
            if let ResourceEvent::MaintenanceEvicted { evicted, key } = evt {
                assert_eq!(key.as_str(), "test-pool");
                assert!(evicted >= 1, "evicted count must be positive");
                saw_event = true;
            }
        }
        saw_event
    })
    .await;
    assert!(
        got_event,
        "background maintenance reaper should have evicted the idle-timed-out \
         instance and emitted MaintenanceEvicted without any manual \
         run_maintenance call"
    );
    // The emit happens after the destroy in `run_maintenance`, so by now the
    // instance is definitely destroyed.
    assert!(
        resource.destroy_counter.load(Ordering::Relaxed) >= 1,
        "the evicted instance must have been destroyed"
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
    let pool_rt = Pooled::<PoolTestResource>::new(pool_config, 1);

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceGuard<PoolTestResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");
    drop(handle);

    // First prove the released instance actually recycled back into idle —
    // otherwise `destroy_counter == 0` could equally mean the release/recycle
    // path never completed, and the no-eviction assertion below would
    // false-pass.
    let mut recycled = false;
    let recycle_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < recycle_deadline {
        if let Some(stats) = manager
            .pool_stats::<PoolTestResource>(&ScopeLevel::Global)
            .await
            && stats.idle >= 1
        {
            recycled = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(recycled, "released instance never recycled back into idle");

    // Observe for LONGER than the floored maintenance cadence (>= 1s): if a
    // reaper were incorrectly spawned for this no-TTL pool, its first sweep
    // (which cannot fire before the 1s floor) would have time to evict the
    // idle instance. With no TTL no reaper exists, so it must stay un-evicted.
    let destroyed = poll_until(std::time::Duration::from_millis(1500), || {
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
        .expect("acquire should succeed");

    assert_eq!(handle.topology_tag(), TopologyTag::Resident);
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn manager_shutdown_rejects_acquire() {
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
        Pooled::<PoolTestResource>::new(pool_config, test_config().fingerprint())
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
        Pooled::<PoolTestResource>::new(pool_config, test_config().fingerprint())
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

#[async_trait::async_trait]
impl Provider for SlowCreatePoolResource {
    type Config = TestConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("slow-create-pool")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, Error> {
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

nebula_resource::no_credential_slots!(SlowCreatePoolResource);

impl PoolProvider for SlowCreatePoolResource {
    fn is_broken(&self, _runtime: &Arc<AtomicU64>) -> BrokenCheck {
        BrokenCheck::Healthy
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
            topology: Pooled::<SlowCreatePoolResource>::new(
                pool_config,
                test_config().fingerprint(),
            ),
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
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    // Acquire the single slot.
    let _held = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");

    // Very short deadline should override the 30s default timeout.
    let opts = AcquireOptions::default()
        .with_deadline(std::time::Instant::now() + std::time::Duration::from_millis(100));
    let start = std::time::Instant::now();
    let result = mgr.acquire_pooled::<PoolTestResource>(&ctx, &opts).await;

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
}

// ---------------------------------------------------------------------------
// Acquire-slow-log threshold (observational only — never affects outcome)
// ---------------------------------------------------------------------------

/// A per-call [`AcquireOptions::with_acquire_slow_threshold`] override well
/// below the resource's actual create latency must not block, delay, or
/// fail the acquire — the threshold only decides whether a WARN is logged.
#[tokio::test]
async fn pool_acquire_over_slow_threshold_still_succeeds() {
    let resource = SlowCreatePoolResource {
        in_flight: Arc::new(AtomicU64::new(0)),
        peak: Arc::new(AtomicU64::new(0)),
    };
    let manager = Manager::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        min_size: 0,
        max_size: 1,
        warmup: nebula_resource::topology::pooled::config::WarmupStrategy::None,
        ..Default::default()
    };
    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Pooled::<SlowCreatePoolResource>::new(
                pool_config,
                test_config().fingerprint(),
            ),
            recovery_gate: None,
        })
        .expect("register");

    let ctx = test_ctx();
    // Far below the fixture's 80ms create delay — every acquire against this
    // resource trips the threshold.
    let opts =
        AcquireOptions::default().with_acquire_slow_threshold(std::time::Duration::from_millis(1));
    let guard = manager
        .acquire_pooled::<SlowCreatePoolResource>(&ctx, &opts)
        .await
        .expect("a slow acquire must still succeed — the threshold is observational only");
    drop(guard);
}

/// The manager-wide [`ManagerConfig::with_acquire_slow_threshold`] default
/// applies when a call's own [`AcquireOptions`] does not override it, and is
/// equally non-interfering.
#[tokio::test]
async fn pool_acquire_under_manager_wide_slow_threshold_still_succeeds() {
    let resource = SlowCreatePoolResource {
        in_flight: Arc::new(AtomicU64::new(0)),
        peak: Arc::new(AtomicU64::new(0)),
    };
    let manager = Manager::with_config(
        ManagerConfig::default().with_acquire_slow_threshold(std::time::Duration::from_millis(1)),
    );
    let pool_config = nebula_resource::topology::pooled::config::Config {
        min_size: 0,
        max_size: 1,
        warmup: nebula_resource::topology::pooled::config::WarmupStrategy::None,
        ..Default::default()
    };
    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Pooled::<SlowCreatePoolResource>::new(
                pool_config,
                test_config().fingerprint(),
            ),
            recovery_gate: None,
        })
        .expect("register");

    let ctx = test_ctx();
    let guard = manager
        .acquire_pooled::<SlowCreatePoolResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("manager-wide slow threshold must not block or fail the acquire");
    drop(guard);
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
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();
    let handle = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");
    drop(handle);
    // Permit should be returned immediately on handle drop (not after async
    // recycle). A short sleep ensures the drop has executed.
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    // Second acquire must succeed — permit was returned.
    let handle2 = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("second acquire must not block — permit should be available");
    drop(handle2);
}

// ---------------------------------------------------------------------------
// Topology mismatch
// ---------------------------------------------------------------------------

// The former `topology_mismatch_returns_permanent_error` exercised a *runtime*
// rejection when a pool-registered resource was acquired via the resident path.
// With the converged `Provider::Topology` associated type a resource pins
// exactly one topology, so `acquire_resident::<PoolTestResource>` no longer
// compiles (`PoolTestResource::Topology = Pooled<Self>`, not `Resident<Self>`)
// — the mismatch is now a compile error, a strictly stronger guarantee. This
// positive test pins the surviving behavior: a pool resource acquired through
// the pool path succeeds, and its guard reports `TopologyTag::Pool`.
#[tokio::test]
async fn pool_resource_acquires_through_pool_path() {
    let manager = Manager::new();
    let resource = PoolTestResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 2,
        ..Default::default()
    };
    let pool_rt = Pooled::<PoolTestResource>::new(pool_config, 1);

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("pool registration must succeed");

    let ctx = test_ctx();
    let guard = manager
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("pool acquire must succeed through the pool path");
    assert_eq!(
        guard.topology_tag(),
        TopologyTag::Pool,
        "a pool-registered resource must report the Pool topology tag"
    );
}
