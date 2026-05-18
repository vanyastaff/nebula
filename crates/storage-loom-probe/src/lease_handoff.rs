//! Loom CAS atomicity probe for the execution-lease handoff pattern.
//!
//! Sibling of the refresh-claim probe; covers the lease half of the
//! spec-16 execution-store atomic boundary.
//!
//! Mirrors the CAS shape used by the in-memory execution adapter's lease
//! operations (`acquire_lease` / `renew_lease` / `release_lease` on the
//! `InMemoryExecutionStore` in `nebula-storage`'s `inmem::execution`): a
//! single mutex over a `HashMap<execution_id, Row>` where each lease
//! operation locks, inspects the row, and either returns an outcome or
//! writes a new row. Loom replaces the mutex with its deterministic
//! scheduler so it can exhaustively explore thread interleavings.
//!
//! ## Invariant-equivalence note (port migration)
//!
//! This probe originally mirrored the legacy `InMemoryExecutionRepo`
//! lease map. The spec-16 `InMemoryExecutionStore` lease ops are
//! CAS-shape-identical: one `parking_lot::Mutex` guarding a row map;
//! "a live lease blocks acquire, every successful acquire bumps the
//! fencing generation, holder/generation fence on renew/release". The
//! probe's `generation` is exactly the new store's `fencing_generation`;
//! `AcquireOutcome::Contended` ↔ `Ok(None)` from
//! `ExecutionStore::acquire_lease`; `HolderOutcome` ↔ the `bool` returned
//! by `renew_lease`/`release_lease`. Every interleaving this probe proves
//! safe therefore holds for the new adapter unchanged — the probe is
//! re-pointed, not weakened, and no safety coverage is lost.
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

/// Minimal lease row mirroring the in-memory execution store's lease
/// state (holder + fencing generation + liveness). Loom doesn't observe
/// time, so `expired` is an explicit flag instead of a deadline.
#[derive(Clone, Debug)]
pub struct LeaseRow {
    /// Runner that last held the lease (meaningful only while `held`).
    pub holder: u32,
    /// Monotone fencing generation. Bumped on **every** successful
    /// acquire and never reset: `release_lease` clears `held` but keeps
    /// the row and its generation (the new adapter does not delete the
    /// row on release), so a subsequent acquire bumps from the current
    /// value. The counter is therefore per-row monotone across the
    /// release→re-acquire and expiry-takeover paths alike.
    pub generation: u64,
    /// Whether the lease has been logically expired (modelling TTL).
    pub expired: bool,
    /// Whether a holder currently owns the lease. `release_lease` clears
    /// this without deleting the row (mirrors the new adapter clearing
    /// `lease_holder`/`lease_expires_at` while retaining the row).
    pub held: bool,
}

/// Shared lease repo. Single-mutex CAS shape matches the in-memory
/// execution store's lease bookkeeping on its row map.
#[derive(Default)]
pub struct LeaseRepo {
    rows: Mutex<HashMap<u32, LeaseRow>>,
}

/// Outcome of `acquire_lease`. Mirrors the production
/// `ExecutionStore::acquire_lease` return: `Acquired(token)` ↔
/// `Ok(Some(FencingToken))`, `Contended` ↔ `Ok(None)`.
#[derive(Debug, PartialEq, Eq)]
pub enum AcquireOutcome {
    /// Acquired by the calling holder; carries the fresh fencing
    /// generation (the token the holder threads into renew/release).
    Acquired(u64),
    /// Another holder owns a live (non-expired, still-held) lease.
    Contended,
}

/// Outcome of `renew_lease` / `release_lease`. Mirrors `Ok(true)` /
/// `Ok(false)` from the production `ExecutionStore`.
#[derive(Debug, PartialEq, Eq)]
pub enum HolderOutcome {
    /// The presented fencing token was current and the operation applied.
    Applied,
    /// Token superseded (or no such row); operation rejected.
    Rejected,
}

impl LeaseRepo {
    /// CAS-acquire a lease on `exec_id` for `holder`. Returns
    /// `Acquired` only if the existing row is absent OR not live (its
    /// `expired` flag is set, or it was released). Every successful
    /// acquire bumps the fencing generation, so a prior holder's token —
    /// including the same holder string after a crash — is dead. Mirrors
    /// `InMemoryExecutionStore::acquire_lease`.
    pub fn acquire_lease(&self, exec_id: u32, holder: u32) -> AcquireOutcome {
        let mut guard = self.rows.lock().unwrap();
        if let Some(existing) = guard.get(&exec_id)
            && existing.held
            && !existing.expired
        {
            return AcquireOutcome::Contended;
        }
        // Generation is monotone per row: a released/expired row keeps its
        // generation, and the next acquire bumps it (the new adapter does
        // not delete the row on release). Absent row starts at 1.
        let generation = guard.get(&exec_id).map_or(1, |r| r.generation + 1);
        guard.insert(
            exec_id,
            LeaseRow {
                holder,
                generation,
                expired: false,
                held: true,
            },
        );
        AcquireOutcome::Acquired(generation)
    }

    /// Renew an existing lease — succeeds only if `token` is the row's
    /// current fencing generation (a superseded token is rejected even
    /// if the same runner string holds it). Loom doesn't model time, so
    /// "extend expiry" is represented by clearing the `expired` flag.
    /// Mirrors `InMemoryExecutionStore::renew_lease` (generation-fenced,
    /// not holder-string-fenced).
    pub fn renew_lease(&self, exec_id: u32, token: u64) -> HolderOutcome {
        let mut guard = self.rows.lock().unwrap();
        match guard.get_mut(&exec_id) {
            Some(row) if row.generation == token => {
                row.expired = false;
                HolderOutcome::Applied
            },
            _ => HolderOutcome::Rejected,
        }
    }

    /// Release a lease — succeeds only if `token` is the row's current
    /// fencing generation. Clears `held` but **keeps** the row and its
    /// generation (mirrors `InMemoryExecutionStore::release_lease`
    /// nulling `lease_holder`/`lease_expires_at` while retaining the
    /// row); the next acquire still bumps the generation.
    pub fn release_lease(&self, exec_id: u32, token: u64) -> HolderOutcome {
        let mut guard = self.rows.lock().unwrap();
        match guard.get_mut(&exec_id) {
            Some(row) if row.generation == token => {
                row.held = false;
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

    /// Read the row's last holder + current fencing generation, if a row
    /// exists (it survives `release_lease`). Test-only inspector.
    pub fn snapshot(&self, exec_id: u32) -> Option<(u32, u64)> {
        let guard = self.rows.lock().unwrap();
        guard.get(&exec_id).map(|r| (r.holder, r.generation))
    }
}
