//! Scope-enforcing [`WorkflowStore`] + [`WorkflowVersionStore`] decorators
//! (spec-16 workflow/version split).

use std::sync::Arc;

use nebula_storage_port::dto::{WorkflowRecord, WorkflowVersionRecord};
use nebula_storage_port::store::{WorkflowStore, WorkflowVersionStore};
use nebula_storage_port::{Scope, StorageError};

/// Wraps a [`WorkflowStore`] and forces every call into the bound
/// [`Scope`]. The caller-supplied `scope` argument is *ignored*, and the
/// `scope` carried inside a [`WorkflowRecord`] is rebound to the bound
/// tenant before it reaches the backend — the api cannot create, read, or
/// CAS-update another tenant's workflow row even with a forged scope
/// (§6.1 confused-deputy, closed by construction).
#[derive(Clone)]
pub struct ScopedWorkflowStore {
    inner: Arc<dyn WorkflowStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedWorkflowStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedWorkflowStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedWorkflowStore {
    /// Bind `inner` to `scope`. Constructed at the composition root from
    /// the request principal via a `ScopeResolver`.
    #[must_use]
    pub fn new(inner: Arc<dyn WorkflowStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }

    /// Rebind a caller-supplied record's embedded scope to the bound
    /// tenant. A record the api built for the wrong tenant is silently
    /// retargeted at the bound tenant, where the unique/CAS predicates
    /// behave exactly as for any in-tenant row — never a cross-tenant
    /// write, never an existence-leaking error.
    fn rebind(&self, mut record: WorkflowRecord) -> WorkflowRecord {
        record.scope = self.bound.clone();
        record
    }
}

#[async_trait::async_trait]
impl WorkflowStore for ScopedWorkflowStore {
    async fn create(&self, _scope: &Scope, record: WorkflowRecord) -> Result<(), StorageError> {
        self.inner.create(&self.bound, self.rebind(record)).await
    }

    async fn get(&self, _scope: &Scope, id: &str) -> Result<Option<WorkflowRecord>, StorageError> {
        self.inner.get(&self.bound, id).await
    }

    async fn get_by_slug(
        &self,
        _scope: &Scope,
        slug: &str,
    ) -> Result<Option<WorkflowRecord>, StorageError> {
        self.inner.get_by_slug(&self.bound, slug).await
    }

    async fn update(
        &self,
        _scope: &Scope,
        record: WorkflowRecord,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        self.inner
            .update(&self.bound, self.rebind(record), expected_version)
            .await
    }

    async fn soft_delete(&self, _scope: &Scope, id: &str) -> Result<(), StorageError> {
        self.inner.soft_delete(&self.bound, id).await
    }

    async fn list(&self, _scope: &Scope) -> Result<Vec<WorkflowRecord>, StorageError> {
        self.inner.list(&self.bound).await
    }
}

/// Wraps a [`WorkflowVersionStore`] and forces every call into the bound
/// [`Scope`]. [`WorkflowVersionRecord`] carries no scope of its own — the
/// scope argument is the sole tenant carrier and is always substituted.
#[derive(Clone)]
pub struct ScopedWorkflowVersionStore {
    inner: Arc<dyn WorkflowVersionStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedWorkflowVersionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedWorkflowVersionStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedWorkflowVersionStore {
    /// Bind `inner` to `scope`.
    #[must_use]
    pub fn new(inner: Arc<dyn WorkflowVersionStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl WorkflowVersionStore for ScopedWorkflowVersionStore {
    async fn create(
        &self,
        _scope: &Scope,
        record: WorkflowVersionRecord,
    ) -> Result<(), StorageError> {
        self.inner.create(&self.bound, record).await
    }

    async fn get(
        &self,
        _scope: &Scope,
        workflow_id: &str,
        number: u32,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError> {
        self.inner.get(&self.bound, workflow_id, number).await
    }

    async fn get_published(
        &self,
        _scope: &Scope,
        workflow_id: &str,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError> {
        self.inner.get_published(&self.bound, workflow_id).await
    }

    async fn list(
        &self,
        _scope: &Scope,
        workflow_id: &str,
    ) -> Result<Vec<WorkflowVersionRecord>, StorageError> {
        self.inner.list(&self.bound, workflow_id).await
    }
}
