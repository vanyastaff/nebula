//! Capability-routed job-dispatch queue port.
//!
//! The orchestrator pulls jobs by advertising the set of [`PluginKey`]s its
//! workers support; the queue delivers only rows whose `required_plugin_key`
//! is a member of that set.  The claim/fence/reclaim shape mirrors
//! `ControlQueue` — `ReclaimOutcome` is reused from that module.
//!
//! [`PluginKey`]: nebula_core::PluginKey
use std::time::Duration;

use nebula_core::PluginKey;

use crate::dto::JobDispatchMsg;
use crate::error::StorageError;
use crate::store::ReclaimOutcome;

/// Storage-minted generation of one claim attempt on a queue row.
///
/// Every successful claim mints a strictly greater generation for that row.
/// Reclaim clears ownership but never decrements or reuses a generation, so a
/// generation identifies **one claim attempt** for the lifetime of the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClaimGeneration(u64);

impl ClaimGeneration {
    /// Wrap a generation minted by a storage backend.
    #[must_use]
    pub const fn new(generation: u64) -> Self {
        Self(generation)
    }

    /// The raw generation, for persistence and diagnostics.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for ClaimGeneration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Proof that its holder owns the *current* claim on one queue row.
///
/// A processor identity cannot serve this purpose. A stable processor may
/// claim a row, lose it to a reclaim sweep, and claim it again — the classic
/// ABA. Fencing on `processed_by` alone accepts an acknowledgement issued
/// against the *first* claim after the *second* one began, terminalising work
/// that the current owner is still performing. The generation makes the two
/// attempts distinguishable, so the stale acknowledgement is rejected.
///
/// Only a storage backend mints these, and only from a row it just
/// transitioned to `Processing`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JobClaimToken {
    row_id: [u8; 16],
    generation: ClaimGeneration,
}

impl JobClaimToken {
    /// Mint a token for a row a backend just claimed.
    #[must_use]
    pub const fn new(row_id: [u8; 16], generation: ClaimGeneration) -> Self {
        Self { row_id, generation }
    }

    /// The row this claim is for.
    #[must_use]
    pub const fn row_id(&self) -> &[u8; 16] {
        &self.row_id
    }

    /// The claim attempt this token proves ownership of.
    #[must_use]
    pub const fn generation(&self) -> ClaimGeneration {
        self.generation
    }
}

/// One claimed job: the message to dispatch plus the token that acknowledges it.
///
/// The two travel together so a caller cannot acknowledge one row with another
/// row's authority — the mismatched-`(id, processor)` argument pair the token
/// replaces made that a plain call-site mistake.
#[derive(Clone, Debug)]
pub struct JobClaim {
    /// The claimed job-dispatch message.
    pub msg: JobDispatchMsg,
    /// Authority to acknowledge exactly this claim attempt.
    pub token: JobClaimToken,
}

/// Durable capability-routed job-dispatch queue.
///
/// The routing predicate is `required_plugins ⊆ available_plugins`: a worker
/// may claim a job only if its advertised plugin set is a superset of every
/// plugin the job requires.  The DTO invariant guarantees
/// `required_plugins ⊇ {required_plugin_key}`, so the superset predicate
/// strictly subsumes the single-key pre-filter —
/// `required_plugin_key ∈ available_plugins` is kept as a sound index
/// pre-filter.
///
/// Postgres uses `FOR UPDATE SKIP LOCKED` on `claim_pending`; SQLite uses a
/// single-consumer status flip.  Both are object-safe and Send+Sync.
#[async_trait::async_trait]
pub trait JobDispatchQueue: Send + Sync + std::fmt::Debug {
    /// Durably enqueue a job-dispatch message.
    async fn enqueue(&self, msg: &JobDispatchMsg) -> Result<(), StorageError>;

    /// Claim up to `batch_size` pending jobs whose `required_plugins ⊆
    /// available_plugins` (the worker's advertised plugin set must be a
    /// superset of every plugin the job requires).
    ///
    /// `required_plugin_key ∈ available_plugins` is retained as an
    /// index-friendly pre-filter (sound by the DTO invariant); the exact
    /// superset check is applied inside the same statement, eliminating any
    /// TOCTOU window.
    ///
    /// Claim mechanics per backend:
    /// - **InMemory**: predicate + status flip inside one `parking_lot` Mutex
    ///   critical section — single atomic step.
    /// - **Postgres**: candidate subquery with `FOR UPDATE SKIP LOCKED` feeds a
    ///   single `UPDATE … RETURNING` — the lock prevents concurrent double-claim.
    /// - **SQLite**: transactional `SELECT` (superset filter) + per-row
    ///   `UPDATE … AND status = 'Pending'` guard inside one transaction; a
    ///   concurrent actor that flips the row first causes `rows_affected = 0`
    ///   and the row is skipped — no double-dispatch (single-consumer boundary,
    ///   spec §5).
    /// `processor` is recorded for observability only; it carries no
    /// authority. The returned [`JobClaimToken`] is what acknowledges the row.
    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        available_plugins: &[PluginKey],
    ) -> Result<Vec<JobClaim>, StorageError>;

    /// Mark a claimed job dispatched (terminal success).
    ///
    /// Fenced on `(row id, status = Processing, claim generation)`. A token
    /// from a superseded claim changes nothing and returns
    /// [`StorageError::FencedOut`]; an unknown row returns
    /// [`StorageError::NotFound`].
    async fn mark_dispatched(&self, claim: &JobClaimToken) -> Result<(), StorageError>;

    /// Mark a claimed job failed (records `error`).  Same generation fence as
    /// [`Self::mark_dispatched`].
    async fn mark_failed(&self, claim: &JobClaimToken, error: &str) -> Result<(), StorageError>;

    /// Reclaim rows stuck in `Processing` past `reclaim_after`.  Rows under
    /// the `max_reclaim_count` budget go back to `Pending`; rows past it go
    /// to `Failed`.
    async fn reclaim_stuck(
        &self,
        reclaim_after: Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError>;

    /// Delete terminal rows older than `retention`; returns the count deleted.
    async fn cleanup(&self, retention: Duration) -> Result<u64, StorageError>;
}
