//! Audit log and slug-history repositories.

use async_trait::async_trait;

use crate::{
    error::StorageError,
    rows::{AuditLogRow, SlugHistoryRow},
};

/// High-level audit log — security and compliance trail.
///
/// Spec 16 layer 8 + spec 18. Append-only; retention is plan-configurable
/// (default 90 days). Separate from `execution_journal` which tracks
/// per-run step events.
#[async_trait]
pub trait AuditRepo: Send + Sync {
    /// Append an audit entry.
    async fn append(&self, row: &AuditLogRow) -> Result<(), StorageError>;

    /// List entries for an org, newest first.
    async fn list_for_org(
        &self,
        org_id: &[u8],
        offset: u64,
        limit: u64,
    ) -> Result<Vec<AuditLogRow>, StorageError>;

    /// List entries by actor, newest first.
    async fn list_for_actor(
        &self,
        actor_kind: &str,
        actor_id: &[u8],
        offset: u64,
        limit: u64,
    ) -> Result<Vec<AuditLogRow>, StorageError>;

    /// List entries filtered by action name (e.g. `'credential.rotated'`).
    async fn list_by_action(
        &self,
        action: &str,
        offset: u64,
        limit: u64,
    ) -> Result<Vec<AuditLogRow>, StorageError>;

    // ── Slug history ────────────────────────────────────────────────────

    /// Record a rename so the old slug can still resolve for the grace period.
    async fn record_rename(&self, row: &SlugHistoryRow) -> Result<(), StorageError>;

    /// Resolve an old slug to its current resource ID.
    ///
    /// Returns `None` when the slug has no history entry or the entry
    /// has expired.
    async fn resolve_old_slug(
        &self,
        kind: &str,
        scope_id: Option<&[u8]>,
        old_slug: &str,
    ) -> Result<Option<Vec<u8>>, StorageError>;

    /// Delete expired slug-history rows. Returns count deleted.
    async fn cleanup_expired_slugs(&self) -> Result<u64, StorageError>;
}
