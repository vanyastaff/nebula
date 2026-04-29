//! Execution repository — the authoritative source of execution state.
//!
//! Spec 16 layer 4. All state transitions go through [`ExecutionRepo::transition`]
//! with CAS on `version`. Leases provide multi-process coordination.
//!
//! # Status (1.0): Sprint E scaffolding — do not implement against pre-Sprint-E
//!
//! This trait is **planned / experimental** per the layered architecture
//! documented in `crates/storage/src/lib.rs` (see "Layer 2 — `repos` module
//! — planned / experimental, canon §11.6"). It is part of the spec-16
//! row-model design, deferred to **Sprint E (1.1)** per the workspace
//! ROADMAP "Out of scope for 1.0" entry ("Storage Layer 2 / spec-16
//! multi-tenant row model").
//!
//! Production execution persistence today goes through the top-level
//! `nebula_storage::ExecutionRepo` (defined in
//! `crates/storage/src/execution_repo.rs`) — that is the trait the engine
//! actually consumes, with its own `acquire_lease` / `renew_lease` /
//! `release_lease` methods backed by the `lease_holder` / `lease_expires_at`
//! columns from common migration `00000000000007_add_execution_leases.sql`.
//! Do not implement against the trait below pre-Sprint E: adding callers
//! requires the spec-16 row-model engine refactor and is explicitly
//! deferred.

use std::{future::Future, time::Duration};

use crate::{error::StorageError, rows::ExecutionRow};

/// Execution storage with versioned CAS and leases.
pub trait ExecutionRepo: Send + Sync {
    /// Insert a new execution.
    fn create(&self, exec: &ExecutionRow) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Fetch an execution by ID.
    fn get(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<Option<ExecutionRow>, StorageError>> + Send;

    /// Apply a state transition with CAS on `version`.
    ///
    /// Returns `Ok(updated_row)` on success, [`StorageError::Conflict`]
    /// on version mismatch, and [`StorageError::NotFound`] if the
    /// execution does not exist.
    fn transition(
        &self,
        id: &[u8],
        expected_version: i64,
        new_status: &str,
        new_state_patch: Option<serde_json::Value>,
    ) -> impl Future<Output = Result<ExecutionRow, StorageError>> + Send;

    /// Update the `output` column (typically on terminal transition).
    fn set_output(
        &self,
        id: &[u8],
        expected_version: i64,
        output: serde_json::Value,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Mark an execution as cancel-requested. Writes `cancel_requested_*`
    /// fields atomically; callers should enqueue a `Cancel` command in
    /// the control queue in the same transaction.
    fn request_cancel(
        &self,
        id: &[u8],
        requested_by: &[u8],
        reason: Option<&str>,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    // ── Lease / claim (multi-process coordination) ──────────────────────

    /// Attempt to acquire a lease on an execution.
    ///
    /// Returns `true` when acquired, `false` when another holder has
    /// a non-expired lease.
    fn acquire_lease(
        &self,
        id: &[u8],
        holder: &[u8],
        ttl: Duration,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    /// Renew an existing lease. Returns `false` when not held by `holder`
    /// or when the lease has expired.
    fn renew_lease(
        &self,
        id: &[u8],
        holder: &[u8],
        ttl: Duration,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    /// Release a lease. Returns `false` when not held by `holder`.
    fn release_lease(
        &self,
        id: &[u8],
        holder: &[u8],
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    // ── Dispatcher queries ──────────────────────────────────────────────

    /// Claim up to `batch_size` pending/queued executions for a holder
    /// with the given TTL. Uses `FOR UPDATE SKIP LOCKED` on Postgres.
    fn claim_pending(
        &self,
        holder: &[u8],
        ttl: Duration,
        batch_size: u32,
    ) -> impl Future<Output = Result<Vec<ExecutionRow>, StorageError>> + Send;

    /// List executions with stale leases (eligible for takeover).
    fn list_stale_leases(
        &self,
        batch_size: u32,
    ) -> impl Future<Output = Result<Vec<ExecutionRow>, StorageError>> + Send;

    /// List executions that have hit their timeout.
    fn list_timed_out(
        &self,
        batch_size: u32,
    ) -> impl Future<Output = Result<Vec<ExecutionRow>, StorageError>> + Send;

    // ── Listing / counts ────────────────────────────────────────────────

    /// List executions in a workspace, ordered by `created_at DESC`.
    fn list_in_workspace(
        &self,
        workspace_id: &[u8],
        offset: u64,
        limit: u64,
    ) -> impl Future<Output = Result<Vec<ExecutionRow>, StorageError>> + Send;

    /// Count running executions in an org (for quota enforcement).
    fn count_running_in_org(
        &self,
        org_id: &[u8],
    ) -> impl Future<Output = Result<u64, StorageError>> + Send;
}
