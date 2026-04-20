//! Organization repository.

use std::future::Future;

use crate::{
    error::StorageError,
    rows::{OrgMemberRow, OrgRow},
};

/// Organization storage — top-level tenant.
pub trait OrgRepo: Send + Sync {
    /// Insert a new organization. Fails on duplicate slug.
    fn create(&self, org: &OrgRow) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Fetch an org by ID (skips soft-deleted).
    fn get(&self, id: &[u8]) -> impl Future<Output = Result<Option<OrgRow>, StorageError>> + Send;

    /// Fetch an org by slug (case-insensitive).
    fn get_by_slug(
        &self,
        slug: &str,
    ) -> impl Future<Output = Result<Option<OrgRow>, StorageError>> + Send;

    /// Update an org with CAS on `version`.
    fn update(
        &self,
        org: &OrgRow,
        expected_version: i64,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Soft-delete an org.
    fn soft_delete(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    // ── Members ─────────────────────────────────────────────────────────

    /// Add a principal as a member of an org.
    fn add_member(
        &self,
        member: &OrgMemberRow,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Remove a member from an org.
    fn remove_member(
        &self,
        org_id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Get a principal's role in an org. Returns `None` if not a member.
    fn get_member_role(
        &self,
        org_id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> impl Future<Output = Result<Option<String>, StorageError>> + Send;

    /// List all members of an org.
    fn list_members(
        &self,
        org_id: &[u8],
    ) -> impl Future<Output = Result<Vec<OrgMemberRow>, StorageError>> + Send;

    /// List all orgs a principal belongs to.
    fn list_for_principal(
        &self,
        principal_kind: &str,
        principal_id: &[u8],
    ) -> impl Future<Output = Result<Vec<OrgMemberRow>, StorageError>> + Send;
}
