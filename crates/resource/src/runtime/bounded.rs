//! Bounded topology — a runtime concurrency cap over a non-pooled resource.
//!
//! `Bounded<R>` is the built-in framework topology for resources that limit how
//! many leases are live at once without keeping a warm idle pool. It backs the
//! gate with a `tokio::Semaphore` whose size is read from a runtime value (not a
//! const generic), so a config-driven seat count flows through the same code.
//! See [`crate::topology::bounded`] for the per-mode table (cap gate / instance
//! lifecycle / use case); this module is the framework-owned drive logic behind
//! it, not a second copy of that contract.
//!
//! Because `Capped`/`Unbounded` keep no idle credentialed instance, the store
//! revoke-fence (which only reaches *pooled* idle entries) has nothing to evict
//! and there is no revoke leak to guard against — they report
//! [`handles_own_revoke`](crate::topology::Topology::handles_own_revoke) so the
//! registration footgun-guard stays quiet. `Exclusive` pools its one instance,
//! so the store fence covers its revoke teardown directly.
//!
//! [`Topology<R>`]: crate::topology::Topology

use std::{
    marker::PhantomData,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

use tokio::sync::{Semaphore, TryAcquireError};

use crate::topology::store::StoreView;
use crate::{
    context::ResourceContext,
    error::Error,
    resource::Provider,
    topology::{AdmissionPhase, Load, Ticket, Topology, Unavailable, bounded::BoundedMode},
    topology_tag::TopologyTag,
};

/// Framework bounded topology — a runtime concurrency cap over a non-pooled
/// resource. See [`crate::topology::bounded`] for the mode table.
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
    /// The live `R::Config` fingerprint, updated by
    /// [`set_fingerprint`](Topology::set_fingerprint) on `Manager::reload_config`.
    /// Seeded on the first [`create_entry`](Topology::create_entry) from the
    /// registered config (so it starts equal to the built instance's
    /// fingerprint and `accept` does not spuriously evict before any reload).
    /// Only consulted for [`BoundedMode::Exclusive`], the one mode that pools
    /// (reuses) its instance; `Capped`/`Unbounded` build fresh per acquire.
    current_fingerprint: AtomicU64,
    /// The `R::Config` fingerprint the currently-stored Exclusive instance was
    /// built against (`0` before the first create). A reload changes
    /// `current_fingerprint`; the next `accept` then evicts the reused instance
    /// so it is rebuilt against the new config — symmetric to the pool's
    /// stale-fingerprint eviction and the resident's master rebuild.
    built_fingerprint: AtomicU64,
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
        let n = NonZeroUsize::new(n)
            .ok_or_else(|| Error::permanent("Bounded::capped requires a cap of at least 1"))?;
        Ok(Self {
            mode: BoundedMode::Capped(n),
            sem: Some(Arc::new(Semaphore::new(n.get()))),
            cap: AtomicUsize::new(n.get()),
            resize_lock: std::sync::Mutex::new(()),
            current_fingerprint: AtomicU64::new(0),
            built_fingerprint: AtomicU64::new(0),
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
            current_fingerprint: AtomicU64::new(0),
            built_fingerprint: AtomicU64::new(0),
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
            current_fingerprint: AtomicU64::new(0),
            built_fingerprint: AtomicU64::new(0),
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
// `Bounded<R>` gates concurrency with a semaphore. `Entry = R::Instance`,
// `entry_instance` / `into_owned_instance` are identity. Only `Exclusive` pools (one
// reused instance, reset on release); `Capped` / `Unbounded` destroy every
// instance on release.

impl<R> Topology<R> for Bounded<R>
where
    R: Provider<Topology = Bounded<R>>
        + crate::topology::bounded::BoundedProvider
        + Send
        + Sync
        + 'static,
{
    type Entry = R::Instance;

    fn try_reserve(&self, _store: StoreView<'_, R::Instance>) -> Result<Ticket, Unavailable> {
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

    async fn create_entry(
        &self,
        resource: &R,
        config: &R::Config,
        ctx: &ResourceContext,
        _retained: &crate::RetainedStore<Self::Entry>,
    ) -> Result<crate::topology::CreatedEntry<R::Instance>, Error> {
        use crate::resource::ResourceConfig as _;
        // No author callback may run after create while the instance is unarmed.
        let fp = config.fingerprint();
        let instance = resource.create(config, ctx).await?;
        // Stamp the config fingerprint this instance was built against, and
        // seed the live fingerprint on the very first build. A reload updates
        // `current_fingerprint` via `set_fingerprint`, so a later rebuild's
        // `compare_exchange` is a no-op that does not clobber the reloaded
        // value. Relevant only to `Exclusive` (the reused instance); harmless
        // for `Capped`/`Unbounded`, which never reach `accept`.
        self.built_fingerprint.store(fp, Ordering::Release);
        let _ =
            self.current_fingerprint
                .compare_exchange(0, fp, Ordering::AcqRel, Ordering::Acquire);
        Ok(crate::topology::CreatedEntry::new(instance))
    }

    fn entry_instance<'s>(&self, entry: &'s R::Instance) -> &'s R::Instance {
        entry
    }

    fn into_owned_instance(&self, entry: R::Instance) -> Option<R::Instance> {
        Some(entry)
    }

    /// Evicts the reused `Exclusive` instance when its config has been
    /// hot-reloaded, so it is rebuilt against the new config on the next
    /// acquire.
    ///
    /// `accept` runs only for a pooling topology — here, `Exclusive`: the
    /// framework calls it on the checked-out reused instance before handing it
    /// to a lease, and `false` makes the framework destroy and recreate it via
    /// `create_entry`. The check compares the built-against vs live config
    /// fingerprint; `Manager::reload_config` bumps the live one through
    /// [`set_fingerprint`](Self::set_fingerprint), so the stale instance is
    /// rebuilt. Normal reuse (unchanged config) matches — the fingerprint is
    /// seeded at first build. `Capped`/`Unbounded` never pool, so `accept` is
    /// not reached for them. This mirrors the pool's stale-fingerprint eviction
    /// and the resident's master rebuild.
    async fn accept(
        &self,
        _entry: &mut R::Instance,
        _resource: &R,
        _ctx: &ResourceContext,
    ) -> bool {
        self.built_fingerprint.load(Ordering::Acquire)
            == self.current_fingerprint.load(Ordering::Acquire)
    }

    fn set_fingerprint(&self, fingerprint: u64) {
        self.current_fingerprint
            .store(fingerprint, Ordering::Release);
    }

    async fn on_release(&self, entry: &mut R::Instance, resource: &R) -> Result<bool, Error> {
        match self.mode {
            // The single instance is reset and reused. A failed reset returns
            // `Err`, so the framework destroys it (never reissues a half-reset
            // instance — S4) and surfaces the error; a fresh one is built next
            // acquire.
            BoundedMode::Exclusive => {
                resource.reset(entry).await?;
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

    fn phase(&self, _store: StoreView<'_, R::Instance>) -> AdmissionPhase {
        match &self.sem {
            Some(sem) if sem.available_permits() == 0 => AdmissionPhase::Saturated,
            _ => AdmissionPhase::Ready,
        }
    }

    fn load(&self, _store: StoreView<'_, R::Instance>) -> Option<Load> {
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
#[path = "bounded_tests.rs"]
mod tests;
