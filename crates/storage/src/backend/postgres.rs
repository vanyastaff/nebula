//! Postgres-backed implementation of [`Storage`](crate::Storage).
//!
//! Uses a simple key-value table (`storage_kv` by default) with
//! `key TEXT PRIMARY KEY`, `value JSONB`, and `updated_at TIMESTAMPTZ`.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use nebula_core::WorkflowId;
use sqlx::{Pool, Postgres, postgres::PgPoolOptions, types::Json};

use crate::{
    StorageError,
    storage::Storage,
    workflow_repo::{WorkflowRepo, WorkflowRepoError},
};

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
            min_connections: 2,
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

    /// Returns a reference to the underlying connection pool.
    #[must_use]
    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }

    /// Run embedded SQLx migrations against the database.
    ///
    /// Uses `sqlx::migrate!()` which embeds migration files from the
    /// `./migrations` directory at compile time.
    pub async fn run_migrations(&self) -> Result<(), StorageError> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|err| StorageError::Backend(err.to_string()))
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
        let version = sqlx::query_scalar::<_, i64>("SELECT version FROM workflows WHERE id = $1")
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
            let result =
                sqlx::query("INSERT INTO workflows (id, version, definition) VALUES ($1, 1, $2)")
                    .bind(uuid)
                    .bind(json_def)
                    .execute(&self.pool)
                    .await;

            match result {
                Ok(_) => Ok(()),
                Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
                    // Unique violation — row already exists. Query current version for conflict.
                    let actual = self.query_current_version(uuid).await?;
                    Err(WorkflowRepoError::conflict(
                        "workflow",
                        id.to_string(),
                        version,
                        actual,
                    ))
                },
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
                let row =
                    sqlx::query_scalar::<_, i64>("SELECT version FROM workflows WHERE id = $1")
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

    async fn delete(&self, id: WorkflowId) -> Result<bool, WorkflowRepoError> {
        let uuid = sqlx::types::Uuid::from_bytes(*id.get().as_bytes());
        let result = sqlx::query("DELETE FROM workflows WHERE id = $1")
            .bind(uuid)
            .execute(&self.pool)
            .await
            .map_err(|err| WorkflowRepoError::Connection(err.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    async fn list(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(WorkflowId, serde_json::Value)>, WorkflowRepoError> {
        let rows = sqlx::query_as::<_, (sqlx::types::Uuid, Json<serde_json::Value>)>(
            "SELECT id, definition FROM workflows ORDER BY created_at, id LIMIT $1 OFFSET $2",
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| WorkflowRepoError::Connection(err.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|(uuid, definition)| (WorkflowId::from_bytes(*uuid.as_bytes()), definition.0))
            .collect())
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

    /// Helper: create a `PgWorkflowRepo` from `DATABASE_URL`, or skip the test.
    async fn pg_repo() -> Option<PgWorkflowRepo> {
        let url = std::env::var("DATABASE_URL").ok()?;
        let storage = PostgresStorage::new(url).await.expect("connect");
        Some(PgWorkflowRepo::new(storage.pool().clone()))
    }

    #[tokio::test]
    async fn pg_get_nonexistent_returns_none() {
        let Some(repo) = pg_repo().await else { return };
        let id = WorkflowId::new();
        let result = repo.get_with_version(id).await.expect("get");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn pg_save_and_get_roundtrip() {
        let Some(repo) = pg_repo().await else { return };
        let id = WorkflowId::new();
        let def = serde_json::json!({"name": "test", "nodes": []});

        repo.save(id, 0, def.clone()).await.expect("save v0");

        let (version, got) = repo.get_with_version(id).await.expect("get").expect("some");
        assert_eq!(version, 1);
        assert_eq!(got, def);

        // cleanup
        repo.delete(id).await.ok();
    }

    #[tokio::test]
    async fn pg_save_version_conflict() {
        let Some(repo) = pg_repo().await else { return };
        let id = WorkflowId::new();
        let def = serde_json::json!({"x": 1});

        repo.save(id, 0, def.clone()).await.expect("save v0");

        // Try saving with wrong version (0 again, but actual is 1)
        let err = repo.save(id, 0, def.clone()).await.unwrap_err();
        assert!(matches!(err, WorkflowRepoError::Conflict { .. }));

        // cleanup
        repo.delete(id).await.ok();
    }

    #[tokio::test]
    async fn pg_save_increments_version() {
        let Some(repo) = pg_repo().await else { return };
        let id = WorkflowId::new();

        for expected_version in 0u64..3 {
            let def = serde_json::json!({"v": expected_version});
            repo.save(id, expected_version, def).await.expect("save");
        }

        let (version, _) = repo.get_with_version(id).await.expect("get").expect("some");
        assert_eq!(version, 3);

        // cleanup
        repo.delete(id).await.ok();
    }

    #[tokio::test]
    async fn pg_delete_existing_returns_true() {
        let Some(repo) = pg_repo().await else { return };
        let id = WorkflowId::new();
        repo.save(id, 0, serde_json::json!({})).await.expect("save");
        assert!(repo.delete(id).await.expect("delete"));
    }

    #[tokio::test]
    async fn pg_delete_nonexistent_returns_false() {
        let Some(repo) = pg_repo().await else { return };
        let id = WorkflowId::new();
        assert!(!repo.delete(id).await.expect("delete"));
    }

    #[tokio::test]
    async fn pg_list_pagination() {
        let Some(repo) = pg_repo().await else { return };
        let ids: Vec<WorkflowId> = (0..3).map(|_| WorkflowId::new()).collect();
        for (i, &id) in ids.iter().enumerate() {
            repo.save(id, 0, serde_json::json!({"i": i}))
                .await
                .expect("save");
        }

        // List all with large limit
        let all = repo.list(0, 100).await.expect("list all");
        assert!(all.len() >= 3);

        // Verify offset/limit work
        let page = repo.list(0, 2).await.expect("list page");
        assert_eq!(page.len(), 2);

        let page2 = repo.list(2, 2).await.expect("list page2");
        assert!(!page2.is_empty());

        // cleanup
        for &id in &ids {
            repo.delete(id).await.ok();
        }
    }

    #[tokio::test]
    async fn pg_list_empty() {
        let Some(repo) = pg_repo().await else { return };
        // Use a very large offset to ensure empty result
        let result = repo.list(999_999, 10).await.expect("list");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn pg_jsonb_large_definition() {
        let Some(repo) = pg_repo().await else { return };
        let id = WorkflowId::new();

        // Build a large definition (~100KB)
        let nodes: Vec<serde_json::Value> = (0..500)
            .map(|i| serde_json::json!({"id": i, "name": format!("node_{i}"), "config": {"key": "x".repeat(100)}}))
            .collect();
        let def = serde_json::json!({"name": "large", "nodes": nodes});

        repo.save(id, 0, def.clone()).await.expect("save large");
        let (_, got) = repo.get_with_version(id).await.expect("get").expect("some");
        assert_eq!(got, def);

        // cleanup
        repo.delete(id).await.ok();
    }

    mod shared {
        use crate::workflow_repo_tests;
        workflow_repo_tests!(async {
            let url = std::env::var("DATABASE_URL")
                .expect("DATABASE_URL required for postgres shared tests");
            let storage = super::PostgresStorage::new(url).await.expect("connect");
            super::PgWorkflowRepo::new(storage.pool().clone())
        });
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

    #[tokio::test]
    async fn run_migrations_is_idempotent() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };

        let storage = PostgresStorage::new(url).await.expect("connect");
        storage.run_migrations().await.expect("first migration run");
        storage
            .run_migrations()
            .await
            .expect("second migration run should be idempotent");
    }

    #[tokio::test]
    async fn migrations_create_storage_kv_table() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };

        let storage = PostgresStorage::new(url).await.expect("connect");
        storage.run_migrations().await.expect("migrations");

        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_name = 'storage_kv'
            )",
        )
        .fetch_one(storage.pool())
        .await
        .expect("query information_schema");

        assert!(exists, "storage_kv table should exist after migrations");

        // Verify expected columns
        let columns: Vec<String> = sqlx::query_scalar(
            "SELECT column_name::text FROM information_schema.columns
             WHERE table_name = 'storage_kv' ORDER BY ordinal_position",
        )
        .fetch_all(storage.pool())
        .await
        .expect("query columns");

        assert_eq!(columns, vec!["key", "value", "created_at", "updated_at"]);
    }

    #[tokio::test]
    async fn migrations_create_workflows_table() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };

        let storage = PostgresStorage::new(url).await.expect("connect");
        storage.run_migrations().await.expect("migrations");

        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_name = 'workflows'
            )",
        )
        .fetch_one(storage.pool())
        .await
        .expect("query information_schema");

        assert!(exists, "workflows table should exist after migrations");

        let columns: Vec<String> = sqlx::query_scalar(
            "SELECT column_name::text FROM information_schema.columns
             WHERE table_name = 'workflows' ORDER BY ordinal_position",
        )
        .fetch_all(storage.pool())
        .await
        .expect("query columns");

        assert_eq!(
            columns,
            vec!["id", "version", "definition", "created_at", "updated_at"]
        );
    }

    #[tokio::test]
    async fn crud_roundtrip_after_migrations() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };

        let storage = PostgresStorage::new(url).await.expect("connect");
        storage.run_migrations().await.expect("migrations");

        let key = "storage:test:post_migration".to_string();
        let value = serde_json::json!({"migrated": true, "count": 42});

        // Clean up any leftover from previous test runs
        let _ = storage.delete(&key).await;

        storage.set(&key, &value).await.expect("set");
        assert!(storage.exists(&key).await.expect("exists"));
        assert_eq!(storage.get(&key).await.expect("get"), Some(value));

        // Update in place (upsert)
        let updated = serde_json::json!({"migrated": true, "count": 99});
        storage.set(&key, &updated).await.expect("upsert");
        assert_eq!(storage.get(&key).await.expect("get updated"), Some(updated));

        storage.delete(&key).await.expect("delete");
        assert!(!storage.exists(&key).await.expect("exists after delete"));
    }
}
