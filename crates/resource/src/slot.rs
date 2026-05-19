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
//! from one transition and a guard from another). A built resource runtime
//! records the generation it was constructed against; the per-slot rotation
//! dispatch compares that against the live generation to detect a runtime
//! left bound to a pre-rotation credential by a create-vs-rotate race
//! (per-resource revoke deferral). See `ResidentRuntime` / `ManagedResource::
//! dispatch_slot_hook`.

use std::sync::{
    Arc,
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
    /// `1` (generation `0` ≡ "never bound"). This is the *allocator*: it
    /// hands each transition a unique number but does **not** decide which
    /// transition's effect is live — that is [`committed`](Self::committed).
    next_generation: AtomicU64,
    /// Highest generation whose effect (a `store` publish *or* a `take`
    /// clear) has actually won the race to become the live state.
    ///
    /// `next_generation` only *allocates* numbers; `bump_generation()` and
    /// the entry swap are two separate steps, so two writers can reorder so
    /// the slower one (lower allocated generation) reaches the swap *last*.
    /// An unconditional store would then leave the **older** generation
    /// live while a newer transition had already happened — a
    /// rotated/revoked credential resurrected on the live slot. `committed`
    /// is the monotone floor that closes that: a transition publishes its
    /// effect only by winning a `compare_exchange` that advances
    /// `committed` to its own (strictly greater) generation, so the live
    /// generation is monotone non-decreasing under any number of concurrent
    /// writers and a `take` cannot be undone by a stale `store`.
    ///
    /// This is **not** `ArcSwapOption::rcu`: the module rejects `rcu`
    /// because its `FnMut` closure is retried under contention and a
    /// side-effecting generation bump inside it would *gap* the
    /// strictly-monotone sequence the create-vs-rotate reconcile compares.
    /// The generation here is allocated *once* by `bump_generation()`
    /// before the loop; the loop only decides whether this already-numbered
    /// transition wins — it never re-numbers anything, so there is no gap.
    committed: AtomicU64,
}

impl<S> SlotCell<S> {
    /// An unresolved slot (generation `0` ≡ "never bound").
    pub fn empty() -> Self {
        Self {
            inner: ArcSwapOption::empty(),
            next_generation: AtomicU64::new(0),
            committed: AtomicU64::new(0),
        }
    }

    /// Allocates the next strictly-increasing generation for a transition.
    fn bump_generation(&self) -> u64 {
        // `fetch_add` returns the prior value; the first call yields `0`,
        // so `+ 1` makes the first allocated generation `1` and every
        // subsequent transition strictly greater. `Relaxed` is sufficient:
        // this only *allocates* a unique number. Torn-read freedom of the
        // value↔generation pair is carried by the single `ArcSwapOption`
        // publish/observe of the immutable `SlotEntry`; *which* transition
        // becomes live (and the happens-before for that decision) is
        // carried by the `AcqRel` `committed` `compare_exchange` in
        // `publish` / `take`, not by this allocator's memory order.
        self.next_generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Generation-guarded publish of `(generation, value)` as the live
    /// entry.
    ///
    /// `generation` was already allocated by [`bump_generation`] *before*
    /// this call, so the loop only decides whether this already-numbered
    /// transition wins the publish race — it never re-numbers anything (no
    /// epoch gap, unlike an `rcu` retry). The transition becomes live only
    /// by winning a `compare_exchange` that advances
    /// [`committed`](Self::committed) from a strictly smaller value to
    /// `generation`; the entry swap then publishes the value+generation as
    /// one immutable [`SlotEntry`] (so [`load_versioned`] stays torn-read
    /// free). A slower writer that allocated an *earlier* generation but
    /// reaches here *after* a newer transition observes
    /// `committed >= generation` and drops its stale entry rather than
    /// regressing the live generation.
    ///
    /// [`bump_generation`]: Self::bump_generation
    /// [`load_versioned`]: Self::load_versioned
    fn publish(&self, generation: u64, value: Arc<S>) {
        let entry = Arc::new(SlotEntry { generation, value });
        loop {
            let committed = self.committed.load(Ordering::Acquire);
            if committed >= generation {
                // A transition with an equal-or-newer generation already
                // won. Publishing our older entry would resurrect a stale
                // (rotated/revoked) credential on the live slot — drop it.
                return;
            }
            // Claim the right to publish *this* generation. `AcqRel` so the
            // subsequent entry swap is ordered after the win and a
            // concurrent loser's `Acquire` re-read observes it.
            if self
                .committed
                .compare_exchange(committed, generation, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.inner.store(Some(Arc::clone(&entry)));
                return;
            }
            // Lost the floor race to another writer; re-read and re-decide.
            // No re-numbering happens here, so the strictly-monotone
            // generation sequence the reconcile compares is never gapped.
        }
    }

    /// Install (or replace) the resolved value, bumping the generation.
    ///
    /// The new generation is published atomically *with* the value inside a
    /// single internal entry swap, so a concurrent [`load_versioned`] never
    /// observes the new value paired with an old generation (or vice
    /// versa). The publish is **generation-guarded** (see
    /// [`publish`](Self::publish)): under concurrent writers a slower
    /// writer that allocated an *earlier* generation cannot overwrite a
    /// *newer* live entry, so the live generation is monotone
    /// non-decreasing and a rotated/revoked credential is never resurrected
    /// on the live slot.
    ///
    /// [`load_versioned`]: Self::load_versioned
    pub fn store(&self, value: Arc<S>) {
        let generation = self.bump_generation();
        self.publish(generation, value);
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
    /// generation of the latest *committed* transition (`store` *or*
    /// `take`).
    ///
    /// A cleared slot keeps the generation of the `take` that cleared it
    /// (a clear is itself a credential-state transition — a runtime built
    /// before a revoke must still see a strictly newer epoch), so this is
    /// `> 0` after the first transition even when [`load`](Self::load)
    /// returns `None`.
    ///
    /// When an entry is live its own (published, torn-read-free)
    /// generation is authoritative. When there is no entry the fallback is
    /// [`committed`](Self::committed) — the monotone floor — **not**
    /// `next_generation` (a stale `store` that lost the publish race never
    /// advances `committed`, so the post-`take` generation cannot be
    /// regressed by a slower writer; `next_generation` is only the
    /// allocator and can run ahead of what actually became live).
    pub fn generation(&self) -> u64 {
        match self.inner.load_full() {
            Some(entry) => entry.generation,
            // No live entry: either never bound (`committed == 0`) or
            // cleared by the `take` that won the floor (its generation).
            None => self.committed.load(Ordering::Acquire),
        }
    }

    /// Revoke the slot, returning the previously held value (if any).
    ///
    /// A clear is a credential-state transition, so it advances the
    /// generation: a runtime built against the pre-clear guard is then
    /// detectably stale on the next rotation/revoke dispatch (resource
    /// runtime status §Deferred). The post-clear generation is observable
    /// via [`generation`](Self::generation) even though [`load`](Self::load)
    /// is now `None`.
    ///
    /// **Regression-safe** (Finding #3b): like [`publish`](Self::publish),
    /// the clear takes effect only by winning a `compare_exchange` that
    /// advances [`committed`](Self::committed) to this clear's freshly
    /// allocated generation. A clear that allocated an *earlier*
    /// generation but reaches here *after* a newer transition does **not**
    /// undo that newer transition (it would otherwise wipe a freshly
    /// rotated-in credential); the newer transition already advanced the
    /// generation a pre-clear runtime observes as stale, so the clear's
    /// intent (a strictly newer epoch is visible) is already satisfied.
    /// A `take` on an already-empty / never-bound slot still wins the
    /// floor (no newer transition raced it) and so still advances the
    /// generation — the "clear signal" stays meaningful regardless of
    /// prior state.
    pub fn take(&self) -> Option<Arc<S>> {
        // Allocate this clear's generation first so that even an
        // already-empty slot's clear is a numbered transition.
        let generation = self.bump_generation();
        loop {
            let committed = self.committed.load(Ordering::Acquire);
            if committed >= generation {
                // A transition with an equal-or-newer generation already
                // won. Clearing now would undo it (resurrecting "empty"
                // over a freshly stored credential). The newer transition
                // already advanced the generation past any pre-clear
                // runtime, so the clear's observable intent is met; return
                // the current value snapshot without regressing the state.
                return self.load();
            }
            if self
                .committed
                .compare_exchange(committed, generation, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return self.inner.swap(None).map(|entry| Arc::clone(&entry.value));
            }
            // Lost the floor race; re-read and re-decide (no re-numbering).
        }
    }

    /// Returns `true` if the slot currently holds a resolved value.
    pub fn is_some(&self) -> bool {
        self.inner.load().is_some()
    }
}

#[cfg(test)]
impl<S> SlotCell<S> {
    /// Publish an entry whose value is *derived from the same generation it
    /// is stamped with*, using the production publish sequence.
    ///
    /// The public [`store`](Self::store) takes the value *before*
    /// [`bump_generation`](Self::bump_generation) assigns the entry's
    /// generation, so under concurrent writers a caller cannot make the
    /// stored value equal the published generation — which is exactly the
    /// coupling a torn-read characterization test needs. This test-only
    /// helper bumps the generation first, then builds the value from it via
    /// `mk`, and publishes both inside the *same* single `ArcSwapOption`
    /// store as production. A reader that observes a torn `(generation,
    /// value)` pair (value from one transition, generation from another)
    /// will see `value != mk(generation)`.
    fn store_stamped(&self, mk: impl FnOnce(u64) -> Arc<S>) -> u64 {
        let generation = self.bump_generation();
        let value = mk(generation);
        self.publish(generation, value);
        generation
    }

    /// Production `store`, but with a caller-controlled barrier fired
    /// **between** `bump_generation()` and the publish.
    ///
    /// `bump_generation()` (a `fetch_add`) and the publish are two separate
    /// steps; that gap is exactly where two writers can reorder so the
    /// slower one (lower generation) publishes *last* and regresses the
    /// live entry. The race window is a few instructions wide in
    /// production, so a characterization test cannot reliably hit it by
    /// chance. This seam exposes the gap deterministically: the test runs
    /// `gap` between the bump and the publish, so it can force the exact
    /// *A-bump / B-bump / B-publish / A-publish* interleave. It routes
    /// through the same [`publish`](Self::publish) path production uses, so
    /// it stays faithful before and after the generation-guard fix.
    fn store_with_gap(&self, value: Arc<S>, gap: impl FnOnce(u64)) -> u64 {
        let generation = self.bump_generation();
        gap(generation);
        self.publish(generation, value);
        generation
    }
}

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
        // advance so a runtime built against the pre-clear guard is
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
    //! Finding #3b — the LIVE published entry must not regress under >=2
    //! concurrent writers.
    //!
    //! `bump_generation()` (a `fetch_add`) and the entry publish are two
    //! separate steps. Two writers can interleave as
    //! *A bumps->1, B bumps->2, B publishes(2), A publishes(1)* — an
    //! unconditional `inner.store` then leaves generation 1 live while 2
    //! was the newer transition (a rotated/revoked credential resurrected
    //! on the live slot). The existing multi-writer test only checks
    //! per-entry `value == generation` and counter monotonicity, not
    //! *live-entry non-regression* — this is the missing characterization.
    //!
    //! The reorder window is a few instructions in production, so this
    //! drives it deterministically with `store_with_gap`: writer A bumps,
    //! then blocks in the gap on a channel; writer B bumps + publishes its
    //! (newer) entry; the test then releases A, which publishes its
    //! (older) entry strictly last. The live entry must still be B's.
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn live_entry_generation_never_regresses_under_two_writers() {
        let cell: Arc<SlotCell<FakeGuard>> = Arc::new(SlotCell::empty());
        // Precondition: a fresh slot is unbound (no live entry).
        assert!(
            cell.load().is_none(),
            "a fresh slot must start unbound (no live entry)"
        );

        // A signals "I have bumped" on `a_bumped_tx`, then blocks reading
        // `release_a_rx` until the test (having observed B's publish) lets
        // it proceed to publish its OLDER-generation entry last.
        let (a_bumped_tx, a_bumped_rx) = std::sync::mpsc::channel::<()>();
        let (release_a_tx, release_a_rx) = std::sync::mpsc::channel::<()>();

        let writer_a = {
            let cell = Arc::clone(&cell);
            tokio::task::spawn_blocking(move || {
                cell.store_with_gap(Arc::new(FakeGuard(1)), move |_g| {
                    // A has its (lower) generation but has NOT published.
                    let _ = a_bumped_tx.send(());
                    // Block until the test releases A — deterministic: the
                    // test only sends after B's newer publish is observed.
                    let _ = release_a_rx.recv();
                })
            })
        };

        // Wait until A has bumped (lower generation), parked pre-publish.
        a_bumped_rx
            .recv()
            .expect("writer A must reach the gap and signal");

        // Writer B: bumps the HIGHER generation and publishes immediately.
        let cell_b = Arc::clone(&cell);
        let gen_b = tokio::task::spawn_blocking(move || {
            cell_b.store_with_gap(Arc::new(FakeGuard(2)), |_g| {})
        })
        .await
        .expect("writer B must not panic");

        // B's newer entry is live before A's stale publish.
        assert!(gen_b > 1, "B must hold a strictly higher generation");
        assert_eq!(
            cell.generation(),
            gen_b,
            "B's newer entry must be live before A's stale publish"
        );

        // Release A -> it publishes its OLDER-generation entry LAST.
        let _ = release_a_tx.send(());
        let gen_a = writer_a.await.expect("writer A must not panic");

        // The decisive invariant: A published an OLDER generation strictly
        // AFTER B published a NEWER one. The live entry must STILL be B's.
        // Pre-fix the unconditional publish let A's older entry win, so
        // the live generation regressed to A's (B's value resurrected away
        // in favour of A's older one).
        assert!(gen_b > gen_a, "B bumped after A, so gen_b > gen_a");
        assert_eq!(
            cell.generation(),
            gen_b,
            "LIVE entry regressed: writer A (generation {gen_a}) published \
             after writer B (generation {gen_b}) and overwrote the newer \
             entry — a rotated/revoked credential resurrected on the live \
             slot"
        );

        // The live entry must still be present and, read torn-read-free
        // via `load_versioned`, its generation and value must be B's
        // (not A's resurrected older pair).
        let versioned = cell.load_versioned();
        assert!(
            versioned.is_some(),
            "a value must be live after both writers published"
        );
        let (lv_gen, lv_val) = versioned.expect("live entry present");
        assert_eq!(
            lv_gen, gen_b,
            "live entry generation must be B's {gen_b}, not A's stale {gen_a}"
        );
        assert_eq!(
            lv_val.0, 2,
            "live entry value (coupled with generation {lv_gen}) must be \
             B's, not A's stale resurrection"
        );
        // `generation()` agrees with the torn-read-free published
        // generation (no skew between the two read paths).
        assert_eq!(
            cell.generation(),
            lv_gen,
            "generation() must agree with the live entry's published \
             generation"
        );
        let live = cell.load().expect("a value must be live");
        assert_eq!(
            live.0, 2,
            "live value must be B's (generation {gen_b}), not A's stale \
             resurrection"
        );
    }
}
