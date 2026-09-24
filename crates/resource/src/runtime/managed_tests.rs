use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};

use arc_swap::ArcSwap;
use nebula_core::{ExecutionId, ResourceKey, resource_key};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::{
    context::ResourceContext,
    error::Error,
    options::AcquireOptions,
    release_queue::ReleaseQueue,
    resource::{ResourceConfig, ResourceMetadataDraft, TeardownCx, TeardownReason},
    runtime::teardown::{destroy_within, teardown_deadline},
    topology::{Pooled, pooled::config::Config as PoolConfig, store::InstanceStore},
};

#[path = "managed/tests/retained.rs"]
mod retained_tests;

// A minimal pooled resource over which the framework acquire loop runs.
#[derive(Clone, nebula_schema::Schema)]
struct PoolCfg;
impl ResourceConfig for PoolCfg {
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[derive(Clone)]
struct Mock {
    created: Arc<AtomicU64>,
    destroyed: Arc<AtomicU64>,
    destroy_finished: Arc<Notify>,
    park_create: Arc<AtomicBool>,
    create_entered: Arc<Notify>,
    release_create: Arc<Notify>,
    checks: Arc<AtomicU64>,
    check_cost: crate::CheckCost,
    check_fails: Arc<AtomicBool>,
    check_panics: Arc<AtomicBool>,
}

impl Mock {
    fn new() -> Self {
        Self {
            created: Arc::new(AtomicU64::new(0)),
            destroyed: Arc::new(AtomicU64::new(0)),
            destroy_finished: Arc::new(Notify::new()),
            park_create: Arc::new(AtomicBool::new(false)),
            create_entered: Arc::new(Notify::new()),
            release_create: Arc::new(Notify::new()),
            checks: Arc::new(AtomicU64::new(0)),
            check_cost: crate::CheckCost::Cheap,
            check_fails: Arc::new(AtomicBool::new(false)),
            check_panics: Arc::new(AtomicBool::new(false)),
        }
    }

    fn with_check_cost(mut self, cost: crate::CheckCost) -> Self {
        self.check_cost = cost;
        self
    }
}

#[async_trait::async_trait]
impl Provider for Mock {
    type Config = PoolCfg;
    type Instance = u64;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("managed-loop-mock")
    }

    async fn create(&self, _config: &PoolCfg, _ctx: &ResourceContext) -> Result<u64, Error> {
        let id = self.created.fetch_add(1, Ordering::SeqCst);
        if self.park_create.swap(false, Ordering::SeqCst) {
            self.create_entered.notify_one();
            self.release_create.notified().await;
        }
        Ok(id)
    }

    async fn check(&self, _instance: &u64) -> Result<(), Error> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        assert!(
            !self.check_panics.load(Ordering::SeqCst),
            "mock health check panics (probe-isolation test)"
        );
        if self.check_fails.load(Ordering::SeqCst) {
            Err(Error::transient("mock health check failed"))
        } else {
            Ok(())
        }
    }

    fn check_cost(&self) -> crate::CheckCost {
        self.check_cost
    }

    async fn destroy(&self, _runtime: u64, _cx: TeardownCx) -> Result<(), Error> {
        self.destroyed.fetch_add(1, Ordering::SeqCst);
        self.destroy_finished.notify_one();
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("managed-loop-mock"), "")
    }
}

crate::no_credential_slots!(Mock);

impl crate::topology::pooled::PoolProvider for Mock {}

fn test_ctx() -> ResourceContext {
    use nebula_core::scope::Scope;
    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

fn managed(resource: Mock, config: PoolConfig) -> Arc<ManagedResource<Mock>> {
    let (rq, _handle) = ReleaseQueue::new(1);
    let topology = Pooled::<Mock>::new(config, 0);
    Arc::new(ManagedResource {
        resource,
        config: ArcSwap::from_pointee(PoolCfg),
        topology,
        store: InstanceStore::with_abandonment_tracker(None, rq.abandonment_tracker()),
        retained: crate::RetainedStore::new(rq.abandonment_tracker()),
        release_queue: Arc::new(rq),
        generation: AtomicU64::new(0),
        status: ArcSwap::from_pointee(ResourceStatus::new()),
        recovery_gate: None,
        tainted: AtomicBool::new(false),
        in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
        maintenance_sweeps: AtomicU64::new(0),
        maintenance: Default::default(),
        rate_limiter: crate::rate_limit::ResourceLimiter::detached(),
    })
}

#[tokio::test]
async fn loop_creates_then_recycles_then_reuses() {
    let resource = Mock::new();
    let created = Arc::clone(&resource.created);
    let mr = managed(
        resource,
        PoolConfig {
            max_size: 2,
            ..Default::default()
        },
    );

    // First acquire creates one entry.
    let g = mr
        .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
        .await
        .expect("first acquire");
    assert_eq!(*g, 0);
    // Release inline so the entry recycles into the framework store.
    let _release_outcome = g.release().await.expect("release recycles");
    assert_eq!(mr.store.len().await, 1, "the entry recycled into the store");

    // Second acquire reuses the idle entry — no new create.
    let g2 = mr
        .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
        .await
        .expect("second acquire");
    assert_eq!(*g2, 0, "reused the recycled entry");
    assert_eq!(
        created.load(Ordering::SeqCst),
        1,
        "the second acquire reused the idle entry — no extra create"
    );
    let _release_outcome = g2.release().await.expect("release");
}

/// The framework loop's revoke fence: an entry idle before a bump is evicted
/// (and destroyed by the framework) on the next acquire — the author writes
/// no fence code.
#[tokio::test]
async fn loop_evicts_revoke_stale_idle_entry_on_acquire() {
    let resource = Mock::new();
    let destroyed = Arc::clone(&resource.destroyed);
    let created = Arc::clone(&resource.created);
    let mr = managed(
        resource,
        PoolConfig {
            max_size: 2,
            ..Default::default()
        },
    );

    // Acquire + release so a clean entry sits idle.
    let g = mr
        .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
        .await
        .expect("acquire");
    let _release_outcome = g.release().await.expect("release");
    assert_eq!(mr.store.len().await, 1);

    // Revoke (the manager phase-1 synchronous bump).
    mr.bump_revoke_epoch();

    // Next acquire: the FRAMEWORK loop checks out, sees the stale entry,
    // destroys it, and creates a fresh one. The author wrote no fence code.
    let g2 = mr
        .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
        .await
        .expect("acquire after revoke");
    mr.resource.destroy_finished.notified().await;
    assert_eq!(
        destroyed.load(Ordering::SeqCst),
        1,
        "the framework destroyed the since-revoked idle entry on checkout"
    );
    assert_eq!(
        created.load(Ordering::SeqCst),
        2,
        "a fresh entry was created after the stale one was fenced"
    );
    // The fresh lease is the post-revoke instance, not the stale one.
    assert_eq!(*g2, 1);
    let _release_outcome = g2.release().await.expect("release");
}

/// Max-lifetime eviction keeps firing because the entry's `created_at`
/// survives the round-trip (entry-centric). An entry older than max_lifetime is
/// not re-handed-out: the loop's `accept` rejects it and the framework
/// creates a fresh one.
#[tokio::test]
async fn loop_max_lifetime_rejects_aged_idle_entry() {
    let resource = Mock::new();
    let created = Arc::clone(&resource.created);
    let mr = managed(
        resource,
        PoolConfig {
            max_size: 2,
            max_lifetime: Some(Duration::from_millis(20)),
            ..Default::default()
        },
    );

    let g = mr
        .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
        .await
        .expect("acquire");
    let _release_outcome = g.release().await.expect("release");
    assert_eq!(mr.store.len().await, 1);

    // Age the idle entry past max_lifetime.
    tokio::time::sleep(Duration::from_millis(40)).await;

    let g2 = mr
        .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
        .await
        .expect("acquire after aging");
    assert_eq!(
        created.load(Ordering::SeqCst),
        2,
        "the aged idle entry was rejected by `accept` (created_at survived \
         the round-trip) and a fresh entry was created"
    );
    let _release_outcome = g2.release().await.expect("release");
}

#[tokio::test(start_paused = true)]
async fn bounded_maintenance_join_acknowledges_worker_abort_before_return() {
    let managed = managed(Mock::new(), PoolConfig::default());
    let (alive, mut abandoned) = tokio::sync::oneshot::channel::<()>();
    managed.maintenance.set_task(tokio::spawn(async move {
        let _alive = alive;
        std::future::pending::<()>().await;
    }));
    assert_eq!(
        *managed.join_maintenance().await.unwrap_err().kind(),
        crate::ErrorKind::Cancelled
    );
    assert!(
        matches!(
            abandoned.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ),
        "bounded join must observe maintenance cancellation, not merely request abort"
    );
}

#[tokio::test(start_paused = true)]
async fn abort_before_terminal_idle_drain_counts_every_stored_owner() {
    let mut managed = managed(Mock::new(), PoolConfig::default());
    let (queue, workers) = ReleaseQueue::new(1);
    let row = Arc::get_mut(&mut managed).unwrap();
    row.store = InstanceStore::with_abandonment_tracker(None, queue.abandonment_tracker());
    row.retained = crate::RetainedStore::new(queue.abandonment_tracker());
    row.release_queue = Arc::new(queue);
    for _ in 0..3 {
        let entry = managed
            .topology
            .create_entry(&managed.resource, &PoolCfg, &test_ctx(), &managed.retained)
            .await
            .unwrap()
            .into_entry();
        assert!(!managed.store.deposit_fresh(entry, 0).await.is_evict());
    }
    let queue = Arc::clone(&managed.release_queue);
    let destroyed = Arc::clone(&managed.resource.destroyed);
    let weak = Arc::downgrade(&managed);
    let (entry_started, entry_observed) = tokio::sync::oneshot::channel();
    queue.submit(move || {
        Box::pin(async move {
            entry_started.send(()).unwrap();
            std::future::pending::<()>().await;
        })
    });
    entry_observed.await.unwrap();
    let (coordinator_started, coordinator_observed) = tokio::sync::oneshot::channel();
    queue
        .submit_coordinator(move || {
            Box::pin(async move {
                coordinator_started.send(()).unwrap();
                std::future::pending::<Result<(), Error>>().await
            })
        })
        .expect("open queue accepts coordinator blocker")
        .detach();
    coordinator_observed.await.unwrap();
    // Entry and coordinator execution are deliberately isolated. Occupy
    // both lanes so the terminal owner remains buffered and the test still
    // exercises abort-before-terminal-drain rather than normal teardown.
    queue
        .submit_coordinator(move || Box::pin(async move { managed.close_retained().await }))
        .expect("open queue must accept terminal cleanup")
        .detach();
    queue.close();
    assert!(
        ReleaseQueue::shutdown_bounded(workers, Duration::from_secs(1))
            .await
            .is_err()
    );
    assert!(
        weak.upgrade().is_none(),
        "aborted buffered terminal owner must release the row"
    );
    assert_eq!(
        destroyed.load(Ordering::SeqCst),
        0,
        "abandonment is not provider teardown"
    );
    assert_eq!(
        queue.dropped_count(),
        4,
        "three idle owners plus the running blocked job are each counted once"
    );
}

async fn settle_maintenance_cleanup(managed: &ManagedResource<Mock>) {
    // This fixture has one primary worker. The FIFO receipt runs after
    // the maintenance batch; publication itself deliberately never waits.
    assert_eq!(
        managed
            .release_queue
            .submit_release(|| Box::pin(async { Ok(()) }))
            .expect("open queue must accept cleanup checkpoint")
            .wait()
            .await
            .expect("cleanup checkpoint"),
        crate::release_queue::SubmissionOutcome::Completed,
    );
}

/// Maintenance over the framework store evicts both revoke-stale and
/// non-revoke (fingerprint) idle entries, destroying each.
#[tokio::test]
async fn maintenance_evicts_stale_and_revoked() -> Result<(), Error> {
    let resource = Mock::new();
    let destroyed = Arc::clone(&resource.destroyed);
    let mr = managed(
        resource,
        PoolConfig {
            max_size: 4,
            idle_timeout: None,
            max_lifetime: None,
            ..Default::default()
        },
    );

    // Two clean idle entries: hold BOTH guards live, then release both. A
    // serial acquire-release reuses the single idle entry (correct pooling),
    // which would deposit only one — so the two leases must overlap to
    // accumulate two distinct entries for the maintenance sweep to evict.
    let g1 = mr
        .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
        .await?;
    let g2 = mr
        .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
        .await?;
    let _first_outcome = g1.release().await?;
    let _second_outcome = g2.release().await?;
    assert_eq!(mr.store.len().await, 2);

    // No change yet → nothing evicted.
    assert_eq!(mr.run_maintenance().await, 0);

    // Bump fingerprint → both become non-revoke-evictable.
    mr.set_fingerprint(99);
    assert_eq!(mr.run_maintenance().await, 2);
    settle_maintenance_cleanup(&mr).await;
    assert_eq!(destroyed.load(Ordering::SeqCst), 2);
    assert_eq!(mr.store.len().await, 0);
    Ok(())
}

/// A11: the background health probe fires at a cadence set by
/// [`CheckCost`](crate::CheckCost) — a `Cheap` check is probed every sweep, an
/// `Expensive` one once per 16 sweeps, so an expensive probe does not hammer
/// an idle pool.
#[tokio::test]
async fn health_probe_cadence_scales_with_check_cost() -> Result<(), Error> {
    async fn one_idle(
        cost: crate::CheckCost,
    ) -> Result<(Arc<AtomicU64>, Arc<ManagedResource<Mock>>), Error> {
        let resource = Mock::new().with_check_cost(cost);
        let checks = Arc::clone(&resource.checks);
        let mr = managed(
            resource,
            PoolConfig {
                max_size: 2,
                ..Default::default()
            },
        );
        let g = mr
            .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
            .await?;
        let _release_outcome = g.release().await?;
        assert_eq!(mr.store.len().await, 1, "one entry recycled into the store");
        Ok((checks, mr))
    }

    let (cheap_checks, cheap) = one_idle(crate::CheckCost::Cheap).await?;
    let (expensive_checks, expensive) = one_idle(crate::CheckCost::Expensive).await?;

    for _ in 0..16 {
        cheap.run_maintenance().await;
        expensive.run_maintenance().await;
    }

    assert_eq!(
        cheap_checks.load(Ordering::SeqCst),
        16,
        "a Cheap check is probed on every one of the 16 sweeps"
    );
    assert_eq!(
        expensive_checks.load(Ordering::SeqCst),
        1,
        "an Expensive check is probed once in 16 sweeps (every 16th)"
    );
    Ok(())
}

/// A11: a probe whose `check` fails evicts and destroys the unhealthy idle
/// entry, so the next acquire rebuilds a fresh one.
#[tokio::test]
async fn health_probe_evicts_unhealthy_idle_entry() -> Result<(), Error> {
    let resource = Mock::new();
    let destroyed = Arc::clone(&resource.destroyed);
    let check_fails = Arc::clone(&resource.check_fails);
    let mr = managed(
        resource,
        PoolConfig {
            max_size: 2,
            ..Default::default()
        },
    );

    let g = mr
        .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
        .await?;
    let _release_outcome = g.release().await?;
    assert_eq!(mr.store.len().await, 1);

    // The entry's health check now fails — the probe must evict + destroy it.
    check_fails.store(true, Ordering::SeqCst);
    let evicted = mr.run_maintenance().await;
    settle_maintenance_cleanup(&mr).await;

    assert_eq!(evicted, 1, "the failing probe evicted the unhealthy entry");
    assert_eq!(
        mr.store.len().await,
        0,
        "the unhealthy entry left the store"
    );
    assert_eq!(
        destroyed.load(Ordering::SeqCst),
        1,
        "the probed-out entry was destroyed"
    );
    Ok(())
}

/// A11 foolproofing: a probe whose `check` PANICS is caught by the framework
/// (routed through `guard_author_hook`) — the reaper is not crashed, and the
/// entry is treated as unhealthy and evicted/destroyed.
#[tokio::test]
async fn health_probe_isolates_panicking_check() -> Result<(), Error> {
    let resource = Mock::new();
    let destroyed = Arc::clone(&resource.destroyed);
    let check_panics = Arc::clone(&resource.check_panics);
    let mr = managed(
        resource,
        PoolConfig {
            max_size: 2,
            ..Default::default()
        },
    );

    let g = mr
        .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
        .await?;
    let _release_outcome = g.release().await?;
    assert_eq!(mr.store.len().await, 1);

    // The probe's `check` now panics — the chokepoint must catch it (not
    // crash the reaper) and evict the entry.
    check_panics.store(true, Ordering::SeqCst);
    let evicted = mr.run_maintenance().await;
    settle_maintenance_cleanup(&mr).await;

    assert_eq!(
        evicted, 1,
        "a panicking probe is isolated and the entry evicted"
    );
    assert_eq!(mr.store.len().await, 0);
    assert_eq!(
        destroyed.load(Ordering::SeqCst),
        1,
        "the panicked-on entry was destroyed"
    );
    Ok(())
}

/// Warmup pre-creates `warmup_target` entries into the framework store.
#[tokio::test]
async fn warmup_fills_store() {
    let resource = Mock::new();
    let mr = managed(
        resource,
        PoolConfig {
            min_size: 3,
            max_size: 5,
            ..Default::default()
        },
    );
    let created = mr.warmup(&test_ctx()).await.expect("no hook fault");
    assert_eq!(created, 3, "warmup creates `min_size` entries");
    assert_eq!(mr.store.len().await, 3, "warmed entries land in the store");
}

// ----- ADR-0093 per-resource teardown deadline -----

/// A resource that declares a short `teardown_budget` and whose `destroy`
/// hangs forever. Drives the per-resource deadline tests.
#[derive(Clone)]
struct SlowTeardown {
    budget: Duration,
    last_reason: Arc<std::sync::Mutex<Option<TeardownReason>>>,
}

impl SlowTeardown {
    fn new(budget: Duration) -> Self {
        Self {
            budget,
            last_reason: Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for SlowTeardown {
    type Config = PoolCfg;
    type Instance = u64;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("slow-teardown-mock")
    }

    async fn create(&self, _config: &PoolCfg, _ctx: &ResourceContext) -> Result<u64, Error> {
        Ok(0)
    }

    fn teardown_budget(&self) -> Duration {
        self.budget
    }

    async fn destroy(&self, _runtime: u64, cx: TeardownCx) -> Result<(), Error> {
        if let Ok(mut slot) = self.last_reason.lock() {
            *slot = Some(cx.reason);
        }
        // Hang forever: the framework's per-resource deadline must abandon
        // this — the test proves the bound bites, not the body.
        std::future::pending::<()>().await;
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("slow-teardown-mock"), "")
    }
}

crate::no_credential_slots!(SlowTeardown);

impl crate::topology::pooled::PoolProvider for SlowTeardown {}

/// A sub-30s per-resource `teardown_budget` bounds a hanging `destroy`: the
/// framework abandons it at the deadline and returns a typed error rather
/// than blocking. `start_paused` fires the deadline deterministically with
/// no wall-clock wait. This is the deferred per-resource-deadline landing —
/// the previous release-hang test relied on the global 30s ceiling.
#[tokio::test(start_paused = true)]
async fn destroy_within_abandons_hanging_destroy_at_short_budget() {
    let resource = SlowTeardown::new(Duration::from_millis(50));
    let started = Instant::now();
    let outcome = destroy_within(&resource, 0u64, TeardownReason::Released).await;
    assert!(
        outcome.is_err(),
        "a hanging destroy must be abandoned at the per-resource deadline"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the 50ms budget — not the 30s global ceiling — bounded the teardown"
    );
}

/// A `Revoked` teardown caps at 5s even when the resource declares a 60s
/// budget: the composed deadline must sit at or below now+5s.
#[tokio::test(start_paused = true)]
async fn revoked_teardown_caps_at_five_seconds() {
    let resource = SlowTeardown::new(Duration::from_mins(1));

    // `teardown_deadline` adds the budget to a fresh `Instant::now()`. The
    // monotonic clock keeps ticking under `start_paused` (only tokio's timer
    // is paused), so bracket the cap with a small tolerance rather than an
    // exact equality: the revoke deadline must land close to now+5s and far
    // below the declared 60s.
    let before = Instant::now();
    let revoke_deadline = teardown_deadline(&resource, TeardownReason::Revoked);
    assert!(
        revoke_deadline <= before + Duration::from_secs(6),
        "a revoke teardown is capped at ~5s regardless of the declared 60s budget"
    );
    assert!(
        revoke_deadline >= before + Duration::from_secs(4),
        "the revoke cap is the 5s budget, not an over-aggressive clamp"
    );

    // The non-revoke path honors the full declared budget (sanity: the cap
    // is revoke-specific, not a blanket clamp).
    let release_deadline = teardown_deadline(&resource, TeardownReason::Released);
    assert!(
        release_deadline >= before + Duration::from_secs(59),
        "a non-revoke teardown keeps the full 60s budget"
    );

    // And a 60s-hanging destroy under revoke is abandoned ~5s in.
    let started = Instant::now();
    let outcome = destroy_within(&resource, 0u64, TeardownReason::Revoked).await;
    assert!(outcome.is_err(), "revoke teardown abandoned at the 5s cap");
    assert!(
        started.elapsed() <= Duration::from_secs(6),
        "the revoke cap (5s), not the 60s budget, bounded the teardown"
    );
}

/// A `destroy` impl observes `cx.reason`: the framework hands the reason it
/// composed the teardown for, so an author can adapt graceful behavior.
#[tokio::test(start_paused = true)]
async fn destroy_observes_teardown_reason() {
    let resource = SlowTeardown::new(Duration::from_millis(10));
    let recorder = Arc::clone(&resource.last_reason);

    // Hanging destroy is abandoned at the 10ms budget, but the reason is
    // recorded synchronously on entry before the hang.
    let _ = destroy_within(&resource, 0u64, TeardownReason::Shutdown).await;

    let observed = recorder.lock().ok().and_then(|g| *g);
    assert_eq!(
        observed,
        Some(TeardownReason::Shutdown),
        "the destroy impl saw the reason the framework tore it down for"
    );
}
