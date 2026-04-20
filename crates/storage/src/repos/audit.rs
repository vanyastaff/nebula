//! Audit log and slug-history repositories.

use std::future::Future;

use crate::{
    error::StorageError,
    rows::{AuditLogRow, SlugHistoryRow},
};

/// High-level audit log — security and compliance trail.
///
/// Spec 16 layer 8 + spec 18. Append-only; retention is plan-configurable
/// (default 90 days). Separate from `execution_journal` which tracks
/// per-run step events.
pub trait AuditRepo: Send + Sync {
    /// Append an audit entry.
    fn append(&self, row: &AuditLogRow) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// List entries for an org, newest first.
    fn list_for_org(
        &self,
        org_id: &[u8],
        offset: u64,
        limit: u64,
    ) -> impl Future<Output = Result<Vec<AuditLogRow>, StorageError>> + Send;

    /// List entries by actor, newest first.
    fn list_for_actor(
        &self,
        actor_kind: &str,
        actor_id: &[u8],
        offset: u64,
        limit: u64,
    ) -> impl Future<Output = Result<Vec<AuditLogRow>, StorageError>> + Send;

    /// List entries filtered by action name (e.g. `'credential.rotated'`).
    fn list_by_action(
        &self,
        action: &str,
        offset: u64,
        limit: u64,
    ) -> impl Future<Output = Result<Vec<AuditLogRow>, StorageError>> + Send;

    // ── Slug history ────────────────────────────────────────────────────

    /// Record a rename so the old slug can still resolve for the grace period.
    fn record_rename(
        &self,
        row: &SlugHistoryRow,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Resolve an old slug to its current resource ID.
    ///
    /// Returns `None` when the slug has no history entry or the entry
    /// has expired.
    fn resolve_old_slug(
        &self,
        kind: &str,
        scope_id: Option<&[u8]>,
        old_slug: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, StorageError>> + Send;

    /// Delete expired slug-history rows. Returns count deleted.
    fn cleanup_expired_slugs(&self) -> impl Future<Output = Result<u64, StorageError>> + Send;
}
