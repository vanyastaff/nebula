//! Resident runtime — one shared instance, clone on acquire.
//!
//! The resident runtime holds a single `Cell` containing the shared
//! runtime. On acquire, the runtime is cloned into an owned handle.
//! If the runtime is missing or stale, it is (re)created.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::sync::Mutex;
use tracing::warn;

use crate::{
    cell::Cell,
    context::ResourceContext,
    error::Error,
    guard::ResourceGuard,
    options::AcquireOptions,
    resource::Resource,
    topology::resident::{Resident, config::Config},
    topology_tag::TopologyTag,
};

/// Runtime state for a resident topology.
///
/// Holds a single shared runtime instance in a lock-free `Cell`.
/// On acquire, the runtime is cloned into an owned [`ResourceGuard`].
///
/// A `create_lock` mutex serialises the slow path (create / recreate) while
/// keeping the fast path (load + liveness check) entirely lock-free.
pub struct ResidentRuntime<R: Resource> {
    cell: Cell<R::Runtime>,
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
    /// The credential epoch ([`Resource::credential_slot_epoch`]) the
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

impl<R: Resource> ResidentRuntime<R> {
    /// Creates a new resident runtime with the given configuration.
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

    /// Returns `true` if the cell currently holds a runtime instance.
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
    /// `create` slow path: a rotation dispatch and a first-acquire create
    /// can never interleave, which is what makes the reconcile
    /// exactly-once. Under the lock:
    ///
    /// - **Runtime present, built epoch ≥ slot epoch** — up to date: deliver
    ///   the hook normally.
    /// - **Runtime present, built epoch < slot epoch** — the runtime was
    ///   bound to a pre-rotation credential (the lost-update): still deliver
    ///   the hook (the resource's `&self` reaction rebinds against the now
    ///   current slot) and, on success, advance the recorded epoch. This is
    ///   the arm that *used to* silently `Ok(())` when `current() == None`
    ///   raced the build; it is now an explicit, observable reconcile —
    ///   never a skipped-but-success.
    /// - **No runtime** — nothing live to refresh. Genuinely a no-op
    ///   `Ok(())`: a never-created resident has no stale runtime to leave
    ///   behind, and a create *racing* this dispatch is serialised by
    ///   `create_lock` — it runs strictly before or after. If it runs
    ///   after, it reads the post-rotation credential (correct by
    ///   construction) and records that newer `built_epoch`; if it ran
    ///   before, its older `built_epoch` makes *this* dispatch's next
    ///   invocation (or the very acquire that materialised it) take the
    ///   stale-reconcile arm. "Never created" vs "created against a stale
    ///   epoch" is exactly the runtime-presence check here.
    ///
    /// `refresh = true` selects `on_credential_refresh`, `false`
    /// `on_credential_revoke`. The revoke direction is symmetric: a runtime
    /// built against an older epoch is still delivered the revoke hook (it
    /// must stop emitting on the now-revoked credential); a never-created
    /// resident has nothing emitting, so the no-op is correct there too.
    pub(crate) async fn dispatch_resident_hook(
        &self,
        resource: &R,
        slot: &str,
        refresh: bool,
    ) -> Result<(), Error> {
        // Serialise against the create slow path: the reconcile must not
        // interleave with a runtime being built / its epoch being
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
            // The lost-update: the live runtime was built against an older
            // credential epoch than the slot now holds. This is the arm
            // that previously could be skipped with a false success when it
            // raced `current() == None`; it is now an explicit, observable
            // reconcile — we still deliver the hook so the rotation
            // actually takes effect. Carries no credential material.
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
                    // Reconciled: the runtime is now consistent with the
                    // current slot epoch. Advance so a subsequent dispatch
                    // does not treat it as stale again.
                    self.built_epoch.store(slot_epoch, Ordering::Release);
                }
                Ok(())
            },
            Err(e) => {
                // On a stale reconcile failure, deliberately do NOT advance
                // `built_epoch`: the runtime is still bound to the old
                // credential, so the next dispatch must re-attempt. The
                // error propagates so the dispatch outcome is recorded as
                // failed — never a skipped-because-stale success.
                Err(e.into())
            },
        }
    }
}

impl<R> ResidentRuntime<R>
where
    R: Resident + Send + Sync + 'static,
    R::Lease: Clone,
    R::Runtime: Clone + Send + 'static,
{
    /// Acquires a clone of the shared runtime instance.
    ///
    /// **Fast path** (lock-free): load from cell, check liveness, clone.
    ///
    /// **Slow path** (mutex-serialised): create or recreate the runtime.
    /// A double-check after lock acquisition prevents duplicate creates
    /// when multiple callers race on an empty or stale cell.
    ///
    /// # Errors
    ///
    /// Returns an error if creation fails or if the runtime is not alive
    /// and `recreate_on_failure` is disabled.
    pub async fn acquire(
        &self,
        resource: &R,
        resource_config: &R::Config,
        ctx: &ResourceContext,
        _options: &AcquireOptions,
    ) -> Result<ResourceGuard<R>, Error>
    where
        R::Runtime: Into<R::Lease>,
    {
        // Fast path — lock-free load + liveness check.
        if let Some(existing) = self.cell.load()
            && resource.is_alive_sync(&existing)
        {
            let lease: R::Lease = (*existing).clone().into();
            return Ok(ResourceGuard::owned(lease, R::key(), TopologyTag::Resident));
        }

        // Slow path — serialise create / recreate.
        let _guard = self.create_lock.lock().await;

        // Double-check: another task may have created while we waited.
        if let Some(existing) = self.cell.load() {
            if resource.is_alive_sync(&existing) {
                let lease: R::Lease = (*existing).clone().into();
                return Ok(ResourceGuard::owned(lease, R::key(), TopologyTag::Resident));
            }

            // Still not alive — destroy and recreate if configured.
            if !self.config.recreate_on_failure {
                return Err(Error::transient("resident runtime is not alive"));
            }

            // Take the old runtime out and best-effort destroy.
            if let Some(old) = self.cell.take() {
                match Arc::try_unwrap(old) {
                    Ok(owned) => {
                        let _ =
                            tokio::time::timeout(Duration::from_secs(10), resource.destroy(owned))
                                .await;
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

        // Create a new runtime. `create` reads each `#[credential]` slot
        // through its derive-emitted accessor (`SlotCell::load`) — a read
        // internal to `create`, distinct from the `credential_slot_epoch()`
        // sample below.
        let runtime = match tokio::time::timeout(
            self.config.create_timeout,
            resource.create(resource_config, ctx),
        )
        .await
        {
            Ok(Ok(rt)) => rt,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err(Error::transient("resident: create timed out")),
        };

        // Capture the credential epoch *after* `create` has read the slot,
        // not before it. A pre-`create` sample is a stale approximation: a
        // lock-free `SlotCell::store` (engine rotation fan-out) landing
        // *between* the sample and `create`'s own slot read builds the
        // runtime against the **fresh** credential while `built_epoch`
        // would record the **old** epoch, so the create-vs-rotate reconcile
        // (`dispatch_resident_hook`) computes `built < slot_epoch` and
        // spuriously classifies an already-fresh runtime as stale —
        // logging a bogus stale-reconcile and re-stamping `built_epoch`
        // for a runtime that was never stale. Sampling *after* `create`
        // returns makes `built_epoch` an at-or-after-read bound: a `create`
        // that read the post-`store` credential records the post-`store`
        // epoch (correctly up-to-date, no spurious stale).
        //
        // A `store` landing in the narrow window *between* `create`'s slot
        // read and this sample makes `built_epoch` at most one epoch
        // newer than what `create` actually read. That does **not** lose
        // the rotation: `dispatch_resident_hook` delivers the hook
        // **unconditionally** when a runtime is present (the `stale` flag
        // gates only the WARN log and the redundant `built_epoch`
        // self-heal, never hook delivery), and the resource's
        // `on_credential_refresh` / `on_credential_revoke` re-reads the
        // *current* slot — so that store's own dispatch (serialised behind
        // this create slow path by `create_lock`) still reconciles the
        // runtime. The pre-`create` sample, by contrast, mis-fires the
        // stale path for a runtime that read the fresh value — the bug
        // this ordering fixes. The sample stays under `create_lock` and
        // is published with the runtime, preserving the exactly-once
        // dispatch / create-vs-rotate semantics.
        let built_epoch = resource.credential_slot_epoch();

        let lease: R::Lease = runtime.clone().into();
        self.cell.store(Arc::new(runtime));
        // Publish the (runtime, built_epoch) pair. Both writes happen
        // under `create_lock`; the rotation dispatch reads both under the
        // same lock, so it never observes a stored runtime with a stale
        // (un-updated) `built_epoch`.
        self.built_epoch.store(built_epoch, Ordering::Release);

        Ok(ResourceGuard::owned(lease, R::key(), TopologyTag::Resident))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use nebula_core::{ExecutionId, ResourceKey, resource_key};

    use super::*;
    use crate::{
        context::ResourceContext,
        options::AcquireOptions,
        resource::{ResourceConfig, ResourceMetadata},
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

    #[derive(Debug, Clone)]
    struct MockError(String);

    impl std::fmt::Display for MockError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for MockError {}

    impl From<MockError> for Error {
        fn from(e: MockError) -> Self {
            Error::transient(e.0)
        }
    }

    impl ResourceConfig for bool {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
    }

    impl Resource for MockResident {
        type Config = bool;
        type Runtime = u32;
        type Lease = u32;
        type Error = MockError;

        fn key() -> ResourceKey {
            resource_key!("mock-resident")
        }

        fn create(
            &self,
            _config: &bool,
            _ctx: &ResourceContext,
        ) -> impl Future<Output = Result<u32, MockError>> + Send {
            let count = self.create_count.fetch_add(1, Ordering::Relaxed);
            async move {
                // Yield to increase the chance of concurrent interleaving.
                tokio::task::yield_now().await;
                Ok(count + 100)
            }
        }

        async fn destroy(&self, _runtime: u32) -> Result<(), MockError> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Resident for MockResident {
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
    async fn concurrent_acquire_creates_only_once() {
        let resource = MockResident::new();
        let rt = Arc::new(ResidentRuntime::<MockResident>::new(Config::default()));
        let ctx = Arc::new(test_ctx());

        // Spawn 10 concurrent acquires on an empty cell.
        let mut handles = Vec::new();
        for _ in 0..10 {
            let r = resource.clone();
            let runtime = Arc::clone(&rt);
            let c = Arc::clone(&ctx);
            handles.push(tokio::spawn(async move {
                runtime
                    .acquire(&r, &true, c.as_ref(), &AcquireOptions::default())
                    .await
                    .unwrap()
            }));
        }

        for h in handles {
            let _ = h.await.unwrap();
        }

        // Only one create should have happened.
        assert_eq!(
            resource.create_count.load(Ordering::Relaxed),
            1,
            "concurrent acquires on empty cell should create exactly once"
        );
    }

    #[tokio::test]
    async fn acquire_creates_on_first_call() {
        let resource = MockResident::new();
        let rt = ResidentRuntime::<MockResident>::new(Config::default());
        let ctx = test_ctx();

        let handle = rt
            .acquire(&resource, &true, &ctx, &AcquireOptions::default())
            .await
            .unwrap();
        assert_eq!(*handle, 100);
        assert_eq!(resource.create_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn acquire_reuses_existing_instance() {
        let resource = MockResident::new();
        let rt = ResidentRuntime::<MockResident>::new(Config::default());
        let ctx = test_ctx();

        let h1 = rt
            .acquire(&resource, &true, &ctx, &AcquireOptions::default())
            .await
            .unwrap();
        let h2 = rt
            .acquire(&resource, &true, &ctx, &AcquireOptions::default())
            .await
            .unwrap();

        // Both should have the same value — only one create.
        assert_eq!(*h1, *h2);
        assert_eq!(resource.create_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn acquire_recreates_when_not_alive_and_configured() {
        let resource = MockResident::new();
        let config = Config {
            recreate_on_failure: true,
            ..Default::default()
        };
        let rt = ResidentRuntime::<MockResident>::new(config);
        let ctx = test_ctx();

        // First acquire — creates.
        let h1 = rt
            .acquire(&resource, &true, &ctx, &AcquireOptions::default())
            .await
            .unwrap();
        assert_eq!(*h1, 100);

        // Mark as not alive.
        resource.alive.store(false, Ordering::Relaxed);

        // Second acquire — should recreate.
        resource.alive.store(true, Ordering::Relaxed);
        // Need to mark not alive for the check, then alive for the new instance.
        resource.alive.store(false, Ordering::Relaxed);
        // Actually, after recreate the new instance will be checked on next acquire.
        // Let's just test that recreate happens.
        resource.alive.store(true, Ordering::Relaxed);

        // Mark not alive so existing is rejected.
        resource.alive.store(false, Ordering::Relaxed);
        // The acquire will destroy old, create new. The new one won't be checked
        // via is_alive_sync on the same acquire call — it's stored and returned.
        let h2 = rt
            .acquire(&resource, &true, &ctx, &AcquireOptions::default())
            .await
            .unwrap();
        assert_eq!(*h2, 101); // Second creation.
        assert_eq!(resource.create_count.load(Ordering::Relaxed), 2);
    }

    // A resource whose `create()` never returns — for timeout tests.
    #[derive(Clone)]
    struct HangingResident;

    impl Resource for HangingResident {
        type Config = bool;
        type Runtime = u32;
        type Lease = u32;
        type Error = MockError;

        fn key() -> ResourceKey {
            resource_key!("hanging-resident")
        }

        async fn create(&self, _config: &bool, _ctx: &ResourceContext) -> Result<u32, MockError> {
            tokio::time::sleep(Duration::from_hours(1)).await;
            Ok(0)
        }

        async fn destroy(&self, _runtime: u32) -> Result<(), MockError> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Resident for HangingResident {}

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resident_create_timeout_does_not_deadlock() {
        let resource = HangingResident;
        let config = Config {
            create_timeout: Duration::from_millis(50),
            ..Default::default()
        };
        let rt = Arc::new(ResidentRuntime::<HangingResident>::new(config));
        let ctx = Arc::new(test_ctx());

        // First acquire should fail quickly with a timeout, not hang.
        let result = rt
            .acquire(&resource, &true, ctx.as_ref(), &AcquireOptions::default())
            .await;
        assert!(result.is_err(), "first acquire should time out");

        // Second acquire must also fail quickly — the create_lock must have
        // been released after the first timeout.
        let result = rt
            .acquire(&resource, &true, ctx.as_ref(), &AcquireOptions::default())
            .await;
        assert!(
            result.is_err(),
            "second acquire should time out (lock released)"
        );
    }

    #[tokio::test]
    async fn acquire_fails_when_not_alive_and_no_recreate() {
        let resource = MockResident::new();
        let config = Config {
            recreate_on_failure: false,
            ..Default::default()
        };
        let rt = ResidentRuntime::<MockResident>::new(config);
        let ctx = test_ctx();

        // First acquire — creates.
        let _h1 = rt
            .acquire(&resource, &true, &ctx, &AcquireOptions::default())
            .await
            .unwrap();

        // Mark as not alive.
        resource.alive.store(false, Ordering::Relaxed);

        // Second acquire — should fail.
        let result = rt
            .acquire(&resource, &true, &ctx, &AcquireOptions::default())
            .await;
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------------
    // Finding #3a — `built_epoch` sampled BEFORE `create()` reads the slot.
    //
    // `acquire`'s slow path samples `resource.credential_slot_epoch()`
    // *before* calling `resource.create()` (which reads the `#[credential]`
    // slot internally — a later, separate read). A lock-free
    // `SlotCell::store` landing *between* that pre-sample and `create()`'s
    // slot read builds the runtime against the **fresh** credential while
    // `built_epoch` records the **old** epoch — so the create-vs-rotate
    // reconcile mis-classifies an already-fresh runtime as stale.
    //
    // Driven deterministically: `create()` parks *before* its slot read;
    // while parked the test does the racing `SlotCell::store` (strictly
    // after `acquire` sampled the epoch, strictly before the slot read);
    // `create()` then reads the fresh credential. The recorded
    // `built_epoch` MUST equal the epoch the runtime actually bound (the
    // post-store generation), not the pre-store approximation.
    // ---------------------------------------------------------------------

    use std::sync::Arc;

    use tokio::sync::Notify;

    use crate::slot::SlotCell;

    #[derive(Default)]
    struct FakeCred(u32);

    impl zeroize::Zeroize for FakeCred {
        fn zeroize(&mut self) {
            self.0 = 0;
        }
    }

    /// Resident resource whose `create()` parks *before* reading its
    /// credential slot and binds the runtime to whatever the slot holds at
    /// read time. `credential_slot_epoch()` is the slot generation (a
    /// single slot, so the derive's order-sensitive fold collapses to it).
    #[derive(Clone)]
    struct SlotReadResident {
        slot: Arc<SlotCell<FakeCred>>,
        /// Fired the instant `create()` is entered, BEFORE the slot read.
        entered_before_read: Arc<Notify>,
        /// `create()` parks here until the test releases it.
        release_read: Arc<Notify>,
        park_before_read: Arc<AtomicBool>,
    }

    impl Resource for SlotReadResident {
        type Config = bool;
        type Runtime = u32;
        type Lease = u32;
        type Error = MockError;

        fn key() -> ResourceKey {
            resource_key!("slot-read-resident")
        }

        async fn create(&self, _config: &bool, _ctx: &ResourceContext) -> Result<u32, MockError> {
            // Signal "entered, NOT yet read the slot", then park (if armed)
            // so the test can store a fresh credential strictly after
            // `acquire` sampled `credential_slot_epoch()` and strictly
            // before this slot read.
            self.entered_before_read.notify_one();
            if self.park_before_read.load(Ordering::SeqCst) {
                self.release_read.notified().await;
            }
            let cred = self
                .slot
                .load()
                .map(|g| g.0)
                .ok_or(MockError("slot unbound at create".to_owned()))?;
            Ok(cred)
        }

        async fn destroy(&self, _runtime: u32) -> Result<(), MockError> {
            Ok(())
        }

        // Mirror the derive: single slot ⇒ epoch == slot generation.
        fn credential_slot_epoch(&self) -> u64 {
            self.slot.generation()
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Resident for SlotReadResident {
        fn is_alive_sync(&self, _runtime: &u32) -> bool {
            true
        }
    }

    const CRED_OLD: u32 = 7;
    const CRED_NEW: u32 = 99;

    /// THE Finding #3a regression. The first acquire's `create()` parks
    /// before reading the slot. While parked — strictly *after* `acquire`
    /// sampled `credential_slot_epoch()` (the old generation) — the test
    /// stores the NEW credential. `create()` then reads the **NEW**
    /// credential, so the runtime is bound to `CRED_NEW` and never served
    /// `CRED_OLD`.
    ///
    /// The decisive invariant: the recorded `built_epoch` must equal the
    /// slot generation the runtime actually read (post-store), so the
    /// create-vs-rotate reconcile sees the runtime as **up-to-date**.
    /// Pre-fix `built_epoch` is the pre-store generation, the reconcile
    /// computes `built < slot_epoch`, and an already-fresh runtime is
    /// spuriously classified stale — a spurious extra `on_credential_*`
    /// reconcile.
    // `#[cfg(test)]` (redundant inside this `#[cfg(test)] mod tests`) makes
    // the test-only context explicit; `.expect()` is the idiomatic
    // test-only failure and is permitted in tests per `clippy.toml`.
    #[cfg(test)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn built_epoch_records_post_create_slot_epoch_not_presample() {
        let slot: SlotCell<FakeCred> = SlotCell::empty();
        slot.store(Arc::new(FakeCred(CRED_OLD)));
        let slot = Arc::new(slot);
        let gen_old = slot.generation();

        let resource = SlotReadResident {
            slot: Arc::clone(&slot),
            entered_before_read: Arc::new(Notify::new()),
            release_read: Arc::new(Notify::new()),
            park_before_read: Arc::new(AtomicBool::new(true)),
        };
        let rt = Arc::new(ResidentRuntime::<SlotReadResident>::new(Config::default()));
        let ctx = Arc::new(test_ctx());

        let acquire_task = {
            let rt = Arc::clone(&rt);
            let resource = resource.clone();
            let ctx = Arc::clone(&ctx);
            tokio::spawn(async move {
                rt.acquire(&resource, &true, ctx.as_ref(), &AcquireOptions::default())
                    .await
            })
        };

        // `create()` has been entered but has NOT read the slot yet. The
        // `credential_slot_epoch()` pre-sample inside `acquire` is
        // sequenced *before* the `create()` call, so it already observed
        // `gen_old`.
        resource.entered_before_read.notified().await;

        // The racing `SlotCell::store`: rotate while the build is parked
        // before its slot read. `store` strictly advances the generation.
        slot.store(Arc::new(FakeCred(CRED_NEW)));
        let gen_new = slot.generation();
        assert!(
            gen_new > gen_old,
            "store must strictly advance the slot generation"
        );

        // Release the parked build → `create()` reads the slot = CRED_NEW.
        resource.release_read.notify_one();

        let guard = acquire_task
            .await
            .expect("acquire task must not panic")
            .expect("first acquire must succeed");

        // The runtime read the slot AFTER the store, so it bound CRED_NEW.
        assert_eq!(
            *guard, CRED_NEW,
            "create() read the slot after the store — runtime bound to the \
             fresh credential, it never served the old one"
        );
        // The runtime is actually live in the cell.
        assert!(
            rt.is_initialized(),
            "the resident runtime must be stored after a successful acquire"
        );
        // The slot still holds the post-store generation (no skew).
        assert_eq!(
            slot.generation(),
            gen_new,
            "the live slot epoch is the post-store generation"
        );

        // The load-bearing assertion: `built_epoch` must equal the
        // generation the slot held when `create()` actually read it
        // (`gen_new`), NOT the pre-create approximation (`gen_old`). With
        // the pre-sample bug this is `gen_old`, so the create-vs-rotate
        // reconcile would compute `built (gen_old) < slot_epoch (gen_new)`
        // and spuriously reconcile an already-fresh runtime.
        assert_eq!(
            rt.built_epoch_for_test(),
            gen_new,
            "built_epoch must be the epoch the runtime actually bound \
             (post-create slot read), not the pre-create sample; recording \
             the stale pre-sample makes the create-vs-rotate reconcile \
             classify an already-fresh runtime as stale"
        );

        // Direct consequence: the reconcile sees the runtime as up-to-date
        // (built_epoch == live slot epoch), so it is NOT stale.
        assert!(
            rt.built_epoch_for_test() >= slot.generation(),
            "a runtime built reading the current slot must not be older \
             than the live slot epoch (no spurious stale reconcile)"
        );
    }

    /// Sanity: with NO racing store (`create()` reads the slot it was
    /// sampled against), `built_epoch` equals the live slot epoch — the
    /// runtime is up-to-date and the reconcile never reports it stale.
    // `#[cfg(test)]` (redundant inside this `#[cfg(test)] mod tests`) makes
    // the test-only context explicit; `.expect()` is permitted in tests
    // per `clippy.toml`.
    #[cfg(test)]
    #[tokio::test]
    async fn built_epoch_matches_slot_epoch_with_no_race() {
        let slot: SlotCell<FakeCred> = SlotCell::empty();
        slot.store(Arc::new(FakeCred(CRED_OLD)));
        let slot = Arc::new(slot);

        let resource = SlotReadResident {
            slot: Arc::clone(&slot),
            entered_before_read: Arc::new(Notify::new()),
            release_read: Arc::new(Notify::new()),
            park_before_read: Arc::new(AtomicBool::new(false)),
        };
        let rt = ResidentRuntime::<SlotReadResident>::new(Config::default());
        let ctx = test_ctx();

        let guard = rt
            .acquire(&resource, &true, &ctx, &AcquireOptions::default())
            .await
            .expect("acquire must succeed");
        assert_eq!(*guard, CRED_OLD);
        assert!(
            rt.is_initialized(),
            "the resident runtime must be stored after a successful acquire"
        );
        assert_eq!(
            rt.built_epoch_for_test(),
            slot.generation(),
            "with no racing store, built_epoch is exactly the live slot epoch"
        );
    }
}
