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
    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        available_plugins: &[PluginKey],
    ) -> Result<Vec<JobDispatchMsg>, StorageError>;

    /// Mark a claimed job dispatched (terminal success).  Only the runner
    /// whose id matches the row's recorded processor may transition it
    /// (stale-worker fence).
    async fn mark_dispatched(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
    ) -> Result<(), StorageError>;

    /// Mark a claimed job failed (records `error`).  Same processor fence as
    /// [`Self::mark_dispatched`].
    async fn mark_failed(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
        error: &str,
    ) -> Result<(), StorageError>;

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
