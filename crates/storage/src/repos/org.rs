//! Organization repository.

use async_trait::async_trait;

use crate::{
    error::StorageError,
    rows::{OrgMemberRow, OrgRow},
};

/// Organization storage — top-level tenant.
#[async_trait]
pub trait OrgRepo: Send + Sync {
    /// Insert a new organization. Fails on duplicate slug.
    async fn create(&self, org: &OrgRow) -> Result<(), StorageError>;

    /// Fetch an org by ID (skips soft-deleted).
    async fn get(&self, id: &[u8]) -> Result<Option<OrgRow>, StorageError>;

    /// Fetch an org by slug (case-insensitive).
    async fn get_by_slug(&self, slug: &str) -> Result<Option<OrgRow>, StorageError>;

    /// Update an org with CAS on `version`.
    async fn update(&self, org: &OrgRow, expected_version: i64) -> Result<(), StorageError>;

    /// Soft-delete an org.
    async fn soft_delete(&self, id: &[u8]) -> Result<(), StorageError>;

    // ── Members ─────────────────────────────────────────────────────────

    /// Add a principal as a member of an org.
    async fn add_member(&self, member: &OrgMemberRow) -> Result<(), StorageError>;

    /// Remove a member from an org.
    async fn remove_member(
        &self,
        org_id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<(), StorageError>;

    /// Get a principal's role in an org. Returns `None` if not a member.
    async fn get_member_role(
        &self,
        org_id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<Option<String>, StorageError>;

    /// List all members of an org.
    async fn list_members(&self, org_id: &[u8]) -> Result<Vec<OrgMemberRow>, StorageError>;

    /// List all orgs a principal belongs to.
    async fn list_for_principal(
        &self,
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<Vec<OrgMemberRow>, StorageError>;
}
