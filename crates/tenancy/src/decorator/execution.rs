//! Scope-enforcing [`ExecutionStore`] decorator.

use std::sync::Arc;
use std::time::Duration;

use nebula_storage_port::dto::ExecutionRecord;
use nebula_storage_port::store::ExecutionStore;
use nebula_storage_port::{FencingToken, Scope, StorageError, TransitionBatch, TransitionOutcome};

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

    /// Rebuild a caller's batch with the bound scope substituted into the
    /// batch itself *and* every outbox row.
    ///
    /// `TransitionBatch` is immutable with a private scope field, so a
    /// caller structurally cannot mutate it post-build. Rebuilding here
    /// (rather than comparing-and-rejecting) keeps the confused-deputy
    /// mitigation uniform: a batch the engine built for the wrong tenant
    /// is silently retargeted at the bound tenant, where the CAS will
    /// simply miss (the id does not exist in the bound scope) and return
    /// a non-Apply outcome — never a cross-tenant write, never an
    /// existence-leaking error (§6.1).
    fn rebind(&self, batch: &TransitionBatch) -> Result<TransitionBatch, StorageError> {
        let outbox = batch
            .outbox()
            .iter()
            .map(|m| {
                let mut m = m.clone();
                m.scope = self.bound.clone();
                m
            })
            .collect();
        TransitionBatch::builder()
            .scope(self.bound.clone())
            .execution_id(batch.execution_id())
            .expected_version(batch.expected_version())
            .fencing(batch.fencing())
            .new_state(batch.new_state().clone())
            .outbox(outbox)
            .journal(batch.journal().to_vec())
            .build()
    }
}

#[async_trait::async_trait]
impl ExecutionStore for ScopedExecutionStore {
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

    async fn commit(&self, batch: TransitionBatch) -> Result<TransitionOutcome, StorageError> {
        let rebound = self.rebind(&batch)?;
        self.inner.commit(rebound).await
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

    async fn list_running(&self, _scope: &Scope) -> Result<Vec<String>, StorageError> {
        self.inner.list_running(&self.bound).await
    }

    async fn list_running_for_workflow(
        &self,
        _scope: &Scope,
        workflow_id: &str,
    ) -> Result<Vec<String>, StorageError> {
        self.inner
            .list_running_for_workflow(&self.bound, workflow_id)
            .await
    }

    async fn count(&self, _scope: &Scope, workflow_id: Option<&str>) -> Result<u64, StorageError> {
        self.inner.count(&self.bound, workflow_id).await
    }
}
