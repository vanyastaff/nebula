use std::{future::Future, sync::atomic::AtomicBool, time::Duration};

use nebula_core::{ExecutionId, ResourceKey, resource_key};

use super::*;
use crate::{
    context::ResourceContext,
    resource::{ResourceConfig, ResourceMetadataDraft},
    topology::{pooled::BrokenCheck, store::ReturnOutcome},
};

#[derive(Clone, nebula_schema::Schema)]
struct PoolTestConfig;

impl ResourceConfig for PoolTestConfig {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        // Unit struct: all instances identical — constant 0 is correct.
        0
    }
}

#[derive(Clone)]
struct MockPool {
    created: Arc<AtomicU64>,
    destroyed: Arc<AtomicU64>,
    broken: Arc<AtomicBool>,
    fail_check: Arc<AtomicBool>,
    recycle_drop: Arc<AtomicBool>,
    revoke_calls: Arc<AtomicU64>,
    /// Panic-isolation fixture: `on_credential_revoke` panics when
    /// called with this instance id (`None` = never panics).
    panic_revoke_for: Arc<std::sync::Mutex<Option<u64>>>,
}

impl MockPool {
    fn new() -> Self {
        Self {
            created: Arc::new(AtomicU64::new(0)),
            destroyed: Arc::new(AtomicU64::new(0)),
            broken: Arc::new(AtomicBool::new(false)),
            fail_check: Arc::new(AtomicBool::new(false)),
            recycle_drop: Arc::new(AtomicBool::new(false)),
            revoke_calls: Arc::new(AtomicU64::new(0)),
            panic_revoke_for: Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for MockPool {
    type Config = PoolTestConfig;
    type Instance = u64;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("mock-pool")
    }

    async fn create(&self, _config: &PoolTestConfig, _ctx: &ResourceContext) -> Result<u64, Error> {
        Ok(self.created.fetch_add(1, Ordering::SeqCst))
    }

    async fn check(&self, _runtime: &u64) -> Result<(), Error> {
        if self.fail_check.load(Ordering::SeqCst) {
            Err(Error::transient("check failed"))
        } else {
            Ok(())
        }
    }

    async fn destroy(&self, _runtime: u64, _cx: crate::TeardownCx) -> Result<(), Error> {
        self.destroyed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn on_credential_revoke(&self, _slot: &str, runtime: &u64) -> Result<(), Error> {
        assert!(
            *self.panic_revoke_for.lock().unwrap() != Some(*runtime),
            "MockPool::on_credential_revoke: forced panic fixture"
        );
        self.revoke_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("mock-pool"), "")
    }
}

crate::no_credential_slots!(MockPool);

impl PoolProvider for MockPool {
    fn is_broken(&self, _runtime: &u64) -> BrokenCheck {
        if self.broken.load(Ordering::SeqCst) {
            BrokenCheck::Broken("forced break".into())
        } else {
            BrokenCheck::Healthy
        }
    }

    fn recycle(
        &self,
        _instance: &u64,
        _metrics: &InstanceMetrics,
    ) -> impl Future<Output = Result<RecycleDecision, Error>> + Send {
        let drop = self.recycle_drop.load(Ordering::SeqCst);
        async move {
            if drop {
                Ok(RecycleDecision::Drop)
            } else {
                Ok(RecycleDecision::Keep)
            }
        }
    }
}

fn test_ctx() -> ResourceContext {
    use nebula_core::scope::Scope;
    use tokio_util::sync::CancellationToken;
    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

fn mock_pool(config: Config, fingerprint: u64) -> Pooled<MockPool> {
    Pooled::<MockPool>::new(config, fingerprint)
}

#[tokio::test]
async fn create_entry_builds_pool_entry_with_metrics() {
    let resource = MockPool::new();
    let topo = mock_pool(Config::default(), 0);
    let entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create_entry must succeed");
    assert_eq!(entry.metrics.checkout_count, 1);
    assert_eq!(resource.created.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn create_entry_tolerates_unrepresentable_create_timeout() {
    // `Instant::now() + Duration::MAX` panicked on the first cold acquire;
    // an operator-supplied huge timeout must mean "no practical deadline".
    let resource = MockPool::new();
    let topo = mock_pool(
        Config {
            create_timeout: Duration::MAX,
            ..Config::default()
        },
        0,
    );
    topo.create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("an unbounded create_timeout must not panic or time out");
    assert_eq!(resource.created.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn entry_instance_and_into_owned_instance_round_trip() {
    let resource = MockPool::new();
    let topo = mock_pool(Config::default(), 0);
    let entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    let id = *topo.entry_instance(&entry);
    let owned = topo.into_owned_instance(entry);
    assert_eq!(
        owned,
        Some(id),
        "ownership conversion returns the same instance"
    );
}

#[tokio::test]
async fn accept_accepts_healthy_and_bumps_checkout() {
    let resource = MockPool::new();
    let topo = mock_pool(Config::default(), 0);
    let mut entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    let before = entry.metrics.checkout_count;
    assert!(topo.accept(&mut entry, &resource, &test_ctx()).await);
    assert_eq!(entry.metrics.checkout_count, before + 1);
}

#[tokio::test]
async fn accept_rejects_broken() {
    let resource = MockPool::new();
    let topo = mock_pool(Config::default(), 0);
    let mut entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    resource.broken.store(true, Ordering::SeqCst);
    assert!(!topo.accept(&mut entry, &resource, &test_ctx()).await);
}

#[tokio::test]
async fn accept_rejects_stale_fingerprint() {
    let resource = MockPool::new();
    let topo = mock_pool(Config::default(), 0);
    let mut entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    topo.set_fingerprint(7);
    assert!(!topo.accept(&mut entry, &resource, &test_ctx()).await);
}

#[tokio::test]
async fn accept_rejects_failed_health_check() {
    let resource = MockPool::new();
    let cfg = Config {
        test_on_checkout: true,
        ..Default::default()
    };
    let topo = mock_pool(cfg, 0);
    let mut entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    resource.fail_check.store(true, Ordering::SeqCst);
    assert!(!topo.accept(&mut entry, &resource, &test_ctx()).await);
}

#[tokio::test]
async fn on_release_keeps_clean_entry() {
    let resource = MockPool::new();
    let topo = mock_pool(Config::default(), 0);
    let mut entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    assert!(
        topo.on_release(&mut entry, &resource)
            .await
            .expect("release")
    );
    assert!(entry.returned_at.is_some(), "on_release stamps returned_at");
}

#[tokio::test]
async fn on_release_drops_stale_fingerprint() {
    let resource = MockPool::new();
    let topo = mock_pool(Config::default(), 0);
    let mut entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    topo.set_fingerprint(99);
    assert!(
        !topo
            .on_release(&mut entry, &resource)
            .await
            .expect("release")
    );
}

#[tokio::test]
async fn on_release_drops_broken() {
    let resource = MockPool::new();
    let topo = mock_pool(Config::default(), 0);
    let mut entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    resource.broken.store(true, Ordering::SeqCst);
    assert!(
        !topo
            .on_release(&mut entry, &resource)
            .await
            .expect("release")
    );
}

#[tokio::test]
async fn on_release_honours_recycle_drop() {
    let resource = MockPool::new();
    let topo = mock_pool(Config::default(), 0);
    let mut entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    resource.recycle_drop.store(true, Ordering::SeqCst);
    assert!(
        !topo
            .on_release(&mut entry, &resource)
            .await
            .expect("release")
    );
}

#[tokio::test]
async fn queue_strategy_forwards_the_configured_strategy() {
    use crate::topology::store::PoolStrategy;
    // The registration path builds the framework store from this hook —
    // a Pooled topology must forward its configured strategy, not the
    // trait's FIFO fallback (the pre-fix behavior: `Config::strategy`
    // was declared but never read, so every pool silently ran FIFO).
    let lifo_pool = mock_pool(Config::default(), 0);
    assert_eq!(
        Topology::<MockPool>::queue_strategy(&lifo_pool),
        PoolStrategy::Lifo,
        "default pool config promises LIFO and the store must honor it"
    );
    let fifo_pool = mock_pool(
        Config {
            strategy: PoolStrategy::Fifo,
            ..Default::default()
        },
        0,
    );
    assert_eq!(
        Topology::<MockPool>::queue_strategy(&fifo_pool),
        PoolStrategy::Fifo
    );
}

#[tokio::test]
async fn idle_evictable_stale_fingerprint() {
    let resource = MockPool::new();
    let topo = mock_pool(Config::default(), 0);
    let entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    assert!(
        !topo.idle_evictable(&entry),
        "a fresh entry is not evictable"
    );
    topo.set_fingerprint(5);
    assert!(
        topo.idle_evictable(&entry),
        "a stale-fingerprint entry is idle-evictable"
    );
}

#[tokio::test]
async fn idle_evictable_max_lifetime() {
    let cfg = Config {
        max_lifetime: Some(Duration::from_nanos(1)),
        ..Default::default()
    };
    let resource = MockPool::new();
    let topo = mock_pool(cfg, 0);
    let entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert!(topo.idle_evictable(&entry));
}

// ── max_lifetime jitter ──────────────────────────────────────────────────

/// Each created entry's jittered max-lifetime threshold stays within
/// HikariCP's `[0.95*L, L]` attenuation band — proportional to the
/// configured lifetime, never below it, never above.
#[tokio::test]
async fn jittered_max_lifetime_stays_within_hikaricp_band() {
    let lifetime = Duration::from_mins(30);
    let lower_bound = lifetime.mul_f64(0.95);
    let cfg = Config {
        max_lifetime: Some(lifetime),
        ..Default::default()
    };
    let resource = MockPool::new();
    let topo = mock_pool(cfg, 0);

    for _ in 0..50 {
        let entry = topo
            .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        let jittered = entry
            .jittered_max_lifetime
            .expect("max_lifetime is configured");
        assert!(
            jittered >= lower_bound,
            "{jittered:?} fell below the 0.95*lifetime band floor"
        );
        assert!(
            jittered <= lifetime,
            "{jittered:?} exceeded the configured lifetime"
        );
    }
}

/// A pool with `max_lifetime: None` is unaffected: entries never carry a
/// jittered threshold and are never max-lifetime-evictable.
#[tokio::test]
async fn no_max_lifetime_configured_leaves_entries_unjittered() {
    let cfg = Config {
        max_lifetime: None,
        ..Default::default()
    };
    let resource = MockPool::new();
    let topo = mock_pool(cfg, 0);
    let entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    assert_eq!(entry.jittered_max_lifetime, None);
    assert!(
        !topo.idle_evictable(&entry),
        "no max_lifetime configured ⇒ never max-lifetime-evictable"
    );
}

/// The jittered threshold is stable across repeated maintenance checks —
/// it is stamped once at creation, not re-drawn on every
/// `idle_evictable` call (which would make eviction timing flap
/// non-deterministically between reaper ticks for the same entry).
#[tokio::test]
async fn jittered_max_lifetime_is_stable_across_repeated_checks() {
    let cfg = Config {
        max_lifetime: Some(Duration::from_mins(30)),
        ..Default::default()
    };
    let resource = MockPool::new();
    let topo = mock_pool(cfg, 0);
    let entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    let first = entry.jittered_max_lifetime;
    for _ in 0..10 {
        assert!(!topo.idle_evictable(&entry), "fresh entry not evictable");
        assert_eq!(
            entry.jittered_max_lifetime, first,
            "the jittered threshold must not change across repeated checks"
        );
    }
}

#[tokio::test]
async fn try_reserve_grants_then_saturates() {
    let topo = mock_pool(
        Config {
            max_size: 1,
            ..Default::default()
        },
        0,
    );
    let store: InstanceStore<PoolEntry<MockPool>> = InstanceStore::new(None);
    let ticket = topo.try_reserve(&store).expect("first ticket");
    assert!(
        matches!(topo.try_reserve(&store), Err(Unavailable::Saturated { .. })),
        "a pool of 1 is saturated after one ticket"
    );
    assert_eq!(topo.phase(&store), AdmissionPhase::Saturated);
    drop(ticket);
    assert_eq!(topo.phase(&store), AdmissionPhase::Ready);
}

#[tokio::test]
async fn load_reflects_usage() {
    let topo = mock_pool(
        Config {
            max_size: 2,
            ..Default::default()
        },
        0,
    );
    let store: InstanceStore<PoolEntry<MockPool>> = InstanceStore::new(None);
    let load = topo.load(&store).expect("pool reports load");
    assert!(load.saturation.abs() < f32::EPSILON, "idle pool is 0.0");
    let _t = topo.try_reserve(&store).expect("ticket");
    let load = topo.load(&store).expect("load");
    assert!(
        (load.saturation - 0.5).abs() < f32::EPSILON,
        "one of two used"
    );
}

#[tokio::test]
async fn stats_reads_store_and_semaphore() {
    let resource = MockPool::new();
    let topo = mock_pool(
        Config {
            max_size: 4,
            ..Default::default()
        },
        0,
    );
    let store: InstanceStore<PoolEntry<MockPool>> = InstanceStore::new(None);
    let entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    let epoch = store.stamp_epoch();
    let _ = store.return_entry(entry, epoch).await;
    let stats = topo.stats(&store).await;
    assert_eq!(stats.idle, 1);
    assert_eq!(stats.capacity, 4);
}

#[tokio::test]
async fn topology_metadata_hooks() {
    let topo = mock_pool(
        Config {
            min_size: 3,
            max_size: 5,
            ..Default::default()
        },
        0,
    );
    assert_eq!(topo.tag(), TopologyTag::Pool);
    assert!(topo.pools(), "the pool topology pools released entries");
    assert_eq!(topo.warmup_target(&PoolTestConfig), 3);
    assert!(
        topo.maintenance_schedule().is_some(),
        "the pool runs a maintenance reaper"
    );
}

#[tokio::test]
async fn dispatch_credential_hook_walks_idle_store() {
    let resource = MockPool::new();
    let topo = mock_pool(Config::default(), 0);
    let store: InstanceStore<PoolEntry<MockPool>> = InstanceStore::new(None);

    // Two idle entries.
    for _ in 0..2 {
        let entry = topo
            .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        let epoch = store.stamp_epoch();
        let _ = store.return_entry(entry, epoch).await;
    }

    topo.dispatch_credential_hook(
        &resource,
        &store,
        &crate::RetainedStore::for_test(),
        "db",
        false,
    )
    .await
    .expect("rotation dispatch");
    assert_eq!(
        resource.revoke_calls.load(Ordering::SeqCst),
        2,
        "the revoke hook visits every idle entry in the framework store"
    );
}

/// Each idle entry's credential hook is bounded + panic-isolated
/// *individually*, not only by the caller's single outer guard. A
/// panicking hook on one entry must not abort the fan-out before the
/// remaining entries get their own hook attempt — before this fix the
/// whole `for entry in &*idle` loop had no per-entry isolation, so an
/// unwind from entry N would propagate straight out of
/// `dispatch_credential_hook` (caught only by the caller's OUTER guard,
/// which by then has already abandoned every entry after N).
#[tokio::test]
async fn dispatch_credential_hook_isolates_a_panicking_entry_and_continues() {
    let resource = MockPool::new();
    let topo = mock_pool(Config::default(), 0);
    let store: InstanceStore<PoolEntry<MockPool>> = InstanceStore::new(None);

    // Two idle entries — instances `0` and `1` (creation order).
    for _ in 0..2 {
        let entry = topo
            .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        let epoch = store.stamp_epoch();
        let _ = store.return_entry(entry, epoch).await;
    }
    // Force the FIRST entry's hook to panic.
    *resource.panic_revoke_for.lock().unwrap() = Some(0);

    let outcome = topo
        .dispatch_credential_hook(
            &resource,
            &store,
            &crate::RetainedStore::for_test(),
            "db",
            false,
        )
        .await;
    assert!(
        outcome.is_err(),
        "a panicking per-entry hook must surface as a typed error, not \
         a silent success"
    );
    let crate::topology::HookFault::Failed(error) =
        outcome.expect_err("panic must produce a hook fault")
    else {
        panic!("an isolated hook panic must not be classified as a timeout")
    };
    assert!(
        !error.is_retryable(),
        "an isolated hook panic is a permanent author-hook bug"
    );
    assert_eq!(
        resource.revoke_calls.load(Ordering::SeqCst),
        1,
        "the SECOND idle entry must still receive its revoke hook — the \
         panic on the first entry must not abort the whole fan-out \
         (per-entry isolation)"
    );
}

#[test]
fn try_new_rejects_max_size_zero() {
    let err = match Pooled::<MockPool>::try_new(
        Config {
            max_size: 0,
            ..Config::default()
        },
        1,
    ) {
        Err(e) => e,
        Ok(_) => panic!("max_size == 0 must be a typed registration error, not a pool"),
    };
    assert_eq!(*err.kind(), crate::error::ErrorKind::Permanent);
    assert!(err.to_string().contains("max_size"));
}

#[test]
fn try_new_rejects_min_greater_than_max() {
    let err = match Pooled::<MockPool>::try_new(
        Config {
            min_size: 5,
            max_size: 2,
            ..Config::default()
        },
        1,
    ) {
        Err(e) => e,
        Ok(_) => panic!("min > max must be a typed registration error, not a pool"),
    };
    assert_eq!(*err.kind(), crate::error::ErrorKind::Permanent);
    assert!(err.to_string().contains("min_size") && err.to_string().contains("max_size"));
}

/// The revoke fence is the store's, framework-owned: an entry returned at the
/// pre-bump epoch must be evicted on return after a bump. This exercises the
/// exact store path the framework `release_entry` uses for the pool.
#[tokio::test]
async fn store_return_is_revoke_fenced() {
    let resource = MockPool::new();
    let topo = mock_pool(Config::default(), 0);
    let store: InstanceStore<PoolEntry<MockPool>> = InstanceStore::new(Some(4));

    let entry = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    let epoch = store.stamp_epoch();
    store.bump_revoke_epoch();
    assert!(
        store.return_entry(entry, epoch).await.is_evict(),
        "an entry checked out before a revoke must be evicted by the store fence"
    );

    let entry2 = topo
        .create_pool_entry(&resource, &PoolTestConfig, &test_ctx())
        .await
        .expect("create");
    let fresh = store.stamp_epoch();
    // `ReturnOutcome<PoolEntry<_>>` is not `Debug`/`PartialEq` (the entry is
    // not), so match the recycled arm rather than comparing for equality.
    assert!(
        matches!(
            store.return_entry(entry2, fresh).await,
            ReturnOutcome::Recycled
        ),
        "an entry checked out after the revoke is unaffected"
    );
    assert_eq!(store.len().await, 1, "the post-revoke entry recycled");
}
