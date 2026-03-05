//! Persistence and coordination interface for workflow executions.
//!
//! State (versioned CAS), journal (append-only), leases. Used by API and engine;
//! implementations in this crate or in adapters.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nebula_core::ExecutionId;
use tokio::sync::RwLock;
use thiserror::Error;

/// Errors returned by execution repository operations.
#[derive(Debug, Error)]
pub enum ExecutionRepoError {
    #[error("{entity} not found: {id}")]
    NotFound { entity: String, id: String },

    #[error("{entity} {id}: expected version {expected_version}, got {actual_version}")]
    Conflict {
        entity: String,
        id: String,
        expected_version: u64,
        actual_version: u64,
    },

    #[error("connection error: {0}")]
    Connection(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("timeout: {operation} after {duration:?}")]
    Timeout {
        operation: String,
        duration: Duration,
    },

    #[error("lease unavailable for execution {execution_id}")]
    LeaseUnavailable { execution_id: String },

    #[error("internal error: {0}")]
    Internal(String),
}

impl ExecutionRepoError {
    pub fn not_found(entity: impl Into<String>, id: impl Into<String>) -> Self {
        Self::NotFound {
            entity: entity.into(),
            id: id.into(),
        }
    }

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
    async fn get_state(
        &self,
        id: ExecutionId,
    ) -> Result<Option<(u64, serde_json::Value)>, ExecutionRepoError>;

    async fn transition(
        &self,
        id: ExecutionId,
        expected_version: u64,
        new_state: serde_json::Value,
    ) -> Result<bool, ExecutionRepoError>;

    async fn get_journal(&self, id: ExecutionId) -> Result<Vec<serde_json::Value>, ExecutionRepoError>;

    async fn append_journal(
        &self,
        id: ExecutionId,
        entry: serde_json::Value,
    ) -> Result<(), ExecutionRepoError>;

    async fn acquire_lease(
        &self,
        id: ExecutionId,
        holder: String,
        ttl: Duration,
    ) -> Result<bool, ExecutionRepoError>;

    async fn renew_lease(
        &self,
        id: ExecutionId,
        holder: &str,
        ttl: Duration,
    ) -> Result<bool, ExecutionRepoError>;

    async fn release_lease(&self, id: ExecutionId, holder: &str) -> Result<bool, ExecutionRepoError>;
}

/// In-memory execution repository for tests and single-process/health-only mode.
#[derive(Default)]
pub struct InMemoryExecutionRepo {
    state: Arc<RwLock<HashMap<ExecutionId, (u64, serde_json::Value)>>>,
    journal: Arc<RwLock<HashMap<ExecutionId, Vec<serde_json::Value>>>>,
    leases: Arc<RwLock<HashMap<ExecutionId, String>>>,
}

impl InMemoryExecutionRepo {
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

    async fn get_journal(&self, id: ExecutionId) -> Result<Vec<serde_json::Value>, ExecutionRepoError> {
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

    async fn release_lease(&self, id: ExecutionId, holder: &str) -> Result<bool, ExecutionRepoError> {
        let mut leases = self.leases.write().await;
        let ok = leases.get(&id).map(|h| h.as_str()) == Some(holder);
        if ok {
            leases.remove(&id);
        }
        Ok(ok)
    }
}
