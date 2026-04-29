//! Loom CAS atomicity probe for the execution-lease handoff pattern.
//!
//! Per ROADMAP §M2.2 / T8 — sibling of the existing refresh-claim probe.
//!
//! Mirrors the CAS shape used in
//! `nebula_storage::InMemoryExecutionRepo::{acquire_lease, renew_lease, release_lease}`:
//! a single mutex over a `HashMap<execution_id, LeaseRow>` where each
//! lease operation locks, inspects the row, and either returns an outcome
//! or writes a new row. Loom replaces the mutex with its deterministic
//! scheduler so it can exhaustively explore thread interleavings.
//!
//! Loom does not observe time, so the lease's wall-clock TTL is modelled
//! as an explicit `expired: bool` flag on the row. Tests can flip the
//! flag between phases when they want to model "the TTL elapsed".
//!
//! Run with:
//!   RUSTFLAGS="--cfg loom" cargo nextest run \
//!     -p nebula-storage-loom-probe --features loom-test \
//!     --profile ci --no-tests=pass

#![cfg(loom)]

use std::collections::HashMap;

use loom::sync::Mutex;

/// Minimal lease row mirroring the InMemoryExecutionRepo lease state.
/// Loom doesn't observe time, so `expired` is an explicit flag instead
/// of a deadline.
#[derive(Clone, Debug)]
pub struct LeaseRow {
    /// Runner that currently holds the lease.
    pub holder: u32,
    /// Generation counter, bumped on each acquire that overwrites a
    /// prior (expired or absent) lease. Lets tests prove handoff
    /// happened by observing a strictly-increasing value.
    pub generation: u64,
    /// Whether the lease has been logically expired (modelling TTL).
    pub expired: bool,
}

/// Shared lease repo. Single-mutex CAS shape matches
/// `InMemoryExecutionRepo`'s `leases` map (`execution_repo.rs:594-641`).
#[derive(Default)]
pub struct LeaseRepo {
    rows: Mutex<HashMap<u32, LeaseRow>>,
}

/// Outcome of `acquire_lease`. Mirrors `Result<bool, _>` from the
/// production repo: `Acquired` ↔ `Ok(true)`, `Contended` ↔ `Ok(false)`.
#[derive(Debug, PartialEq, Eq)]
pub enum AcquireOutcome {
    /// Acquired by the calling holder.
    Acquired,
    /// Another holder owns a non-expired lease.
    Contended,
}

/// Outcome of `renew_lease` / `release_lease`. Mirrors `Ok(true)` /
/// `Ok(false)` from the production repo.
#[derive(Debug, PartialEq, Eq)]
pub enum HolderOutcome {
    /// The caller's holder string matched and the operation applied.
    Applied,
    /// Holder mismatch (or expired-and-released-row); operation rejected.
    Rejected,
}

impl LeaseRepo {
    /// CAS-acquire a lease on `exec_id` for `holder`. Returns
    /// `Acquired` only if the existing row is absent OR has been
    /// flagged expired. Mirrors
    /// `InMemoryExecutionRepo::acquire_lease`.
    pub fn acquire_lease(&self, exec_id: u32, holder: u32) -> AcquireOutcome {
        let mut guard = self.rows.lock().unwrap();
        if let Some(existing) = guard.get(&exec_id)
            && !existing.expired
        {
            return AcquireOutcome::Contended;
        }
        let generation = guard.get(&exec_id).map_or(0, |r| r.generation + 1);
        guard.insert(
            exec_id,
            LeaseRow {
                holder,
                generation,
                expired: false,
            },
        );
        AcquireOutcome::Acquired
    }

    /// Renew an existing lease — succeeds only if the calling holder
    /// matches the row's current holder. Loom doesn't model time, so
    /// "extend expiry" is represented by clearing the `expired` flag.
    pub fn renew_lease(&self, exec_id: u32, holder: u32) -> HolderOutcome {
        let mut guard = self.rows.lock().unwrap();
        match guard.get_mut(&exec_id) {
            Some(row) if row.holder == holder => {
                row.expired = false;
                HolderOutcome::Applied
            },
            _ => HolderOutcome::Rejected,
        }
    }

    /// Release a lease — succeeds only if the calling holder matches.
    /// Removes the row on success (mirrors
    /// `InMemoryExecutionRepo::release_lease`'s `leases.remove`).
    pub fn release_lease(&self, exec_id: u32, holder: u32) -> HolderOutcome {
        let mut guard = self.rows.lock().unwrap();
        match guard.get(&exec_id) {
            Some(row) if row.holder == holder => {
                guard.remove(&exec_id);
                HolderOutcome::Applied
            },
            _ => HolderOutcome::Rejected,
        }
    }

    /// Test-only helper: flip the row's `expired` flag to model TTL
    /// elapse. Returns `false` if no row exists.
    pub fn flag_expired(&self, exec_id: u32) -> bool {
        let mut guard = self.rows.lock().unwrap();
        if let Some(row) = guard.get_mut(&exec_id) {
            row.expired = true;
            true
        } else {
            false
        }
    }

    /// Read the current holder + generation, if any. Test-only inspector.
    pub fn snapshot(&self, exec_id: u32) -> Option<(u32, u64)> {
        let guard = self.rows.lock().unwrap();
        guard.get(&exec_id).map(|r| (r.holder, r.generation))
    }
}
