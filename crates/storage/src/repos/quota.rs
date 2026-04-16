//! Quota enforcement repository (atomic CAS counters).

use async_trait::async_trait;

use crate::{
    error::StorageError,
    rows::{OrgQuotaRow, OrgQuotaUsageRow, WorkspaceQuotaUsageRow},
};

/// Quota limits and usage counters.
///
/// Spec 16 layer 7 + spec 10. Increment/decrement operations are
/// atomic against the DB — callers must check limits before attempting
/// to raise a counter.
#[async_trait]
pub trait QuotaRepo: Send + Sync {
    // ── Limits ──────────────────────────────────────────────────────────

    /// Fetch org quota limits (plan-based).
    async fn get_org_limits(&self, org_id: &[u8]) -> Result<Option<OrgQuotaRow>, StorageError>;

    /// Upsert org quota limits (typically on plan change).
    async fn upsert_org_limits(&self, row: &OrgQuotaRow) -> Result<(), StorageError>;

    // ── Usage counters ──────────────────────────────────────────────────

    /// Fetch current org usage.
    async fn get_org_usage(&self, org_id: &[u8]) -> Result<Option<OrgQuotaUsageRow>, StorageError>;

    /// Fetch current workspace usage.
    async fn get_workspace_usage(
        &self,
        workspace_id: &[u8],
    ) -> Result<Option<WorkspaceQuotaUsageRow>, StorageError>;

    /// Atomically increment a usage counter. Fails with
    /// [`StorageError::Conflict`] if doing so would exceed the limit
    /// when `check_limit = true`.
    async fn increment(
        &self,
        key: QuotaCounter<'_>,
        by: i64,
        check_limit: bool,
    ) -> Result<(), StorageError>;

    /// Atomically decrement a usage counter (never below zero).
    async fn decrement(&self, key: QuotaCounter<'_>, by: i64) -> Result<(), StorageError>;

    /// Reset monthly counters (called by scheduled job at month rollover).
    async fn reset_monthly(&self) -> Result<(), StorageError>;
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
