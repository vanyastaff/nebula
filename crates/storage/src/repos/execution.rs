//! Execution repository — the authoritative source of execution state.
//!
//! Spec 16 layer 4. All state transitions go through [`ExecutionRepo::transition`]
//! with CAS on `version`. Leases provide multi-process coordination.

use std::time::Duration;

use async_trait::async_trait;

use crate::{error::StorageError, rows::ExecutionRow};

/// Execution storage with versioned CAS and leases.
#[async_trait]
pub trait ExecutionRepo: Send + Sync {
    /// Insert a new execution.
    async fn create(&self, exec: &ExecutionRow) -> Result<(), StorageError>;

    /// Fetch an execution by ID.
    async fn get(&self, id: &[u8]) -> Result<Option<ExecutionRow>, StorageError>;

    /// Apply a state transition with CAS on `version`.
    ///
    /// Returns `Ok(updated_row)` on success, [`StorageError::Conflict`]
    /// on version mismatch, and [`StorageError::NotFound`] if the
    /// execution does not exist.
    async fn transition(
        &self,
        id: &[u8],
        expected_version: i64,
        new_status: &str,
        new_state_patch: Option<serde_json::Value>,
    ) -> Result<ExecutionRow, StorageError>;

    /// Update the `output` column (typically on terminal transition).
    async fn set_output(
        &self,
        id: &[u8],
        expected_version: i64,
        output: serde_json::Value,
    ) -> Result<(), StorageError>;

    /// Mark an execution as cancel-requested. Writes `cancel_requested_*`
    /// fields atomically; callers should enqueue a `Cancel` command in
    /// the control queue in the same transaction.
    async fn request_cancel(
        &self,
        id: &[u8],
        requested_by: &[u8],
        reason: Option<&str>,
    ) -> Result<(), StorageError>;

    // ── Lease / claim (multi-process coordination) ──────────────────────

    /// Attempt to acquire a lease on an execution.
    ///
    /// Returns `true` when acquired, `false` when another holder has
    /// a non-expired lease.
    async fn acquire_lease(
        &self,
        id: &[u8],
        holder: &[u8],
        ttl: Duration,
    ) -> Result<bool, StorageError>;

    /// Renew an existing lease. Returns `false` when not held by `holder`
    /// or when the lease has expired.
    async fn renew_lease(
        &self,
        id: &[u8],
        holder: &[u8],
        ttl: Duration,
    ) -> Result<bool, StorageError>;

    /// Release a lease. Returns `false` when not held by `holder`.
    async fn release_lease(&self, id: &[u8], holder: &[u8]) -> Result<bool, StorageError>;

    // ── Dispatcher queries ──────────────────────────────────────────────

    /// Claim up to `batch_size` pending/queued executions for a holder
    /// with the given TTL. Uses `FOR UPDATE SKIP LOCKED` on Postgres.
    async fn claim_pending(
        &self,
        holder: &[u8],
        ttl: Duration,
        batch_size: u32,
    ) -> Result<Vec<ExecutionRow>, StorageError>;

    /// List executions with stale leases (eligible for takeover).
    async fn list_stale_leases(&self, batch_size: u32) -> Result<Vec<ExecutionRow>, StorageError>;

    /// List executions that have hit their timeout.
    async fn list_timed_out(&self, batch_size: u32) -> Result<Vec<ExecutionRow>, StorageError>;

    // ── Listing / counts ────────────────────────────────────────────────

    /// List executions in a workspace, ordered by `created_at DESC`.
    async fn list_in_workspace(
        &self,
        workspace_id: &[u8],
        offset: u64,
        limit: u64,
    ) -> Result<Vec<ExecutionRow>, StorageError>;

    /// Count running executions in an org (for quota enforcement).
    async fn count_running_in_org(&self, org_id: &[u8]) -> Result<u64, StorageError>;
}
