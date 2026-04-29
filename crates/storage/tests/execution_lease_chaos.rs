//! Chaos test — InMemoryExecutionRepo lease contention under high
//! concurrency (ROADMAP §M2.2 / T9).
//!
//! # Why a chaos test in addition to loom + integration?
//!
//! - Loom (`crates/storage-loom-probe/tests/lease_handoff_loom.rs`) exhaustively explores **small**
//!   thread interleavings — 2-3 threads, narrow scenarios.
//! - PG integration (`execution_lease_pg_integration.rs`) covers the real SQL fence on a single
//!   execution.
//! - This test fills the middle gap: many concurrent runners (M) contending over a pool of N
//!   executions for thousands of acquire/release cycles, asserting the holder-uniqueness invariant
//!   that the production code's `acquire_lease` SQL fence + InMemory mutex are designed to enforce.
//!
//! # Gating
//!
//! The test is `#[ignore]`'d by default (mirrors the convention used
//! by `refresh_coordinator_chaos.rs` in the engine crate). It runs via
//! `cargo nextest run -p nebula-storage --test execution_lease_chaos
//! --include-ignored`, which the nightly-chaos workflow opts into.
//! The default `cargo nextest -p nebula-storage` skips it.
//!
//! # Plane
//!
//! 4 runners × 4 executions × 200 iterations each = 800 acquire/release
//! cycles total. ~50ms wall-clock on a warm machine. Sized so the test
//! is fast enough to keep in `--include-ignored` runs without
//! padding CI but wide enough to catch any CAS race a future refactor
//! might introduce.

#![cfg(not(loom))]

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_core::id::ExecutionId;
use nebula_storage::{ExecutionRepo, InMemoryExecutionRepo};

/// Per-execution "currently held by N runners" counter. Production
/// invariant: this MUST never exceed 1 — only one runner can hold a
/// lease at any given moment. The test increments on observed
/// `acquire_lease == Ok(true)` and decrements on observed
/// `release_lease == Ok(true)`. Any observation of `>1` proves a CAS
/// race that the storage layer's holder fence failed to prevent.
struct HolderTracker {
    /// Per-execution active-holder counter (one slot per execution
    /// in the test's pool).
    holders: Vec<AtomicU32>,
    /// Accumulated max-observed holder count across the run. Polled
    /// at the end; must remain `<= 1` to pass.
    max_observed: AtomicU32,
}

impl HolderTracker {
    fn new(n_executions: usize) -> Self {
        let holders = (0..n_executions).map(|_| AtomicU32::new(0)).collect();
        Self {
            holders,
            max_observed: AtomicU32::new(0),
        }
    }

    /// Record a successful acquire and update the running max. The
    /// `compare_exchange`-style increment lets us detect any
    /// transient state where two runners observed themselves as
    /// holder simultaneously.
    fn enter(&self, idx: usize) {
        let current = self.holders[idx].fetch_add(1, Ordering::SeqCst) + 1;
        // Track the high-water mark so we can assert it stays at 1
        // even if a transient violation gets corrected before test
        // teardown reads the per-execution slot.
        self.max_observed.fetch_max(current, Ordering::SeqCst);
    }

    fn leave(&self, idx: usize) {
        self.holders[idx].fetch_sub(1, Ordering::SeqCst);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "chaos test — wall-clock cost; run via --include-ignored"]
async fn no_double_holder_under_concurrent_acquire_release() {
    const N_EXECUTIONS: usize = 4;
    const N_RUNNERS: usize = 4;
    const ITERATIONS_PER_RUNNER: usize = 200;

    let repo = Arc::new(InMemoryExecutionRepo::new());
    let tracker = Arc::new(HolderTracker::new(N_EXECUTIONS));

    // Pre-allocate stable ExecutionIds so each runner can pick by index.
    let exec_ids: Vec<ExecutionId> = (0..N_EXECUTIONS).map(|_| ExecutionId::new()).collect();

    let mut handles = Vec::with_capacity(N_RUNNERS);
    for runner_idx in 0..N_RUNNERS {
        let repo = Arc::clone(&repo);
        let tracker = Arc::clone(&tracker);
        let exec_ids = exec_ids.clone();
        let holder = format!("runner-{runner_idx}");

        let handle = tokio::spawn(async move {
            for i in 0..ITERATIONS_PER_RUNNER {
                // Round-robin across the execution pool with an
                // offset per runner so different runners hit different
                // execs at the same iteration.
                let exec_idx = (i + runner_idx) % N_EXECUTIONS;
                let exec_id = exec_ids[exec_idx];

                // The InMemory repo clamps TTL to >= 1.0s
                // (`normalized_lease_ttl`). We do not advance time here
                // — the test is about ACQUIRE/RELEASE contention
                // before TTL fires. Each acquire that succeeds is
                // explicitly released; expiration is not the path
                // exercised in this test.
                let acquired = repo
                    .acquire_lease(exec_id, holder.clone(), Duration::from_mins(1))
                    .await
                    .expect("acquire never errors with InMemory repo");

                if acquired {
                    tracker.enter(exec_idx);
                    // Yield to the runtime to maximize the chance of
                    // another runner observing the holder before we
                    // release.
                    tokio::task::yield_now().await;
                    tracker.leave(exec_idx);

                    let released = repo
                        .release_lease(exec_id, &holder)
                        .await
                        .expect("release never errors with InMemory repo");
                    assert!(
                        released,
                        "runner {holder} acquired exec[{exec_idx}] but release returned false"
                    );
                }
                // If contended, just spin to the next iteration —
                // production runners would back off; for chaos load
                // we want maximum attempts.
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.await.expect("runner task panicked");
    }

    // The headline invariant: at no point during the run did two
    // runners simultaneously hold a lease for the same execution.
    let max = tracker.max_observed.load(Ordering::SeqCst);
    assert_eq!(
        max, 1,
        "lease holder uniqueness violated — max observed concurrent holders per execution was {max} \
         (must be <= 1)"
    );

    // Sanity: every per-execution slot returned to 0 (no leaks from
    // unbalanced enter/leave or missed releases).
    for (idx, slot) in tracker.holders.iter().enumerate() {
        assert_eq!(
            slot.load(Ordering::SeqCst),
            0,
            "execution[{idx}] tracker did not return to 0 — release path leak"
        );
    }
}
