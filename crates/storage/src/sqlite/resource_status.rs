//! SQLite persistence for cross-process resource runtime status, over
//! `resource_status_heartbeats` and `resource_status_snapshots`.
//!
//! Every expiry comparison reads SQLite's own clock inside the statement, so
//! a skewed worker clock can neither keep a dead worker live nor expire a
//! live one early. Instants are INTEGER microseconds.
//!
//! A snapshot belongs to its stored resource: publish checks the resource is
//! live inside its `BEGIN IMMEDIATE` transaction (`NotFound` when it is
//! missing or archived), purging the resource purges its snapshots, and an
//! archived resource has no live status.

use std::time::Duration;

use nebula_storage_port::dto::{LiveResourceStatus, ResourceStatusSnapshot, StatusWorkerId};
use nebula_storage_port::store::ResourceStatusStore;
use nebula_storage_port::{Scope, StorageError};
use sqlx::{Row as _, SqlitePool};

use crate::resource_status::{
    HEARTBEAT_RETENTION_MS, decode_live, heartbeat_ttl_ms, row_version_to_stored,
};
use crate::sql_error::storage_error;

/// SQLite's clock as INTEGER microseconds since the Unix epoch.
const NOW_MICROS: &str = "CAST((julianday('now') - 2440587.5) * 86400000000.0 AS INTEGER)";

/// SQLite implementation of [`ResourceStatusStore`].
#[derive(Clone, Debug)]
pub struct SqliteResourceStatusStore {
    pool: SqlitePool,
}

impl SqliteResourceStatusStore {
    /// Wrap a pool initialized through [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ResourceStatusStore for SqliteResourceStatusStore {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "heartbeat"))]
    async fn heartbeat(&self, worker: &StatusWorkerId, ttl: Duration) -> Result<(), StorageError> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO resource_status_heartbeats (worker_id, expires_at) \
             VALUES (?, {NOW_MICROS} + ? * 1000) \
             ON CONFLICT (worker_id) DO UPDATE SET expires_at = excluded.expires_at"
        )))
        .bind(worker.as_str())
        .bind(heartbeat_ttl_ms(ttl))
        .execute(&mut *transaction)
        .await
        .map_err(storage_error)?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM resource_status_snapshots WHERE worker_id IN \
             (SELECT worker_id FROM resource_status_heartbeats \
              WHERE expires_at < {NOW_MICROS} - ? * 1000)"
        )))
        .bind(HEARTBEAT_RETENTION_MS)
        .execute(&mut *transaction)
        .await
        .map_err(storage_error)?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM resource_status_heartbeats WHERE expires_at < {NOW_MICROS} - ? * 1000"
        )))
        .bind(HEARTBEAT_RETENTION_MS)
        .execute(&mut *transaction)
        .await
        .map_err(storage_error)?;
        transaction.commit().await.map_err(storage_error)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "publish"))]
    async fn publish(
        &self,
        scope: &Scope,
        worker: &StatusWorkerId,
        snapshot: &ResourceStatusSnapshot,
    ) -> Result<(), StorageError> {
        let row_version = row_version_to_stored(snapshot.row_version)?;
        // `BEGIN IMMEDIATE` serializes the live-resource check with a
        // concurrent archive (the foreign key proves existence only).
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        let live: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM resources \
             WHERE org_id = ? AND workspace_id = ? AND id = ? AND deleted_at IS NULL",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&snapshot.resource_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage_error)?;
        if live.is_none() {
            return Err(StorageError::not_found(
                "resource",
                snapshot.resource_id.clone(),
            ));
        }
        sqlx::query(
            "INSERT INTO resource_status_snapshots (org_id, workspace_id, resource_id, \
             worker_id, phase, healthy, accepting, row_version) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (org_id, workspace_id, resource_id, worker_id) DO UPDATE SET \
             phase = excluded.phase, healthy = excluded.healthy, \
             accepting = excluded.accepting, row_version = excluded.row_version",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&snapshot.resource_id)
        .bind(worker.as_str())
        .bind(snapshot.phase.as_str())
        .bind(snapshot.healthy)
        .bind(snapshot.accepting)
        .bind(row_version)
        .execute(&mut *transaction)
        .await
        .map_err(storage_error)?;
        transaction.commit().await.map_err(storage_error)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "withdraw"))]
    async fn withdraw(
        &self,
        scope: &Scope,
        worker: &StatusWorkerId,
        resource_id: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "DELETE FROM resource_status_snapshots \
             WHERE org_id = ? AND workspace_id = ? AND resource_id = ? AND worker_id = ?",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(resource_id)
        .bind(worker.as_str())
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "withdraw_worker"))]
    async fn withdraw_worker(&self, worker: &StatusWorkerId) -> Result<(), StorageError> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        sqlx::query("DELETE FROM resource_status_snapshots WHERE worker_id = ?")
            .bind(worker.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(storage_error)?;
        sqlx::query("DELETE FROM resource_status_heartbeats WHERE worker_id = ?")
            .bind(worker.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(storage_error)?;
        transaction.commit().await.map_err(storage_error)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "live_for"))]
    async fn live_for(
        &self,
        scope: &Scope,
        resource_id: &str,
    ) -> Result<Vec<LiveResourceStatus>, StorageError> {
        // An archived resource has no live status.
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT s.worker_id, s.phase, s.healthy, s.accepting, s.row_version \
             FROM resource_status_snapshots s \
             JOIN resource_status_heartbeats h ON h.worker_id = s.worker_id \
             JOIN resources r ON r.org_id = s.org_id AND r.workspace_id = s.workspace_id \
               AND r.id = s.resource_id AND r.deleted_at IS NULL \
             WHERE s.org_id = ? AND s.workspace_id = ? AND s.resource_id = ? \
               AND h.expires_at > {NOW_MICROS} \
             ORDER BY s.worker_id"
        )))
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(resource_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;
        rows.iter()
            .map(|row| {
                decode_live(
                    resource_id,
                    row.try_get("worker_id").map_err(storage_error)?,
                    row.try_get("phase").map_err(storage_error)?,
                    row.try_get("healthy").map_err(storage_error)?,
                    row.try_get("accepting").map_err(storage_error)?,
                    row.try_get("row_version").map_err(storage_error)?,
                )
            })
            .collect()
    }
}
