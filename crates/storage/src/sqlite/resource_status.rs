//! SQLite persistence for cross-process resource runtime status.
//!
//! Every expiry comparison reads SQLite's own clock inside the statement, so
//! a skewed worker clock can neither keep a dead worker live nor expire a
//! live one early.

use std::time::Duration;

use nebula_storage_port::dto::{LiveResourceStatus, ResourceStatusSnapshot, StatusWorkerId};
use nebula_storage_port::store::ResourceStatusStore;
use nebula_storage_port::{Scope, StorageError};
use sqlx::{Row as _, SqlitePool};

use super::resource_runtime::{commit_unknown, unavailable};
use crate::resource_status::{
    HEARTBEAT_RETENTION_MS, decode_live, heartbeat_ttl_ms, row_version_to_stored,
};

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
            .map_err(unavailable)?;
        sqlx::query("INSERT INTO port_worker_heartbeats (worker_id, expires_at_ms) VALUES (?, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) + ?) ON CONFLICT (worker_id) DO UPDATE SET expires_at_ms = excluded.expires_at_ms")
            .bind(worker.as_str())
            .bind(heartbeat_ttl_ms(ttl))
            .execute(&mut *transaction)
            .await
            .map_err(unavailable)?;
        sqlx::query("DELETE FROM port_resource_status WHERE worker_id IN (SELECT worker_id FROM port_worker_heartbeats WHERE expires_at_ms < CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) - ?)")
            .bind(HEARTBEAT_RETENTION_MS)
            .execute(&mut *transaction)
            .await
            .map_err(unavailable)?;
        sqlx::query("DELETE FROM port_worker_heartbeats WHERE expires_at_ms < CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) - ?")
            .bind(HEARTBEAT_RETENTION_MS)
            .execute(&mut *transaction)
            .await
            .map_err(unavailable)?;
        transaction.commit().await.map_err(commit_unknown)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "publish"))]
    async fn publish(
        &self,
        scope: &Scope,
        worker: &StatusWorkerId,
        snapshot: &ResourceStatusSnapshot,
    ) -> Result<(), StorageError> {
        let row_version = row_version_to_stored(snapshot.row_version)?;
        sqlx::query("INSERT INTO port_resource_status (workspace_id, org_id, resource_id, worker_id, phase, healthy, accepting, row_version) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (workspace_id, org_id, resource_id, worker_id) DO UPDATE SET phase = excluded.phase, healthy = excluded.healthy, accepting = excluded.accepting, row_version = excluded.row_version")
            .bind(&scope.workspace_id)
            .bind(&scope.org_id)
            .bind(&snapshot.resource_id)
            .bind(worker.as_str())
            .bind(snapshot.phase.as_str())
            .bind(snapshot.healthy)
            .bind(snapshot.accepting)
            .bind(row_version)
            .execute(&self.pool)
            .await
            .map_err(unavailable)?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "withdraw"))]
    async fn withdraw(
        &self,
        scope: &Scope,
        worker: &StatusWorkerId,
        resource_id: &str,
    ) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM port_resource_status WHERE workspace_id = ? AND org_id = ? AND resource_id = ? AND worker_id = ?")
            .bind(&scope.workspace_id)
            .bind(&scope.org_id)
            .bind(resource_id)
            .bind(worker.as_str())
            .execute(&self.pool)
            .await
            .map_err(unavailable)?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "withdraw_worker"))]
    async fn withdraw_worker(&self, worker: &StatusWorkerId) -> Result<(), StorageError> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(unavailable)?;
        sqlx::query("DELETE FROM port_resource_status WHERE worker_id = ?")
            .bind(worker.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(unavailable)?;
        sqlx::query("DELETE FROM port_worker_heartbeats WHERE worker_id = ?")
            .bind(worker.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(unavailable)?;
        transaction.commit().await.map_err(commit_unknown)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "live_for"))]
    async fn live_for(
        &self,
        scope: &Scope,
        resource_id: &str,
    ) -> Result<Vec<LiveResourceStatus>, StorageError> {
        let rows = sqlx::query("SELECT s.worker_id, s.phase, s.healthy, s.accepting, s.row_version FROM port_resource_status s JOIN port_worker_heartbeats h ON h.worker_id = s.worker_id WHERE s.workspace_id = ? AND s.org_id = ? AND s.resource_id = ? AND h.expires_at_ms > CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) ORDER BY s.worker_id")
            .bind(&scope.workspace_id)
            .bind(&scope.org_id)
            .bind(resource_id)
            .fetch_all(&self.pool)
            .await
            .map_err(unavailable)?;
        rows.iter()
            .map(|row| {
                decode_live(
                    resource_id,
                    row.try_get("worker_id").map_err(unavailable)?,
                    row.try_get("phase").map_err(unavailable)?,
                    row.try_get("healthy").map_err(unavailable)?,
                    row.try_get("accepting").map_err(unavailable)?,
                    row.try_get("row_version").map_err(unavailable)?,
                )
            })
            .collect()
    }
}
