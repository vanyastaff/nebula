//! Resident topology — one shared instance, clone on acquire.
//!
//! `Resident<R>` is the built-in framework resident topology. It holds a single
//! shared runtime in a lock-free `Cell` and, on each acquire, clones it into an
//! owned lease. It supplies the slot-centric [`Topology<R>`] hooks the framework
//! acquire loop drives:
//!
//! - `Slot = R::Instance` (the cloned shared handle the guard holds).
//! - `pools() == false`: a released clone is dropped, never pooled, so the
//!   framework idle store stays empty and every acquire is an idle-miss that
//!   calls `create_slot`.
//! - `create_slot` clones the live master handle (building it under the create
//!   lock on first acquire / after a failed liveness check).
//! - `slot_instance` / `into_instance` are identity.
//! - `dispatch_credential_hook` runs the create-vs-rotate reconcile against the
//!   master cell.
//!
//! The resident keeps its own master-handle cell (separate from the empty
//! framework store); the store-fence never reaches the master, so revoke
//! teardown for a resident runs through `dispatch_credential_hook`.
//!
//! [`Topology<R>`]: crate::topology::Topology

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use tokio::sync::Mutex;
use tracing::warn;

use crate::{
    cell::Cell,
    context::ResourceContext,
    error::Error,
    resource::{HasCredentialSlots, Provider, TeardownReason},
    runtime::managed::destroy_within,
    topology::{Ticket, Topology, Unavailable, resident::config::Config, store::InstanceStore},
    topology_tag::TopologyTag,
};

/// Framework resident topology — one shared instance, clone on acquire.
///
/// Holds a single shared runtime instance in a lock-free `Cell`. On acquire, the
/// framework calls [`create_slot`](Topology::create_slot), which clones the live
/// master handle (building it under the create lock on the first acquire or
/// after a failed liveness check). The framework idle store stays empty —
/// `pools()` is `false`, so a released clone is dropped, never recycled.
///
/// A `create_lock` mutex serialises the slow path (create / recreate) **and**
/// the per-slot rotation hook dispatch, so the create-vs-rotate reconcile is
/// exactly-once.
///
/// [`Topology<R>`]: crate::topology::Topology
pub struct Resident<R: Provider> {
    cell: Cell<R::Instance>,
    config: Config,
    /// Serialises the create / recreate slow path **and** the per-slot
    /// rotation hook dispatch (see [`dispatch_resident_hook`]). The
    /// rotation dispatch holding this same lock is what makes the
    /// create-vs-rotate reconcile *exactly-once*: a `create` slow path
    /// and a rotation dispatch can never interleave, so the freshly-built
    /// runtime's epoch is either reconciled inside `create` (dispatch ran
    /// first / sees the post-reconcile epoch) or by the dispatch (it sees
    /// the stored runtime + its recorded epoch) — never both.
    ///
    /// [`dispatch_resident_hook`]: Self::dispatch_resident_hook
    create_lock: Mutex<()>,
    /// The credential epoch ([`HasCredentialSlots::credential_slot_epoch`]) the
    /// currently-stored runtime was built against. `0` when no runtime has
    /// been created. Written only under `create_lock`; read under it by
    /// the rotation dispatch. A stored runtime whose `built_epoch` is
    /// *older* than the live slot epoch was bound to a pre-rotation
    /// credential — the lost-update the dispatch must reconcile rather
    /// than silently report success for (per-resource revoke deferral / #680).
    ///
    /// **Intentionally NOT the same counter as `SlotCell::generation()`,
    /// and must not be folded into it.** `built_epoch` advances *only* on
    /// a successful stale reconcile (the `stale && Ok(())` arm of
    /// [`dispatch_resident_hook`]) or when stamped at create time;
    /// `SlotCell::generation()` bumps on *every* slot transition, including
    /// a no-op `take()` on an already-empty or never-bound slot (a "clear"
    /// signal is meaningful to a rotation observer regardless of prior
    /// state). A resident runtime can "catch up" to its own `built_epoch`
    /// by completing a reconcile; it has no way to advance
    /// `SlotCell::generation()`. Backing this counter with the slot
    /// generation would leave a correctly-bound runtime perpetually
    /// re-classified as stale after any unrelated no-op `take()`, forcing
    /// redundant `on_credential_revoke` / `on_credential_refresh`
    /// re-delivery — a credential-isolation behavior change. Keep the two
    /// counters separate.
    ///
    /// [`dispatch_resident_hook`]: Self::dispatch_resident_hook
    built_epoch: AtomicU64,
}

impl<R: Provider + HasCredentialSlots> Resident<R> {
    /// Creates a new resident topology with the given configuration.
    pub fn new(config: Config) -> Self {
        Self {
            cell: Cell::new(),
            config,
            create_lock: Mutex::new(()),
            built_epoch: AtomicU64::new(0),
        }
    }

    /// Returns the current configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns `true` if the cell currently holds an instance.
    pub fn is_initialized(&self) -> bool {
        self.cell.is_some()
    }

    /// The credential epoch the currently-stored runtime was built against.
    ///
    /// Test-only: the create-vs-rotate reconcile classifies a runtime as
    /// stale iff this is *older* than the live slot epoch, so a test that
    /// drives a `SlotCell::store` into the create slow path's
    /// sample-vs-read window needs to assert this equals the epoch the
    /// runtime actually bound (not the pre-create approximation).
    #[cfg(test)]
    pub(crate) fn built_epoch_for_test(&self) -> u64 {
        self.built_epoch.load(Ordering::Acquire)
    }

    /// Per-slot rotation hook dispatch for the Resident topology, with the
    /// create-vs-rotate reconcile (per-resource revoke deferral / #680).
    ///
    /// Takes `create_lock` so it is **mutually excluded** with the
    /// `create_slot` slow path: a rotation dispatch and a first-acquire create
    /// can never interleave, which is what makes the reconcile exactly-once.
    /// Under the lock:
    ///
    /// - **Instance present, built epoch ≥ slot epoch** — up to date: deliver
    ///   the hook normally.
    /// - **Instance present, built epoch < slot epoch** — the instance was
    ///   bound to a pre-rotation credential (the lost-update): still deliver
    ///   the hook (the resource's `&self` reaction rebinds against the now
    ///   current slot) and, on success, advance the recorded epoch.
    /// - **No instance** — nothing live to refresh. Genuinely a no-op
    ///   `Ok(())`: a never-created resident has no stale instance to leave
    ///   behind, and a create *racing* this dispatch is serialised by
    ///   `create_lock` — it runs strictly before or after.
    ///
    /// `refresh = true` selects `on_credential_refresh`, `false`
    /// `on_credential_revoke`. The revoke direction is symmetric: an instance
    /// built against an older epoch is still delivered the revoke hook (it
    /// must stop emitting on the now-revoked credential); a never-created
    /// resident has nothing emitting, so the no-op is correct there too.
    ///
    /// # Errors
    ///
    /// Propagates the resource's `on_credential_*` error; on a stale-reconcile
    /// failure the recorded epoch is deliberately not advanced so the next
    /// dispatch re-attempts.
    pub(crate) async fn dispatch_resident_hook(
        &self,
        resource: &R,
        slot: &str,
        refresh: bool,
    ) -> Result<(), Error> {
        // Serialise against the create slow path: the reconcile must not
        // interleave with an instance being built / its epoch being
        // published, so delivery is exactly-once.
        let _guard = self.create_lock.lock().await;

        let Some(runtime) = self.cell.load() else {
            // No live runtime. Not a stale-skip: nothing is bound to a
            // credential at all, and a concurrent first create is excluded
            // by `create_lock` (it runs strictly before/after this and
            // records its own `built_epoch`). A genuinely never-created,
            // never-bound resident is a legitimate no-op.
            tracing::debug!(
                resource = %R::key(),
                slot,
                refresh,
                "resident slot hook: no live runtime — legitimate no-op \
                 (never created; not a stale-skip)"
            );
            return Ok(());
        };

        let slot_epoch = resource.credential_slot_epoch();
        let built = self.built_epoch.load(Ordering::Acquire);
        let stale = built < slot_epoch;
        if stale {
            tracing::warn!(
                resource = %R::key(),
                slot,
                refresh,
                built_epoch = built,
                slot_epoch,
                "resident slot hook: live runtime is stale (built against an \
                 older credential epoch) — reconciling by delivering the hook"
            );
        }

        let res = if refresh {
            resource.on_credential_refresh(slot, &runtime).await
        } else {
            resource.on_credential_revoke(slot, &runtime).await
        };

        match res {
            Ok(()) => {
                if stale {
                    self.built_epoch.store(slot_epoch, Ordering::Release);
                }
                Ok(())
            },
            Err(e) => Err(e),
        }
    }
}

impl<R> Resident<R>
where
    R: crate::topology::resident::ResidentProvider + HasCredentialSlots + Send + Sync + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    /// Clones the shared runtime, building it under the create lock on the
    /// first acquire / after a failed liveness check.
    ///
    /// This is the body of [`Topology::create_slot`] for the resident: because
    /// `pools()` is `false`, the framework store stays empty and the framework
    /// calls this on **every** acquire — the clone-or-create logic lives here,
    /// not a fast/slow split in the framework loop.
    async fn clone_or_create(
        &self,
        resource: &R,
        resource_config: &R::Config,
        ctx: &ResourceContext,
    ) -> Result<R::Instance, Error> {
        // Fast path — lock-free load + liveness check.
        if let Some(existing) = self.cell.load()
            && resource.is_alive_sync(&existing)
        {
            return Ok((*existing).clone());
        }

        // Slow path — serialise create / recreate.
        let _guard = self.create_lock.lock().await;

        // Double-check: another task may have created while we waited.
        if let Some(existing) = self.cell.load() {
            if resource.is_alive_sync(&existing) {
                return Ok((*existing).clone());
            }

            // Still not alive — destroy and recreate if configured.
            if !self.config.recreate_on_failure {
                return Err(Error::transient("resident runtime is not alive"));
            }

            // Take the old runtime out and best-effort destroy under the
            // resource's per-instance teardown budget (an evict-and-recreate).
            if let Some(old) = self.cell.take() {
                match Arc::try_unwrap(old) {
                    Ok(owned) => {
                        let _ = destroy_within(resource, owned, TeardownReason::Evicted).await;
                    },
                    Err(arc) => {
                        warn!(
                            resource = %R::key(),
                            refs = Arc::strong_count(&arc),
                            "cannot exclusively destroy resident runtime; \
                             another handle still held — dropping Arc"
                        );
                    },
                }
            }
        }

        // Create a new runtime.
        let runtime = match tokio::time::timeout(
            self.config.create_timeout,
            resource.create(resource_config, ctx),
        )
        .await
        {
            Ok(Ok(rt)) => rt,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(Error::transient("resident: create timed out")),
        };

        // Capture the credential epoch *after* `create` has read the slot, not
        // before it. A pre-`create` sample is a stale approximation: a
        // lock-free `SlotCell::store` (engine rotation fan-out) landing
        // *between* the sample and `create`'s own slot read builds the instance
        // against the **fresh** credential while `built_epoch` would record the
        // **old** epoch, so the create-vs-rotate reconcile spuriously
        // classifies an already-fresh instance as stale. Sampling *after*
        // `create` returns makes `built_epoch` an at-or-after-read bound. The
        // sample stays under `create_lock` and is published with the instance,
        // preserving the exactly-once dispatch / create-vs-rotate semantics.
        let built_epoch = resource.credential_slot_epoch();

        let cloned = runtime.clone();
        self.cell.store(Arc::new(runtime));
        self.built_epoch.store(built_epoch, Ordering::Release);

        Ok(cloned)
    }
}

// ─── Topology impl for Resident ───────────────────────────────────────────────
//
// `Resident<R>` clones one shared instance on acquire. `Slot = R::Instance`,
// `pools() == false`, so the framework store stays empty and every acquire is an
// idle-miss that calls `create_slot` (clone-or-create). The revoke fence cannot
// reach the master cell (it is not in the store), so revoke teardown runs
// through `dispatch_credential_hook`.

#[async_trait]
impl<R> Topology<R> for Resident<R>
where
    R: Provider<Topology = Resident<R>>
        + crate::topology::resident::ResidentProvider
        + HasCredentialSlots
        + Send
        + Sync
        + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    type Slot = R::Instance;

    /// Always succeeds — resident is unbounded (one shared instance).
    fn try_reserve(&self, _store: &InstanceStore<R::Instance>) -> Result<Ticket, Unavailable> {
        Ok(Ticket::infallible())
    }

    async fn create_slot(
        &self,
        resource: &R,
        config: &R::Config,
        ctx: &ResourceContext,
    ) -> Result<R::Instance, Error> {
        self.clone_or_create(resource, config, ctx).await
    }

    fn slot_instance<'s>(&self, slot: &'s R::Instance) -> &'s R::Instance {
        slot
    }

    fn into_instance(&self, slot: R::Instance) -> R::Instance {
        slot
    }

    /// Resident does not pool: a released clone is dropped, never recycled, so
    /// the framework idle store stays empty.
    fn pools(&self) -> bool {
        false
    }

    /// Resident tears down its credential-bound master handle on revoke via the
    /// cell reconcile in [`dispatch_credential_hook`](Self::dispatch_credential_hook)
    /// (the master handle is never in the framework store, so the store fence
    /// cannot reach it). Declaring this lets the registration footgun-guard
    /// distinguish a correctly-revoke-handling non-pooling topology from an
    /// under-built custom one.
    fn handles_own_revoke(&self) -> bool {
        true
    }

    async fn dispatch_credential_hook(
        &self,
        resource: &R,
        _store: &InstanceStore<R::Instance>,
        slot: &str,
        refresh: bool,
    ) -> Result<(), Error> {
        // The resident's master handle is NOT in the framework store, so the
        // store-fence cannot reach it: revoke / refresh teardown runs the
        // create-vs-rotate reconcile against the master cell instead.
        self.dispatch_resident_hook(resource, slot, refresh).await
    }

    fn tag(&self) -> TopologyTag {
        TopologyTag::Resident
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicBool, AtomicU32, Ordering},
        time::Duration,
    };

    use nebula_core::{ExecutionId, ResourceKey, resource_key};

    use super::*;
    use crate::{
        context::ResourceContext,
        resource::{HasCredentialSlots, ResourceConfig, ResourceMetadata},
        topology::resident::ResidentProvider,
    };

    #[derive(Clone)]
    struct MockResident {
        alive: Arc<AtomicBool>,
        create_count: Arc<AtomicU32>,
    }

    impl MockResident {
        fn new() -> Self {
            Self {
                alive: Arc::new(AtomicBool::new(true)),
                create_count: Arc::new(AtomicU32::new(0)),
            }
        }
    }

    impl ResourceConfig for bool {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }

        fn fingerprint(&self) -> u64 {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            self.hash(&mut h);
            h.finish()
        }
    }

    #[async_trait::async_trait]
    impl Provider for MockResident {
        type Config = bool;
        type Instance = u32;
        type Topology = Resident<Self>;

        fn key() -> ResourceKey {
            resource_key!("mock-resident")
        }

        async fn create(&self, _config: &bool, _ctx: &ResourceContext) -> Result<u32, Error> {
            let count = self.create_count.fetch_add(1, Ordering::Relaxed);
            tokio::task::yield_now().await;
            Ok(count + 100)
        }

        async fn destroy(&self, _runtime: u32, _cx: crate::TeardownCx) -> Result<(), Error> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl HasCredentialSlots for MockResident {
        fn credential_slot_epoch(&self) -> u64 {
            0
        }
    }

    impl ResidentProvider for MockResident {
        fn is_alive_sync(&self, _runtime: &u32) -> bool {
            self.alive.load(Ordering::Relaxed)
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

    #[tokio::test]
    async fn create_slot_creates_on_first_call() {
        let resource = MockResident::new();
        let rt = Resident::<MockResident>::new(Config::default());
        let ctx = test_ctx();

        let inst = rt
            .clone_or_create(&resource, &true, &ctx)
            .await
            .expect("first create");
        assert_eq!(inst, 100);
        assert_eq!(resource.create_count.load(Ordering::Relaxed), 1);
        assert!(rt.is_initialized());
    }

    #[tokio::test]
    async fn create_slot_reuses_existing_instance() {
        let resource = MockResident::new();
        let rt = Resident::<MockResident>::new(Config::default());
        let ctx = test_ctx();

        let a = rt.clone_or_create(&resource, &true, &ctx).await.unwrap();
        let b = rt.clone_or_create(&resource, &true, &ctx).await.unwrap();
        assert_eq!(a, b, "both clones share the one master instance");
        assert_eq!(
            resource.create_count.load(Ordering::Relaxed),
            1,
            "the second clone reuses the master — only one create"
        );
    }

    #[tokio::test]
    async fn concurrent_create_slot_creates_only_once() {
        let resource = MockResident::new();
        let rt = Arc::new(Resident::<MockResident>::new(Config::default()));
        let ctx = Arc::new(test_ctx());

        let mut handles = Vec::new();
        for _ in 0..10 {
            let r = resource.clone();
            let runtime = Arc::clone(&rt);
            let c = Arc::clone(&ctx);
            handles.push(tokio::spawn(async move {
                runtime
                    .clone_or_create(&r, &true, c.as_ref())
                    .await
                    .unwrap()
            }));
        }
        for h in handles {
            let _ = h.await.unwrap();
        }
        assert_eq!(
            resource.create_count.load(Ordering::Relaxed),
            1,
            "concurrent clone-or-create on an empty cell creates exactly once"
        );
    }

    #[tokio::test]
    async fn recreates_when_not_alive_and_configured() {
        let resource = MockResident::new();
        let config = Config {
            recreate_on_failure: true,
            ..Default::default()
        };
        let rt = Resident::<MockResident>::new(config);
        let ctx = test_ctx();

        let a = rt.clone_or_create(&resource, &true, &ctx).await.unwrap();
        assert_eq!(a, 100);
        resource.alive.store(false, Ordering::Relaxed);
        let b = rt.clone_or_create(&resource, &true, &ctx).await.unwrap();
        assert_eq!(b, 101, "a fresh master was built after liveness failed");
        assert_eq!(resource.create_count.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn fails_when_not_alive_and_no_recreate() {
        let resource = MockResident::new();
        let config = Config {
            recreate_on_failure: false,
            ..Default::default()
        };
        let rt = Resident::<MockResident>::new(config);
        let ctx = test_ctx();

        let _a = rt.clone_or_create(&resource, &true, &ctx).await.unwrap();
        resource.alive.store(false, Ordering::Relaxed);
        assert!(
            rt.clone_or_create(&resource, &true, &ctx).await.is_err(),
            "a dead master with recreate disabled must fail"
        );
    }

    #[tokio::test]
    async fn topology_does_not_pool() {
        let rt = Resident::<MockResident>::new(Config::default());
        assert!(
            !Topology::<MockResident>::pools(&rt),
            "resident does not pool — released clones are dropped"
        );
        assert_eq!(Topology::<MockResident>::tag(&rt), TopologyTag::Resident);
    }

    // A resource whose `create()` never returns — for timeout tests.
    #[derive(Clone)]
    struct HangingResident;

    #[async_trait::async_trait]
    impl Provider for HangingResident {
        type Config = bool;
        type Instance = u32;
        type Topology = Resident<Self>;

        fn key() -> ResourceKey {
            resource_key!("hanging-resident")
        }

        async fn create(&self, _config: &bool, _ctx: &ResourceContext) -> Result<u32, Error> {
            tokio::time::sleep(Duration::from_hours(1)).await;
            Ok(0)
        }

        async fn destroy(&self, _runtime: u32, _cx: crate::TeardownCx) -> Result<(), Error> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl HasCredentialSlots for HangingResident {
        fn credential_slot_epoch(&self) -> u64 {
            0
        }
    }

    impl ResidentProvider for HangingResident {}

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn create_timeout_does_not_deadlock() {
        let resource = HangingResident;
        let config = Config {
            create_timeout: Duration::from_millis(50),
            ..Default::default()
        };
        let rt = Arc::new(Resident::<HangingResident>::new(config));
        let ctx = Arc::new(test_ctx());

        assert!(
            rt.clone_or_create(&resource, &true, ctx.as_ref())
                .await
                .is_err(),
            "first create should time out"
        );
        assert!(
            rt.clone_or_create(&resource, &true, ctx.as_ref())
                .await
                .is_err(),
            "second create should time out (lock released)"
        );
    }

    // ---------------------------------------------------------------------
    // `built_epoch` sampled AFTER `create()` reads the slot (not before).
    // ---------------------------------------------------------------------

    use std::sync::Arc as StdArc;

    use tokio::sync::Notify;

    use crate::slot::SlotCell;

    #[derive(Default)]
    struct FakeCred(u32);

    impl zeroize::Zeroize for FakeCred {
        fn zeroize(&mut self) {
            self.0 = 0;
        }
    }

    #[derive(Clone)]
    struct SlotReadResident {
        slot: StdArc<SlotCell<FakeCred>>,
        entered_before_read: StdArc<Notify>,
        release_read: StdArc<Notify>,
        park_before_read: StdArc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl Provider for SlotReadResident {
        type Config = bool;
        type Instance = u32;
        type Topology = Resident<Self>;

        fn key() -> ResourceKey {
            resource_key!("slot-read-resident")
        }

        async fn create(&self, _config: &bool, _ctx: &ResourceContext) -> Result<u32, Error> {
            self.entered_before_read.notify_one();
            if self.park_before_read.load(Ordering::SeqCst) {
                self.release_read.notified().await;
            }
            let cred = self
                .slot
                .load()
                .map(|g| g.0)
                .ok_or_else(|| Error::permanent("slot unbound at create"))?;
            Ok(cred)
        }

        async fn destroy(&self, _runtime: u32, _cx: crate::TeardownCx) -> Result<(), Error> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl crate::resource::HasCredentialSlots for SlotReadResident {
        fn credential_slot_epoch(&self) -> u64 {
            self.slot.generation()
        }
    }

    impl ResidentProvider for SlotReadResident {
        fn is_alive_sync(&self, _runtime: &u32) -> bool {
            true
        }
    }

    const CRED_OLD: u32 = 7;
    const CRED_NEW: u32 = 99;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn built_epoch_records_post_create_slot_epoch_not_presample() {
        let slot: SlotCell<FakeCred> = SlotCell::empty();
        slot.store(StdArc::new(FakeCred(CRED_OLD)));
        let slot = StdArc::new(slot);
        let gen_old = slot.generation();

        let resource = SlotReadResident {
            slot: StdArc::clone(&slot),
            entered_before_read: StdArc::new(Notify::new()),
            release_read: StdArc::new(Notify::new()),
            park_before_read: StdArc::new(AtomicBool::new(true)),
        };
        let rt = Arc::new(Resident::<SlotReadResident>::new(Config::default()));
        let ctx = Arc::new(test_ctx());

        let acquire_task = {
            let rt = Arc::clone(&rt);
            let resource = resource.clone();
            let ctx = Arc::clone(&ctx);
            tokio::spawn(async move { rt.clone_or_create(&resource, &true, ctx.as_ref()).await })
        };

        resource.entered_before_read.notified().await;
        slot.store(StdArc::new(FakeCred(CRED_NEW)));
        let gen_new = slot.generation();
        assert!(
            gen_new > gen_old,
            "store must strictly advance the generation"
        );
        resource.release_read.notify_one();

        let inst = acquire_task
            .await
            .expect("task must not panic")
            .expect("first create must succeed");
        assert_eq!(inst, CRED_NEW, "create read the slot after the store");
        assert_eq!(
            rt.built_epoch_for_test(),
            gen_new,
            "built_epoch must be the epoch the instance actually bound \
             (post-create slot read), not the pre-create sample"
        );
        assert!(
            rt.built_epoch_for_test() >= slot.generation(),
            "an instance built reading the current slot must not be older \
             than the live slot epoch (no spurious stale reconcile)"
        );
    }

    #[tokio::test]
    async fn built_epoch_matches_slot_epoch_with_no_race() {
        let slot: SlotCell<FakeCred> = SlotCell::empty();
        slot.store(StdArc::new(FakeCred(CRED_OLD)));
        let slot = StdArc::new(slot);

        let resource = SlotReadResident {
            slot: StdArc::clone(&slot),
            entered_before_read: StdArc::new(Notify::new()),
            release_read: StdArc::new(Notify::new()),
            park_before_read: StdArc::new(AtomicBool::new(false)),
        };
        let rt = Resident::<SlotReadResident>::new(Config::default());
        let ctx = test_ctx();

        let inst = rt
            .clone_or_create(&resource, &true, &ctx)
            .await
            .expect("create must succeed");
        assert_eq!(inst, CRED_OLD);
        assert!(rt.is_initialized());
        assert_eq!(
            rt.built_epoch_for_test(),
            slot.generation(),
            "with no racing store, built_epoch is exactly the live slot epoch"
        );
    }
}
