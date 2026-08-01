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
    ///
    /// **The adapter owns the clock**, and no caller supplies one.
    ///
    /// A lease is a claim about liveness, so whichever clock decides
    /// live-vs-expired is the one every replica must agree on. A backend
    /// serving multiple processes therefore decides inside the database
    /// (`clock_timestamp()` for PostgreSQL), and a caller has no way to hand in
    /// a reading of its own: a worker whose clock ran fast would judge a
    /// healthy peer's lease expired and fence it out, and one running slow
    /// would mint a lease that was already dead — neither detectable from the
    /// row afterwards.
    ///
    /// Tests that need to control lease expiry do it where there is no shared
    /// clock to disagree about: the in-memory adapter takes an injectable
    /// clock, and the SQL backends take a short TTL.
    async fn acquire_lease(
        &self,
        scope: &Scope,
        id: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<FencingToken>, StorageError>;

    /// Extend the lease TTL. Returns `false` if `token` was superseded.
    ///
    /// Same clock ownership as [`Self::acquire_lease`].
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

    /// List every execution row across ALL tenant scopes.
    ///
    /// Used by the durable-timer wake scanner to find parked timers with no live
    /// owner regardless of tenant. Returns full [`ExecutionRecord`]s (scope + state)
    /// so the caller applies its own predicate and re-drives under each row's own
    /// scope. MVP: returns all active rows; a backend-side overdue-timer pushdown
    /// (e.g. a Postgres JSONB predicate) is a follow-up optimization.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a backend failure.
    async fn list_all_running(&self) -> Result<Vec<ExecutionRecord>, StorageError>;

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
