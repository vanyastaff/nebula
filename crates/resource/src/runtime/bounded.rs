//! Bounded topology — a runtime concurrency cap over a non-pooled resource.
//!
//! `Bounded<R>` is the built-in framework topology for resources that limit how
//! many leases are live at once without keeping a warm idle pool. It backs the
//! gate with a `tokio::Semaphore` whose size is read from a runtime value (not a
//! const generic), so a config-driven seat count flows through the same code:
//!
//! - [`BoundedMode::Capped`] (cap `n`) / [`BoundedMode::Unbounded`] — `pools()`
//!   is `false`: each acquire builds a fresh instance via
//!   [`create_slot`](crate::topology::Topology::create_slot) and destroys it on
//!   release. There is no idle reuse (that is [`Pooled`](crate::topology::Pooled)).
//! - [`BoundedMode::Exclusive`] — `pools()` is `true` with a store capacity of
//!   one: the single instance is reset on release and returned to the framework
//!   store for the next lease. A failed reset destroys it instead (the S4
//!   invariant — a half-reset instance is never reissued), and the next acquire
//!   builds a fresh one.
//!
//! Because `Capped`/`Unbounded` keep no idle credentialed instance, the store
//! revoke-fence (which only reaches *pooled* idle slots) has nothing to evict
//! and there is no revoke leak to guard against — they report
//! [`handles_own_revoke`](crate::topology::Topology::handles_own_revoke) so the
//! registration footgun-guard stays quiet. `Exclusive` pools its one instance,
//! so the store fence covers its revoke teardown directly.
//!
//! [`Topology<R>`]: crate::topology::Topology

use std::{
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use tokio::sync::{Semaphore, TryAcquireError};

use crate::{
    context::ResourceContext,
    error::Error,
    resource::{HasCredentialSlots, Provider},
    topology::{
        AdmissionPhase, Load, Ticket, Topology, Unavailable, bounded::BoundedMode,
        store::InstanceStore,
    },
    topology_tag::TopologyTag,
};

/// Framework bounded topology — a runtime concurrency cap over a non-pooled
/// resource. See the [module docs](self) for the mode table.
///
/// [`Topology<R>`]: crate::topology::Topology
pub struct Bounded<R: Provider> {
    mode: BoundedMode,
    /// The concurrency gate. `None` for [`BoundedMode::Unbounded`] (no cap);
    /// `Some(Semaphore(n))` for `Capped(n)` / `Exclusive` (`n == 1`).
    sem: Option<Arc<Semaphore>>,
    /// The current effective cap (permit total). `0` for `Unbounded`. Tracked
    /// alongside the semaphore so [`load`](Topology::load) can report saturation
    /// and [`set_cap`](Self::set_cap) can diff against it.
    cap: AtomicUsize,
    /// Serializes [`set_cap`](Self::set_cap): the resize is a compound
    /// read-modify-write over both the semaphore (`add_permits` /
    /// `forget_permits`) and `cap`, so two concurrent `&self` calls would
    /// otherwise lose an update and leave `cap` inconsistent with the real
    /// permit count. Uncontended in the common case (resize is a rare admin op).
    resize_lock: std::sync::Mutex<()>,
    _marker: PhantomData<fn() -> R>,
}

impl<R: Provider> std::fmt::Debug for Bounded<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bounded")
            .field("mode", &self.mode)
            .field("cap", &self.cap.load(Ordering::Relaxed))
            .finish()
    }
}

impl<R: Provider> Bounded<R> {
    /// Builds a `Capped(n)` topology: at most `n` concurrent leases.
    ///
    /// # Errors
    ///
    /// Returns a permanent [`Error`] if `n == 0` — a zero cap can never admit a
    /// lease, so it is rejected at construction rather than silently dead-locked
    /// every acquire.
    pub fn capped(n: usize) -> Result<Self, Error> {
        if n == 0 {
            return Err(Error::permanent(
                "Bounded::capped requires a cap of at least 1",
            ));
        }
        Ok(Self {
            mode: BoundedMode::Capped(n),
            sem: Some(Arc::new(Semaphore::new(n))),
            cap: AtomicUsize::new(n),
            resize_lock: std::sync::Mutex::new(()),
            _marker: PhantomData,
        })
    }

    /// Builds an `Exclusive` topology: exactly one lease at a time over a single
    /// reused instance, reset between leases.
    #[must_use]
    pub fn exclusive() -> Self {
        Self {
            mode: BoundedMode::Exclusive,
            sem: Some(Arc::new(Semaphore::new(1))),
            cap: AtomicUsize::new(1),
            resize_lock: std::sync::Mutex::new(()),
            _marker: PhantomData,
        }
    }

    /// Builds an `Unbounded` topology: no concurrency limit, fresh instance per
    /// lease.
    #[must_use]
    pub fn unbounded() -> Self {
        Self {
            mode: BoundedMode::Unbounded,
            sem: None,
            cap: AtomicUsize::new(0),
            resize_lock: std::sync::Mutex::new(()),
            _marker: PhantomData,
        }
    }

    /// Returns the topology's mode.
    #[must_use]
    pub fn mode(&self) -> BoundedMode {
        self.mode
    }

    /// Resizes a `Capped` topology's concurrency limit at runtime.
    ///
    /// Growing adds permits immediately. Shrinking forgets currently-available
    /// permits immediately; if fewer permits are free than the requested
    /// reduction (leases are in flight), only the free ones are forgotten and
    /// the effective cap settles at `cur - forgotten` — call again once leases
    /// return to complete the shrink. The effective cap reported by
    /// [`load`](Topology::load) always reflects what actually took effect.
    ///
    /// # Errors
    ///
    /// Returns a permanent [`Error`] if the topology is not `Capped` (an
    /// `Exclusive` cap is fixed at one and `Unbounded` has no cap) or if `n ==
    /// 0`.
    pub fn set_cap(&self, n: usize) -> Result<(), Error> {
        let sem = match (self.mode, &self.sem) {
            (BoundedMode::Capped(_), Some(sem)) => sem,
            _ => {
                return Err(Error::permanent(
                    "Bounded::set_cap applies only to a Capped topology",
                ));
            },
        };
        if n == 0 {
            return Err(Error::permanent(
                "Bounded::set_cap requires a cap of at least 1",
            ));
        }
        // Serialize the compound semaphore+cap read-modify-write so two
        // concurrent resizes cannot lose an update. Recover from a poisoned
        // lock — the guarded `()` carries no state a prior panic could corrupt.
        let _resize = self
            .resize_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cur = self.cap.load(Ordering::Acquire);
        match n.cmp(&cur) {
            std::cmp::Ordering::Greater => {
                sem.add_permits(n - cur);
                self.cap.store(n, Ordering::Release);
            },
            std::cmp::Ordering::Less => {
                let forgotten = sem.forget_permits(cur - n);
                self.cap.fetch_sub(forgotten, Ordering::Release);
            },
            std::cmp::Ordering::Equal => {},
        }
        Ok(())
    }
}

// ─── Topology impl for Bounded ────────────────────────────────────────────────
//
// `Bounded<R>` gates concurrency with a semaphore. `Slot = R::Instance`,
// `slot_instance` / `into_instance` are identity. Only `Exclusive` pools (one
// reused instance, reset on release); `Capped` / `Unbounded` destroy every
// instance on release.

#[async_trait]
impl<R> Topology<R> for Bounded<R>
where
    R: Provider<Topology = Bounded<R>>
        + crate::topology::bounded::BoundedProvider
        + HasCredentialSlots
        + Send
        + Sync
        + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    type Slot = R::Instance;

    fn try_reserve(&self, _store: &InstanceStore<R::Instance>) -> Result<Ticket, Unavailable> {
        match &self.sem {
            // Unbounded: no gate.
            None => Ok(Ticket::infallible()),
            Some(sem) => match Arc::clone(sem).try_acquire_owned() {
                Ok(permit) => Ok(Ticket::permit(permit)),
                Err(TryAcquireError::NoPermits) => {
                    Err(Unavailable::Saturated { retry_after: None })
                },
                // A closed semaphore means the topology is being torn down — no
                // new leases. Surface it as tainted so the acquire fails closed.
                Err(TryAcquireError::Closed) => Err(Unavailable::Tainted),
            },
        }
    }

    async fn create_slot(
        &self,
        resource: &R,
        config: &R::Config,
        ctx: &ResourceContext,
    ) -> Result<R::Instance, Error> {
        resource.create(config, ctx).await
    }

    fn slot_instance<'s>(&self, slot: &'s R::Instance) -> &'s R::Instance {
        slot
    }

    fn into_instance(&self, slot: R::Instance) -> R::Instance {
        slot
    }

    async fn on_release(&self, slot: &mut R::Instance, resource: &R) -> Result<bool, Error> {
        match self.mode {
            // The single instance is reset and reused. A failed reset returns
            // `Err`, so the framework destroys it (never reissues a half-reset
            // instance — S4) and surfaces the error; a fresh one is built next
            // acquire.
            BoundedMode::Exclusive => {
                resource.reset(slot).await?;
                Ok(true)
            },
            // No reuse: every released instance is destroyed.
            BoundedMode::Capped(_) | BoundedMode::Unbounded => Ok(false),
        }
    }

    fn pools(&self) -> bool {
        matches!(self.mode, BoundedMode::Exclusive)
    }

    fn handles_own_revoke(&self) -> bool {
        // Capped / Unbounded keep no idle credentialed instance to leak on
        // revoke (every lease is created fresh and destroyed on release), so
        // they legitimately "handle their own revoke" and the footgun-guard
        // stays quiet. Exclusive pools its one instance, so `pools() == true`
        // and this is ignored — the store fence covers it.
        !matches!(self.mode, BoundedMode::Exclusive)
    }

    fn store_capacity(&self) -> Option<usize> {
        match self.mode {
            // The one reused instance lives in the framework store.
            BoundedMode::Exclusive => Some(1),
            // Non-pooling: the store stays empty.
            BoundedMode::Capped(_) | BoundedMode::Unbounded => None,
        }
    }

    fn phase(&self, _store: &InstanceStore<R::Instance>) -> AdmissionPhase {
        match &self.sem {
            Some(sem) if sem.available_permits() == 0 => AdmissionPhase::Saturated,
            _ => AdmissionPhase::Ready,
        }
    }

    fn load(&self, _store: &InstanceStore<R::Instance>) -> Option<Load> {
        let sem = self.sem.as_ref()?;
        let total = self.cap.load(Ordering::Acquire);
        let used = total.saturating_sub(sem.available_permits());
        Some(Load::permits(used, total))
    }

    fn tag(&self) -> TopologyTag {
        TopologyTag::Bounded
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use nebula_core::{ExecutionId, ResourceKey, resource_key};

    use super::*;
    use crate::{
        resource::{ResourceConfig, ResourceMetadata},
        topology::bounded::BoundedProvider,
    };

    #[derive(Debug, Clone, Copy)]
    struct BoundedCfg;

    nebula_schema::impl_empty_has_schema!(BoundedCfg);

    impl ResourceConfig for BoundedCfg {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }

        fn fingerprint(&self) -> u64 {
            // Unit struct: all instances identical — constant 0 is correct.
            0
        }
    }

    #[derive(Clone)]
    struct MockBounded {
        reset_ok: Arc<AtomicBool>,
        reset_calls: Arc<AtomicUsize>,
    }

    impl MockBounded {
        fn new() -> Self {
            Self {
                reset_ok: Arc::new(AtomicBool::new(true)),
                reset_calls: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait]
    impl Provider for MockBounded {
        type Config = BoundedCfg;
        type Instance = u32;
        type Topology = Bounded<Self>;

        fn key() -> ResourceKey {
            resource_key!("mock-bounded")
        }

        async fn create(&self, _config: &BoundedCfg, _ctx: &ResourceContext) -> Result<u32, Error> {
            Ok(7)
        }

        async fn destroy(&self, _instance: u32, _cx: crate::TeardownCx) -> Result<(), Error> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl HasCredentialSlots for MockBounded {
        fn credential_slot_epoch(&self) -> u64 {
            0
        }
    }

    #[async_trait]
    impl BoundedProvider for MockBounded {
        async fn reset(&self, _instance: &mut u32) -> Result<(), Error> {
            self.reset_calls.fetch_add(1, Ordering::Relaxed);
            if self.reset_ok.load(Ordering::Relaxed) {
                Ok(())
            } else {
                Err(Error::transient("reset failed"))
            }
        }
    }

    fn store() -> InstanceStore<u32> {
        InstanceStore::new(None)
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

    #[test]
    fn capped_rejects_zero_cap() {
        assert!(
            Bounded::<MockBounded>::capped(0).is_err(),
            "a zero cap can never admit and must be rejected at construction"
        );
        assert!(Bounded::<MockBounded>::capped(1).is_ok());
    }

    #[tokio::test]
    async fn capped_gate_admits_up_to_n() {
        let topo = Bounded::<MockBounded>::capped(2).expect("cap >= 1");
        let st = store();

        let t1 = topo.try_reserve(&st).expect("first lease admitted");
        let t2 = topo.try_reserve(&st).expect("second lease admitted");
        assert!(
            topo.try_reserve(&st).is_err(),
            "third lease exceeds the cap of 2 — must be rejected"
        );
        assert_eq!(topo.phase(&st), AdmissionPhase::Saturated);
        assert_eq!(topo.load(&st).expect("capped reports load").saturation, 1.0);

        // Releasing a ticket returns its permit; capacity frees up.
        drop(t1);
        let t3 = topo.try_reserve(&st).expect("a freed permit re-admits");
        drop((t2, t3));
    }

    #[tokio::test]
    async fn exclusive_serialises_to_one() {
        let topo = Bounded::<MockBounded>::exclusive();
        let st = store();

        let held = topo.try_reserve(&st).expect("first exclusive lease");
        assert!(
            topo.try_reserve(&st).is_err(),
            "exclusive admits exactly one at a time"
        );
        drop(held);
        let _next = topo
            .try_reserve(&st)
            .expect("the next lease admits once the first releases");
    }

    #[tokio::test]
    async fn exclusive_resets_and_keeps_on_release() {
        let resource = MockBounded::new();
        let topo = Bounded::<MockBounded>::exclusive();
        let mut slot = 7u32;

        let keep = topo
            .on_release(&mut slot, &resource)
            .await
            .expect("a clean reset keeps the instance");
        assert!(
            keep,
            "exclusive reuses its one instance after a clean reset"
        );
        assert_eq!(resource.reset_calls.load(Ordering::Relaxed), 1);
        assert!(
            topo.pools(),
            "exclusive pools its single reused instance (store cap 1)"
        );
        assert_eq!(topo.store_capacity(), Some(1));
    }

    #[tokio::test]
    async fn exclusive_reset_error_discards_and_surfaces() {
        let resource = MockBounded::new();
        resource.reset_ok.store(false, Ordering::Relaxed);
        let topo = Bounded::<MockBounded>::exclusive();
        let mut slot = 7u32;

        let outcome = topo.on_release(&mut slot, &resource).await;
        assert!(
            outcome.is_err(),
            "a failed reset surfaces the error so the framework destroys the \
             instance (never reissues a half-reset one — S4)"
        );
    }

    #[tokio::test]
    async fn capped_destroys_on_release() {
        let resource = MockBounded::new();
        let topo = Bounded::<MockBounded>::capped(4).expect("cap >= 1");
        let mut slot = 7u32;

        let keep = topo.on_release(&mut slot, &resource).await.expect("ok");
        assert!(
            !keep,
            "capped does not pool — released instances are destroyed"
        );
        assert_eq!(
            resource.reset_calls.load(Ordering::Relaxed),
            0,
            "only exclusive resets; capped never calls reset"
        );
        assert!(!topo.pools());
        assert!(
            topo.handles_own_revoke(),
            "non-pooling bounded keeps no idle credentialed state to leak"
        );
    }

    #[tokio::test]
    async fn unbounded_always_admits() {
        let topo = Bounded::<MockBounded>::unbounded();
        let st = store();

        let held: Vec<_> = (0..64).map(|_| topo.try_reserve(&st).ok()).collect();
        assert!(
            held.iter().all(Option::is_some),
            "unbounded never rejects a lease"
        );
        assert_eq!(topo.phase(&st), AdmissionPhase::Ready);
        assert!(topo.load(&st).is_none(), "unbounded reports no load");
    }

    #[tokio::test]
    async fn set_cap_grows_and_shrinks() {
        let topo = Bounded::<MockBounded>::capped(2).expect("cap >= 1");
        let st = store();

        // Grow 2 → 4: two more leases now fit.
        topo.set_cap(4).expect("grow");
        let leases: Vec<_> = (0..4)
            .map(|_| topo.try_reserve(&st).expect("4 leases fit after grow"))
            .collect();
        assert!(
            topo.try_reserve(&st).is_err(),
            "the 5th exceeds the grown cap"
        );
        drop(leases);

        // Shrink 4 → 1 while idle: only one lease fits.
        topo.set_cap(1).expect("shrink while idle");
        let _one = topo.try_reserve(&st).expect("one lease fits");
        assert!(
            topo.try_reserve(&st).is_err(),
            "the cap shrank to 1 — a second lease is rejected"
        );
    }

    #[test]
    fn set_cap_rejects_non_capped_and_zero() {
        let exclusive = Bounded::<MockBounded>::exclusive();
        assert!(
            exclusive.set_cap(4).is_err(),
            "exclusive cap is fixed at one"
        );
        let unbounded = Bounded::<MockBounded>::unbounded();
        assert!(unbounded.set_cap(4).is_err(), "unbounded has no cap to set");
        let capped = Bounded::<MockBounded>::capped(2).expect("cap >= 1");
        assert!(capped.set_cap(0).is_err(), "a zero cap is rejected");
    }

    #[tokio::test]
    async fn create_slot_builds_a_fresh_instance() {
        let resource = MockBounded::new();
        let topo = Bounded::<MockBounded>::capped(2).expect("cap >= 1");
        let inst = topo
            .create_slot(&resource, &BoundedCfg, &test_ctx())
            .await
            .expect("create");
        assert_eq!(inst, 7);
        assert_eq!(topo.tag(), TopologyTag::Bounded);
    }
}
