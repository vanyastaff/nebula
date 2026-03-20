//! Postgres-backed implementation of [`Storage`](crate::Storage).
//!
//! Uses a simple key-value table (`storage_kv` by default) with
//! `key TEXT PRIMARY KEY`, `value JSONB`, and `updated_at TIMESTAMPTZ`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Json;
use sqlx::{Pool, Postgres};

use crate::StorageError;
use crate::storage::Storage;
use crate::workflow_repo::{WorkflowRepo, WorkflowRepoError};
use nebula_core::WorkflowId;

/// Configuration for [`PostgresStorage`].
#[derive(Clone, Debug)]
pub struct PostgresStorageConfig {
    /// PostgreSQL connection string (`postgres://user:pass@host:port/db`).
    pub connection_string: String,
    /// Table name for key-value storage (default: `storage_kv`).
    pub table: String,
    /// Maximum number of connections in the pool.
    pub max_connections: u32,
    /// Minimum number of connections in the pool.
    pub min_connections: u32,
    /// Reserved for future tuning (currently unused).
    pub connect_timeout: Duration,
}

impl Default for PostgresStorageConfig {
    fn default() -> Self {
        Self {
            connection_string: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://nebula:nebula@localhost:5432/nebula".to_string()),
            table: "storage_kv".to_string(),
            max_connections: 10,
            min_connections: 1,
            connect_timeout: Duration::from_secs(5),
        }
    }
}

impl PostgresStorageConfig {
    /// Build config from a connection string using sensible defaults.
    #[must_use]
    pub fn from_connection_string(connection_string: impl Into<String>) -> Self {
        Self {
            connection_string: connection_string.into(),
            ..Self::default()
        }
    }
}

/// Postgres-backed key-value storage.
#[derive(Clone)]
pub struct PostgresStorage {
    pool: Pool<Postgres>,
    select_sql: Arc<str>,
    insert_sql: Arc<str>,
    delete_sql: Arc<str>,
    exists_sql: Arc<str>,
}

impl PostgresStorage {
    /// Create a new [`PostgresStorage`] from a connection string.
    ///
    /// The caller is responsible for running SQL migrations that create
    /// the `storage_kv` table (or the configured table name).
    pub async fn new(connection_string: impl Into<String>) -> Result<Self, StorageError> {
        Self::with_config(PostgresStorageConfig::from_connection_string(
            connection_string,
        ))
        .await
    }

    /// Create a new [`PostgresStorage`] using explicit configuration.
    pub async fn with_config(config: PostgresStorageConfig) -> Result<Self, StorageError> {
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(config.connect_timeout)
            .connect(&config.connection_string)
            .await
            .map_err(|err| StorageError::Backend(err.to_string()))?;

        let table = &config.table;
        let select_sql = format!("SELECT value FROM {} WHERE key = $1", table);
        let insert_sql = format!(
            "INSERT INTO {} (key, value) \
             VALUES ($1, $2) \
             ON CONFLICT (key) DO UPDATE \
             SET value = EXCLUDED.value, updated_at = NOW()",
            table
        );
        let delete_sql = format!("DELETE FROM {} WHERE key = $1", table);
        let exists_sql = format!("SELECT 1 FROM {} WHERE key = $1", table);

        Ok(Self {
            pool,
            select_sql: Arc::from(select_sql),
            insert_sql: Arc::from(insert_sql),
            delete_sql: Arc::from(delete_sql),
            exists_sql: Arc::from(exists_sql),
        })
    }
}

/// Postgres-backed workflow repository.
#[derive(Clone)]
pub struct PgWorkflowRepo {
    pool: Pool<Postgres>,
}

impl PgWorkflowRepo {
    /// Create a new [`PgWorkflowRepo`] from an existing connection pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    /// Query the current version of a workflow by UUID.
    async fn query_current_version(
        &self,
        uuid: sqlx::types::Uuid,
    ) -> Result<u64, WorkflowRepoError> {
        let version = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM workflows WHERE id = $1",
        )
        .bind(uuid)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| WorkflowRepoError::Connection(err.to_string()))?;

        Ok(version as u64)
    }
}

#[async_trait]
impl Storage for PostgresStorage {
    type Key = String;
    type Value = serde_json::Value;

    async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, StorageError> {
        let row = sqlx::query_scalar::<_, Json<serde_json::Value>>(&self.select_sql)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| StorageError::Backend(err.to_string()))?;

        Ok(row.map(|v| v.0))
    }

    async fn set(&self, key: &Self::Key, value: &Self::Value) -> Result<(), StorageError> {
        sqlx::query(&self.insert_sql)
            .bind(key)
            .bind(Json(value.clone()))
            .execute(&self.pool)
            .await
            .map_err(|err| StorageError::Backend(err.to_string()))?;

        Ok(())
    }

    async fn delete(&self, key: &Self::Key) -> Result<(), StorageError> {
        sqlx::query(&self.delete_sql)
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|err| StorageError::Backend(err.to_string()))?;

        Ok(())
    }

    async fn exists(&self, key: &Self::Key) -> Result<bool, StorageError> {
        let row = sqlx::query_scalar::<_, i32>(&self.exists_sql)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| StorageError::Backend(err.to_string()))?;

        Ok(row.is_some())
    }
}

#[async_trait]
impl WorkflowRepo for PgWorkflowRepo {
    async fn get_with_version(
        &self,
        id: WorkflowId,
    ) -> Result<Option<(u64, serde_json::Value)>, WorkflowRepoError> {
        let row = sqlx::query_as::<_, (i64, Json<serde_json::Value>)>(
            "SELECT version, definition FROM workflows WHERE id = $1",
        )
        .bind(sqlx::types::Uuid::from_bytes(*id.get().as_bytes()))
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| WorkflowRepoError::Connection(err.to_string()))?;

        Ok(row.map(|(version, definition)| (version as u64, definition.0)))
    }

    async fn save(
        &self,
        id: WorkflowId,
        version: u64,
        definition: serde_json::Value,
    ) -> Result<(), WorkflowRepoError> {
        let uuid = sqlx::types::Uuid::from_bytes(*id.get().as_bytes());
        let json_def = Json(&definition);

        if version == 0 {
            // New workflow: INSERT with version 1.
            let result = sqlx::query(
                "INSERT INTO workflows (id, version, definition) VALUES ($1, 1, $2)",
            )
            .bind(uuid)
            .bind(json_def)
            .execute(&self.pool)
            .await;

            match result {
                Ok(_) => Ok(()),
                Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
                    // Unique violation — row already exists. Query current version for conflict.
                    let actual = self.query_current_version(uuid).await?;
                    Err(WorkflowRepoError::conflict("workflow", id.to_string(), version, actual))
                }
                Err(err) => Err(WorkflowRepoError::Connection(err.to_string())),
            }
        } else {
            // Existing workflow: CAS update.
            let result = sqlx::query(
                "UPDATE workflows SET version = $3 + 1, definition = $2, updated_at = NOW() \
                 WHERE id = $1 AND version = $3",
            )
            .bind(uuid)
            .bind(json_def)
            .bind(version as i64)
            .execute(&self.pool)
            .await
            .map_err(|err| WorkflowRepoError::Connection(err.to_string()))?;

            if result.rows_affected() == 0 {
                // Either wrong version or row missing.
                let row = sqlx::query_scalar::<_, i64>(
                    "SELECT version FROM workflows WHERE id = $1",
                )
                .bind(uuid)
                .fetch_optional(&self.pool)
                .await
                .map_err(|err| WorkflowRepoError::Connection(err.to_string()))?;

                match row {
                    Some(actual) => Err(WorkflowRepoError::conflict(
                        "workflow",
                        id.to_string(),
                        version,
                        actual as u64,
                    )),
                    None => Err(WorkflowRepoError::Internal(format!(
                        "workflow {} not found during save",
                        id,
                    ))),
                }
            } else {
                Ok(())
            }
        }
    }

    async fn delete(&self, _id: WorkflowId) -> Result<bool, WorkflowRepoError> {
        todo!()
    }

    async fn list(
        &self,
        _offset: usize,
        _limit: usize,
    ) -> Result<Vec<(WorkflowId, serde_json::Value)>, WorkflowRepoError> {
        todo!()
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn postgres_new_from_connection_string() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };

        let storage = PostgresStorage::new(url).await;
        assert!(storage.is_ok());
    }

    #[tokio::test]
    async fn postgres_get_set_delete_exists_roundtrip() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };

        let storage = PostgresStorage::new(url).await.expect("connect");
        let key = "storage:test:roundtrip".to_string();
        let value = serde_json::json!({"a": 1, "b": "ok"});

        storage.set(&key, &value).await.expect("set");
        assert!(storage.exists(&key).await.expect("exists"));
        assert_eq!(storage.get(&key).await.expect("get"), Some(value.clone()));

        storage.delete(&key).await.expect("delete");
        assert!(!storage.exists(&key).await.expect("exists false"));
        assert_eq!(storage.get(&key).await.expect("get none"), None);
    }
}
