//! Loom CAS atomicity probe for the credential refresh-claim pattern.
//!
//! Per ADR-0041 + sub-spec §10 DoD requirement.
//!
//! # Why a standalone crate?
//!
//! Setting `RUSTFLAGS="--cfg loom"` propagates to **every crate** in the
//! build, including transitive deps like `concurrent-queue` (pulled in
//! via `moka` → `async-lock` → `event-listener` from `nebula-storage`).
//! Those crates have `cfg(loom)` blocks that try to import `loom::sync`
//! from their own dep graph — they never declared `loom`, so the build
//! fails.
//!
//! This probe lives in a sibling crate that depends only on `loom`, so
//! `--cfg loom` only activates blocks in code that has `loom` in scope.
//!
//! The probe re-creates the CAS shape used in
//! `nebula_storage::credential::InMemoryRefreshClaimRepo::try_claim`:
//! a single mutex over a `HashMap<id, ClaimRow>`, where a `try_claim`
//! call locks, checks the existing row's expiry, and either returns
//! `Contended` or writes a new row. The probe's mirror exercises the
//! same shape with `loom::sync::Mutex` so loom can explore the lock
//! acquisition itself.
//!
//! Run with:
//!   RUSTFLAGS="--cfg loom" cargo nextest run \
//!     -p nebula-storage-loom-probe --features loom-test \
//!     --profile ci --no-tests=pass

#![cfg(loom)]

pub mod lease_handoff;

use std::collections::HashMap;

use loom::sync::Mutex;

/// Minimal CAS row, matching `InMemoryRefreshClaimRepo::ClaimRow`'s
/// invariant: at most one valid claim per `credential_id` at any time.
/// Loom doesn't observe time, so we model "expired" as an explicit flag.
#[derive(Clone, Debug)]
pub struct ClaimRow {
    /// Replica that holds the claim.
    pub holder: u32,
    /// Generation counter, bumped on overwrite.
    pub generation: u64,
    /// Whether the claim has been logically expired (modelling time).
    pub expired: bool,
}

/// In-memory CAS repo mirroring `InMemoryRefreshClaimRepo`.
#[derive(Default)]
pub struct Repo {
    rows: Mutex<HashMap<u32, ClaimRow>>,
}

/// `try_claim` outcome — mirrors `nebula_storage::credential::ClaimAttempt`.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Claim acquired; caller is now the holder.
    Acquired,
    /// Another holder already has a non-expired claim.
    Contended,
}

impl Repo {
    /// CAS-acquire a claim for `cid` on behalf of `holder`. Returns
    /// `Acquired` only if the existing row is absent or expired; otherwise
    /// `Contended`. Mirrors `InMemoryRefreshClaimRepo::try_claim`.
    pub fn try_claim(&self, cid: u32, holder: u32) -> Outcome {
        let mut guard = self.rows.lock().unwrap();
        if let Some(existing) = guard.get(&cid)
            && !existing.expired
        {
            return Outcome::Contended;
        }
        let generation = guard.get(&cid).map_or(0, |r| r.generation + 1);
        guard.insert(
            cid,
            ClaimRow {
                holder,
                generation,
                expired: false,
            },
        );
        Outcome::Acquired
    }
}
