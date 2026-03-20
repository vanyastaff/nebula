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

        assert_eq!(columns, vec!["key", "value", "updated_at"]);
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
