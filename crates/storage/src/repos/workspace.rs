//! Workspace repository.

use std::future::Future;

use crate::{
    error::StorageError,
    rows::{WorkspaceMemberRow, WorkspaceRow},
};

/// Workspace storage — second-level tenant under an org.
pub trait WorkspaceRepo: Send + Sync {
    /// Insert a new workspace. Fails on duplicate (org_id, slug).
    fn create(&self, ws: &WorkspaceRow) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Fetch a workspace by ID.
    fn get(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<Option<WorkspaceRow>, StorageError>> + Send;

    /// Fetch a workspace by (org, slug).
    fn get_by_slug(
        &self,
        org_id: &[u8],
        slug: &str,
    ) -> impl Future<Output = Result<Option<WorkspaceRow>, StorageError>> + Send;

    /// Get the default workspace for an org.
    fn get_default(
        &self,
        org_id: &[u8],
    ) -> impl Future<Output = Result<Option<WorkspaceRow>, StorageError>> + Send;

    /// List all workspaces under an org.
    fn list_for_org(
        &self,
        org_id: &[u8],
    ) -> impl Future<Output = Result<Vec<WorkspaceRow>, StorageError>> + Send;

    /// Update a workspace with CAS on `version`.
    fn update(
        &self,
        ws: &WorkspaceRow,
        expected_version: i64,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Soft-delete a workspace.
    fn soft_delete(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    // ── Members ─────────────────────────────────────────────────────────

    /// Add a principal as a member of a workspace.
    fn add_member(
        &self,
        member: &WorkspaceMemberRow,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Remove a member from a workspace.
    fn remove_member(
        &self,
        workspace_id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Get a principal's role in a workspace.
    fn get_member_role(
        &self,
        workspace_id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> impl Future<Output = Result<Option<String>, StorageError>> + Send;

    /// List members of a workspace.
    fn list_members(
        &self,
        workspace_id: &[u8],
    ) -> impl Future<Output = Result<Vec<WorkspaceMemberRow>, StorageError>> + Send;
}
