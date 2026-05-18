//! Loom tests asserting CAS atomicity for the execution-lease handoff
//! pattern.
//!
//! Loom replaces std's atomic primitives with a deterministic scheduler
//! that exhaustively explores thread interleavings. These tests prove:
//! - At most one acquirer wins a concurrent acquire on the same execution.
//! - After a takeover (a competing acquire bumps the fencing generation),
//!   the superseded holder's renew/release is always rejected — the
//!   fencing token closes the zombie-runner hole across handoff.
//! - Expiry-takeover followed by a stale release from the old holder is a
//!   no-op against the new holder.
//!
//! ## Invariant-equivalence note (port migration)
//!
//! These tests originally modelled the legacy `InMemoryExecutionRepo`,
//! whose `release_lease` deleted the row, making "stale renew after
//! release" unconditionally fail. The spec-16 `InMemoryExecutionStore`
//! instead keeps the row and only nulls the holder/expiry on release, and
//! fences renew/release on the monotone **fencing generation**, not on
//! the holder string. The faithful invariant is therefore "a superseded
//! generation token is rejected once a competing acquire bumps the
//! generation" (the real zombie-runner closure) — these tests assert
//! exactly that against the re-pointed probe. No safety coverage is lost:
//! the property proven is the production fencing guarantee, not an
//! artifact of the old row-deleting model.
//!
//! The companion runtime verification lives in
//! `crates/engine/tests/lease_takeover.rs` (in-memory port path) +
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
/// one must win regardless of scheduling.
#[test]
fn at_most_one_concurrent_acquire_wins() {
    loom::model(|| {
        let repo = Arc::new(LeaseRepo::default());
        let acquired = Arc::new(AtomicU32::new(0));

        let h1 = thread::spawn({
            let repo = Arc::clone(&repo);
            let acquired = Arc::clone(&acquired);
            move || {
                if matches!(repo.acquire_lease(42, 1), AcquireOutcome::Acquired(_)) {
                    acquired.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

        let h2 = thread::spawn({
            let repo = Arc::clone(&repo);
            let acquired = Arc::clone(&acquired);
            move || {
                if matches!(repo.acquire_lease(42, 2), AcquireOutcome::Acquired(_)) {
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

/// Holder releases (row retained, generation unchanged), then a fresh
/// runner acquires concurrently with a stale renew from the original
/// holder's token. Whichever order the race resolves, the post-handoff
/// holder is runner 2 and the old token can never renew once runner 2's
/// acquire has bumped the generation.
#[test]
fn stale_renew_after_release_is_fenced_by_takeover() {
    loom::model(|| {
        let repo = Arc::new(LeaseRepo::default());

        // Pre-state: runner 1 holds the lease, then releases it. The row
        // (and its generation) survives the release on the new adapter.
        let AcquireOutcome::Acquired(token1) = repo.acquire_lease(42, 1) else {
            unreachable!("first acquire on an empty row must succeed");
        };
        assert_eq!(repo.release_lease(42, token1), HolderOutcome::Applied);

        let acquire_outcome = Arc::new(loom::sync::Mutex::new(AcquireOutcome::Contended));

        // Runner 1 (stale): tries to renew with its now-old token.
        let h_renew = thread::spawn({
            let repo = Arc::clone(&repo);
            move || repo.renew_lease(42, token1)
        });

        // Runner 2: acquires the released lease (bumps the generation).
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

        // The fresh acquire must always succeed (the row was released, so
        // it is not live regardless of interleaving).
        let token2 = match &*acquire_outcome.lock().unwrap() {
            AcquireOutcome::Acquired(t) => *t,
            other => panic!("post-release acquire must always succeed, got {other:?}"),
        };
        assert!(
            token2 > token1,
            "every acquire must bump the fencing generation (token2={token2}, token1={token1})"
        );
        // Final holder is runner 2.
        assert_eq!(
            repo.snapshot(42).map(|(h, _)| h),
            Some(2),
            "post-handoff holder must be runner 2"
        );
        // Once runner 2 has acquired, the old token is permanently fenced
        // — this is the zombie-runner closure the fencing token provides.
        assert_eq!(
            repo.renew_lease(42, token1),
            HolderOutcome::Rejected,
            "a superseded token must never renew after the generation bumped"
        );
    });
}

/// After TTL expires (modelled by flipping `expired`), a fresh runner
/// takes over via `acquire_lease`, bumping the generation. The original
/// holder's stale `release_lease` (old token) must be a no-op and the new
/// holder's `renew_lease` (new token) must apply — under any scheduling.
#[test]
fn takeover_after_expiry_then_stale_release_is_noop() {
    loom::model(|| {
        let repo = Arc::new(LeaseRepo::default());

        // Pre-state: runner 1 holds, then its TTL "elapses".
        let AcquireOutcome::Acquired(token1) = repo.acquire_lease(42, 1) else {
            unreachable!("first acquire must succeed");
        };
        assert!(repo.flag_expired(42), "must mark row expired");

        // Runner 2 takes over (bumps the generation past token1).
        let AcquireOutcome::Acquired(token2) = repo.acquire_lease(42, 2) else {
            unreachable!("acquire over an expired lease must succeed");
        };
        assert!(token2 > token1, "takeover must bump the fencing generation");

        // Concurrent: runner 1 (stale, old token) issues a release;
        // runner 2 (current token) issues a renew. The race must leave
        // runner 2 as the holder with its lease live.
        let h_stale_release = thread::spawn({
            let repo = Arc::clone(&repo);
            move || repo.release_lease(42, token1)
        });
        let h_renew = thread::spawn({
            let repo = Arc::clone(&repo);
            move || repo.renew_lease(42, token2)
        });

        let stale_release_outcome = h_stale_release.join().unwrap();
        let renew_outcome = h_renew.join().unwrap();

        assert_eq!(
            stale_release_outcome,
            HolderOutcome::Rejected,
            "post-takeover, runner 1's old-token release must be rejected"
        );
        assert_eq!(
            renew_outcome,
            HolderOutcome::Applied,
            "runner 2's renew on its current token must succeed"
        );
        assert_eq!(
            repo.snapshot(42).map(|(h, _)| h),
            Some(2),
            "runner 2 remains the holder after the race"
        );
    });
}
