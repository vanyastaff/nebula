//! Persistence interface for workflow definitions.
//!
//! Used by API and app; implementations (in-memory, Postgres) live in this crate or in adapters.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use nebula_core::WorkflowId;
use thiserror::Error;
use tokio::sync::RwLock;

/// Errors returned by workflow repository operations.
#[derive(Debug, Error)]
pub enum WorkflowRepoError {
    /// Requested entity is absent in storage.
    #[error("{entity} not found: {id}")]
    NotFound {
        /// Entity kind (for example: `workflow`).
        entity: String,
        /// Entity identifier.
        id: String,
    },

    /// Optimistic concurrency check failed.
    #[error("{entity} {id}: expected version {expected_version}, got {actual_version}")]
    Conflict {
        /// Entity kind (for example: `workflow`).
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

    /// Unexpected internal repository failure.
    #[error("internal error: {0}")]
    Internal(String),
}

impl WorkflowRepoError {
    /// Builds a [`WorkflowRepoError::NotFound`].
    pub fn not_found(entity: impl Into<String>, id: impl Into<String>) -> Self {
        Self::NotFound {
            entity: entity.into(),
            id: id.into(),
        }
    }

    /// Builds a [`WorkflowRepoError::Conflict`].
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
}

/// Persistence interface for workflow definitions.
#[async_trait]
pub trait WorkflowRepo: Send + Sync {
    /// Get workflow definition and current version by ID.
    async fn get_with_version(
        &self,
        id: WorkflowId,
    ) -> Result<Option<(u64, serde_json::Value)>, WorkflowRepoError>;

    /// Get a workflow definition by ID.
    async fn get(&self, id: WorkflowId) -> Result<Option<serde_json::Value>, WorkflowRepoError> {
        Ok(self
            .get_with_version(id)
            .await?
            .map(|(_, definition)| definition))
    }

    /// Save with optimistic concurrency (version = expected current).
    async fn save(
        &self,
        id: WorkflowId,
        version: u64,
        definition: serde_json::Value,
    ) -> Result<(), WorkflowRepoError>;

    /// Delete by ID. Returns true if it existed.
    async fn delete(&self, id: WorkflowId) -> Result<bool, WorkflowRepoError>;

    /// List with pagination.
    async fn list(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(WorkflowId, serde_json::Value)>, WorkflowRepoError>;
}

/// In-memory workflow repository for tests and desktop/single-process.
#[derive(Default)]
pub struct InMemoryWorkflowRepo {
    definitions: Arc<RwLock<HashMap<WorkflowId, serde_json::Value>>>,
    versions: Arc<RwLock<HashMap<WorkflowId, u64>>>,
}

impl InMemoryWorkflowRepo {
    /// Creates empty in-memory repository.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl WorkflowRepo for InMemoryWorkflowRepo {
    async fn get_with_version(
        &self,
        id: WorkflowId,
    ) -> Result<Option<(u64, serde_json::Value)>, WorkflowRepoError> {
        let definitions = self.definitions.read().await;
        let Some(definition) = definitions.get(&id).cloned() else {
            return Ok(None);
        };
        let versions = self.versions.read().await;
        let version = versions.get(&id).copied().unwrap_or(0);
        Ok(Some((version, definition)))
    }

    async fn save(
        &self,
        id: WorkflowId,
        version: u64,
        definition: serde_json::Value,
    ) -> Result<(), WorkflowRepoError> {
        let mut versions = self.versions.write().await;
        let current = versions.get(&id).copied().unwrap_or(0);
        if current != version {
            return Err(WorkflowRepoError::conflict(
                "workflow",
                id.to_string(),
                version,
                current,
            ));
        }
        versions.insert(id, current + 1);
        self.definitions.write().await.insert(id, definition);
        Ok(())
    }

    async fn delete(&self, id: WorkflowId) -> Result<bool, WorkflowRepoError> {
        self.versions.write().await.remove(&id);
        Ok(self.definitions.write().await.remove(&id).is_some())
    }

    async fn list(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(WorkflowId, serde_json::Value)>, WorkflowRepoError> {
        let map = self.definitions.read().await;
        let mut rows: Vec<(WorkflowId, serde_json::Value)> =
            map.iter().map(|(id, value)| (*id, value.clone())).collect();
        rows.sort_by_key(|(id, _)| id.to_string());
        Ok(rows.into_iter().skip(offset).take(limit).collect())
    }
}
