//! Loom tests asserting CAS atomicity for the execution-lease handoff
//! pattern (ROADMAP §M2.2 / T8).
//!
//! Loom replaces std's atomic primitives with a deterministic scheduler
//! that exhaustively explores thread interleavings. These tests prove:
//! - At most one acquirer wins a concurrent acquire on the same execution.
//! - After the holder releases, a fresh acquirer reliably wins (handoff without contention).
//! - A stale renew on a released-and-reacquired row is always rejected (the holder fence holds
//!   across handoff).
//!
//! The companion runtime verification lives in
//! `crates/engine/tests/lease_takeover.rs` (in-memory) +
//! `crates/storage/tests/execution_lease_pg_integration.rs` (real PG).

#![cfg(loom)]

use loom::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    thread,
};
use nebula_storage_loom_probe::lease_handoff::{AcquireOutcome, HolderOutcome, LeaseRepo};

/// Two replicas race to acquire the same execution's lease — exactly
/// one must win regardless of scheduling. Mirrors the existing
/// `at_most_one_concurrent_try_claim_acquires` test for refresh-claim
/// (ADR-0041 §10) but specialised to the execution-lease shape.
#[test]
fn at_most_one_concurrent_acquire_wins() {
    loom::model(|| {
        let repo = Arc::new(LeaseRepo::default());
        let acquired = Arc::new(AtomicU32::new(0));

        let h1 = thread::spawn({
            let repo = Arc::clone(&repo);
            let acquired = Arc::clone(&acquired);
            move || {
                if repo.acquire_lease(42, 1) == AcquireOutcome::Acquired {
                    acquired.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

        let h2 = thread::spawn({
            let repo = Arc::clone(&repo);
            let acquired = Arc::clone(&acquired);
            move || {
                if repo.acquire_lease(42, 2) == AcquireOutcome::Acquired {
                    acquired.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

        h1.join().unwrap();
        h2.join().unwrap();

        assert_eq!(
            acquired.load(Ordering::Relaxed),
            1,
            "lease CAS atomicity violated: both replicas observed Acquired"
        );
    });
}

/// Holder releases, then a fresh runner acquires concurrently with a
/// stale renew from the original holder. The fresh acquire must
/// succeed and the stale renew must be rejected — under any scheduling.
#[test]
fn stale_renew_after_release_is_always_rejected() {
    loom::model(|| {
        let repo = Arc::new(LeaseRepo::default());

        // Pre-state: runner 1 holds the lease.
        assert_eq!(repo.acquire_lease(42, 1), AcquireOutcome::Acquired);
        assert_eq!(repo.release_lease(42, 1), HolderOutcome::Applied);

        let renew_outcome = Arc::new(loom::sync::Mutex::new(HolderOutcome::Applied));
        let acquire_outcome = Arc::new(loom::sync::Mutex::new(AcquireOutcome::Contended));

        // Runner 1 (stale): tries to renew after its release.
        let h_renew = thread::spawn({
            let repo = Arc::clone(&repo);
            let out = Arc::clone(&renew_outcome);
            move || {
                let r = repo.renew_lease(42, 1);
                *out.lock().unwrap() = r;
            }
        });

        // Runner 2: tries to acquire the released lease.
        let h_acquire = thread::spawn({
            let repo = Arc::clone(&repo);
            let out = Arc::clone(&acquire_outcome);
            move || {
                let r = repo.acquire_lease(42, 2);
                *out.lock().unwrap() = r;
            }
        });

        h_renew.join().unwrap();
        h_acquire.join().unwrap();

        // Stale renew on a released row must always be rejected.
        assert_eq!(
            *renew_outcome.lock().unwrap(),
            HolderOutcome::Rejected,
            "stale renew after release must always be rejected"
        );
        // The fresh acquire must always succeed (no concurrent holder).
        assert_eq!(
            *acquire_outcome.lock().unwrap(),
            AcquireOutcome::Acquired,
            "post-release acquire must always succeed"
        );
        // Final holder is runner 2.
        assert_eq!(
            repo.snapshot(42).map(|(h, _)| h),
            Some(2),
            "post-handoff holder must be runner 2"
        );
    });
}

/// After TTL expires (modelled by flipping the `expired` flag), a fresh
/// runner can take over via `acquire_lease`, and the original holder's
/// stale `release_lease` is a no-op against the new holder.
#[test]
fn takeover_after_expiry_then_stale_release_is_noop() {
    loom::model(|| {
        let repo = Arc::new(LeaseRepo::default());

        // Pre-state: runner 1 holds, then its TTL "elapses".
        assert_eq!(repo.acquire_lease(42, 1), AcquireOutcome::Acquired);
        assert!(repo.flag_expired(42), "must mark row expired");

        // Runner 2 takes over.
        assert_eq!(repo.acquire_lease(42, 2), AcquireOutcome::Acquired);

        // Concurrent: runner 1 (stale) issues a release; runner 2 issues
        // a renew. The race must leave runner 2 as the holder.
        let h_stale_release = thread::spawn({
            let repo = Arc::clone(&repo);
            move || repo.release_lease(42, 1)
        });
        let h_renew = thread::spawn({
            let repo = Arc::clone(&repo);
            move || repo.renew_lease(42, 2)
        });

        let stale_release_outcome = h_stale_release.join().unwrap();
        let renew_outcome = h_renew.join().unwrap();

        assert_eq!(
            stale_release_outcome,
            HolderOutcome::Rejected,
            "post-takeover, runner 1's release must be rejected"
        );
        assert_eq!(
            renew_outcome,
            HolderOutcome::Applied,
            "runner 2's renew on its own lease must succeed"
        );
        assert_eq!(
            repo.snapshot(42).map(|(h, _)| h),
            Some(2),
            "runner 2 remains the holder after the race"
        );
    });
}
