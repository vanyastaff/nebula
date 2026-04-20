//! Workflow and workflow-version repositories.

use std::future::Future;

use crate::{
    error::StorageError,
    rows::{WorkflowRow, WorkflowVersionRow},
};

/// Workflow entity storage.
///
/// A workflow is a logical container — its executable definition
/// lives in [`WorkflowVersionRepo`]. `current_version_id` points
/// at the active published version.
pub trait WorkflowRepo: Send + Sync {
    /// Insert a new workflow. Fails on duplicate (workspace_id, slug).
    fn create(&self, wf: &WorkflowRow) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Fetch a workflow by ID.
    fn get(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<Option<WorkflowRow>, StorageError>> + Send;

    /// Fetch a workflow by (workspace, slug).
    fn get_by_slug(
        &self,
        workspace_id: &[u8],
        slug: &str,
    ) -> impl Future<Output = Result<Option<WorkflowRow>, StorageError>> + Send;

    /// Update a workflow with CAS on `version`.
    fn update(
        &self,
        wf: &WorkflowRow,
        expected_version: i64,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Soft-delete a workflow.
    fn soft_delete(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// List workflows in a workspace, ordered by `created_at DESC`.
    fn list(
        &self,
        workspace_id: &[u8],
        offset: u64,
        limit: u64,
    ) -> impl Future<Output = Result<Vec<WorkflowRow>, StorageError>> + Send;

    /// Count non-deleted workflows in a workspace.
    fn count(&self, workspace_id: &[u8]) -> impl Future<Output = Result<u64, StorageError>> + Send;
}

/// Workflow version storage — immutable published snapshots.
pub trait WorkflowVersionRepo: Send + Sync {
    /// Insert a new version. The `version_number` must be unique per workflow.
    fn create(
        &self,
        version: &WorkflowVersionRow,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Fetch a version by ID.
    fn get(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<Option<WorkflowVersionRow>, StorageError>> + Send;

    /// Fetch the currently-published version of a workflow.
    fn get_published(
        &self,
        workflow_id: &[u8],
    ) -> impl Future<Output = Result<Option<WorkflowVersionRow>, StorageError>> + Send;

    /// Fetch a version by (workflow_id, version_number).
    fn get_by_number(
        &self,
        workflow_id: &[u8],
        version_number: i32,
    ) -> impl Future<Output = Result<Option<WorkflowVersionRow>, StorageError>> + Send;

    /// List versions for a workflow, ordered by `version_number DESC`.
    fn list_for_workflow(
        &self,
        workflow_id: &[u8],
        offset: u64,
        limit: u64,
    ) -> impl Future<Output = Result<Vec<WorkflowVersionRow>, StorageError>> + Send;

    /// Transition a version from `Draft` to `Published`. Atomically
    /// archives any previously-published version of the same workflow
    /// (the unique index on published state makes this necessary).
    fn publish(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Transition a version to `Archived`.
    fn archive(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Pin a version to protect it from retention GC.
    fn set_pinned(
        &self,
        id: &[u8],
        pinned: bool,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;
}
