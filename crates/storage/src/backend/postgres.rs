//! Postgres-backed implementation of [`Storage`](crate::Storage).
//!
//! Uses a simple key-value table (`storage_kv` by default) with
//! `key TEXT PRIMARY KEY`, `value BYTEA`, and `updated_at TIMESTAMPTZ`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres};

use crate::storage::Storage;
use crate::StorageError;

/// Configuration for [`PostgresStorage`].
#[derive(Clone, Debug)]
pub struct PostgresStorageConfig {
    /// PostgreSQL connection string (`postgres://user:pass@host:port/db`).
    pub database_url: String,
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
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://nebula:nebula@localhost:5432/nebula".to_string()),
            table: "storage_kv".to_string(),
            max_connections: 10,
            min_connections: 1,
            connect_timeout: Duration::from_secs(5),
        }
    }
}

/// Postgres-backed key-value storage.
#[derive(Clone)]
pub struct PostgresStorage {
    pool: Pool<Postgres>,
    table: Arc<str>,
}

impl PostgresStorage {
    /// Create a new [`PostgresStorage`] using the given configuration.
    ///
    /// The caller is responsible for running SQL migrations that create
    /// the `storage_kv` table (or the configured table name).
    pub async fn new(config: PostgresStorageConfig) -> Result<Self, StorageError> {
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .connect(&config.database_url)
            .await
            .map_err(|err| StorageError::Backend(err.to_string()))?;

        Ok(Self {
            pool,
            table: Arc::from(config.table),
        })
    }

    fn select_sql(&self) -> String {
        format!("SELECT value FROM {} WHERE key = $1", self.table)
    }

    fn insert_sql(&self) -> String {
        format!(
            "INSERT INTO {} (key, value) \
             VALUES ($1, $2) \
             ON CONFLICT (key) DO UPDATE \
             SET value = EXCLUDED.value, updated_at = NOW()",
            self.table
        )
    }

    fn delete_sql(&self) -> String {
        format!("DELETE FROM {} WHERE key = $1", self.table)
    }

    fn exists_sql(&self) -> String {
        format!("SELECT 1 FROM {} WHERE key = $1", self.table)
    }
}

#[async_trait]
impl Storage for PostgresStorage {
    type Key = String;
    type Value = Vec<u8>;

    async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, StorageError> {
        let row = sqlx::query_scalar::<_, Vec<u8>>(&self.select_sql())
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| StorageError::Backend(err.to_string()))?;

        Ok(row)
    }

    async fn set(&self, key: &Self::Key, value: &Self::Value) -> Result<(), StorageError> {
        sqlx::query(&self.insert_sql())
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await
            .map_err(|err| StorageError::Backend(err.to_string()))?;

        Ok(())
    }

    async fn delete(&self, key: &Self::Key) -> Result<(), StorageError> {
        sqlx::query(&self.delete_sql())
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|err| StorageError::Backend(err.to_string()))?;

        Ok(())
    }

    async fn exists(&self, key: &Self::Key) -> Result<bool, StorageError> {
        let row = sqlx::query_scalar::<_, i32>(&self.exists_sql())
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| StorageError::Backend(err.to_string()))?;

        Ok(row.is_some())
    }
}

