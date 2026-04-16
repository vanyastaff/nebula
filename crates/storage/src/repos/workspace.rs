//! Workspace repository.

use async_trait::async_trait;

use crate::{
    error::StorageError,
    rows::{WorkspaceMemberRow, WorkspaceRow},
};

/// Workspace storage — second-level tenant under an org.
#[async_trait]
pub trait WorkspaceRepo: Send + Sync {
    /// Insert a new workspace. Fails on duplicate (org_id, slug).
    async fn create(&self, ws: &WorkspaceRow) -> Result<(), StorageError>;

    /// Fetch a workspace by ID.
    async fn get(&self, id: &[u8]) -> Result<Option<WorkspaceRow>, StorageError>;

    /// Fetch a workspace by (org, slug).
    async fn get_by_slug(
        &self,
        org_id: &[u8],
        slug: &str,
    ) -> Result<Option<WorkspaceRow>, StorageError>;

    /// Get the default workspace for an org.
    async fn get_default(&self, org_id: &[u8]) -> Result<Option<WorkspaceRow>, StorageError>;

    /// List all workspaces under an org.
    async fn list_for_org(&self, org_id: &[u8]) -> Result<Vec<WorkspaceRow>, StorageError>;

    /// Update a workspace with CAS on `version`.
    async fn update(&self, ws: &WorkspaceRow, expected_version: i64) -> Result<(), StorageError>;

    /// Soft-delete a workspace.
    async fn soft_delete(&self, id: &[u8]) -> Result<(), StorageError>;

    // ── Members ─────────────────────────────────────────────────────────

    /// Add a principal as a member of a workspace.
    async fn add_member(&self, member: &WorkspaceMemberRow) -> Result<(), StorageError>;

    /// Remove a member from a workspace.
    async fn remove_member(
        &self,
        workspace_id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<(), StorageError>;

    /// Get a principal's role in a workspace.
    async fn get_member_role(
        &self,
        workspace_id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<Option<String>, StorageError>;

    /// List members of a workspace.
    async fn list_members(
        &self,
        workspace_id: &[u8],
    ) -> Result<Vec<WorkspaceMemberRow>, StorageError>;
}
