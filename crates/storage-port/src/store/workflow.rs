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

    /// Atomically persist a workflow row **and** its published version as
    /// one unit of work (spec-16: a workflow's definition lives on its
    /// version records, so a row without a published version is invisible
    /// to every reader — "the workflow vanished"). Splitting the row write
    /// and the version write into two awaits leaves that orphan window on
    /// any partial failure; this method closes it.
    ///
    /// - `expected_version == None` — **create**: insert `row` and
    ///   `version` together. Either both land or neither does.
    /// - `expected_version == Some(v)` — **CAS update**: the row is
    ///   rewritten only if its stored version equals `v`, and the new
    ///   `version` record is appended in the same unit. A version miss
    ///   (`Conflict`) or missing row (`NotFound`) leaves both untouched.
    ///
    /// Atomicity is per backend: one SQL transaction for SQLite/Postgres,
    /// one mutex-guarded critical section for the in-memory store. This is
    /// a real unit of work, not a best-effort/compensation sequence.
    async fn save_with_published_version(
        &self,
        scope: &Scope,
        row: WorkflowRecord,
        version: WorkflowVersionRecord,
        expected_version: Option<u64>,
    ) -> Result<(), StorageError>;

    /// Soft-delete a workflow row.
    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError>;

    /// List active workflows in `scope`.
    async fn list(&self, scope: &Scope) -> Result<Vec<WorkflowRecord>, StorageError>;

    /// Count active (non-soft-deleted) workflows in `scope`.
    ///
    /// Semantically equivalent to `list(scope).await?.len()` but the SQL
    /// backends answer it with a `SELECT COUNT(*)` rather than
    /// materializing every row — the readiness probe and pagination
    /// totals call it on the hot path, so it must not be `O(n)` in the
    /// row count.
    async fn count(&self, scope: &Scope) -> Result<u64, StorageError>;
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
