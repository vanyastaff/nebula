//! Per-slot runtime storage for a resolved credential.
//!
//! A resource declares `#[credential]` slots; the engine resolves each into a
//! `CredentialGuard<C>` and stores it here before `Resource::create`. On
//! rotation the engine swaps a fresh guard in without `&mut` on the
//! resource (the `&self` refresh-hook model, resource runtime status). Lock-free via
//! `arc-swap`.
//!
//! # Generation / epoch (per-resource revoke deferral — create-vs-rotate ordering)
//!
//! Every credential-state transition (`store`, `take`) bumps a strictly
//! monotonically increasing **generation**. `0` is reserved for "never
//! bound" — the first `store` lands at generation `1`. The generation is
//! coupled to the stored value inside a single immutable internal entry
//! published through one `ArcSwapOption` swap, so a reader observes the
//! generation and the guard it belongs to with **no torn read** (a separate
//! `AtomicU64` read alongside an `ArcSwap` load could observe a generation
//! from one transition and a guard from another). A built resource instance
//! records the generation it was constructed against; the per-slot rotation
//! dispatch compares that against the live generation to detect an instance
//! left bound to a pre-rotation credential by a create-vs-rotate race
//! (per-resource revoke deferral). See `Resident` / `ManagedResource::
//! dispatch_slot_hook`.

use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicU64, Ordering},
};

use arc_swap::ArcSwapOption;

/// Internal authority marker for a terminal credential revoke.
const REVOKED_AUTHORITY: u64 = u64::MAX;

/// An immutable (generation, value) pair published as one unit.
///
/// Storing the generation *inside* the swapped `Arc` (rather than in a
/// sibling atomic) is what makes [`SlotCell::load_versioned`] torn-read
/// free: a single `ArcSwapOption` load yields the guard and the exact
/// generation it was published at, never a generation from a different
/// transition.
#[derive(Debug)]
struct SlotEntry<S> {
    /// Strictly monotonically increasing; `>= 1` for any published value.
    generation: u64,
    /// Authoritative credential-material epoch supplied by the resolver.
    material_epoch: u64,
    /// The resolved slot value (`CredentialGuard<C>` in production).
    value: Arc<S>,
}

/// Lock-free interior-mutable holder for one resolved credential slot.
///
/// Holds an `Arc` of an internal generation+value entry: a real slot value
/// is `CredentialGuard<C>`,
/// which is `!Clone` and zeroizes on `Drop`, so the `Arc<S>` indirection
/// inside the entry lets the engine swap a rotated guard in with no
/// secret-byte clone. Every transition carries a fresh generation so a
/// runtime built against an older guard is detectable on rotation
/// (per-resource revoke deferral).
#[derive(Debug)]
pub struct SlotCell<S> {
    inner: ArcSwapOption<SlotEntry<S>>,
    /// Source of strictly increasing generations. `fetch_add` returns the
    /// *previous* value, so the first transition observes `0` and stamps
    /// `1` (generation `0` ≡ "never bound"). Only ever advanced while
    /// `write_lock` is held, so the number a transition allocates and the
    /// entry it then publishes cannot be reordered against another
    /// writer's.
    next_generation: AtomicU64,
    /// Highest authoritative material epoch accepted by this slot, including
    /// a revoke tombstone. Read and written only while `write_lock` is held
    /// for mutation; the atomic supports lock-free diagnostics.
    material_epoch: AtomicU64,
    /// Serializes writers (`store` / `take`).
    ///
    /// `bump_generation()` and the entry swap are two steps. If they could
    /// interleave across writers the slower one (lower allocated
    /// generation) could publish *last* and leave the **older** generation
    /// live while a newer transition had already happened — a
    /// rotated/revoked credential resurrected on the live slot. A
    /// `compare_exchange` floor does not close this on its own: the floor
    /// claim and the entry swap are still separate, so a writer preempted
    /// between them can be overtaken and then overwrite the newer entry.
    /// Holding this lock across *both* the bump and the swap makes the
    /// transition indivisible, so the live generation is monotone
    /// non-decreasing under any number of concurrent writers and a `take`
    /// cannot be undone by a stale `store` — correct by construction
    /// rather than by a lock-free ordering argument.
    ///
    /// Writes are rare (credential rotation / revoke events, not a hot
    /// path), so serializing them has no practical contention cost.
    /// **Readers never take this lock** — [`load`](Self::load),
    /// [`load_versioned`](Self::load_versioned),
    /// [`generation`](Self::generation) and [`is_some`](Self::is_some)
    /// stay lock-free on the `ArcSwapOption`.
    write_lock: Mutex<()>,
}

impl<S> SlotCell<S> {
    /// An unresolved slot (generation `0` ≡ "never bound").
    pub fn empty() -> Self {
        Self {
            inner: ArcSwapOption::empty(),
            next_generation: AtomicU64::new(0),
            material_epoch: AtomicU64::new(0),
            write_lock: Mutex::new(()),
        }
    }

    /// Allocates the next strictly-increasing generation for a transition.
    ///
    /// Only ever called while `write_lock` is held, so the number it
    /// allocates and the entry the same critical section then publishes
    /// cannot be reordered against another writer. `fetch_add` returns the
    /// prior value; the first call yields `0`, so `+ 1` makes the first
    /// allocated generation `1` and every subsequent transition strictly
    /// greater. `Relaxed` is sufficient: the write lock provides the
    /// happens-before for which transition becomes live, and torn-read
    /// freedom of the value↔generation pair is carried by the single
    /// `ArcSwapOption` publish/observe of the immutable `SlotEntry`.
    fn bump_generation(&self) -> u64 {
        self.next_generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Runs `mutate` with this transition's freshly allocated generation
    /// while holding `write_lock`, so the bump and the entry swap `mutate`
    /// performs are one indivisible transition.
    ///
    /// This is what makes the live generation monotone non-decreasing
    /// under any number of concurrent writers: a second writer cannot bump
    /// (let alone publish) until the first has both bumped *and* published,
    /// so a lower generation can never reach the swap after a higher one.
    /// The lock is poison-tolerant — the critical section is a counter
    /// bump plus an `ArcSwapOption` swap and cannot panic, so a poisoned
    /// guard (from an unrelated panic elsewhere) is recovered rather than
    /// cascading.
    fn with_write<R>(&self, mutate: impl FnOnce(u64) -> R) -> R {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let generation = self.bump_generation();
        mutate(generation)
    }

    /// Install (or replace) the resolved value, advancing the generation.
    ///
    /// The new generation is published atomically *with* the value inside
    /// a single internal entry swap, so a concurrent
    /// [`load_versioned`](Self::load_versioned) never observes the new
    /// value paired with an old generation (or vice versa). The bump and
    /// the swap run under the write lock as one transition, so under
    /// concurrent writers a slower writer that allocated an *earlier*
    /// generation cannot overwrite a *newer* live entry: the live
    /// generation is monotone non-decreasing and a rotated/revoked
    /// credential is never resurrected on the live slot.
    pub fn store(&self, value: Arc<S>) {
        self.with_write(|generation| {
            let material_epoch = self.material_epoch.load(Ordering::Relaxed);
            self.inner.store(Some(Arc::new(SlotEntry {
                generation,
                material_epoch,
                value,
            })));
        });
    }

    /// Installs credential material when `material_epoch` is newer than every
    /// material or revoke transition already observed by this slot.
    ///
    /// The comparison and publication are serialized with revoke, so a delayed
    /// refresh cannot overwrite newer material or resurrect a revoked value.
    ///
    /// # Errors
    ///
    /// Returns [`SlotInstallError::InvalidMaterialEpoch`] when passed an
    /// epoch reserved for internal unresolved or revoked authority.
    pub fn install_at_material_epoch(
        &self,
        material_epoch: u64,
        value: Arc<S>,
    ) -> Result<SlotUpdate, SlotInstallError> {
        if material_epoch == 0 || material_epoch == REVOKED_AUTHORITY {
            return Err(SlotInstallError::InvalidMaterialEpoch);
        }
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let current = self.material_epoch.load(Ordering::Relaxed);
        if current == REVOKED_AUTHORITY {
            return Ok(SlotUpdate::Revoked);
        }
        if material_epoch <= current {
            return Ok(SlotUpdate::Stale {
                current_material_epoch: current,
            });
        }
        let generation = self.bump_generation();
        self.material_epoch.store(material_epoch, Ordering::Relaxed);
        self.inner.store(Some(Arc::new(SlotEntry {
            generation,
            material_epoch,
            value,
        })));
        Ok(SlotUpdate::Installed)
    }

    /// Snapshot the current value, if resolved.
    pub fn load(&self) -> Option<Arc<S>> {
        self.inner.load_full().map(|entry| Arc::clone(&entry.value))
    }

    /// Snapshot the current `(generation, value)` together.
    ///
    /// The generation and the value come from the *same* internal entry
    /// (one `ArcSwapOption` load) — there is no window in which they can be
    /// from different transitions. Returns `None` (and the caller treats
    /// the epoch as `0`/"never bound") while the slot is unresolved.
    pub fn load_versioned(&self) -> Option<(u64, Arc<S>)> {
        self.inner
            .load_full()
            .map(|entry| (entry.generation, Arc::clone(&entry.value)))
    }

    /// Snapshot the authoritative material epoch and value from one published
    /// entry. Returns `None` when the slot is unresolved or revoked.
    pub fn load_material_versioned(&self) -> Option<(u64, Arc<S>)> {
        self.inner
            .load_full()
            .map(|entry| (entry.material_epoch, Arc::clone(&entry.value)))
    }

    /// Highest authoritative material epoch accepted by this slot, if live.
    ///
    /// Returns `None` for both unresolved and terminally revoked slots because
    /// a revoke tombstone does not carry a material epoch.
    pub fn material_epoch(&self) -> Option<u64> {
        match self.material_epoch.load(Ordering::Relaxed) {
            0 | REVOKED_AUTHORITY => None,
            material_epoch => Some(material_epoch),
        }
    }

    /// The current generation: `0` if never bound, otherwise the
    /// generation of the latest transition (`store` *or* `take`).
    ///
    /// A cleared slot keeps the generation of the `take` that cleared it
    /// (a clear is itself a credential-state transition — an instance built
    /// before a revoke must still see a strictly newer epoch), so this is
    /// `> 0` after the first transition even when [`load`](Self::load)
    /// returns `None`.
    ///
    /// When an entry is live its own (published, torn-read-free)
    /// generation is authoritative. When there is no entry the fallback is
    /// `next_generation`: writers serialize on the write lock and `take`
    /// bumps it, so after a clear it holds that clear's generation, and it
    /// is monotone non-decreasing (it only ever `fetch_add`s). A reader
    /// may observe a bump from an in-flight `store` a moment before that
    /// store's value is published; that only makes a pre-rotation runtime
    /// look stale *slightly early*, never stale late — the conservative
    /// direction for the create-vs-rotate reconcile.
    pub fn generation(&self) -> u64 {
        match self.inner.load_full() {
            Some(entry) => entry.generation,
            // No live entry: never bound (0) or cleared by a `take`.
            // `Relaxed` is correct — a reader that observes the cleared slot
            // has already synchronized with that `take`'s `inner.swap(None)`
            // (arc-swap acquire/release), which is sequenced *after* the take's
            // generation bump, so this load sees the post-clear generation
            // without an acquire of its own; per-location coherence keeps it
            // monotone for a single observer. The tag guards no payload and
            // pairs with no `Release` (the bump is `Relaxed`) — do not add one.
            None => self.next_generation.load(Ordering::Relaxed),
        }
    }

    /// Revoke the slot, returning the previously held value (if any).
    ///
    /// A clear is a credential-state transition, so it advances the
    /// generation: an instance built against the pre-clear guard is then
    /// detectably stale on the next rotation/revoke dispatch (resource
    /// instance status §Deferred). The post-clear generation is observable
    /// via [`generation`](Self::generation) even though [`load`](Self::load)
    /// is now `None`.
    ///
    /// **Regression-safe by construction** (Finding #3b): the bump and the
    /// clear run under the write lock as one transition, so no concurrent
    /// `store` can interleave between them. A later `store` cannot begin
    /// until this `take` has completed, so a stale store can never
    /// resurrect a credential over a newer clear, and this clear can never
    /// wipe a newer store. A `take` on an already-empty / never-bound slot
    /// still bumps the generation — the "clear signal" stays meaningful
    /// regardless of prior state.
    pub fn take(&self) -> Option<Arc<S>> {
        self.with_write(|_generation| self.inner.swap(None).map(|entry| Arc::clone(&entry.value)))
    }

    /// Revokes this slot with terminal authority.
    ///
    /// Once applied, no later or delayed refresh can repopulate the slot.
    /// Repeating revoke is an idempotent terminal no-op.
    pub fn revoke(&self) -> SlotUpdate {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let current = self.material_epoch.load(Ordering::Relaxed);
        if current == REVOKED_AUTHORITY {
            return SlotUpdate::AlreadyRevoked;
        }
        let _generation = self.bump_generation();
        self.material_epoch
            .store(REVOKED_AUTHORITY, Ordering::Relaxed);
        self.inner.store(None);
        SlotUpdate::Revoked
    }

    /// Returns `true` if the slot currently holds a resolved value.
    pub fn is_some(&self) -> bool {
        self.inner.load().is_some()
    }
}

#[cfg(test)]
impl<S> SlotCell<S> {
    /// Publish an entry whose value is *derived from the same generation it
    /// is stamped with*, through the production serialized write path.
    ///
    /// The public [`store`](Self::store) takes the value *before*
    /// [`bump_generation`](Self::bump_generation) assigns the entry's
    /// generation, so under concurrent writers a caller cannot make the
    /// stored value equal the published generation — which is exactly the
    /// coupling a torn-read characterization test needs. This test-only
    /// helper bumps the generation first (still under the same
    /// [`with_write`](Self::with_write) lock production uses), then builds
    /// the value from it via `mk`, and publishes both inside the *same*
    /// single `ArcSwapOption` store. A reader that observed a torn
    /// `(generation, value)` pair (value from one transition, generation
    /// from another) would see `value != mk(generation)`.
    fn store_stamped(&self, mk: impl FnOnce(u64) -> Arc<S>) -> u64 {
        self.with_write(|generation| {
            let value = mk(generation);
            self.inner.store(Some(Arc::new(SlotEntry {
                generation,
                material_epoch: self.material_epoch.load(Ordering::Relaxed),
                value,
            })));
            generation
        })
    }
}

/// Result of an authoritative credential-slot transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "slot update outcomes must be observed"]
pub enum SlotUpdate {
    /// A newer projected guard was installed.
    Installed,
    /// The slot was cleared and fenced against stale refresh.
    Revoked,
    /// The slot was already terminally revoked.
    AlreadyRevoked,
    /// The requested transition was older than existing slot authority.
    Stale {
        /// Highest authoritative material epoch already observed.
        current_material_epoch: u64,
    },
}

/// Typed failure to route or install an erased credential guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SlotInstallError {
    /// The resource does not declare the requested slot.
    #[error("resource does not declare the requested credential slot")]
    UnknownSlot,
    /// The projected guard type differs from the slot's declared type.
    #[error("projected credential guard type does not match the declared slot type")]
    CredentialTypeMismatch,
    /// The epoch collides with a resource-internal authority marker.
    #[error("credential material epoch is reserved for resource slot authority")]
    InvalidMaterialEpoch,
}

/// Convenience alias for the standard credential slot field type.
///
/// `CredentialSlot<C>` stores `CredentialGuard<C::Scheme>` for credential
/// definition `C`.
/// Use this alias in your resource struct's `#[credential]` fields to reduce
/// field-type noise — e.g. a `#[credential(key = "db")] iam: CredentialSlot<IamToken>`
/// field. Both syntactic shapes — `SlotCell<CredentialGuard<C>>` and
/// `CredentialSlot<C>` — are accepted by `#[derive(Resource)]`.
///
/// A `CredentialSlot<C>` is a [`SlotCell`] over the projected scheme guard, so
/// it carries the same generation-stamped, lock-free cell mechanics:
///
/// ```
/// use std::sync::Arc;
///
/// use nebula_resource::SlotCell;
///
/// // The cell underlying a `CredentialSlot` (here over a plain `u32`; in a
/// // resource the slot value is `CredentialGuard<C>`).
/// let cell: SlotCell<u32> = SlotCell::empty();
/// assert_eq!(cell.generation(), 0, "an unbound slot's epoch is 0");
/// assert!(cell.load().is_none());
///
/// cell.store(Arc::new(7));
/// assert_eq!(cell.load().as_deref(), Some(&7));
/// assert_eq!(cell.generation(), 1, "the first store lands at generation 1");
/// ```
pub type CredentialSlot<C> =
    SlotCell<nebula_credential::CredentialGuard<<C as nebula_credential::Credential>::Scheme>>;

impl<S> Default for SlotCell<S> {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
#[path = "slot_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "slot_slot_publish_race_tests.rs"]
mod slot_publish_race_tests;
