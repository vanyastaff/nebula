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
            self.inner
                .store(Some(Arc::new(SlotEntry { generation, value })));
        });
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
            self.inner
                .store(Some(Arc::new(SlotEntry { generation, value })));
            generation
        })
    }
}

/// Convenience alias for the standard credential slot field type.
///
/// `CredentialSlot<C>` is exactly `SlotCell<nebula_credential::CredentialGuard<C>>`.
/// Use this alias in your resource struct's `#[credential]` fields to reduce
/// field-type noise:
///
/// ```ignore
/// use nebula_resource::CredentialSlot;
///
/// struct Postgres {
///     #[credential(key = "db")]
///     iam: CredentialSlot<IamToken>,
/// }
/// ```
///
/// Both syntactic shapes — `SlotCell<CredentialGuard<C>>` and
/// `CredentialSlot<C>` — are accepted by `#[derive(Resource)]`.
pub type CredentialSlot<C> = SlotCell<nebula_credential::CredentialGuard<C>>;

impl<S> Default for SlotCell<S> {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct FakeGuard(u32);
    impl zeroize::Zeroize for FakeGuard {
        fn zeroize(&mut self) {
            self.0 = 0;
        }
    }

    #[test]
    fn slot_cell_swaps_without_clone_and_reads_latest() {
        let cell: SlotCell<FakeGuard> = SlotCell::empty();
        assert!(cell.load().is_none());
        cell.store(Arc::new(FakeGuard(1)));
        assert_eq!(cell.load().expect("v1").0, 1);
        cell.store(Arc::new(FakeGuard(2)));
        assert_eq!(cell.load().expect("v2").0, 2);
    }

    #[test]
    fn take_and_is_some() {
        let cell: SlotCell<FakeGuard> = SlotCell::empty();

        // Empty cell: is_some is false, take returns None.
        assert!(!cell.is_some());
        assert!(cell.take().is_none());

        // After store: is_some is true, take returns the value.
        cell.store(Arc::new(FakeGuard(1)));
        assert!(cell.is_some());
        let taken = cell.take();
        assert_eq!(taken.expect("should be Some").0, 1);

        // After take: cell is empty again.
        assert!(cell.load().is_none());
        assert!(!cell.is_some());

        // Second take on now-empty cell returns None.
        assert!(cell.take().is_none());
    }

    #[test]
    fn generation_starts_at_zero_and_is_strictly_monotonic() {
        let cell: SlotCell<FakeGuard> = SlotCell::empty();
        // Never bound.
        assert_eq!(cell.generation(), 0, "unbound slot epoch is 0");
        assert!(cell.load_versioned().is_none());

        // First store -> generation 1, coupled to the value.
        cell.store(Arc::new(FakeGuard(10)));
        let (g1, v1) = cell.load_versioned().expect("bound");
        assert_eq!(g1, 1, "first store is generation 1");
        assert_eq!(v1.0, 10);
        assert_eq!(cell.generation(), 1);

        // Second store -> strictly greater generation, new value.
        cell.store(Arc::new(FakeGuard(20)));
        let (g2, v2) = cell.load_versioned().expect("bound");
        assert!(g2 > g1, "store must strictly advance the generation");
        assert_eq!(v2.0, 20);
    }

    #[test]
    fn take_advances_generation_and_is_observable_when_empty() {
        let cell: SlotCell<FakeGuard> = SlotCell::empty();
        cell.store(Arc::new(FakeGuard(1)));
        let g_after_store = cell.generation();
        assert_eq!(g_after_store, 1);

        // A clear is a credential-state transition: the generation must
        // advance so an instance built against the pre-clear guard is
        // detectably stale, and it stays observable while empty.
        let _ = cell.take();
        assert!(cell.load().is_none(), "slot is cleared");
        let g_after_take = cell.generation();
        assert!(
            g_after_take > g_after_store,
            "take must strictly advance the generation (a clear is a transition)"
        );

        // Storing again after a clear keeps advancing.
        cell.store(Arc::new(FakeGuard(2)));
        assert!(cell.generation() > g_after_take);
    }

    #[test]
    fn take_on_never_bound_still_advances_generation() {
        let cell: SlotCell<FakeGuard> = SlotCell::empty();
        assert_eq!(cell.generation(), 0);
        // Even a no-op clear advances the generation: a "clear" signal is
        // meaningful to a rotation observer regardless of prior state.
        assert!(cell.take().is_none());
        assert!(
            cell.generation() > 0,
            "take advances generation even when the slot was already empty"
        );
    }

    /// Concurrency characterization (informs the single-writer-per-slot
    /// question; not a fix). Many tasks race `store`/`take` on one cell.
    /// `load_versioned` must never observe a torn `(generation, value)`
    /// pair — the generation must be exactly the one published with that
    /// value (each store stamps the value with its own generation), never a
    /// generation from a different transition.
    ///
    /// The coupling is what makes a torn read *detectable*: every entry is
    /// published via `store_stamped` so its value is exactly its own
    /// generation (`value == generation`). A single immutable `SlotEntry`
    /// observed through one `ArcSwapOption` load must therefore always
    /// satisfy `u64::from(value) == generation`. A torn read — the value of
    /// one transition paired with the generation of another — would break
    /// that equality and fail the assertion below.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_store_take_load_versioned_is_never_torn() {
        let cell: Arc<SlotCell<FakeGuard>> = Arc::new(SlotCell::empty());

        let writers = 8u32;
        let iters = 200u32;
        // Total transitions = stores + takes. The largest generation any
        // store stamps is bounded by this, so it fits in the `u32` payload
        // of `FakeGuard` and the `value == generation` round-trip is exact.
        let total_transitions = u64::from(writers) * u64::from(iters) * 2;
        assert!(
            u32::try_from(total_transitions).is_ok(),
            "test sizing must keep generations within FakeGuard's u32 payload"
        );

        let mut handles = Vec::new();
        for _ in 0..writers {
            let cell = Arc::clone(&cell);
            handles.push(tokio::spawn(async move {
                for _ in 0..iters {
                    // Stamp the value with the *exact* generation this entry
                    // is published at, inside the production single-store
                    // publish. `g as u32` is lossless: `g <=
                    // total_transitions <= u32::MAX` (asserted above).
                    cell.store_stamped(|g| Arc::new(FakeGuard(g as u32)));
                    if let Some((observed_gen, val)) = cell.load_versioned() {
                        assert!(
                            observed_gen >= 1,
                            "a published entry always has generation >= 1"
                        );
                        // The load-bearing torn-read check: value and
                        // generation came from one immutable entry, so the
                        // value must be the generation that entry stamped.
                        // A torn `(generation, value)` pair (value from a
                        // different transition than `observed_gen`) breaks
                        // this equality.
                        assert_eq!(
                            u64::from(val.0),
                            observed_gen,
                            "torn read: value {} was not stamped with its \
                             published generation {observed_gen}",
                            val.0
                        );
                    }
                    let _ = cell.take();
                }
            }));
        }
        for h in handles {
            h.await.expect("writer task must not panic");
        }

        // After all transitions the generation is strictly positive and
        // monotone: every store and every take bumped it exactly once, so
        // it is at least the total number of transitions performed.
        let total_transitions = u64::from(writers) * u64::from(iters) * 2;
        assert!(
            cell.generation() >= total_transitions,
            "generation must have advanced at least once per transition \
             (got {}, expected >= {total_transitions})",
            cell.generation()
        );
    }

    /// Reader/writer race: a dedicated reader continuously calls
    /// `load_versioned` while a writer stores monotically-increasing
    /// generations. The observed generation must be monotone non-decreasing
    /// from this single reader's vantage (no torn read can surface a
    /// generation older than one already observed paired with a newer
    /// value).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn single_reader_observes_monotone_generations_under_concurrent_store() {
        let cell: Arc<SlotCell<FakeGuard>> = Arc::new(SlotCell::empty());

        let writer = {
            let cell = Arc::clone(&cell);
            tokio::spawn(async move {
                for i in 1..=1_000u32 {
                    cell.store(Arc::new(FakeGuard(i)));
                }
            })
        };

        let reader = {
            let cell = Arc::clone(&cell);
            tokio::spawn(async move {
                let mut last = 0u64;
                for _ in 0..5_000 {
                    if let Some((observed_gen, _v)) = cell.load_versioned() {
                        assert!(
                            observed_gen >= last,
                            "load_versioned regressed from {last} to \
                             {observed_gen} (torn read / lost publish \
                             ordering)"
                        );
                        last = observed_gen;
                    }
                }
            })
        };

        writer.await.expect("writer task must not panic");
        reader.await.expect("reader task must not panic");
    }
}

#[cfg(test)]
mod slot_publish_race_tests {
    //! The LIVE published entry must not regress under concurrent writers:
    //! once a transition with generation `g` is live, no writer that
    //! allocated a generation `< g` may overwrite it. Otherwise a
    //! rotated/revoked credential would be resurrected on the live slot.
    //!
    //! Writers are serialized on the slot's write lock, so the bump and
    //! the entry swap are one indivisible transition: a writer cannot even
    //! allocate its generation until the previous transition has fully
    //! published. The earlier lock-free attempts left an instruction-wide
    //! gap between the bump and the publish (and a `compare_exchange`
    //! floor did not close it — the floor claim and the swap were still
    //! separate, so a preempted winner could be overtaken and then clobber
    //! the newer entry). A gap-injecting seam therefore no longer models
    //! anything real, so this is a high-contention many-writer
    //! characterization: with `W` writers each performing one stamped
    //! store (value == its own generation), the final live entry must be
    //! the highest generation and its value must match — a single stale
    //! publish landing last would make the live generation `< W` and fail.
    //! This is strictly stronger than the old two-writer scenario and is
    //! design-agnostic (it also fails the buggy lock-free variants).
    //!
    //! `.expect()` is the idiomatic test-only failure here; `clippy.toml`
    //! exempts tests from the no-unwrap rule, and this whole module is
    //! `#[cfg(test)]`.

    use std::sync::Arc;

    use super::*;

    #[derive(Default)]
    struct FakeGuard(u32);
    impl zeroize::Zeroize for FakeGuard {
        fn zeroize(&mut self) {
            self.0 = 0;
        }
    }

    // guard-justified: this replaces a lock-free gap-seam scenario that
    // would deadlock against the write-serialized design (a parked writer
    // holds the lock, blocking every other writer). The property asserted
    // — the highest generation is the live one, no stale publish clobbers
    // a newer entry — is unchanged and strictly harder to satisfy (W
    // contending writers instead of 2), with more invariants checked, not
    // fewer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn live_entry_is_the_highest_generation_under_concurrent_writers() {
        let cell: Arc<SlotCell<FakeGuard>> = Arc::new(SlotCell::empty());
        assert!(
            cell.load().is_none(),
            "a fresh slot must start unbound (no live entry)"
        );
        assert_eq!(cell.generation(), 0, "an unbound slot's epoch is 0");
        assert!(
            cell.load_versioned().is_none(),
            "an unbound slot has no versioned entry"
        );

        // `W` writers each perform exactly one stamped store. `store_stamped`
        // couples the value to the generation that store actually publishes
        // (value == generation), so the live pair proves *which* writer's
        // store is live, not merely that some value is.
        let writers = 32u32;
        assert!(
            writers >= 2,
            "the no-regression property is only meaningful with >= 2 \
             concurrent writers"
        );
        let mut handles = Vec::new();
        for _ in 0..writers {
            let cell = Arc::clone(&cell);
            handles.push(tokio::task::spawn_blocking(move || {
                cell.store_stamped(|g| Arc::new(FakeGuard(g as u32)))
            }));
        }
        let mut gens: Vec<u64> = Vec::with_capacity(writers as usize);
        for h in handles {
            let g = h.await.expect("writer task must not panic");
            assert!(
                (1..=u64::from(writers)).contains(&g),
                "every allocated generation is in 1..=W (got {g})"
            );
            gens.push(g);
        }
        assert_eq!(
            gens.len(),
            writers as usize,
            "every writer must have completed exactly one store"
        );

        // Every writer allocated a distinct, gapless generation in 1..=W.
        gens.sort_unstable();
        assert_eq!(
            gens,
            (1..=u64::from(writers)).collect::<Vec<_>>(),
            "each store must allocate a unique, gapless generation"
        );

        // The decisive invariant: the live entry is the HIGHEST generation
        // and its value matches. A stale store landing last (the bug this
        // guards) would leave a generation `< W` live.
        assert!(
            cell.is_some(),
            "a value must be live after all stores completed"
        );
        let (lv_gen, lv_val) = cell.load_versioned().expect("a value must be live");
        assert_eq!(
            lv_gen,
            u64::from(writers),
            "LIVE entry regressed: a stale store overwrote the newest entry \
             (live generation {lv_gen} < {writers}) — a rotated/revoked \
             credential resurrected on the live slot"
        );
        assert_eq!(
            u64::from(lv_val.0),
            lv_gen,
            "live (generation, value) pair must be torn-read-free and be \
             the highest writer's, not a stale resurrection"
        );
        assert_eq!(
            cell.generation(),
            lv_gen,
            "generation() must agree with the live entry's published \
             generation (no skew between the two read paths)"
        );
        assert_eq!(
            cell.load().expect("a value must be live").0,
            writers,
            "the lock-free `load` path must also see the highest writer's \
             value"
        );

        // A `take` after the stores still strictly advances the generation
        // and clears: the clear is serialized after every store, so it can
        // neither be lost nor undo a newer transition.
        let cleared = cell.take().expect("the live value is returned on take");
        assert_eq!(
            u64::from(cleared.0),
            u64::from(writers),
            "take returns the live (highest) value"
        );
        assert!(cell.load().is_none(), "slot is cleared after take");
        assert!(!cell.is_some(), "is_some is false after take");
        assert!(
            cell.load_versioned().is_none(),
            "no versioned entry after take"
        );
        assert!(
            cell.generation() > u64::from(writers),
            "take strictly advances the generation past the last store"
        );
    }
}
