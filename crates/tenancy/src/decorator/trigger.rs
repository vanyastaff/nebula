//! Scope-enforcing [`TriggerStore`] decorator (spec §6.2).

use std::sync::Arc;

use nebula_storage_port::dto::TriggerRow;
use nebula_storage_port::store::TriggerStore;
use nebula_storage_port::{Scope, StorageError};

/// Wraps a [`TriggerStore`] and forces every call into a single bound
/// [`Scope`]. The caller-supplied `scope` argument is *ignored* — the
/// adapter partitions `port_triggers` solely by the `scope` argument's
/// `(workspace_id, org_id)` (it never reads `row.workspace_id` for the
/// `WHERE`/key), so substituting the bound scope here makes a forged
/// scope a clean miss: a cross-tenant `get`/`list` returns
/// `Ok(None)`/empty, a cross-tenant `update`/`soft_delete` misses the
/// CAS, and a cross-tenant `create` lands only in the bound tenant's
/// partition (BOLA/IDOR closed by construction, §6.1 — never the row,
/// never an existence-leaking error).
///
/// `TriggerRow` additionally carries a denormalized `workspace_id`. That
/// field is *not* used by the adapter for partitioning, but it is
/// persisted and returned to callers; a row the caller built for tenant
/// B would otherwise be stored inside tenant A's partition while still
/// *claiming* to belong to B. `rebind` retargets the embedded owner at
/// the bound tenant on every write path so the persisted denormalized
/// owner can never disagree with the partition it actually lives in
/// (defence-in-depth, mirroring [`ScopedWorkflowStore`]). `run_as` and
/// `workflow_id` are intentionally *not* rewritten — they reference
/// principals/workflows whose own access is enforced by their stores;
/// rebinding them would corrupt legitimate intra-tenant references.
///
/// [`ScopedWorkflowStore`]: crate::ScopedWorkflowStore
#[derive(Clone)]
pub struct ScopedTriggerStore {
    inner: Arc<dyn TriggerStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedTriggerStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedTriggerStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedTriggerStore {
    /// Bind `inner` to `scope`. Constructed at the composition root from
    /// the request principal via a `ScopeResolver`.
    #[must_use]
    pub fn new(inner: Arc<dyn TriggerStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }

    /// Retarget a caller-supplied row's denormalized `workspace_id` at
    /// the bound tenant. A row the api built for the wrong tenant is
    /// silently retargeted at the bound tenant, where the unique/CAS
    /// predicates behave exactly as for any in-tenant row — never a
    /// cross-tenant write, never an existence-leaking error.
    fn rebind(&self, mut row: TriggerRow) -> TriggerRow {
        row.workspace_id = self.bound.workspace_id.clone();
        row
    }
}

#[async_trait::async_trait]
impl TriggerStore for ScopedTriggerStore {
    async fn create(&self, _scope: &Scope, row: TriggerRow) -> Result<(), StorageError> {
        self.inner.create(&self.bound, self.rebind(row)).await
    }

    async fn get(&self, _scope: &Scope, id: &str) -> Result<Option<TriggerRow>, StorageError> {
        self.inner.get(&self.bound, id).await
    }

    async fn list(&self, _scope: &Scope) -> Result<Vec<TriggerRow>, StorageError> {
        self.inner.list(&self.bound).await
    }

    async fn update(
        &self,
        _scope: &Scope,
        row: TriggerRow,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        self.inner
            .update(&self.bound, self.rebind(row), expected_version)
            .await
    }

    async fn soft_delete(&self, _scope: &Scope, id: &str) -> Result<(), StorageError> {
        self.inner.soft_delete(&self.bound, id).await
    }
}
