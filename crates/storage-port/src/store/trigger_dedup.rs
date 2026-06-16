//! Trigger-dedup inbox port.
//!
//! The atomic compose method (`claim_and_materialize_start`) inserts the
//! dedup guard row, the `Start` job-dispatch row, and the `Created` execution
//! row in **one transaction** — dedup-guard ∧ Start-job ∧ execution-row are
//! inseparable.  On `Duplicate`, the winner's execution id is read back
//! in-transaction so the caller receives the canonical id without a second
//! round-trip.
use std::time::Duration;

use crate::Scope;
use crate::dto::{DispatchOutcome, JobDispatchMsg, NewExecution, TriggerDedupRow};
use crate::error::StorageError;

/// Trigger-dedup inbox: first-writer-wins guard for trigger fan-out.
///
/// The `PRIMARY KEY(workspace_id, org_id, trigger_id, event_id)` constraint
/// is the CAS.  A second delivery of the same event **within the same tenant
/// scope** finds the row present and returns a `DispatchOutcome` with
/// `kind == Duplicate` and the original winner's execution id — no new rows
/// are written.  Two distinct tenants sharing the same `(trigger_id, event_id)`
/// pair are NOT deduplicated — the scope columns ensure cross-tenant isolation.
///
/// Both methods are object-safe (concrete params only, no generics on methods).
#[async_trait::async_trait]
pub trait TriggerDedupInbox: Send + Sync + std::fmt::Debug {
    /// Atomically insert three rows in **one transaction** and return the
    /// effective execution id.
    ///
    /// **Compose ordering (all backends):**
    ///
    /// 1. If `row` is `Some`, attempt the dedup `INSERT … ON CONFLICT DO NOTHING`.
    ///    - `affected == 0` (row already present): read back the winner's
    ///      `execution_id` from the dedup table in the same transaction, then
    ///      return `DispatchOutcome::new(winner_id, Duplicate)` — no further
    ///      writes.
    ///    - `affected == 1` (first writer): continue to steps 2–3.
    /// 2. INSERT the execution row (`port_executions`, status='Created',
    ///    version=0, fencing_generation=0).  The execution id and scope come
    ///    from `start.execution_id` and `start.scope`.  If this INSERT fails
    ///    (e.g. id collision), the whole transaction rolls back — no dedup row,
    ///    no job row.
    /// 3. INSERT the `Start` job-dispatch row into `port_job_dispatch_queue`.
    /// 4. Commit and return `DispatchOutcome::new(start.execution_id, Dispatched)`.
    ///
    /// `row = None` skips the dedup guard and always performs steps 2–4.
    ///
    /// This method **owns** all three writes and **must not** call
    /// [`crate::store::JobDispatchQueue::enqueue`] or
    /// [`crate::store::ExecutionStore::create`] — doing so would require
    /// separate connections and break atomicity.
    async fn claim_and_materialize_start(
        &self,
        row: Option<&TriggerDedupRow>,
        start: &JobDispatchMsg,
        execution: &NewExecution<'_>,
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
