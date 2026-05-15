//! The atomic execution aggregate trait.
use std::time::Duration;

use crate::batch::{TransitionBatch, TransitionOutcome};
use crate::dto::ExecutionRecord;
use crate::error::StorageError;
use crate::ids::FencingToken;
use crate::scope::Scope;

/// Execution state + lease + the §12.2 atomic transition.
///
/// `commit` applies the [`TransitionBatch`] (state + outbox + journal) in one
/// transaction gated by the CAS version **and** the lease fencing token. A
/// superseded/expired holder is rejected even when the version matches —
/// this closes the zombie-runner hole.
#[async_trait::async_trait]
pub trait ExecutionStore: Send + Sync + std::fmt::Debug {
    /// Create a new execution row in `scope`.
    async fn create(
        &self,
        scope: &Scope,
        id: &str,
        workflow_id: &str,
        initial_state: serde_json::Value,
    ) -> Result<(), StorageError>;

    /// Read an execution row. A scope mismatch yields `Ok(None)` (the row's
    /// existence never leaks across tenants).
    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ExecutionRecord>, StorageError>;

    /// Apply an atomic state transition (CAS + fencing + state + outbox +
    /// journal in one transaction).
    async fn commit(&self, batch: TransitionBatch) -> Result<TransitionOutcome, StorageError>;

    /// Acquire the execution lease for `holder`. Returns the fresh
    /// [`FencingToken`] on success, `None` if another holder owns a live
    /// lease.
    async fn acquire_lease(
        &self,
        scope: &Scope,
        id: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<FencingToken>, StorageError>;

    /// Extend the lease TTL. Returns `false` if `token` was superseded.
    async fn renew_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: FencingToken,
        ttl: Duration,
    ) -> Result<bool, StorageError>;

    /// Release the lease. Returns `false` if `token` no longer owns it
    /// (idempotent).
    async fn release_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: FencingToken,
    ) -> Result<bool, StorageError>;

    /// List running execution ids in `scope`.
    async fn list_running(&self, scope: &Scope) -> Result<Vec<String>, StorageError>;

    /// List running execution ids for one workflow in `scope`.
    async fn list_running_for_workflow(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Vec<String>, StorageError>;

    /// Count executions in `scope`, optionally filtered by workflow.
    async fn count(&self, scope: &Scope, workflow_id: Option<&str>) -> Result<u64, StorageError>;
}
