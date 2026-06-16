//! Postgres `JobDispatchQueue` + `TriggerDedupInbox` over the port-scoped
//! schema.
//!
//! `claim_pending` uses `FOR UPDATE SKIP LOCKED` so multiple consumers can
//! drain the queue concurrently without double-dispatch.  The routing
//! predicate is `required_plugins ⊆ available_plugins`: the worker's
//! available plugin set must be a superset of every plugin the job requires.
//!
//! Implementation: `required_plugin_key = ANY($pre_filter)` is an
//! index-friendly pre-filter (sound because `required_plugins ⊇
//! {required_plugin_key}` by DTO invariant), then `required_plugins <@
//! $available_jsonb` applies the exact JSONB "is contained by" superset check
//! in the same statement.  `available_plugins` is bound twice — once as
//! `text[]` for `= ANY` and once as `JSONB` for `<@`.
//!
//! PERF: `<@` is NOT GIN-accelerated — `jsonb_ops` indexes `@>` / existence,
//! not `<@` — so it runs as a filter over the rows the `(status,
//! required_plugin_key)` B-tree returns.  Fine pre-fleet; a `<@`-indexable
//! representation (text[] + `array_ops`, or a tag-membership table) is the
//! tracked pre-production follow-up.  See `schema.sql` PERF NOTE.
//!
//! Ids are raw 16-byte ULID (`BYTEA`).  `required_plugins` is a `JSONB` array.

use std::time::Duration;

use chrono::Utc;
use nebula_core::PluginKey;
use nebula_storage_port::dto::{
    DispatchKind, DispatchOutcome, JobDispatchMsg, NewExecution, TriggerDedupRow,
};
use nebula_storage_port::store::{JobDispatchQueue, ReclaimOutcome, TriggerDedupInbox};
use nebula_storage_port::{Scope, StorageError};
use sqlx::{PgPool, Row};

use super::execution::{conn_err, insert_created_execution};

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

fn plugins_to_jsonb(plugins: &[PluginKey]) -> serde_json::Value {
    serde_json::Value::Array(
        plugins
            .iter()
            .map(|k| serde_json::Value::String(k.as_str().to_owned()))
            .collect(),
    )
}

fn row_to_msg(row: &sqlx::postgres::PgRow) -> Result<JobDispatchMsg, StorageError> {
    let id_bytes: Vec<u8> = row.try_get("id").map_err(conn_err)?;
    let plugins_val: serde_json::Value = row.try_get("required_plugins").map_err(conn_err)?;
    let required_plugins: Vec<PluginKey> = plugins_val
        .as_array()
        .ok_or_else(|| StorageError::Serialization("required_plugins not a JSON array".to_owned()))?
        .iter()
        .map(|v| {
            v.as_str()
                .ok_or_else(|| {
                    StorageError::Serialization(
                        "required_plugins element is not a string".to_owned(),
                    )
                })
                .and_then(|s| {
                    s.parse::<PluginKey>()
                        .map_err(|e| StorageError::Serialization(e.to_string()))
                })
        })
        .collect::<Result<_, _>>()?;
    let required_plugin_key: PluginKey = row
        .try_get::<String, _>("required_plugin_key")
        .map_err(conn_err)?
        .parse::<PluginKey>()
        .map_err(|e| StorageError::Serialization(e.to_string()))?;
    Ok(JobDispatchMsg::new(
        decode_id(&id_bytes)?,
        row.try_get::<String, _>("execution_id").map_err(conn_err)?,
        decode_command(&row.try_get::<String, _>("command").map_err(conn_err)?)?,
        Scope::new(
            row.try_get::<String, _>("workspace_id").map_err(conn_err)?,
            row.try_get::<String, _>("org_id").map_err(conn_err)?,
        ),
        row.try_get("payload").map_err(conn_err)?,
        row.try_get::<Option<String>, _>("event_id")
            .map_err(conn_err)?,
        row.try_get::<String, _>("target_flavor_sha")
            .map_err(conn_err)?,
        required_plugin_key,
        required_plugins,
        row.try_get::<Option<String>, _>("w3c_traceparent")
            .map_err(conn_err)?,
        row.try_get::<i32, _>("reclaim_count").map_err(conn_err)? as u32,
    ))
}

// ── JobDispatchQueue ─────────────────────────────────────────────────────────

/// Postgres-backed job-dispatch queue handle.
#[derive(Clone, Debug)]
pub struct PgJobDispatchQueue {
    pool: PgPool,
}

impl PgJobDispatchQueue {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl JobDispatchQueue for PgJobDispatchQueue {
    #[tracing::instrument(level = "debug", skip(self, msg), fields(id = ?msg.id, command = msg.command.as_str()))]
    async fn enqueue(&self, msg: &JobDispatchMsg) -> Result<(), StorageError> {
        let plugins = plugins_to_jsonb(&msg.required_plugins);
        sqlx::query(
            "INSERT INTO port_job_dispatch_queue \
             (id, execution_id, workspace_id, org_id, command, status, \
              payload, event_id, target_flavor_sha, required_plugin_key, \
              required_plugins, w3c_traceparent, reclaim_count) \
             VALUES ($1, $2, $3, $4, $5, 'Pending', $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(msg.id.as_slice())
        .bind(&msg.execution_id)
        .bind(&msg.scope.workspace_id)
        .bind(&msg.scope.org_id)
        .bind(msg.command.as_str())
        .bind(&msg.payload)
        .bind(msg.event_id.as_deref())
        .bind(&msg.target_flavor_sha)
        .bind(msg.required_plugin_key.as_str())
        .bind(&plugins)
        .bind(msg.w3c_traceparent.as_deref())
        .bind(i32::try_from(msg.reclaim_count).unwrap_or(i32::MAX))
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        tracing::debug!(target: "nebula_storage::postgres", "job_dispatch: enqueued");
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, available_plugins), fields(batch_size))]
    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        available_plugins: &[PluginKey],
    ) -> Result<Vec<JobDispatchMsg>, StorageError> {
        if available_plugins.is_empty() {
            return Ok(Vec::new());
        }
        // `processed_at_ms` is epoch-millis (BIGINT) — same representation as
        // `port_control_queue`, so reclaim cutoff arithmetic is identical on both dialects.
        let now_ms = Utc::now().timestamp_millis();
        let plugin_strs: Vec<&str> = available_plugins.iter().map(PluginKey::as_str).collect();
        // `required_plugins <@ $available_jsonb`: JSONB "is contained by" operator
        // (NOT GIN-accelerated — see the module PERF note; runs as a filter over
        // the pre-filtered rows).  A job's `required_plugins` must be a subset of
        // the available JSONB array.  Built via `plugins_to_jsonb` — the same
        // helper `enqueue` uses — so `$3` (text[]) and `$4` (jsonb) are always
        // derived from the same source and can never diverge.
        let available_jsonb = plugins_to_jsonb(available_plugins);
        let rows = sqlx::query(
            "UPDATE port_job_dispatch_queue \
             SET status = 'Processing', processed_by = $1, processed_at_ms = $2 \
             WHERE id IN ( \
                 SELECT id FROM port_job_dispatch_queue \
                 WHERE status = 'Pending' \
                   AND required_plugin_key = ANY($3) \
                   AND required_plugins <@ $4 \
                 ORDER BY id \
                 LIMIT $5 \
                 FOR UPDATE SKIP LOCKED \
             ) \
             RETURNING id, execution_id, workspace_id, org_id, command, \
                       payload, event_id, target_flavor_sha, required_plugin_key, \
                       required_plugins, w3c_traceparent, reclaim_count",
        )
        .bind(processor.as_slice())
        .bind(now_ms)
        .bind(&plugin_strs)
        .bind(&available_jsonb)
        .bind(i64::from(batch_size))
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        let claimed = rows.iter().map(row_to_msg).collect::<Result<Vec<_>, _>>()?;
        tracing::debug!(
            target: "nebula_storage::postgres",
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
             WHERE id = $1 AND status = 'Processing' AND processed_by = $2",
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
             SET status = 'Failed', error_message = $1 \
             WHERE id = $2 AND status = 'Processing' AND processed_by = $3",
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
        // `processed_at_ms` is epoch-millis (BIGINT) — same representation and
        // cutoff arithmetic as `port_control_queue`, so reclaim fires at the
        // identical instant on both dialects.
        let cutoff_ms = Utc::now()
            .timestamp_millis()
            .saturating_sub(i64::try_from(reclaim_after.as_millis()).unwrap_or(i64::MAX));
        let exhausted = sqlx::query(
            "UPDATE port_job_dispatch_queue \
             SET status = 'Failed', \
                 error_message = 'reclaim exhausted: presumed dead' \
             WHERE status = 'Processing' AND processed_at_ms < $1 \
               AND reclaim_count >= $2",
        )
        .bind(cutoff_ms)
        .bind(i32::try_from(max_reclaim_count).unwrap_or(i32::MAX))
        .execute(&self.pool)
        .await
        .map_err(conn_err)?
        .rows_affected();
        let reclaimed = sqlx::query(
            "UPDATE port_job_dispatch_queue \
             SET status = 'Pending', reclaim_count = reclaim_count + 1, \
                 processed_by = NULL, processed_at_ms = NULL \
             WHERE status = 'Processing' AND processed_at_ms < $1 \
               AND reclaim_count < $2",
        )
        .bind(cutoff_ms)
        .bind(i32::try_from(max_reclaim_count).unwrap_or(i32::MAX))
        .execute(&self.pool)
        .await
        .map_err(conn_err)?
        .rows_affected();
        Ok(ReclaimOutcome {
            reclaimed,
            exhausted,
        })
    }

    async fn cleanup(&self, retention: Duration) -> Result<u64, StorageError> {
        let cutoff_ms = Utc::now()
            .timestamp_millis()
            .saturating_sub(i64::try_from(retention.as_millis()).unwrap_or(i64::MAX));
        let deleted = sqlx::query(
            "DELETE FROM port_job_dispatch_queue \
             WHERE status IN ('Dispatched', 'Failed') AND processed_at_ms < $1",
        )
        .bind(cutoff_ms)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?
        .rows_affected();
        Ok(deleted)
    }
}

// ── TriggerDedupInbox ────────────────────────────────────────────────────────

/// Postgres-backed trigger-dedup inbox handle.
#[derive(Clone, Debug)]
pub struct PgTriggerDedupInbox {
    pool: PgPool,
}

impl PgTriggerDedupInbox {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl TriggerDedupInbox for PgTriggerDedupInbox {
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
        let plugins = plugins_to_jsonb(&start.required_plugins);

        // All three writes live inside one transaction — atomicity by
        // construction.  An error from any write propagates via `?` and rolls
        // back the whole transaction.
        let mut tx = self.pool.begin().await.map_err(conn_err)?;

        if let Some(r) = row {
            // INSERT … ON CONFLICT(workspace_id, org_id, trigger_id, event_id) DO NOTHING
            // is the CAS.  The scope columns are inside the conflict target so two tenants
            // sharing the same (trigger_id, event_id) never collide.
            // First writer wins; second writer gets affected == 0.
            let affected = sqlx::query(
                "INSERT INTO port_trigger_dedup_inbox \
                 (workspace_id, org_id, trigger_id, event_id, execution_id, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 ON CONFLICT (workspace_id, org_id, trigger_id, event_id) DO NOTHING",
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
                     WHERE workspace_id = $1 AND org_id = $2 \
                       AND trigger_id = $3 AND event_id = $4",
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
                    target: "nebula_storage::postgres",
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
              required_plugins, w3c_traceparent, reclaim_count) \
             VALUES ($1, $2, $3, $4, $5, 'Pending', $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(start.id.as_slice())
        .bind(&start.execution_id)
        .bind(&start.scope.workspace_id)
        .bind(&start.scope.org_id)
        .bind(start.command.as_str())
        .bind(&start.payload)
        .bind(start.event_id.as_deref())
        .bind(&start.target_flavor_sha)
        .bind(start.required_plugin_key.as_str())
        .bind(&plugins)
        .bind(start.w3c_traceparent.as_deref())
        .bind(i32::try_from(start.reclaim_count).unwrap_or(i32::MAX))
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;

        tx.commit().await.map_err(conn_err)?;
        tracing::debug!(
            target: "nebula_storage::postgres",
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
             WHERE trigger_id = $1 AND event_id = $2 \
               AND workspace_id = $3 AND org_id = $4",
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
