//! Workflow and workflow-version repositories.

use async_trait::async_trait;

use crate::{
    error::StorageError,
    rows::{WorkflowRow, WorkflowVersionRow},
};

/// Workflow entity storage.
///
/// A workflow is a logical container — its executable definition
/// lives in [`WorkflowVersionRepo`]. `current_version_id` points
/// at the active published version.
#[async_trait]
pub trait WorkflowRepo: Send + Sync {
    /// Insert a new workflow. Fails on duplicate (workspace_id, slug).
    async fn create(&self, wf: &WorkflowRow) -> Result<(), StorageError>;

    /// Fetch a workflow by ID.
    async fn get(&self, id: &[u8]) -> Result<Option<WorkflowRow>, StorageError>;

    /// Fetch a workflow by (workspace, slug).
    async fn get_by_slug(
        &self,
        workspace_id: &[u8],
        slug: &str,
    ) -> Result<Option<WorkflowRow>, StorageError>;

    /// Update a workflow with CAS on `version`.
    async fn update(&self, wf: &WorkflowRow, expected_version: i64) -> Result<(), StorageError>;

    /// Soft-delete a workflow.
    async fn soft_delete(&self, id: &[u8]) -> Result<(), StorageError>;

    /// List workflows in a workspace, ordered by `created_at DESC`.
    async fn list(
        &self,
        workspace_id: &[u8],
        offset: u64,
        limit: u64,
    ) -> Result<Vec<WorkflowRow>, StorageError>;

    /// Count non-deleted workflows in a workspace.
    async fn count(&self, workspace_id: &[u8]) -> Result<u64, StorageError>;
}

/// Workflow version storage — immutable published snapshots.
#[async_trait]
pub trait WorkflowVersionRepo: Send + Sync {
    /// Insert a new version. The `version_number` must be unique per workflow.
    async fn create(&self, version: &WorkflowVersionRow) -> Result<(), StorageError>;

    /// Fetch a version by ID.
    async fn get(&self, id: &[u8]) -> Result<Option<WorkflowVersionRow>, StorageError>;

    /// Fetch the currently-published version of a workflow.
    async fn get_published(
        &self,
        workflow_id: &[u8],
    ) -> Result<Option<WorkflowVersionRow>, StorageError>;

    /// Fetch a version by (workflow_id, version_number).
    async fn get_by_number(
        &self,
        workflow_id: &[u8],
        version_number: i32,
    ) -> Result<Option<WorkflowVersionRow>, StorageError>;

    /// List versions for a workflow, ordered by `version_number DESC`.
    async fn list_for_workflow(
        &self,
        workflow_id: &[u8],
        offset: u64,
        limit: u64,
    ) -> Result<Vec<WorkflowVersionRow>, StorageError>;

    /// Transition a version from `Draft` to `Published`. Atomically
    /// archives any previously-published version of the same workflow
    /// (the unique index on published state makes this necessary).
    async fn publish(&self, id: &[u8]) -> Result<(), StorageError>;

    /// Transition a version to `Archived`.
    async fn archive(&self, id: &[u8]) -> Result<(), StorageError>;

    /// Pin a version to protect it from retention GC.
    async fn set_pinned(&self, id: &[u8], pinned: bool) -> Result<(), StorageError>;
}
