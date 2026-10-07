//! PostgreSQL persistence for cross-process resource runtime status, over
//! `resource_status_heartbeats` and `resource_status_snapshots`.
//!
//! Every expiry comparison reads the server's `clock_timestamp()` inside the
//! statement, so a skewed worker clock can neither keep a dead worker live nor
//! expire a live one early.
//!
//! A snapshot belongs to its stored resource: publish share-locks the live
//! resource row first (`NotFound` when it is missing or archived), purging the
//! resource purges its snapshots, and an archived resource has no live status.

use std::time::Duration;

use nebula_storage_port::dto::{LiveResourceStatus, ResourceStatusSnapshot, StatusWorkerId};
use nebula_storage_port::store::ResourceStatusStore;
use nebula_storage_port::{Scope, StorageError};
use sqlx::{PgPool, Row as _};

use crate::resource_status::{
    HEARTBEAT_RETENTION_MS, decode_live, heartbeat_ttl_ms, row_version_to_stored,
};
use crate::sql_error::storage_error;

/// PostgreSQL implementation of [`ResourceStatusStore`].
#[derive(Clone, Debug)]
pub struct PgResourceStatusStore {
    pool: PgPool,
}

impl PgResourceStatusStore {
    /// Wrap a pool initialized through [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ResourceStatusStore for PgResourceStatusStore {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "heartbeat"))]
    async fn heartbeat(&self, worker: &StatusWorkerId, ttl: Duration) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO resource_status_heartbeats (worker_id, expires_at) \
             VALUES ($1, clock_timestamp() + $2 * INTERVAL '1 millisecond') \
             ON CONFLICT (worker_id) DO UPDATE SET expires_at = EXCLUDED.expires_at",
        )
        .bind(worker.as_str())
        .bind(heartbeat_ttl_ms(ttl))
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        // One statement prunes a long-dead heartbeat together with its
        // snapshots, so a worker that renews concurrently keeps both: the
        // renewed row no longer matches when the DELETE re-checks it.
        //
        // Best effort: the renewal above has already committed, so a failed
        // cleanup must not report the heartbeat as failed (the publisher
        // would then stop publishing and withdrawing while its lease stays
        // renewed). Dead workers are pruned by a later heartbeat instead.
        if let Err(error) = sqlx::query(
            "WITH pruned AS (DELETE FROM resource_status_heartbeats \
             WHERE expires_at < clock_timestamp() - $1 * INTERVAL '1 millisecond' \
             RETURNING worker_id) \
             DELETE FROM resource_status_snapshots WHERE worker_id IN (SELECT worker_id FROM pruned)",
        )
        .bind(HEARTBEAT_RETENTION_MS)
        .execute(&self.pool)
        .await
        {
            tracing::warn!(
                target: "nebula_storage::resource_status",
                %error,
                "pruning dead status workers failed; retried on a later heartbeat"
            );
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_status", storage.operation = "publish"))]
    async fn publish(
        &self,
        scope: &Scope,
        worker: &StatusWorkerId,
        snapshot: &ResourceStatusSnapshot,
    ) -> Result<(), StorageError> {
        let row_version = row_version_to_stored(snapshot.row_version)?;
        let mut transaction = self.pool.begin().await.map_err(storage_error)?;
        // Share-lock the live resource, so a concurrent archive serializes
        // with the snapshot (the foreign key proves existence only).
        let live: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM resources \
             WHERE org_id = $1 AND workspace_id = $2 AND id = $3 AND deleted_at IS NULL \
             FOR SHARE",
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
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (org_id, workspace_id, resource_id, worker_id) DO UPDATE SET \
             phase = EXCLUDED.phase, healthy = EXCLUDED.healthy, \
             accepting = EXCLUDED.accepting, row_version = EXCLUDED.row_version",
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
             WHERE org_id = $1 AND workspace_id = $2 AND resource_id = $3 AND worker_id = $4",
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
        let mut transaction = self.pool.begin().await.map_err(storage_error)?;
        sqlx::query("DELETE FROM resource_status_snapshots WHERE worker_id = $1")
            .bind(worker.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(storage_error)?;
        sqlx::query("DELETE FROM resource_status_heartbeats WHERE worker_id = $1")
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
        // An archived resource has no live status. `COLLATE "C"` orders by
        // bytes, as the other backends do, whatever the database's default
        // collation.
        let rows = sqlx::query(
            "SELECT s.worker_id, s.phase, s.healthy, s.accepting, s.row_version \
             FROM resource_status_snapshots s \
             JOIN resource_status_heartbeats h ON h.worker_id = s.worker_id \
             JOIN resources r ON r.org_id = s.org_id AND r.workspace_id = s.workspace_id \
               AND r.id = s.resource_id AND r.deleted_at IS NULL \
             WHERE s.org_id = $1 AND s.workspace_id = $2 AND s.resource_id = $3 \
               AND h.expires_at > clock_timestamp() \
             ORDER BY s.worker_id COLLATE \"C\"",
        )
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
