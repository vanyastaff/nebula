//! The atomic execution aggregate trait.
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::batch::{TransitionBatch, TransitionOutcome};
use crate::dto::{
    DueTimer, DueTimerCursor, ExecutionHistoryPage, ExecutionHistoryQuery, ExecutionRecord,
};
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
    /// Record a real runtime-owned admission throttle before provider egress.
    ///
    /// Storage verifies scope, the live execution lease and the retained
    /// accepted-turn source. The refusal never changes the aggregate, so no
    /// expected version gates it. Required of every implementation, decorators
    /// included, so a wrapper cannot silently drop the observation.
    ///
    /// # Errors
    /// Backend failures and invalid stored representations return bounded errors.
    async fn record_execution_admission_refusal(
        &self,
        refusal: &super::ExecutionAdmissionRefusal<'_>,
    ) -> Result<super::ExecutionAdmissionRefusalOutcome, StorageError>;

    /// Closed identity of the owning backend, preserved by decorators. Labels
    /// telemetry for decisions whose observation could not be recorded.
    fn backend_kind(&self) -> crate::StorageBackendKind;

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

    /// Durably record that a cancel was requested for this execution.
    ///
    /// The control consumer that drains a `Cancel` cannot reach the cancel
    /// registry of a runner on another worker. It records the request here
    /// instead, and the lease holder observes it from its heartbeat; any
    /// later lease holder observes it as soon as it acquires. The first
    /// request's instant is kept. The write is not lease-gated and never
    /// changes the aggregate's version, so it cannot conflict with the
    /// holder's commits.
    ///
    /// Returns `false` when no execution `id` exists in `scope`.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a backend failure.
    async fn request_cancel(&self, scope: &Scope, id: &str) -> Result<bool, StorageError>;

    /// Whether [`Self::request_cancel`] was recorded for this execution.
    /// A scope mismatch or a missing row reads as `false`.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a backend failure.
    async fn cancel_requested(&self, scope: &Scope, id: &str) -> Result<bool, StorageError>;

    /// Up to `limit` executions across ALL tenant scopes whose earliest durable
    /// timer (the listing's `next_wake_at`) is at or before `due_at`.
    ///
    /// Used by the durable-timer wake scanner to find parked timers with no
    /// live owner regardless of tenant. Rows are ordered by
    /// `(next_wake_at, id)` and start after `after` when given, so a sweep
    /// pages through due rows without rereading the ones it skipped. Only
    /// rows with a timer are read; the caller re-checks the state and
    /// re-drives each row under its own scope.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a backend failure.
    async fn list_due_timers(
        &self,
        due_at: DateTime<Utc>,
        after: Option<&DueTimerCursor>,
        limit: u32,
    ) -> Result<Vec<DueTimer>, StorageError>;

    /// One page of execution history in `scope`, newest first.
    ///
    /// Filters and keyset position come from `query`; see
    /// [`ExecutionHistoryQuery::admits`] for the exact predicate every backend
    /// implements. A cursor from another tenant can only page this scope.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a backend failure or an undecodable row.
    async fn list_history(
        &self,
        scope: &Scope,
        query: &ExecutionHistoryQuery,
    ) -> Result<ExecutionHistoryPage, StorageError>;

    /// Count executions in `scope`, optionally filtered by workflow.
    async fn count(&self, scope: &Scope, workflow_id: Option<&str>) -> Result<u64, StorageError>;
}
