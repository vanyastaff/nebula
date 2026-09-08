//! SQLite [`JobDispatchQueue`] over the port-scoped schema.
//!
//! Single-consumer status flip (no `FOR UPDATE SKIP LOCKED` equivalent —
//! spec §5 SQLite boundary, documented not hidden).  Ids are the raw 16-byte
//! ULID (`BLOB`).  `required_plugins` is a JSON array; the **routing predicate**
//! is `required_plugins ⊆ available_plugins`: a worker claims a job only when
//! its available plugin set covers every plugin the job requires.
//!
//! Implementation: `required_plugin_key IN (<available>)` is an index-friendly
//! pre-filter (sound because `required_plugins ⊇ {required_plugin_key}` by DTO
//! invariant), then the exact superset test is applied in the same SELECT via
//! `NOT EXISTS (SELECT 1 FROM json_each(required_plugins) je WHERE je.value NOT
//! IN (<available>))`.  Both clauses bind the same available list, eliminating
//! any TOCTOU window between pre-filter and claim.

use std::time::Duration;

use nebula_core::{PluginKey, WorkerFlavorRevisionId};
use nebula_storage_port::dto::JobDispatchMsg;
use nebula_storage_port::store::{
    ClaimGeneration, JobClaim, JobClaimToken, JobDispatchQueue, ReclaimOutcome,
};
use nebula_storage_port::{Scope, StorageError};
use sqlx::{Row, SqlitePool};

use crate::sqlite::execution::conn_err;

// ── helpers ──────────────────────────────────────────────────────────────────

/// Hex-encode a 16-byte ULID for use in error messages without pulling in the
/// `hex` crate (which is optional and only enabled by the `postgres` feature).
fn ulid_hex(id: &[u8; 16]) -> String {
    id.iter().fold(String::with_capacity(32), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}
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

fn plugins_to_json(plugins: &[PluginKey]) -> String {
    let strs: Vec<&str> = plugins.iter().map(PluginKey::as_str).collect();
    serde_json::to_string(&strs).unwrap_or_else(|_| "[]".to_owned())
}

fn row_to_msg(row: &sqlx::sqlite::SqliteRow) -> Result<JobDispatchMsg, StorageError> {
    let id_bytes: Vec<u8> = row.try_get("id").map_err(conn_err)?;
    let plugins_json: String = row.try_get("required_plugins").map_err(conn_err)?;
    let plugin_strs: Vec<String> = serde_json::from_str(&plugins_json)
        .map_err(|e| StorageError::Serialization(e.to_string()))?;
    let required_plugins: Vec<PluginKey> = plugin_strs
        .iter()
        .map(|s| {
            s.parse::<PluginKey>()
                .map_err(|e| StorageError::Serialization(e.to_string()))
        })
        .collect::<Result<_, _>>()?;
    let required_plugin_key: PluginKey = row
        .try_get::<String, _>("required_plugin_key")
        .map_err(conn_err)?
        .parse::<PluginKey>()
        .map_err(|e| StorageError::Serialization(e.to_string()))?;
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
        required_plugin_key,
        required_plugins,
        row.try_get::<Option<String>, _>("w3c_traceparent")
            .map_err(conn_err)?,
        row.try_get::<i64, _>("reclaim_count").map_err(conn_err)? as u32,
        WorkerFlavorRevisionId::from_bytes(
            row.try_get::<Vec<u8>, _>("required_worker_flavor_id")
                .map_err(conn_err)?
                .try_into()
                .map_err(|_| {
                    StorageError::Serialization(
                        "job dispatch worker flavor identity must be 32 bytes".to_owned(),
                    )
                })?,
        ),
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

    /// Explain why a fenced acknowledgement matched no row.
    ///
    /// The fenced `UPDATE` reports only "zero rows", which conflates "the row
    /// is gone" with "this token is stale". Callers need them apart: an absent
    /// row is a lost write, a superseded token is the fence doing its job.
    /// The follow-up read is on the failure path only, so the acknowledged
    /// path stays a single statement.
    async fn unacknowledgeable(&self, claim: &JobClaimToken) -> Result<StorageError, StorageError> {
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM port_job_dispatch_queue \
                 WHERE id = ? AND workspace_id = ? AND org_id = ?",
        )
        .bind(claim.row_id().as_slice())
        .bind(&claim.scope().workspace_id)
        .bind(&claim.scope().org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(if exists.is_some() {
            StorageError::FencedOut {
                entity: "job_dispatch",
                id: ulid_hex(claim.row_id()),
            }
        } else {
            StorageError::NotFound {
                entity: "job_dispatch",
                id: ulid_hex(claim.row_id()),
            }
        })
    }
}

/// Widen a persisted generation to the port's `u64`.
///
/// `claim_generation` is a non-negative `INTEGER`; a negative value is
/// persisted corruption, and treating it as `0` would make a stale token
/// match. Fail closed instead.
fn decode_generation(generation: i64, id: &[u8; 16]) -> Result<ClaimGeneration, StorageError> {
    u64::try_from(generation)
        .map(ClaimGeneration::new)
        .map_err(|_| {
            StorageError::Serialization(format!(
                "invalid job_dispatch claim_generation {generation} (id={})",
                ulid_hex(id)
            ))
        })
}

/// Narrow a token's generation for binding against the `INTEGER` column.
///
/// A generation beyond `i64::MAX` cannot name a persisted row, so binding a
/// saturated value would fence against the wrong row. Fail closed instead.
fn generation_bind(claim: &JobClaimToken) -> Result<i64, StorageError> {
    i64::try_from(claim.generation().get()).map_err(|_| {
        StorageError::Serialization(format!(
            "job_dispatch claim generation {} exceeds the persisted range (id={})",
            claim.generation(),
            ulid_hex(claim.row_id())
        ))
    })
}

#[async_trait::async_trait]
impl JobDispatchQueue for SqliteJobDispatchQueue {
    #[tracing::instrument(level = "debug", skip(self, msg), fields(id = ?msg.id, command = msg.command.as_str()))]
    async fn enqueue(&self, msg: &JobDispatchMsg) -> Result<(), StorageError> {
        let payload = serde_json::to_string(&msg.payload)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let plugins = plugins_to_json(&msg.required_plugins);
        sqlx::query(
            "INSERT INTO port_job_dispatch_queue \
             (id, execution_id, workspace_id, org_id, command, status, \
              payload, event_id, required_plugin_key, \
              required_plugins, w3c_traceparent, reclaim_count, required_worker_flavor_id) \
             VALUES (?, ?, ?, ?, ?, 'Pending', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(msg.id.as_slice())
        .bind(&msg.execution_id)
        .bind(&msg.scope.workspace_id)
        .bind(&msg.scope.org_id)
        .bind(msg.command.as_str())
        .bind(&payload)
        .bind(msg.event_id.as_deref())
        .bind(msg.required_plugin_key.as_str())
        .bind(&plugins)
        .bind(msg.w3c_traceparent.as_deref())
        .bind(i64::from(msg.reclaim_count))
        .bind(msg.required_worker_flavor_id.as_bytes().as_slice())
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        tracing::debug!(target: "nebula_storage::sqlite", "job_dispatch: enqueued");
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, available_plugins), fields(batch_size, advertised_worker_flavor_id = %worker_flavor_id))]
    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        available_plugins: &[PluginKey],
        worker_flavor_id: WorkerFlavorRevisionId,
    ) -> Result<Vec<JobClaim>, StorageError> {
        if available_plugins.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.pool.begin().await.map_err(conn_err)?;

        // Superset predicate: a job is claimable when `required_plugins ⊆
        // available_plugins`.  The available set is bound ONCE as a JSON array
        // and unfolded via `json_each` into a CTE, referenced by both the
        // index-friendly `required_plugin_key IN (…)` pre-filter and the exact
        // `NOT EXISTS` superset check — no dynamic placeholder expansion and no
        // per-plugin bind, so the SQLite bound-variable limit is never a concern.
        //
        // The pre-filter and superset check share one SELECT, so no row is
        // fetched that fails the superset test.  A concurrent actor that flips
        // a selected row to 'Processing' first is caught by the per-row
        // `UPDATE … AND status = 'Pending'` guard below (rows_affected = 0 →
        // row skipped); the whole exchange is inside one transaction
        // (single-consumer SQLite boundary, spec §5).
        //
        // `NOT EXISTS` reads: "no element in json_each(required_plugins) is
        // absent from the available set", i.e. every required plugin is covered.
        // The primary-plugin pre-filter remains an independent requirement
        // when malformed metadata omits it from `required_plugins`.
        let available_json = plugins_to_json(available_plugins);
        let rows = sqlx::query(
            "WITH available(plugin) AS (SELECT value FROM json_each(?1)) \
             SELECT id, execution_id, workspace_id, org_id, command, \
                    payload, event_id, required_plugin_key, \
                    required_plugins, w3c_traceparent, reclaim_count, required_worker_flavor_id \
             FROM port_job_dispatch_queue \
             WHERE status = 'Pending' AND required_worker_flavor_id = ?3 \
               AND required_plugin_key IN (SELECT plugin FROM available) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM json_each(required_plugins) je \
                   WHERE je.value NOT IN (SELECT plugin FROM available) \
               ) \
             ORDER BY id LIMIT ?2",
        )
        .bind(&available_json)
        .bind(i64::from(batch_size))
        .bind(worker_flavor_id.as_bytes().as_slice())
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
            //
            // `claim_generation` is incremented by the same statement that
            // makes the row `Processing`, and `RETURNING` hands back the value
            // this claim minted — no separate read that a concurrent claim
            // could interleave with.
            let minted: Option<i64> = sqlx::query_scalar(
                "UPDATE port_job_dispatch_queue \
                 SET status = 'Processing', processed_by = ?, \
                     processed_at_ms = ?, \
                     claim_generation = claim_generation + 1 \
                 WHERE id = ? AND status = 'Pending' \
                 RETURNING claim_generation",
            )
            .bind(processor.as_slice())
            .bind(now_ms)
            .bind(id_bytes.as_slice())
            .fetch_optional(&mut *tx)
            .await
            .map_err(conn_err)?;
            if let Some(generation) = minted {
                let id = decode_id(&id_bytes)?;
                let msg = row_to_msg(row)?;
                let token =
                    JobClaimToken::new(id, decode_generation(generation, &id)?, msg.scope.clone());
                claimed.push(JobClaim { msg, token });
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

    async fn mark_dispatched(&self, claim: &JobClaimToken) -> Result<(), StorageError> {
        let terminal_at_ms = chrono::Utc::now().timestamp_millis();
        let rows_updated = sqlx::query(
            "UPDATE port_job_dispatch_queue \
             SET status = 'Dispatched', processed_at_ms = ? \
             WHERE id = ? AND workspace_id = ? AND org_id = ? \
               AND status = 'Processing' AND claim_generation = ?",
        )
        .bind(terminal_at_ms)
        .bind(claim.row_id().as_slice())
        .bind(&claim.scope().workspace_id)
        .bind(&claim.scope().org_id)
        .bind(generation_bind(claim)?)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?
        .rows_affected();
        if rows_updated == 0 {
            return Err(self.unacknowledgeable(claim).await?);
        }
        Ok(())
    }

    async fn mark_failed(&self, claim: &JobClaimToken, error: &str) -> Result<(), StorageError> {
        let terminal_at_ms = chrono::Utc::now().timestamp_millis();
        let rows_updated = sqlx::query(
            "UPDATE port_job_dispatch_queue \
             SET status = 'Failed', error_message = ?, processed_at_ms = ? \
             WHERE id = ? AND workspace_id = ? AND org_id = ? \
               AND status = 'Processing' AND claim_generation = ?",
        )
        .bind(error)
        .bind(terminal_at_ms)
        .bind(claim.row_id().as_slice())
        .bind(&claim.scope().workspace_id)
        .bind(&claim.scope().org_id)
        .bind(generation_bind(claim)?)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?
        .rows_affected();
        if rows_updated == 0 {
            return Err(self.unacknowledgeable(claim).await?);
        }
        Ok(())
    }

    async fn reclaim_stuck(
        &self,
        reclaim_after: Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError> {
        // `processed_at_ms` is epoch-millis (INTEGER) — same representation
        // as `port_control_queue`, so reclaim cutoff arithmetic is identical.
        let terminal_at_ms = chrono::Utc::now().timestamp_millis();
        let cutoff = terminal_at_ms - i64::try_from(reclaim_after.as_millis()).unwrap_or(i64::MAX);
        let mut tx = self.pool.begin().await.map_err(conn_err)?;
        let exhausted = sqlx::query(
            "UPDATE port_job_dispatch_queue \
             SET status = 'Failed', \
                 error_message = 'reclaim exhausted: presumed dead', \
                 processed_at_ms = ? \
             WHERE status = 'Processing' AND processed_at_ms < ? \
               AND reclaim_count >= ?",
        )
        .bind(terminal_at_ms)
        .bind(cutoff)
        .bind(i64::from(max_reclaim_count))
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?
        .rows_affected();
        // `claim_generation` is deliberately untouched here: ownership is
        // cleared, but the counter only ever moves forward, so the next claim
        // mints a value strictly greater than the token this reclaim just
        // invalidated. Resetting it would let a stale token match again.
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
