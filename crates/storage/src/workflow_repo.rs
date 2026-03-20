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

/// Shared test suite for any [`WorkflowRepo`] implementation.
///
/// Call [`workflow_repo_tests!`] with a zero-arg async factory that returns an
/// `impl WorkflowRepo`. Each test gets a fresh repo instance.
#[cfg(test)]
#[macro_export]
macro_rules! workflow_repo_tests {
    ($factory:expr) => {
        use $crate::workflow_repo::{WorkflowRepo, WorkflowRepoError};
        use nebula_core::WorkflowId;

        #[tokio::test]
        async fn save_and_get() {
            let repo = $factory.await;
            let id = WorkflowId::new();
            let def = serde_json::json!({"name": "test-wf", "nodes": [1, 2, 3]});

            repo.save(id, 0, def.clone()).await.expect("save v0");

            let (version, got) = repo
                .get_with_version(id)
                .await
                .expect("get")
                .expect("should exist");
            assert_eq!(version, 1);
            assert_eq!(got, def);

            // Also check get() default method
            let got2 = repo.get(id).await.expect("get").expect("should exist");
            assert_eq!(got2, def);

            // Non-existent id returns None
            let missing = repo.get(WorkflowId::new()).await.expect("get missing");
            assert!(missing.is_none());

            // cleanup
            repo.delete(id).await.ok();
        }

        #[tokio::test]
        async fn optimistic_concurrency() {
            let repo = $factory.await;
            let id = WorkflowId::new();
            let def = serde_json::json!({"v": 0});

            repo.save(id, 0, def.clone()).await.expect("save v0");

            // Stale write with version 0 (actual is 1) must fail
            let err = repo.save(id, 0, def.clone()).await.unwrap_err();
            match err {
                WorkflowRepoError::Conflict {
                    expected_version,
                    actual_version,
                    ..
                } => {
                    assert_eq!(expected_version, 0);
                    assert_eq!(actual_version, 1);
                }
                other => panic!("expected Conflict, got: {other}"),
            }

            // Correct version succeeds
            repo.save(id, 1, serde_json::json!({"v": 1}))
                .await
                .expect("save v1");

            // cleanup
            repo.delete(id).await.ok();
        }

        #[tokio::test]
        async fn delete_semantics() {
            let repo = $factory.await;
            let id = WorkflowId::new();

            // Delete non-existent returns false
            assert!(!repo.delete(id).await.expect("delete missing"));

            // Delete existing returns true
            repo.save(id, 0, serde_json::json!({})).await.expect("save");
            assert!(repo.delete(id).await.expect("delete existing"));

            // After delete, get returns None
            assert!(repo.get(id).await.expect("get after delete").is_none());

            // Double-delete returns false
            assert!(!repo.delete(id).await.expect("double delete"));
        }

        #[tokio::test]
        async fn list_ordering() {
            let repo = $factory.await;
            let ids: Vec<WorkflowId> = (0..5).map(|_| WorkflowId::new()).collect();
            for (i, &id) in ids.iter().enumerate() {
                repo.save(id, 0, serde_json::json!({"i": i}))
                    .await
                    .expect("save");
            }

            // Full list
            let all = repo.list(0, 100).await.expect("list all");
            assert!(all.len() >= 5);

            // Pagination: pages should not overlap
            let page1 = repo.list(0, 3).await.expect("page1");
            let page2 = repo.list(3, 3).await.expect("page2");
            assert_eq!(page1.len(), 3);
            assert!(!page2.is_empty());
            for (id, _) in &page1 {
                assert!(!page2.iter().any(|(pid, _)| pid == id), "overlap detected");
            }

            // cleanup
            for &id in &ids {
                repo.delete(id).await.ok();
            }
        }

        #[tokio::test]
        async fn version_lifecycle() {
            let repo = $factory.await;
            let id = WorkflowId::new();

            // v0 -> 1
            repo.save(id, 0, serde_json::json!({"step": 0}))
                .await
                .expect("v0");
            let (v, _) = repo.get_with_version(id).await.unwrap().unwrap();
            assert_eq!(v, 1);

            // v1 -> 2
            repo.save(id, 1, serde_json::json!({"step": 1}))
                .await
                .expect("v1");
            let (v, _) = repo.get_with_version(id).await.unwrap().unwrap();
            assert_eq!(v, 2);

            // v2 -> 3
            repo.save(id, 2, serde_json::json!({"step": 2}))
                .await
                .expect("v2");
            let (v, def) = repo.get_with_version(id).await.unwrap().unwrap();
            assert_eq!(v, 3);
            assert_eq!(def, serde_json::json!({"step": 2}));

            // cleanup
            repo.delete(id).await.ok();
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    mod in_memory {
        workflow_repo_tests!(async { super::InMemoryWorkflowRepo::new() });
    }
}
