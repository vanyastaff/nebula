//! SQLite `JobDispatchQueue` + `TriggerDedupInbox` over the port-scoped schema.
//!
//! Single-consumer status flip (no `FOR UPDATE SKIP LOCKED` equivalent —
//! spec §5 SQLite boundary, documented not hidden).  Ids are the raw 16-byte
//! ULID (`BLOB`).  `capability_tags` is a JSON array; the **routing predicate**
//! is `capability_tags ⊆ advertised_tags`: a worker claims a job only when its
//! advertised set covers every tag in the job's `capability_tags`.
//!
//! Implementation: `required_plugin_key IN (<advertised>)` is an index-friendly
//! pre-filter (sound because `capability_tags ⊇ {required_plugin_key}` by DTO
//! invariant), then the exact superset test is applied in the same SELECT via
//! `NOT EXISTS (SELECT 1 FROM json_each(capability_tags) je WHERE je.value NOT
//! IN (<advertised>))`.  Both clauses bind the same advertised list, eliminating
//! any TOCTOU window between pre-filter and claim.

use std::time::Duration;

use nebula_storage_port::dto::{
    CapabilityTag, DispatchKind, DispatchOutcome, JobDispatchMsg, NewExecution, TriggerDedupRow,
};
use nebula_storage_port::store::{JobDispatchQueue, ReclaimOutcome, TriggerDedupInbox};
use nebula_storage_port::{Scope, StorageError};
use sqlx::{Row, SqlitePool};

use crate::sqlite::execution::{conn_err, insert_created_execution};

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

        // Superset predicate: a job is claimable when `capability_tags ⊆
        // advertised_tags`.  The advertised set is bound ONCE as a JSON array
        // and unfolded via `json_each` into a CTE, referenced by both the
        // index-friendly `required_plugin_key IN (…)` pre-filter and the exact
        // `NOT EXISTS` superset check — no dynamic placeholder expansion and no
        // per-tag bind, so the SQLite bound-variable limit is never a concern.
        //
        // The pre-filter and superset check share one SELECT, so no row is
        // fetched that fails the superset test.  A concurrent actor that flips
        // a selected row to 'Processing' first is caught by the per-row
        // `UPDATE … AND status = 'Pending'` guard below (rows_affected = 0 →
        // row skipped); the whole exchange is inside one transaction
        // (single-consumer SQLite boundary, spec §5).
        //
        // `NOT EXISTS` reads: "no element in json_each(capability_tags) is
        // absent from the advertised set", i.e. every required tag is covered.
        // An empty `capability_tags` JSON array yields no rows from json_each
        // ⇒ NOT EXISTS is vacuously true ⇒ claimable by anyone (consistent
        // with the InMemory backend).
        let advertised_json = tags_to_json(advertised_tags);
        let rows = sqlx::query(
            "WITH advertised(tag) AS (SELECT value FROM json_each(?1)) \
             SELECT id, execution_id, workspace_id, org_id, command, \
                    payload, event_id, target_flavor_sha, required_plugin_key, \
                    capability_tags, w3c_traceparent, reclaim_count \
             FROM port_job_dispatch_queue \
             WHERE status = 'Pending' \
               AND required_plugin_key IN (SELECT tag FROM advertised) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM json_each(capability_tags) je \
                   WHERE je.value NOT IN (SELECT tag FROM advertised) \
               ) \
             ORDER BY id LIMIT ?2",
        )
        .bind(&advertised_json)
        .bind(i64::from(batch_size))
        .fetch_all(&mut *tx)
        .await
        .map_err(conn_err)?;

        let mut claimed = Vec::with_capacity(rows.len());
        let now_ms = chrono::Utc::now().timestamp_millis();
        for row in &rows {
            let id_bytes: Vec<u8> = row.try_get("id").map_err(conn_err)?;
            // Conditional claim — AND status = 'Pending' guard prevents
            // double-claim if a concurrent actor flipped the row between the
            // SELECT above and this UPDATE (single-consumer SQLite boundary).
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
    #[tracing::instrument(level = "debug", skip(self, row, start, execution), fields(
        trigger_id   = row.as_ref().map(|r| r.trigger_id.as_str()),
        event_id     = row.as_ref().map(|r| r.event_id.as_str()),
        job_id       = ?start.id,
        execution_id = start.execution_id.as_str(),
    ))]
    async fn claim_and_materialize_start(
        &self,
        row: Option<&TriggerDedupRow>,
        start: &JobDispatchMsg,
        execution: &NewExecution<'_>,
    ) -> Result<DispatchOutcome, StorageError> {
        let payload = serde_json::to_string(&start.payload)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let tags = tags_to_json(&start.capability_tags);

        // All three writes live inside one transaction — atomicity by
        // construction.  An error from any write propagates via `?` and rolls
        // back the whole transaction (sqlx drops the connection on error).
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
            // Bind `start.execution_id`, not `r.execution_id` — the dedup guard must
            // record the id that the Start job uses so the SELECT read-back on the
            // Duplicate path returns the first writer's effective execution id.
            .bind(&start.execution_id)
            .bind(&r.created_at)
            .execute(&mut *tx)
            .await
            .map_err(conn_err)?
            .rows_affected();

            if affected == 0 {
                // Duplicate: read the winner's execution_id back in the same
                // transaction so the caller has the canonical id without a
                // second connection.
                let winner_id: String = sqlx::query(
                    "SELECT execution_id FROM port_trigger_dedup_inbox \
                     WHERE workspace_id = ? AND org_id = ? \
                       AND trigger_id = ? AND event_id = ?",
                )
                .bind(&r.scope.workspace_id)
                .bind(&r.scope.org_id)
                .bind(&r.trigger_id)
                .bind(&r.event_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(conn_err)?
                .try_get("execution_id")
                .map_err(conn_err)?;

                tx.commit().await.map_err(conn_err)?;
                tracing::debug!(
                    target: "nebula_storage::sqlite",
                    trigger_id = %r.trigger_id,
                    event_id   = %r.event_id,
                    winner_execution_id = %winner_id,
                    "trigger_dedup: duplicate — returning winner id"
                );
                return Ok(DispatchOutcome::new(winner_id, DispatchKind::Duplicate));
            }
        }

        // Insert the execution row (status='Created', version=0,
        // fencing_generation=0).  Id + scope come from `start`.
        // A unique-violation here rolls back the whole transaction — no dedup
        // row, no job row.
        insert_created_execution(
            &mut tx,
            &start.scope,
            &start.execution_id,
            execution.workflow_id,
            execution.initial_state,
        )
        .await?;

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
            execution_id = %start.execution_id,
            "trigger_dedup: materialized (dedup guard + execution row + Start job)"
        );
        Ok(DispatchOutcome::new(
            start.execution_id.clone(),
            DispatchKind::Dispatched,
        ))
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
