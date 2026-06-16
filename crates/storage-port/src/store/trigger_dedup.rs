//! Trigger-dedup inbox port.
//!
//! The atomic compose method (`claim_and_enqueue_start`) inserts the dedup
//! guard row and the `Start` job-dispatch row in a single transaction, so
//! the first-writer-wins invariant and the job enqueue are inseparable.
use std::time::Duration;

use crate::Scope;
use crate::dto::{DispatchOutcome, JobDispatchMsg, TriggerDedupRow};
use crate::error::StorageError;

/// Trigger-dedup inbox: first-writer-wins guard for trigger fan-out.
///
/// The `UNIQUE(trigger_id, event_id)` constraint is the CAS.  A second
/// delivery of the same event finds the row present and returns
/// `DispatchOutcome::Duplicate` without enqueuing a second job.
///
/// Both methods are object-safe (concrete params only, no generics).
#[async_trait::async_trait]
pub trait TriggerDedupInbox: Send + Sync + std::fmt::Debug {
    /// Atomically insert the dedup guard row (when `row` is `Some`) **and**
    /// the `Start` job-dispatch row in **one transaction**.
    ///
    /// - `row = None` — unconditional dispatch: insert `start` into the job
    ///   queue with no dedup row.  Always returns `Dispatched`.
    /// - `row = Some(r)` — guarded dispatch: attempt
    ///   `INSERT INTO port_trigger_dedup_inbox … ON CONFLICT DO NOTHING`.
    ///   If the row was inserted (affected == 1) then also insert `start` and
    ///   return `Dispatched`.  If the row was already present (affected == 0)
    ///   skip the job insert and return `Duplicate`.
    ///
    /// This method **owns** both writes and **must not** call
    /// [`crate::store::JobDispatchQueue::enqueue`] — doing so would require a
    /// second connection and break atomicity.
    async fn claim_and_enqueue_start(
        &self,
        row: Option<&TriggerDedupRow>,
        start: &JobDispatchMsg,
    ) -> Result<DispatchOutcome, StorageError>;

    /// Returns `true` when a dedup row with the given
    /// `(scope, trigger_id, event_id)` already exists.
    async fn exists(
        &self,
        scope: &Scope,
        trigger_id: &str,
        event_id: &str,
    ) -> Result<bool, StorageError>;

    /// Delete dedup rows older than `retention`; returns the count deleted.
    ///
    /// Stub — no-op now (TTL sweep wired later without a trait break).
    async fn cleanup(&self, retention: Duration) -> Result<u64, StorageError>;
}
