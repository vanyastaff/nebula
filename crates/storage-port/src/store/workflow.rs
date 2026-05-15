//! Workflow + workflow-version store traits (spec-16 split).
use crate::dto::{WorkflowRecord, WorkflowVersionRecord};
use crate::error::StorageError;
use crate::scope::Scope;

/// Workflow aggregate (the workflow row, not its versions).
#[async_trait::async_trait]
pub trait WorkflowStore: Send + Sync + std::fmt::Debug {
    /// Create a workflow row in `scope`.
    async fn create(&self, scope: &Scope, record: WorkflowRecord) -> Result<(), StorageError>;

    /// Read a workflow row. Scope mismatch yields `Ok(None)`.
    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<WorkflowRecord>, StorageError>;

    /// Resolve an active workflow by slug within `scope`.
    async fn get_by_slug(
        &self,
        scope: &Scope,
        slug: &str,
    ) -> Result<Option<WorkflowRecord>, StorageError>;

    /// CAS-update a workflow row; `expected_version` must match.
    async fn update(
        &self,
        scope: &Scope,
        record: WorkflowRecord,
        expected_version: u64,
    ) -> Result<(), StorageError>;

    /// Soft-delete a workflow row.
    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError>;

    /// List active workflows in `scope`.
    async fn list(&self, scope: &Scope) -> Result<Vec<WorkflowRecord>, StorageError>;
}

/// Workflow-version aggregate.
#[async_trait::async_trait]
pub trait WorkflowVersionStore: Send + Sync + std::fmt::Debug {
    /// Create a new workflow version.
    async fn create(
        &self,
        scope: &Scope,
        record: WorkflowVersionRecord,
    ) -> Result<(), StorageError>;

    /// Read one version by workflow id + version number.
    async fn get(
        &self,
        scope: &Scope,
        workflow_id: &str,
        number: u32,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError>;

    /// Read the published version for a workflow, if one is published.
    async fn get_published(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError>;

    /// List all versions for a workflow, newest first.
    async fn list(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Vec<WorkflowVersionRecord>, StorageError>;
}
