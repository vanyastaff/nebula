//! Persistence and coordination interface for workflow executions.
//!
//! State (versioned CAS), journal (append-only), leases. Used by API and engine;
//! implementations in this crate or in adapters.

use std::time::Duration;

use async_trait::async_trait;
use nebula_core::ExecutionId;
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
