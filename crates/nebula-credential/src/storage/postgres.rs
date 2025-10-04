//! PostgreSQL-based state storage implementation
//!
//! This module is only available with the `storage-postgres` feature.

#![cfg(feature = "storage-postgres")]

use crate::core::CredentialError;
use crate::traits::{StateStore, StateVersion};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::sync::Arc;

/// PostgreSQL implementation of StateStore
pub struct PostgresStateStore {
    pool: PgPool,
    config: PostgresConfig,
}

/// Configuration for PostgreSQL storage
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    /// Table name
    pub table_name: String,
    /// Schema name  
    pub schema: String,
    /// Enable soft delete
    pub soft_delete: bool,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            table_name: "credential_states".to_string(),
            schema: "public".to_string(),
            soft_delete: true,
        }
    }
}

impl PostgresStateStore {
    /// Create new store
    pub fn new(pool: PgPool, config: PostgresConfig) -> Arc<Self> {
        Arc::new(Self { pool, config })
    }

    /// Create with defaults
    pub fn with_pool(pool: PgPool) -> Arc<Self> {
        Self::new(pool, PostgresConfig::default())
    }

    fn table_name(&self) -> String {
        format!("{}.{}", self.config.schema, self.config.table_name)
    }
}

#[async_trait::async_trait]
impl StateStore for PostgresStateStore {
    async fn load(&self, id: &str) -> Result<(Value, StateVersion), CredentialError> {
        let table = self.table_name();
        let query = if self.config.soft_delete {
            format!(
                "SELECT state_data, version FROM {} WHERE id = $1 AND deleted_at IS NULL",
                table
            )
        } else {
            format!("SELECT state_data, version FROM {} WHERE id = $1", table)
        };

        let row = sqlx::query(&query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| CredentialError::StorageFailed {
                operation: "load".to_string(),
                reason: e.to_string(),
            })?
            .ok_or_else(|| CredentialError::not_found(id))?;

        let state_data: Value = row
            .try_get("state_data")
            .map_err(|e| CredentialError::DeserializationFailed(e.to_string()))?;

        let version: i32 = row
            .try_get("version")
            .map_err(|e| CredentialError::StorageFailed {
                operation: "load_version".to_string(),
                reason: e.to_string(),
            })?;

        Ok((state_data, StateVersion(version as u64)))
    }

    async fn save(
        &self,
        id: &str,
        version: StateVersion,
        state: &Value,
    ) -> Result<StateVersion, CredentialError> {
        let table = self.table_name();

        let update_query = format!(
            "UPDATE {} SET state_data = $1, version = version + 1, updated_at = NOW() WHERE id = $2 AND version = $3 RETURNING version",
            table
        );

        let result = sqlx::query(&update_query)
            .bind(state)
            .bind(id)
            .bind(version.0 as i32)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| CredentialError::StorageFailed {
                operation: "save".to_string(),
                reason: e.to_string(),
            })?;

        if let Some(row) = result {
            let new_version: i32 =
                row.try_get("version")
                    .map_err(|e| CredentialError::StorageFailed {
                        operation: "get_version".to_string(),
                        reason: e.to_string(),
                    })?;
            return Ok(StateVersion(new_version as u64));
        }

        let insert_query = format!(
            "INSERT INTO {} (id, state_data, version) VALUES ($1, $2, 1) RETURNING version",
            table
        );

        let row = sqlx::query(&insert_query)
            .bind(id)
            .bind(state)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| CredentialError::StorageFailed {
                operation: "insert".to_string(),
                reason: e.to_string(),
            })?;

        let v: i32 = row
            .try_get("version")
            .map_err(|e| CredentialError::StorageFailed {
                operation: "get_version".to_string(),
                reason: e.to_string(),
            })?;
        Ok(StateVersion(v as u64))
    }

    async fn delete(&self, id: &str) -> Result<(), CredentialError> {
        let table = self.table_name();

        let query = if self.config.soft_delete {
            format!(
                "UPDATE {} SET deleted_at = NOW() WHERE id = $1 AND deleted_at IS NULL",
                table
            )
        } else {
            format!("DELETE FROM {} WHERE id = $1", table)
        };

        let result = sqlx::query(&query)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| CredentialError::StorageFailed {
                operation: "delete".to_string(),
                reason: e.to_string(),
            })?;

        if result.rows_affected() == 0 {
            return Err(CredentialError::not_found(id));
        }

        Ok(())
    }

    async fn exists(&self, id: &str) -> Result<bool, CredentialError> {
        let table = self.table_name();

        let query = if self.config.soft_delete {
            format!(
                "SELECT 1 FROM {} WHERE id = $1 AND deleted_at IS NULL LIMIT 1",
                table
            )
        } else {
            format!("SELECT 1 FROM {} WHERE id = $1 LIMIT 1", table)
        };

        let result = sqlx::query(&query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| CredentialError::StorageFailed {
                operation: "exists".to_string(),
                reason: e.to_string(),
            })?;

        Ok(result.is_some())
    }

    async fn list(&self) -> Result<Vec<String>, CredentialError> {
        let table = self.table_name();

        let query = if self.config.soft_delete {
            format!(
                "SELECT id FROM {} WHERE deleted_at IS NULL ORDER BY created_at DESC",
                table
            )
        } else {
            format!("SELECT id FROM {} ORDER BY created_at DESC", table)
        };

        let rows = sqlx::query(&query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| CredentialError::StorageFailed {
                operation: "list".to_string(),
                reason: e.to_string(),
            })?;

        let ids = rows
            .into_iter()
            .map(|row| {
                row.try_get("id")
                    .map_err(|e| CredentialError::StorageFailed {
                        operation: "get_id".to_string(),
                        reason: e.to_string(),
                    })
            })
            .collect::<Result<Vec<String>, _>>()?;

        Ok(ids)
    }
}
