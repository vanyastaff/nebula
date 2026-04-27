//! Loom test asserting CAS atomicity under 2-thread interleaving.
//!
//! Per sub-spec §10 DoD requirement.
//!
//! Loom replaces std's atomic primitives with a deterministic scheduler
//! that exhaustively explores thread interleavings. Two concurrent
//! `try_claim` attempts must yield exactly one `Acquired` under any
//! scheduling.

#![cfg(loom)]

use loom::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    thread,
};
use nebula_storage_loom_probe::{Outcome, Repo};

#[test]
fn at_most_one_concurrent_try_claim_acquires() {
    loom::model(|| {
        let repo = Arc::new(Repo::default());
        let acquired = Arc::new(AtomicU32::new(0));

        let h1 = thread::spawn({
            let repo = Arc::clone(&repo);
            let acquired = Arc::clone(&acquired);
            move || {
                if repo.try_claim(42, 1) == Outcome::Acquired {
                    acquired.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

        let h2 = thread::spawn({
            let repo = Arc::clone(&repo);
            let acquired = Arc::clone(&acquired);
            move || {
                if repo.try_claim(42, 2) == Outcome::Acquired {
                    acquired.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

        h1.join().unwrap();
        h2.join().unwrap();

        // Exactly one acquirer wins under any interleaving.
        assert_eq!(
            acquired.load(Ordering::Relaxed),
            1,
            "CAS atomicity violated: both replicas observed Acquired"
        );
    });
}
