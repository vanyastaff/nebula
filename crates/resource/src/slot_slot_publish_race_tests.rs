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
