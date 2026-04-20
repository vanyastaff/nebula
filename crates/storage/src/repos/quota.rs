//! Quota enforcement repository (atomic CAS counters).

use std::future::Future;

use crate::{
    error::StorageError,
    rows::{OrgQuotaRow, OrgQuotaUsageRow, WorkspaceQuotaUsageRow},
};

/// Quota limits and usage counters.
///
/// Spec 16 layer 7 + spec 10. Increment/decrement operations are
/// atomic against the DB — callers must check limits before attempting
/// to raise a counter.
pub trait QuotaRepo: Send + Sync {
    // ── Limits ──────────────────────────────────────────────────────────

    /// Fetch org quota limits (plan-based).
    fn get_org_limits(
        &self,
        org_id: &[u8],
    ) -> impl Future<Output = Result<Option<OrgQuotaRow>, StorageError>> + Send;

    /// Upsert org quota limits (typically on plan change).
    fn upsert_org_limits(
        &self,
        row: &OrgQuotaRow,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    // ── Usage counters ──────────────────────────────────────────────────

    /// Fetch current org usage.
    fn get_org_usage(
        &self,
        org_id: &[u8],
    ) -> impl Future<Output = Result<Option<OrgQuotaUsageRow>, StorageError>> + Send;

    /// Fetch current workspace usage.
    fn get_workspace_usage(
        &self,
        workspace_id: &[u8],
    ) -> impl Future<Output = Result<Option<WorkspaceQuotaUsageRow>, StorageError>> + Send;

    /// Atomically increment a usage counter. Fails with
    /// [`StorageError::Conflict`] if doing so would exceed the limit
    /// when `check_limit = true`.
    fn increment(
        &self,
        key: QuotaCounter<'_>,
        by: i64,
        check_limit: bool,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Atomically decrement a usage counter (never below zero).
    fn decrement(
        &self,
        key: QuotaCounter<'_>,
        by: i64,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Reset monthly counters (called by scheduled job at month rollover).
    fn reset_monthly(&self) -> impl Future<Output = Result<(), StorageError>> + Send;
}

/// Identifies a specific counter to adjust.
#[derive(Debug, Clone, Copy)]
pub enum QuotaCounter<'a> {
    /// Per-org counter.
    Org {
        /// Org identifier.
        org_id: &'a [u8],
        /// Column name (e.g. `"concurrent_executions"`).
        field: &'static str,
    },
    /// Per-workspace counter.
    Workspace {
        /// Workspace identifier.
        workspace_id: &'a [u8],
        /// Column name.
        field: &'static str,
    },
}
