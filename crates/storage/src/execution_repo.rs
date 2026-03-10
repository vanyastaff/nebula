//! Persistence and coordination interface for workflow executions.
//!
//! State (versioned CAS), journal (append-only), leases. Used by API and engine;
//! implementations in this crate or in adapters.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nebula_core::ExecutionId;
use thiserror::Error;
use tokio::sync::RwLock;

/// Errors returned by execution repository operations.
#[derive(Debug, Error)]
pub enum ExecutionRepoError {
    /// Requested entity is absent in storage.
    #[error("{entity} not found: {id}")]
    NotFound {
        /// Entity kind (for example: `execution`).
        entity: String,
        /// Entity identifier.
        id: String,
    },

    /// Optimistic concurrency check failed.
    #[error("{entity} {id}: expected version {expected_version}, got {actual_version}")]
    Conflict {
        /// Entity kind (for example: `execution`).
        entity: String,
        /// Entity identifier.
        id: String,
        /// Version expected by caller.
        expected_version: u64,
        /// Actual persisted version.
        actual_version: u64,
    },

    /// Backend/network connection failure.
    #[error("connection error: {0}")]
    Connection(String),

    /// Serialization or deserialization failure.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Operation timed out.
    #[error("timeout: {operation} after {duration:?}")]
    Timeout {
        /// Name of the operation that timed out.
        operation: String,
        /// Timeout duration that elapsed.
        duration: Duration,
    },

    /// Execution lease is currently held by another owner.
    #[error("lease unavailable for execution {execution_id}")]
    LeaseUnavailable {
        /// Execution identifier for the contested lease.
        execution_id: String,
    },

    /// Unexpected internal repository failure.
    #[error("internal error: {0}")]
    Internal(String),
}

impl ExecutionRepoError {
    /// Builds a [`ExecutionRepoError::NotFound`].
    pub fn not_found(entity: impl Into<String>, id: impl Into<String>) -> Self {
        Self::NotFound {
            entity: entity.into(),
            id: id.into(),
        }
    }

    /// Builds a [`ExecutionRepoError::Conflict`].
    pub fn conflict(
        entity: impl Into<String>,
        id: impl Into<String>,
        expected: u64,
        actual: u64,
    ) -> Self {
        Self::Conflict {
            entity: entity.into(),
            id: id.into(),
            expected_version: expected,
            actual_version: actual,
        }
    }

    /// Builds a [`ExecutionRepoError::Timeout`].
    pub fn timeout(operation: impl Into<String>, duration: Duration) -> Self {
        Self::Timeout {
            operation: operation.into(),
            duration,
        }
    }
}

/// Persistence and coordination interface for workflow executions.
#[async_trait]
pub trait ExecutionRepo: Send + Sync {
    /// Returns current state snapshot and CAS version for an execution.
    async fn get_state(
        &self,
        id: ExecutionId,
    ) -> Result<Option<(u64, serde_json::Value)>, ExecutionRepoError>;

    /// Applies a state transition if `expected_version` matches current version.
    ///
    /// Returns `Ok(true)` when transition is committed, `Ok(false)` on CAS mismatch.
    async fn transition(
        &self,
        id: ExecutionId,
        expected_version: u64,
        new_state: serde_json::Value,
    ) -> Result<bool, ExecutionRepoError>;

    /// Returns full journal entries for an execution.
    async fn get_journal(
        &self,
        id: ExecutionId,
    ) -> Result<Vec<serde_json::Value>, ExecutionRepoError>;

    /// Appends a single journal entry for an execution.
    async fn append_journal(
        &self,
        id: ExecutionId,
        entry: serde_json::Value,
    ) -> Result<(), ExecutionRepoError>;

    /// Attempts to acquire execution lease for a holder and TTL.
    ///
    /// Returns `Ok(true)` when acquired, `Ok(false)` when already held.
    async fn acquire_lease(
        &self,
        id: ExecutionId,
        holder: String,
        ttl: Duration,
    ) -> Result<bool, ExecutionRepoError>;

    /// Renews an existing lease held by `holder`.
    ///
    /// Returns `Ok(true)` when renewed, `Ok(false)` when holder mismatches or lease is absent.
    async fn renew_lease(
        &self,
        id: ExecutionId,
        holder: &str,
        ttl: Duration,
    ) -> Result<bool, ExecutionRepoError>;

    /// Releases lease if currently owned by `holder`.
    ///
    /// Returns `Ok(true)` when released, `Ok(false)` otherwise.
    async fn release_lease(
        &self,
        id: ExecutionId,
        holder: &str,
    ) -> Result<bool, ExecutionRepoError>;
}

/// In-memory execution repository for tests and single-process/health-only mode.
#[derive(Default)]
pub struct InMemoryExecutionRepo {
    state: Arc<RwLock<HashMap<ExecutionId, (u64, serde_json::Value)>>>,
    journal: Arc<RwLock<HashMap<ExecutionId, Vec<serde_json::Value>>>>,
    leases: Arc<RwLock<HashMap<ExecutionId, String>>>,
}

impl InMemoryExecutionRepo {
    /// Creates empty in-memory repository.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ExecutionRepo for InMemoryExecutionRepo {
    async fn get_state(
        &self,
        id: ExecutionId,
    ) -> Result<Option<(u64, serde_json::Value)>, ExecutionRepoError> {
        let state = self.state.read().await;
        Ok(state.get(&id).cloned())
    }

    async fn transition(
        &self,
        id: ExecutionId,
        expected_version: u64,
        new_state: serde_json::Value,
    ) -> Result<bool, ExecutionRepoError> {
        let mut state = self.state.write().await;
        let current = state.get(&id).map(|(v, _)| *v).unwrap_or(0);
        if current != expected_version {
            return Ok(false);
        }
        state.insert(id, (expected_version + 1, new_state));
        Ok(true)
    }

    async fn get_journal(
        &self,
        id: ExecutionId,
    ) -> Result<Vec<serde_json::Value>, ExecutionRepoError> {
        let journal = self.journal.read().await;
        Ok(journal.get(&id).cloned().unwrap_or_default())
    }

    async fn append_journal(
        &self,
        id: ExecutionId,
        entry: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        let mut journal = self.journal.write().await;
        journal.entry(id).or_default().push(entry);
        Ok(())
    }

    async fn acquire_lease(
        &self,
        id: ExecutionId,
        holder: String,
        _ttl: Duration,
    ) -> Result<bool, ExecutionRepoError> {
        let mut leases = self.leases.write().await;
        if leases.contains_key(&id) {
            return Ok(false);
        }
        leases.insert(id, holder);
        Ok(true)
    }

    async fn renew_lease(
        &self,
        id: ExecutionId,
        holder: &str,
        _ttl: Duration,
    ) -> Result<bool, ExecutionRepoError> {
        let leases = self.leases.read().await;
        Ok(leases.get(&id).map(|h| h.as_str()) == Some(holder))
    }

    async fn release_lease(
        &self,
        id: ExecutionId,
        holder: &str,
    ) -> Result<bool, ExecutionRepoError> {
        let mut leases = self.leases.write().await;
        let ok = leases.get(&id).map(|h| h.as_str()) == Some(holder);
        if ok {
            leases.remove(&id);
        }
        Ok(ok)
    }
}
