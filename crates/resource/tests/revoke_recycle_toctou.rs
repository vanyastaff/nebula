//! Pool revoke→recycle TOCTOU (F1 / R16) — the revoke-epoch fence.
//!
//! `Manager::drain_and_revoke` runs the pool revoke hook only over the
//! instances **currently in the idle queue** under the idle lock
//! (`PoolRuntime::dispatch_slot_hook_over_idle`). Before the fence, every
//! path that returns an instance to the idle queue — the `ReleaseQueue`
//! recycle, an in-flight `create` that completes after the drain, a
//! concurrent warmup, and the `run_maintenance` re-deposit — consulted only
//! `tainted` / `fingerprint` / `max_lifetime` / `is_broken` / `recycle`,
//! none of which knows about a credential revoke. So a credential revoked
//! via `drain_and_revoke` could still be reached through a post-drain idle
//! instance: the revoke walked an empty (or pre-revoke) idle set, then the
//! escaped instance (re)entered idle un-revoked and was served to the next
//! acquirer (cross-tenant reuse).
//!
//! The contract these tests pin: a credential revoked via
//! `drain_and_revoke` is unreachable through ANY post-drain idle instance.
//! The pool carries a per-row revoke counter bumped synchronously when the
//! credential is revoked (before the hook is dispatched, the same
//! synchronous-before-`.await` discipline as the resource taint). Every
//! instance snapshots that counter at the start of its creation; every
//! return-to-idle path destroys (never recycles or admits) an instance
//! whose snapshot is behind the live counter. The synchronous taint also
//! rejects any *new* acquire on the revoked credential — so the
//! end-to-end guarantee is: the escaped instance is destroyed, never
//! re-enters idle, and no subsequent acquire on the revoked credential
//! yields a usable handle.
//!
//! Observable used here (no new API): each `create` stamps a unique
//! sequence id on the runtime, `destroy` counts, and `on_credential_revoke`
//! marks a shared flag. The scenarios drive the precise interleaving that
//! exposed the defect and assert the fenced outcome.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::time::Duration;

use nebula_core::{ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_resource::{
    AcquireOptions, Manager, PoolConfig, RegistrationSpec, Resource, ResourceConfig,
    ResourceContext, SlotIdentity,
    error::{Error, ErrorKind},
    resource::ResourceMetadata,
    runtime::{TopologyRuntime, pool::PoolRuntime},
    topology::pooled::{Pooled, RecycleDecision, config::WarmupStrategy},
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
    /// `create` parks on this (variant b / d).
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

/// Awaits until the pool's idle queue is non-empty or an instance was
/// destroyed (the two terminal outcomes of a release settling), bounded by a
/// generous spin so a slow `ReleaseQueue` worker does not flake the test.
async fn settle_release(mgr: &Manager, resource: &PoolResource) {
    for _ in 0..400 {
        let idle = mgr
            .pool_stats::<PoolResource>(&ScopeLevel::Global)
            .await
            .map_or(0, |s| s.idle);
        if idle >= 1 || resource.destroy_calls.load(Ordering::SeqCst) >= 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Variant (a): acquire → drop (release queued, parked in `recycle`) →
/// `revoke_slot` runs while idle is empty (the revoke hook walks nothing) →
/// release the parked recycle. The recycle decision is `Keep`, but the
/// release-path revoke-epoch re-check (re-read live, after the parked
/// `recycle` returns) destroys the instance instead of pushing it back, so
/// it never re-enters idle and the revoked credential is unreachable.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoked_credential_not_reserved_via_idle_recycle() {
    let resource = PoolResource::new();
    let mgr = Arc::new(Manager::new());
    mgr.register(RegistrationSpec {
        resource: resource.clone(),
        config: PoolCfg,
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology: TopologyRuntime::Pool(PoolRuntime::<PoolResource>::new(
            pool_config(),
            PoolCfg.fingerprint(),
        )),
        acquire: Manager::erased_acquire_pooled::<PoolResource>(
            nebula_resource::SLOT_IDENTITY_UNBOUND,
        ),
        resilience: None,
        recovery_gate: None,
    })
    .expect("pooled registration must succeed");

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
    //    release path), so the pool revoke hook walks nothing — this is the
    //    TOCTOU: the escaped instance is never visited by the hook.
    mgr.revoke_slot(&PoolResource::key(), ScopeLevel::Global, "db")
        .await
        .expect("revoke_slot must succeed");
    assert_eq!(
        resource.revoke_calls.load(Ordering::SeqCst),
        0,
        "the revoke walked an empty idle set — the escaped instance was \
         never visited by on_credential_revoke (this is the TOCTOU the \
         epoch fence has to close on the recycle path instead)"
    );

    // 4. Release the parked recycle. `recycle` returns `Keep`, but the
    //    release-path epoch re-check sees the bumped counter and destroys
    //    the instance rather than pushing it to idle.
    resource.gate.park_in_recycle.store(false, Ordering::SeqCst);
    resource.gate.hold_recycle.notify_one();
    settle_release(&mgr, &resource).await;

    assert!(
        resource.destroy_calls.load(Ordering::SeqCst) >= 1,
        "the escaped (revoked) instance must have been destroyed by the \
         revoke-epoch re-check on the recycle path, not recycled to idle"
    );
    assert_eq!(
        mgr.pool_stats::<PoolResource>(&ScopeLevel::Global)
            .await
            .map_or(0, |s| s.idle),
        0,
        "the revoked instance must never have re-entered the idle queue"
    );

    // 5. The revoked credential is fully unreachable: a fresh acquire on the
    //    tainted resource is rejected (the synchronous taint stops new
    //    leases), so there is no path — idle reuse or fresh create — that
    //    hands a caller a runtime authenticated with the revoked credential.
    let err = mgr
        .acquire_pooled::<PoolResource>(&ctx(), &AcquireOptions::default())
        .await
        .expect_err("post-revoke acquire on the tainted resource must be rejected");
    assert_eq!(
        *err.kind(),
        ErrorKind::Revoked,
        "post-revoke acquire must be rejected with Revoked (got {err:?}); \
         escaped_seq={escaped_seq} must never be re-served"
    );
}

/// Variant (b): an in-flight `create` started before the revoke, completing
/// after `drain_and_revoke` (HikariCP #1836). The instance snapshots the
/// revoke counter at the *start* of its creation; the revoke (bumped
/// synchronously in the taint phase) advances the counter while the create
/// is parked, so on completion every return-to-idle path — and the
/// acquire-side checkout guard — fences it: it is destroyed, never admitted
/// or handed onward as a healthy un-revoked handle.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn in_flight_create_completing_after_revoke_is_destroyed() {
    let resource = PoolResource::new();
    let mgr = Arc::new(Manager::new());
    mgr.register(RegistrationSpec {
        resource: resource.clone(),
        config: PoolCfg,
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology: TopologyRuntime::Pool(PoolRuntime::<PoolResource>::new(
            pool_config(),
            PoolCfg.fingerprint(),
        )),
        acquire: Manager::erased_acquire_pooled::<PoolResource>(
            nebula_resource::SLOT_IDENTITY_UNBOUND,
        ),
        resilience: None,
        recovery_gate: None,
    })
    .expect("pooled registration must succeed");

    // 1. Start an acquire whose `create` will park (idle empty → it creates).
    resource.gate.park_in_create.store(true, Ordering::SeqCst);
    let acquire_task = {
        let mgr = Arc::clone(&mgr);
        tokio::spawn(async move {
            mgr.acquire_pooled::<PoolResource>(&ctx(), &AcquireOptions::default())
                .await
        })
    };
    // Wait until `create` is parked (in flight, not yet returned). The
    // revoke-epoch snapshot was taken at `create_entry` start, before this
    // park — i.e. before the revoke below.
    resource.gate.create_entered.notified().await;

    // 2. Revoke while the create is in flight. Idle is empty; nothing to
    //    walk. The taint AND the revoke-counter bump are applied
    //    synchronously here, before the create completes.
    let tainted = mgr
        .taint_slot(&PoolResource::key(), ScopeLevel::Global, "db")
        .expect("taint_slot must resolve synchronously");

    // 3. Release the parked create so it completes strictly after the
    //    counter bump (its snapshot is now behind the live counter).
    resource.gate.hold_create.notify_one();

    let drain_task = {
        let mgr = Arc::clone(&mgr);
        tokio::spawn(async move { mgr.drain_and_revoke(tainted, Duration::from_secs(30)).await })
    };

    let guard = acquire_task.await.expect("acquire task must not panic");
    let _ = drain_task.await.expect("drain task must not panic");

    // The instance created after the revoke must NOT be handed onward as a
    // usable handle: the fresh-create fence (HikariCP #1836) destroys it
    // and the acquire fails closed rather than returning a runtime
    // authenticated with the revoked credential. A silently-admitted Ok
    // here would be the exact cross-tenant-reuse defect.
    let err = guard.expect_err(
        "an in-flight create completing after drain_and_revoke must NOT be \
         admitted — the fresh-create revoke-epoch fence must destroy it and \
         fail the acquire (HikariCP #1836)",
    );
    assert!(
        matches!(*err.kind(), ErrorKind::Permanent | ErrorKind::Revoked),
        "the fenced acquire must fail closed (got {err:?})"
    );
    assert!(
        resource.destroy_calls.load(Ordering::SeqCst) >= 1,
        "the post-revoke-created instance must have been destroyed by the \
         revoke-epoch fence"
    );
}

/// Variant (c): a pre-revoke idle instance that is non-stale and
/// non-timed-out. `drain_and_revoke`'s idle walk runs the revoke hook over
/// it (the hook does not evict — the entry stays in idle); the revoke
/// counter is bumped synchronously so the maintenance re-deposit path
/// (`should_evict`, exercised in the in-crate pool unit test for its
/// revoke-epoch arm) would destroy it, and the synchronous taint
/// independently rejects any new acquire. Through the public `Manager`
/// surface the observable end-to-end guarantee is: the revoked instance is
/// visited by the hook and the revoked credential is unreachable to any
/// subsequent acquirer (the only Manager-reachable path to a pooled idle
/// instance is acquire, which is taint-rejected — `PoolRuntime` and its
/// `run_maintenance` are not Manager-reachable, so the maintenance arm is
/// pinned at the unit level).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoked_pre_existing_idle_instance_not_reserved() {
    let resource = PoolResource::new();
    let mgr = Arc::new(Manager::new());
    mgr.register(RegistrationSpec {
        resource: resource.clone(),
        config: PoolCfg,
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology: TopologyRuntime::Pool(PoolRuntime::<PoolResource>::new(
            pool_config(),
            PoolCfg.fingerprint(),
        )),
        acquire: Manager::erased_acquire_pooled::<PoolResource>(
            nebula_resource::SLOT_IDENTITY_UNBOUND,
        ),
        resilience: None,
        recovery_gate: None,
    })
    .expect("pooled registration must succeed");

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
        let mut ok = false;
        for _ in 0..400 {
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
    //    not evict; the revoke counter was bumped synchronously).
    mgr.revoke_slot(&PoolResource::key(), ScopeLevel::Global, "db")
        .await
        .expect("revoke_slot must succeed");
    assert_eq!(
        resource.revoke_calls.load(Ordering::SeqCst),
        1,
        "the revoke hook must have visited the single pre-revoke idle entry"
    );

    // 3. The revoke hook marked the visited instance (its shared revoked
    //    flag is set) but did NOT evict it — the entry is still in idle.
    //    This is precisely the maintenance re-deposit gap: `should_evict`
    //    must destroy a still-idle revoked entry on the next sweep. That
    //    arm is pinned at the unit level (`run_maintenance` / `PoolRuntime`
    //    are not Manager-reachable); here we assert the Manager-observable
    //    pre-conditions that make the unit-level fence necessary.
    assert!(
        resource.revoked.load(Ordering::SeqCst),
        "the revoke hook must have marked the visited idle instance"
    );
    assert_eq!(
        mgr.pool_stats::<PoolResource>(&ScopeLevel::Global)
            .await
            .map_or(0, |s| s.idle),
        1,
        "the revoke hook visits but does not evict — the entry stays idle, \
         which is why the maintenance re-deposit arm must fence it"
    );

    // 4. The revoked credential is unreachable for any subsequent acquirer:
    //    the synchronous taint rejects a fresh acquire, so the only
    //    Manager-reachable path to that still-idle revoked instance
    //    (idle_seq={idle_seq}) is closed — it can never be served.
    let err = mgr
        .acquire_pooled::<PoolResource>(&ctx(), &AcquireOptions::default())
        .await
        .expect_err("post-revoke acquire on the tainted resource must be rejected");
    assert!(
        err.resource_key().is_some(),
        "the rejection must carry the resource key for operator triage"
    );
    assert_eq!(
        *err.kind(),
        ErrorKind::Revoked,
        "post-revoke acquire must be rejected with Revoked (got {err:?}); \
         idle_seq={idle_seq} must never be re-served"
    );
}

/// Variant (d): a warmup running concurrently with — and completing strictly
/// after — `drain_and_revoke`. A staggered/sequential warmup whose
/// `create` was in flight when the credential was revoked must NOT deposit
/// the warmed instance into idle: it snapshots the revoke counter at create
/// start, the synchronous revoke advances the counter while the create is
/// parked, and `admit_warmed_entry` destroys it under the idle lock instead
/// of pushing it back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn warmup_after_revoke_does_not_admit_revoked_instance() {
    let resource = PoolResource::new();
    let mgr = Arc::new(Manager::new());
    // `min_size: 1` + `Sequential` warmup so `warmup` actually pre-creates
    // one instance: the shared `pool_config()` uses `min_size: 0` and the
    // default `WarmupStrategy::None`, both of which make `warmup` a no-op.
    let cfg = PoolConfig {
        min_size: 1,
        warmup: WarmupStrategy::Sequential,
        ..pool_config()
    };
    mgr.register(RegistrationSpec {
        resource: resource.clone(),
        config: PoolCfg,
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology: TopologyRuntime::Pool(PoolRuntime::<PoolResource>::new(
            cfg,
            PoolCfg.fingerprint(),
        )),
        acquire: Manager::erased_acquire_pooled::<PoolResource>(
            nebula_resource::SLOT_IDENTITY_UNBOUND,
        ),
        resilience: None,
        recovery_gate: None,
    })
    .expect("pooled registration must succeed");

    // 1. Kick off a warmup whose first `create` parks (in flight). The
    //    warmup passes its own taint gate before entering `rt.warmup()`;
    //    the revoke-epoch snapshot is then taken at `create_entry` start,
    //    before the park — i.e. before the revoke below.
    resource.gate.park_in_create.store(true, Ordering::SeqCst);
    let warmup_task = {
        let mgr = Arc::clone(&mgr);
        tokio::spawn(async move { mgr.warmup_pool::<PoolResource>(&ctx()).await })
    };
    resource.gate.create_entered.notified().await;

    // 2. Taint synchronously while the warmup create is in flight. This
    //    applies the taint AND bumps the revoke counter before the create
    //    completes. The two-phase split is required here: the in-flight
    //    warmup holds the resource's in-flight counter, and `revoke_slot`'s
    //    drain phase would block on it — while the warmup is itself blocked
    //    on the parked create, a deadlock. Splitting lets the counter bump
    //    land first, then the create completes and the counter drops so the
    //    separately-spawned drain can finish.
    let tainted = mgr
        .taint_slot(&PoolResource::key(), ScopeLevel::Global, "db")
        .expect("taint_slot must resolve synchronously");

    // 3. Release the parked create so the warmed instance completes strictly
    //    after the counter bump (its snapshot is now behind the live
    //    counter), then run the drain/hook tail concurrently.
    resource.gate.hold_create.notify_one();
    let drain_task = {
        let mgr = Arc::clone(&mgr);
        tokio::spawn(async move { mgr.drain_and_revoke(tainted, Duration::from_secs(30)).await })
    };
    let _warmed = warmup_task.await.expect("warmup task must not panic");
    let _ = drain_task.await.expect("drain task must not panic");

    // The warmed instance was created against the now-revoked credential:
    // `admit_warmed_entry` must have destroyed it, never pushed it to idle.
    settle_release(&mgr, &resource).await;
    assert!(
        resource.destroy_calls.load(Ordering::SeqCst) >= 1,
        "the post-revoke-warmed instance must have been destroyed by the \
         warmup revoke-epoch fence (admit_warmed_entry), not admitted"
    );
    assert_eq!(
        mgr.pool_stats::<PoolResource>(&ScopeLevel::Global)
            .await
            .map_or(0, |s| s.idle),
        0,
        "the warmup must not have deposited a revoked-credential instance \
         into the idle queue"
    );

    // And the revoked credential stays unreachable for a fresh acquire.
    let err = mgr
        .acquire_pooled::<PoolResource>(&ctx(), &AcquireOptions::default())
        .await
        .expect_err("post-revoke acquire on the tainted resource must be rejected");
    assert_eq!(
        *err.kind(),
        ErrorKind::Revoked,
        "post-revoke acquire must be rejected with Revoked (got {err:?})"
    );
}
