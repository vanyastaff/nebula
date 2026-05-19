//! Pool revoke→recycle TOCTOU (F1 / R16) — characterization.
//!
//! `Manager::drain_and_revoke` runs the pool revoke hook only over the
//! instances **currently in the idle queue** under the idle lock
//! (`PoolRuntime::dispatch_slot_hook_over_idle`). Every path that returns an
//! instance to the idle queue — the `ReleaseQueue` recycle, an in-flight
//! `create` that completes after the drain, and the `run_maintenance`
//! re-deposit — consults only `tainted` / `fingerprint` / `max_lifetime` /
//! `is_broken` / `recycle`. **None consults a per-row revoke epoch.** So a
//! credential revoked via `drain_and_revoke` can still be reached through a
//! post-drain idle instance: the revoke walked an empty (or pre-revoke) idle
//! set, then the escaped instance (re)entered idle un-revoked and was served
//! to the next acquirer.
//!
//! The contract R16 must enforce: a credential revoked via
//! `drain_and_revoke` is unreachable through ANY post-drain idle instance —
//! every return-to-idle path destroys (not recycles/admits) an instance
//! whose row revoke epoch advanced past its checkout/creation, before the
//! revoke hook is dispatched.
//!
//! Observable used here (no new API, compiles today): each `create` stamps a
//! unique sequence id on the runtime. After the racing scenario a fresh
//! acquire's id is compared against the escaped instance's id. If the
//! revoked instance was re-served the id matches (RED today). Once the fix
//! destroys the escaped instance the next acquire is a fresh `create` with a
//! new id, and `destroy` was invoked on the escaped one.
//!
//! All three variants are RED today and marked `#[ignore]` with a reason so
//! the suite stays green while the defect is recorded; the unit that lands
//! the revoke-epoch fence removes the ignore.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::time::Duration;

use nebula_core::{ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_resource::{
    AcquireOptions, Manager, PoolConfig, Resource, ResourceConfig, ResourceContext,
    error::Error,
    resource::ResourceMetadata,
    topology::pooled::{Pooled, RecycleDecision},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct PoolErr(String);

impl std::fmt::Display for PoolErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PoolErr {}

impl From<PoolErr> for Error {
    fn from(e: PoolErr) -> Self {
        Error::transient(e.0)
    }
}

#[derive(Clone)]
struct PoolCfg;

nebula_resource::impl_empty_has_schema!(PoolCfg);

impl ResourceConfig for PoolCfg {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
}

/// The pooled instance. `seq` is the unique creation id; `revoked` is set by
/// `on_credential_revoke` so a re-served instance is detectable as one whose
/// credential was revoked.
#[derive(Clone)]
struct PoolRt {
    seq: u64,
    revoked: Arc<AtomicBool>,
}

/// Coordinates the deterministic interleaving between the release/create
/// path and the revoke idle-walk.
#[derive(Clone, Default)]
struct Gate {
    /// `recycle` parks on this until the test releases it (variant a: the
    /// release-to-idle is held until strictly after the revoke walk).
    hold_recycle: Arc<Notify>,
    /// `recycle` fires this the instant it is entered (so the test knows the
    /// release worker is parked and can run the revoke walk next).
    recycle_entered: Arc<Notify>,
    /// When true, `recycle` performs the park.
    park_in_recycle: Arc<AtomicBool>,
    /// `create` parks on this (variant b).
    hold_create: Arc<Notify>,
    /// `create` fires this once it has been entered for the gated creation.
    create_entered: Arc<Notify>,
    /// When true, the *next* `create` parks (set only for the in-flight
    /// create the test wants to complete after the drain).
    park_in_create: Arc<AtomicBool>,
}

#[derive(Clone)]
struct PoolResource {
    create_seq: Arc<AtomicU64>,
    destroy_calls: Arc<AtomicUsize>,
    revoke_calls: Arc<AtomicUsize>,
    /// Shared "this credential was revoked" flag, stamped onto every runtime
    /// the revoke hook touches.
    revoked: Arc<AtomicBool>,
    gate: Gate,
}

impl PoolResource {
    fn new() -> Self {
        Self {
            create_seq: Arc::new(AtomicU64::new(1)),
            destroy_calls: Arc::new(AtomicUsize::new(0)),
            revoke_calls: Arc::new(AtomicUsize::new(0)),
            revoked: Arc::new(AtomicBool::new(false)),
            gate: Gate::default(),
        }
    }
}

impl Resource for PoolResource {
    type Config = PoolCfg;
    type Runtime = PoolRt;
    type Lease = PoolRt;
    type Error = PoolErr;

    fn key() -> ResourceKey {
        resource_key!("r16-pool")
    }

    async fn create(&self, _config: &PoolCfg, _ctx: &ResourceContext) -> Result<PoolRt, PoolErr> {
        let seq = self.create_seq.fetch_add(1, Ordering::SeqCst);
        if self.gate.park_in_create.swap(false, Ordering::SeqCst) {
            self.gate.create_entered.notify_one();
            self.gate.hold_create.notified().await;
        }
        Ok(PoolRt {
            seq,
            revoked: Arc::clone(&self.revoked),
        })
    }

    async fn destroy(&self, _runtime: PoolRt) -> Result<(), PoolErr> {
        self.destroy_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn on_credential_revoke(&self, _slot: &str, runtime: &PoolRt) -> Result<(), PoolErr> {
        // Model "stop serving the revoked credential": mark the shared flag.
        runtime.revoked.store(true, Ordering::SeqCst);
        self.revoke_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Pooled for PoolResource {
    async fn recycle(
        &self,
        _runtime: &PoolRt,
        _metrics: &nebula_resource::InstanceMetrics,
    ) -> Result<RecycleDecision, PoolErr> {
        if self.gate.park_in_recycle.load(Ordering::SeqCst) {
            self.gate.recycle_entered.notify_one();
            self.gate.hold_recycle.notified().await;
        }
        Ok(RecycleDecision::Keep)
    }
}

fn ctx() -> ResourceContext {
    ResourceContext::minimal(Scope::default(), CancellationToken::new())
}

fn pool_config() -> PoolConfig {
    PoolConfig {
        min_size: 0,
        max_size: 4,
        // Disable maintenance-driven eviction so the only thing that can
        // remove a revoked instance is the revoke-epoch fence under test.
        idle_timeout: None,
        max_lifetime: None,
        create_timeout: Duration::from_secs(5),
        ..PoolConfig::default()
    }
}

/// Variant (a): acquire → drop (release queued, parked in `recycle`) →
/// `revoke_slot` runs while idle is empty (the revoke hook walks nothing) →
/// release the parked recycle so the instance re-enters idle → a fresh
/// acquire is served that same instance, whose credential was revoked.
///
/// RED: today the recycled instance is served again (its `seq` matches and
/// `destroy` was never called). The fix must destroy it on the revoke-epoch
/// re-check at the recycle path, so the next acquire is a fresh `create`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "RED until the R16 revoke-epoch fence — proves the release/recycle TOCTOU"]
async fn revoked_credential_not_reserved_via_idle_recycle() {
    let resource = PoolResource::new();
    let mgr = Arc::new(Manager::new());
    mgr.register_pooled(resource.clone(), PoolCfg, pool_config())
        .expect("register_pooled must succeed");

    // 1. Acquire an instance and capture its creation id.
    let g = mgr
        .acquire_pooled::<PoolResource>(&ctx(), &AcquireOptions::default())
        .await
        .expect("first acquire must succeed");
    let escaped_seq = g.seq;

    // 2. Arm the recycle park, then drop the handle. The release task is
    //    enqueued and parks inside `recycle` — the instance is NOT yet back
    //    in the idle queue.
    resource.gate.park_in_recycle.store(true, Ordering::SeqCst);
    drop(g);
    resource.gate.recycle_entered.notified().await;

    // 3. Revoke now. The idle queue is empty (the instance is parked in the
    //    release path), so the pool revoke hook walks nothing.
    mgr.revoke_slot(&PoolResource::key(), ScopeLevel::Global, "db")
        .await
        .expect("revoke_slot must succeed");
    assert_eq!(
        resource.revoke_calls.load(Ordering::SeqCst),
        0,
        "the revoke walked an empty idle set — the escaped instance was \
         never visited by on_credential_revoke (this is the TOCTOU)"
    );

    // 4. Release the parked recycle: the instance re-enters the idle queue
    //    (or, post-fix, is destroyed by the revoke-epoch re-check). Wait for
    //    the release worker to settle either way.
    resource.gate.park_in_recycle.store(false, Ordering::SeqCst);
    resource.gate.hold_recycle.notify_one();
    for _ in 0..200 {
        let idle = mgr
            .pool_stats::<PoolResource>(&ScopeLevel::Global)
            .await
            .map_or(0, |s| s.idle);
        if idle >= 1 || resource.destroy_calls.load(Ordering::SeqCst) >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // 5. A fresh acquire must NOT be served the revoked instance. Post-fix
    //    the recycled instance is destroyed on the revoke-epoch re-check, so
    //    this acquire is a fresh `create` with a new id.
    let g2 = mgr
        .acquire_pooled::<PoolResource>(&ctx(), &AcquireOptions::default())
        .await
        .expect("second acquire must succeed");

    assert!(
        g2.seq != escaped_seq,
        "the revoked credential's instance (seq={escaped_seq}) was re-served \
         via idle recycle after drain_and_revoke (cross-tenant reuse); the \
         revoke-epoch fence must destroy it so the next acquire is a fresh \
         create"
    );
    assert!(
        resource.destroy_calls.load(Ordering::SeqCst) >= 1,
        "the escaped (revoked) instance must have been destroyed, not \
         recycled"
    );
}

/// Variant (b): an in-flight `create` started before the revoke, completing
/// after `drain_and_revoke` (HikariCP #1836). The post-drain-created
/// instance must be destroyed via the revoke-epoch re-check, never admitted
/// to idle / handed onward.
///
/// RED: today the create completes and the instance is handed to the caller
/// / admitted with no epoch check.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "RED until the R16 revoke-epoch fence — proves the in-flight-create-after-revoke race (HikariCP #1836)"]
async fn in_flight_create_completing_after_revoke_is_destroyed() {
    let resource = PoolResource::new();
    let mgr = Arc::new(Manager::new());
    mgr.register_pooled(resource.clone(), PoolCfg, pool_config())
        .expect("register_pooled must succeed");

    // 1. Start an acquire whose `create` will park (idle empty → it creates).
    resource.gate.park_in_create.store(true, Ordering::SeqCst);
    let acquire_task = {
        let mgr = Arc::clone(&mgr);
        tokio::spawn(async move {
            mgr.acquire_pooled::<PoolResource>(&ctx(), &AcquireOptions::default())
                .await
        })
    };
    // Wait until `create` is parked (in flight, not yet returned).
    resource.gate.create_entered.notified().await;

    // 2. Revoke while the create is in flight. Idle is empty; nothing to
    //    walk. The taint is applied synchronously.
    let tainted = mgr
        .taint_slot(&PoolResource::key(), ScopeLevel::Global, "db")
        .expect("taint_slot must resolve synchronously");

    // 3. Release the parked create so it completes strictly after the taint
    //    (and will complete after the drain runs).
    resource.gate.hold_create.notify_one();

    let drain_task = {
        let mgr = Arc::clone(&mgr);
        tokio::spawn(async move { mgr.drain_and_revoke(tainted, Duration::from_secs(30)).await })
    };

    let guard = acquire_task.await.expect("acquire task must not panic");
    let _ = drain_task.await.expect("drain task must not panic");

    // The instance created after the revoke must not be a usable, un-revoked
    // handle: post-fix the revoke-epoch re-check destroys it (the acquire
    // then either errors or yields a fresh post-fence instance — never a
    // silently-admitted revoked one). The minimal invariant we can assert
    // without the new API: the post-revoke-created instance was destroyed.
    if let Ok(g) = &guard {
        assert!(
            g.revoked.load(Ordering::SeqCst) || resource.destroy_calls.load(Ordering::SeqCst) >= 1,
            "an instance created after drain_and_revoke must be fenced \
             (destroyed by the revoke-epoch re-check), never admitted as a \
             healthy un-revoked handle (HikariCP #1836)"
        );
    }
    assert!(
        resource.destroy_calls.load(Ordering::SeqCst) >= 1,
        "the post-revoke-created instance must have been destroyed by the \
         revoke-epoch fence"
    );
}

/// Variant (c): a pre-revoke idle instance that is non-stale and
/// non-timed-out. `drain_and_revoke`'s idle walk runs the revoke hook over
/// it, but the entry STAYS in the idle queue (the hook does not evict it),
/// and `run_maintenance`'s `should_evict` consults only
/// `fingerprint`/`max_lifetime`/`idle_timeout` — so a maintenance cycle
/// `keep.push_back`s it. The revoked instance is then served to the next
/// acquirer.
///
/// RED: today the revoked idle instance is re-served (same `seq`, never
/// destroyed). The revoke-epoch arm of the return-to-idle paths (including
/// the `run_maintenance` re-deposit) must destroy it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "RED until the R16 revoke-epoch fence — proves the run_maintenance/should_evict re-deposit gap"]
async fn revoked_pre_existing_idle_instance_not_reserved() {
    let resource = PoolResource::new();
    let mgr = Arc::new(Manager::new());
    mgr.register_pooled(resource.clone(), PoolCfg, pool_config())
        .expect("register_pooled must succeed");

    // 1. Warm one idle instance (pre-revoke), capture its id. Acquire then
    //    drop so a fully-recycled instance sits in idle (recycle is NOT
    //    parked here — park flag defaults false).
    let g = mgr
        .acquire_pooled::<PoolResource>(&ctx(), &AcquireOptions::default())
        .await
        .expect("warm acquire must succeed");
    let idle_seq = g.seq;
    drop(g);

    // Let the release worker recycle it back into idle.
    let recycled = {
        let mgr = Arc::clone(&mgr);
        let mut ok = false;
        for _ in 0..200 {
            if let Some(stats) = mgr.pool_stats::<PoolResource>(&ScopeLevel::Global).await
                && stats.idle >= 1
            {
                ok = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        ok
    };
    assert!(recycled, "the dropped instance must recycle back into idle");

    // 2. Revoke. The pool revoke hook now walks the one idle entry and marks
    //    it revoked — but the entry remains in the idle queue (the hook does
    //    not evict; `should_evict` does not know about revoke).
    mgr.revoke_slot(&PoolResource::key(), ScopeLevel::Global, "db")
        .await
        .expect("revoke_slot must succeed");
    assert_eq!(
        resource.revoke_calls.load(Ordering::SeqCst),
        1,
        "the revoke hook must have visited the single pre-revoke idle entry"
    );

    // 3. A fresh acquire must NOT be served that revoked idle instance.
    //    Post-fix the revoke-epoch arm (on the recycle / maintenance
    //    re-deposit path) destroys it, so the next acquire is a fresh
    //    `create` with a new id.
    let g2 = mgr
        .acquire_pooled::<PoolResource>(&ctx(), &AcquireOptions::default())
        .await
        .expect("post-revoke acquire must succeed");

    assert!(
        g2.seq != idle_seq,
        "the revoked pre-existing idle instance (seq={idle_seq}) was \
         re-served (should_evict / the return-to-idle paths ignore the \
         revoke epoch today); the fence must destroy it"
    );
    assert!(
        !g2.revoked.load(Ordering::SeqCst),
        "the freshly served instance must not carry the revoked flag"
    );
    assert!(
        resource.destroy_calls.load(Ordering::SeqCst) >= 1,
        "the revoked idle instance must have been destroyed, not kept"
    );
}
