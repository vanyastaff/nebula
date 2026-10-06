//! Scope-enforcing [`ExecutionStore`] decorator.

use std::sync::Arc;
use std::time::Duration;

use nebula_storage_port::dto::ExecutionRecord;
use nebula_storage_port::store::ExecutionStore;
use nebula_storage_port::{
    ExecutionHistoryPage, ExecutionHistoryQuery, FencingToken, Scope, StorageError,
    TransitionBatch, TransitionOutcome,
};

/// Wraps an [`ExecutionStore`] and forces every call into a single bound
/// [`Scope`]. The caller-supplied `scope` argument is *ignored* — the
/// engine cannot read, transition, or lease another tenant's execution
/// even if it passes a forged scope.
#[derive(Clone)]
pub struct ScopedExecutionStore {
    inner: Arc<dyn ExecutionStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedExecutionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedExecutionStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedExecutionStore {
    /// Bind `inner` to `scope`. Constructed at the composition root from
    /// the request principal via a `ScopeResolver`.
    #[must_use]
    pub fn new(inner: Arc<dyn ExecutionStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl ExecutionStore for ScopedExecutionStore {
    async fn record_execution_admission_refusal(
        &self,
        refusal: &nebula_storage_port::store::ExecutionAdmissionRefusal<'_>,
    ) -> Result<nebula_storage_port::store::ExecutionAdmissionRefusalOutcome, StorageError> {
        let rebound = nebula_storage_port::store::ExecutionAdmissionRefusal::new(
            &self.bound,
            refusal.execution_id(),
            refusal.fence(),
            refusal.node_key(),
            refusal.attempt(),
        );
        self.inner
            .record_execution_admission_refusal(&rebound)
            .await
    }

    fn backend_kind(&self) -> nebula_storage_port::StorageBackendKind {
        self.inner.backend_kind()
    }

    async fn create(
        &self,
        _scope: &Scope,
        id: &str,
        workflow_id: &str,
        initial_state: serde_json::Value,
    ) -> Result<(), StorageError> {
        self.inner
            .create(&self.bound, id, workflow_id, initial_state)
            .await
    }

    async fn get(&self, _scope: &Scope, id: &str) -> Result<Option<ExecutionRecord>, StorageError> {
        self.inner.get(&self.bound, id).await
    }

    /// The batch is retargeted at the bound tenant — itself, every outbox
    /// row and every resume-token row — rather than compared and rejected.
    /// A batch built for the wrong tenant then simply misses the CAS (the
    /// id does not exist in the bound scope): never a cross-tenant write,
    /// never an existence-leaking error (§6.1).
    async fn commit(&self, batch: TransitionBatch) -> Result<TransitionOutcome, StorageError> {
        self.inner.commit(batch.rebound_to(&self.bound)).await
    }

    async fn acquire_lease(
        &self,
        _scope: &Scope,
        id: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<FencingToken>, StorageError> {
        self.inner.acquire_lease(&self.bound, id, holder, ttl).await
    }

    async fn renew_lease(
        &self,
        _scope: &Scope,
        id: &str,
        token: FencingToken,
        ttl: Duration,
    ) -> Result<bool, StorageError> {
        self.inner.renew_lease(&self.bound, id, token, ttl).await
    }

    async fn release_lease(
        &self,
        _scope: &Scope,
        id: &str,
        token: FencingToken,
    ) -> Result<bool, StorageError> {
        self.inner.release_lease(&self.bound, id, token).await
    }

    async fn list_all_running(&self) -> Result<Vec<ExecutionRecord>, StorageError> {
        // No scope filter: cross-tenant scan is the point of this method.
        // The caller (timer scanner) re-drives each row under its own scope.
        self.inner.list_all_running().await
    }

    async fn list_history(
        &self,
        _scope: &Scope,
        query: &ExecutionHistoryQuery,
    ) -> Result<ExecutionHistoryPage, StorageError> {
        self.inner.list_history(&self.bound, query).await
    }

    async fn count(&self, _scope: &Scope, workflow_id: Option<&str>) -> Result<u64, StorageError> {
        self.inner.count(&self.bound, workflow_id).await
    }
}
