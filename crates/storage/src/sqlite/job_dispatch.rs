//! SQLite `JobDispatchQueue` + `TriggerDedupInbox` over the port-scoped schema.
//!
//! Single-consumer status flip (no `FOR UPDATE SKIP LOCKED` equivalent —
//! spec §5 SQLite boundary, documented not hidden).  Ids are the raw 16-byte
//! ULID (`BLOB`).  `capability_tags` is a JSON array stored for informational
//! purposes; the **routing predicate** filters on `required_plugin_key` directly
//! (`WHERE required_plugin_key = ?` per advertised tag, equivalent to Postgres
//! `required_plugin_key = ANY($tags)`).  `capability_tags` is never used for
//! routing — only `required_plugin_key` is the dispatch selector.

use std::time::Duration;

use nebula_storage_port::dto::{CapabilityTag, DispatchOutcome, JobDispatchMsg, TriggerDedupRow};
use nebula_storage_port::store::{JobDispatchQueue, ReclaimOutcome, TriggerDedupInbox};
use nebula_storage_port::{Scope, StorageError};
use sqlx::{Row, SqlitePool};

use crate::sqlite::execution::conn_err;

// ── helpers ──────────────────────────────────────────────────────────────────

fn decode_id(bytes: &[u8]) -> Result<[u8; 16], StorageError> {
    <[u8; 16]>::try_from(bytes).map_err(|_| {
        StorageError::Serialization(format!(
            "job-dispatch id must be 16 bytes, got {}",
            bytes.len()
        ))
    })
}

fn decode_command(s: &str) -> Result<nebula_storage_port::dto::ControlCommand, StorageError> {
    use nebula_storage_port::dto::ControlCommand as C;
    match s {
        "Start" => Ok(C::Start),
        "Cancel" => Ok(C::Cancel),
        "Terminate" => Ok(C::Terminate),
        "Resume" => Ok(C::Resume),
        "Restart" => Ok(C::Restart),
        other => Err(StorageError::Serialization(format!(
            "unknown control command: {other}"
        ))),
    }
}

fn tags_to_json(tags: &[CapabilityTag]) -> String {
    let strs: Vec<&str> = tags.iter().map(CapabilityTag::as_str).collect();
    serde_json::to_string(&strs).unwrap_or_else(|_| "[]".to_owned())
}

fn row_to_msg(row: &sqlx::sqlite::SqliteRow) -> Result<JobDispatchMsg, StorageError> {
    let id_bytes: Vec<u8> = row.try_get("id").map_err(conn_err)?;
    let tags_json: String = row.try_get("capability_tags").map_err(conn_err)?;
    let tags: Vec<String> =
        serde_json::from_str(&tags_json).map_err(|e| StorageError::Serialization(e.to_string()))?;
    let payload_json: String = row.try_get("payload").map_err(conn_err)?;
    Ok(JobDispatchMsg::new(
        decode_id(&id_bytes)?,
        row.try_get::<String, _>("execution_id").map_err(conn_err)?,
        decode_command(&row.try_get::<String, _>("command").map_err(conn_err)?)?,
        Scope::new(
            row.try_get::<String, _>("workspace_id").map_err(conn_err)?,
            row.try_get::<String, _>("org_id").map_err(conn_err)?,
        ),
        serde_json::from_str(&payload_json)
            .map_err(|e| StorageError::Serialization(e.to_string()))?,
        row.try_get::<Option<String>, _>("event_id")
            .map_err(conn_err)?,
        row.try_get::<String, _>("target_flavor_sha")
            .map_err(conn_err)?,
        row.try_get::<String, _>("required_plugin_key")
            .map_err(conn_err)?,
        tags.into_iter().map(CapabilityTag).collect(),
        row.try_get::<Option<String>, _>("w3c_traceparent")
            .map_err(conn_err)?,
        row.try_get::<i64, _>("reclaim_count").map_err(conn_err)? as u32,
    ))
}

// ── JobDispatchQueue ─────────────────────────────────────────────────────────

/// SQLite-backed job-dispatch queue handle.
#[derive(Clone, Debug)]
pub struct SqliteJobDispatchQueue {
    pool: SqlitePool,
}

impl SqliteJobDispatchQueue {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl JobDispatchQueue for SqliteJobDispatchQueue {
    #[tracing::instrument(level = "debug", skip(self, msg), fields(id = ?msg.id, command = msg.command.as_str()))]
    async fn enqueue(&self, msg: &JobDispatchMsg) -> Result<(), StorageError> {
        let payload = serde_json::to_string(&msg.payload)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let tags = tags_to_json(&msg.capability_tags);
        sqlx::query(
            "INSERT INTO port_job_dispatch_queue \
             (id, execution_id, workspace_id, org_id, command, status, \
              payload, event_id, target_flavor_sha, required_plugin_key, \
              capability_tags, w3c_traceparent, reclaim_count) \
             VALUES (?, ?, ?, ?, ?, 'Pending', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(msg.id.as_slice())
        .bind(&msg.execution_id)
        .bind(&msg.scope.workspace_id)
        .bind(&msg.scope.org_id)
        .bind(msg.command.as_str())
        .bind(&payload)
        .bind(msg.event_id.as_deref())
        .bind(&msg.target_flavor_sha)
        .bind(&msg.required_plugin_key)
        .bind(&tags)
        .bind(msg.w3c_traceparent.as_deref())
        .bind(i64::from(msg.reclaim_count))
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        tracing::debug!(target: "nebula_storage::sqlite", "job_dispatch: enqueued");
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, advertised_tags), fields(batch_size))]
    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        advertised_tags: &[CapabilityTag],
    ) -> Result<Vec<JobDispatchMsg>, StorageError> {
        if advertised_tags.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.pool.begin().await.map_err(conn_err)?;
        // Routing: claim rows whose `required_plugin_key` is a member of the
        // advertised set.  This mirrors Postgres `required_plugin_key = ANY($3)`
        // exactly — the `capability_tags` JSON array is NOT the routing key.
        // SQLite has no array-bind, so we build one SELECT per advertised tag
        // and union them.  Each branch checks `required_plugin_key = ?` directly.
        let union_selects: String = advertised_tags
            .iter()
            .map(|_| {
                "SELECT id FROM port_job_dispatch_queue \
                 WHERE status = 'Pending' AND required_plugin_key = ?"
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join(" UNION ");
        let select_sql = format!(
            "SELECT id, execution_id, workspace_id, org_id, command, \
                    payload, event_id, target_flavor_sha, required_plugin_key, \
                    capability_tags, w3c_traceparent, reclaim_count \
             FROM port_job_dispatch_queue \
             WHERE id IN ({union_selects}) \
             ORDER BY id LIMIT ?"
        );
        let mut q = sqlx::query(sqlx::AssertSqlSafe(select_sql));
        for tag in advertised_tags {
            q = q.bind(tag.as_str());
        }
        q = q.bind(i64::from(batch_size));
        let rows = q.fetch_all(&mut *tx).await.map_err(conn_err)?;

        let mut claimed = Vec::with_capacity(rows.len());
        let now_ms = chrono::Utc::now().timestamp_millis();
        for row in &rows {
            let id_bytes: Vec<u8> = row.try_get("id").map_err(conn_err)?;
            // Conditional claim — AND status = 'Pending' guard prevents
            // double-claim if a concurrent actor flipped the row.
            let won = sqlx::query(
                "UPDATE port_job_dispatch_queue \
                 SET status = 'Processing', processed_by = ?, \
                     processed_at_ms = ? \
                 WHERE id = ? AND status = 'Pending'",
            )
            .bind(processor.as_slice())
            .bind(now_ms)
            .bind(id_bytes.as_slice())
            .execute(&mut *tx)
            .await
            .map_err(conn_err)?
            .rows_affected()
                == 1;
            if won {
                claimed.push(row_to_msg(row)?);
            }
        }
        tx.commit().await.map_err(conn_err)?;
        tracing::debug!(
            target: "nebula_storage::sqlite",
            claimed = claimed.len(),
            "job_dispatch: claimed"
        );
        Ok(claimed)
    }

    async fn mark_dispatched(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE port_job_dispatch_queue SET status = 'Dispatched' \
             WHERE id = ? AND status = 'Processing' AND processed_by = ?",
        )
        .bind(id.as_slice())
        .bind(processor.as_slice())
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }

    async fn mark_failed(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
        error: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE port_job_dispatch_queue \
             SET status = 'Failed', error_message = ? \
             WHERE id = ? AND status = 'Processing' AND processed_by = ?",
        )
        .bind(error)
        .bind(id.as_slice())
        .bind(processor.as_slice())
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }

    async fn reclaim_stuck(
        &self,
        reclaim_after: Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError> {
        // `processed_at_ms` is epoch-millis (INTEGER) — same representation
        // as `port_control_queue`, so reclaim cutoff arithmetic is identical.
        let cutoff = chrono::Utc::now().timestamp_millis()
            - i64::try_from(reclaim_after.as_millis()).unwrap_or(i64::MAX);
        let mut tx = self.pool.begin().await.map_err(conn_err)?;
        let exhausted = sqlx::query(
            "UPDATE port_job_dispatch_queue \
             SET status = 'Failed', \
                 error_message = 'reclaim exhausted: presumed dead' \
             WHERE status = 'Processing' AND processed_at_ms < ? \
               AND reclaim_count >= ?",
        )
        .bind(cutoff)
        .bind(i64::from(max_reclaim_count))
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?
        .rows_affected();
        let reclaimed = sqlx::query(
            "UPDATE port_job_dispatch_queue \
             SET status = 'Pending', reclaim_count = reclaim_count + 1, \
                 processed_by = NULL, processed_at_ms = NULL \
             WHERE status = 'Processing' AND processed_at_ms < ? \
               AND reclaim_count < ?",
        )
        .bind(cutoff)
        .bind(i64::from(max_reclaim_count))
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?
        .rows_affected();
        tx.commit().await.map_err(conn_err)?;
        Ok(ReclaimOutcome {
            reclaimed,
            exhausted,
        })
    }

    async fn cleanup(&self, retention: Duration) -> Result<u64, StorageError> {
        let cutoff = chrono::Utc::now().timestamp_millis()
            - i64::try_from(retention.as_millis()).unwrap_or(i64::MAX);
        let deleted = sqlx::query(
            "DELETE FROM port_job_dispatch_queue \
             WHERE status IN ('Dispatched', 'Failed') AND processed_at_ms < ?",
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?
        .rows_affected();
        Ok(deleted)
    }
}

// ── TriggerDedupInbox ────────────────────────────────────────────────────────

/// SQLite-backed trigger-dedup inbox handle.
#[derive(Clone, Debug)]
pub struct SqliteTriggerDedupInbox {
    pool: SqlitePool,
}

impl SqliteTriggerDedupInbox {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl TriggerDedupInbox for SqliteTriggerDedupInbox {
    #[tracing::instrument(level = "debug", skip(self, row, start), fields(
        trigger_id = row.as_ref().map(|r| r.trigger_id.as_str()),
        event_id   = row.as_ref().map(|r| r.event_id.as_str()),
        job_id     = ?start.id,
    ))]
    async fn claim_and_enqueue_start(
        &self,
        row: Option<&TriggerDedupRow>,
        start: &JobDispatchMsg,
    ) -> Result<DispatchOutcome, StorageError> {
        let payload = serde_json::to_string(&start.payload)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let tags = tags_to_json(&start.capability_tags);

        // Both writes live inside one transaction — atomicity by construction.
        let mut tx = self.pool.begin().await.map_err(conn_err)?;

        if let Some(r) = row {
            // INSERT OR IGNORE: first-writer-wins by
            // PRIMARY KEY(workspace_id, org_id, trigger_id, event_id).
            // The scope columns are inside the key so two tenants sharing the
            // same (trigger_id, event_id) never collide.
            let affected = sqlx::query(
                "INSERT OR IGNORE INTO port_trigger_dedup_inbox \
                 (workspace_id, org_id, trigger_id, event_id, execution_id, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&r.scope.workspace_id)
            .bind(&r.scope.org_id)
            .bind(&r.trigger_id)
            .bind(&r.event_id)
            .bind(&r.execution_id)
            .bind(&r.created_at)
            .execute(&mut *tx)
            .await
            .map_err(conn_err)?
            .rows_affected();

            if affected == 0 {
                tx.commit().await.map_err(conn_err)?;
                tracing::debug!(
                    target: "nebula_storage::sqlite",
                    trigger_id = %r.trigger_id,
                    event_id   = %r.event_id,
                    "trigger_dedup: duplicate"
                );
                return Ok(DispatchOutcome::Duplicate);
            }
        }

        // Insert the Start job (same transaction).
        sqlx::query(
            "INSERT INTO port_job_dispatch_queue \
             (id, execution_id, workspace_id, org_id, command, status, \
              payload, event_id, target_flavor_sha, required_plugin_key, \
              capability_tags, w3c_traceparent, reclaim_count) \
             VALUES (?, ?, ?, ?, ?, 'Pending', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(start.id.as_slice())
        .bind(&start.execution_id)
        .bind(&start.scope.workspace_id)
        .bind(&start.scope.org_id)
        .bind(start.command.as_str())
        .bind(&payload)
        .bind(start.event_id.as_deref())
        .bind(&start.target_flavor_sha)
        .bind(&start.required_plugin_key)
        .bind(&tags)
        .bind(start.w3c_traceparent.as_deref())
        .bind(i64::from(start.reclaim_count))
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;

        tx.commit().await.map_err(conn_err)?;
        tracing::debug!(
            target: "nebula_storage::sqlite",
            job_id = ?start.id,
            "trigger_dedup: dispatched"
        );
        Ok(DispatchOutcome::Dispatched)
    }

    async fn exists(
        &self,
        scope: &Scope,
        trigger_id: &str,
        event_id: &str,
    ) -> Result<bool, StorageError> {
        let row = sqlx::query(
            "SELECT 1 AS ok FROM port_trigger_dedup_inbox \
             WHERE trigger_id = ? AND event_id = ? \
               AND workspace_id = ? AND org_id = ?",
        )
        .bind(trigger_id)
        .bind(event_id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(row.is_some())
    }

    async fn cleanup(&self, _retention: Duration) -> Result<u64, StorageError> {
        // No-op stub — TTL sweep wired later without a trait break.
        Ok(0)
    }
}
