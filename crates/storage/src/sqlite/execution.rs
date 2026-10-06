//! SQLite `ExecutionStore` + `IdempotencyGuard`.
//!
//! `commit` runs the §12.2 triple inside one `BEGIN IMMEDIATE` transaction:
//! a single-writer lock is taken up front so the CAS + fencing check + state
//! write + outbox append + journal append either all land or none do. Scope
//! is enforced as `WHERE workspace_id = ? AND org_id = ?` on every query so
//! a cross-tenant `get` yields `None` and a cross-tenant `commit` can never
//! Apply.

use std::time::Duration;

use nebula_storage_port::dto::ExecutionRecord;
use nebula_storage_port::store::{ExecutionStore, IdempotencyGuard};
use nebula_storage_port::{
    ExecutionHistoryPage, ExecutionHistoryQuery, ExecutionListingStatus, ExecutionSummary,
    FencingToken, MicrosInstant, Scope, StorageError, TransitionBatch, TransitionOutcome,
};
use sqlx::{Row, SqlitePool};

use crate::execution_listing as listing;
use crate::sql_error::{decode_u64, storage_error};

/// SQLite-backed execution aggregate. Wrap a pool whose schema was
/// installed via [`super::init_schema`].
#[derive(Clone, Debug)]
pub struct SqliteExecutionStore {
    pool: SqlitePool,
}

impl SqliteExecutionStore {
    /// Wrap an existing pool. The caller installs the port schema (see
    /// [`super::init_schema`]).
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// Read an RFC 3339 text timestamp column of an execution row.
fn text_datetime(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<chrono::DateTime<chrono::Utc>, StorageError> {
    listing::decode_text_instant(&row.try_get::<String, _>(column).map_err(storage_error)?)
        .map(MicrosInstant::to_datetime)
}

/// Decode one `port_executions` row selected as `id, workspace_id, org_id,
/// workflow_id, status, state, version, lease_holder, fencing_generation,
/// created_at, updated_at` — `state` already NULLed by the size cap. The one
/// decoder of the table for this backend.
fn decode_execution(row: &sqlx::sqlite::SqliteRow) -> Result<ExecutionRecord, StorageError> {
    let state: String = row
        .try_get::<Option<String>, _>("state")
        .map_err(storage_error)?
        .ok_or_else(crate::execution_state::oversized_execution_state)?;
    Ok(ExecutionRecord {
        id: row.try_get("id").map_err(storage_error)?,
        workflow_id: row.try_get("workflow_id").map_err(storage_error)?,
        scope: Scope::new(
            row.try_get::<String, _>("workspace_id")
                .map_err(storage_error)?,
            row.try_get::<String, _>("org_id").map_err(storage_error)?,
        ),
        version: decode_u64(row.try_get("version").map_err(storage_error)?, "version")?,
        status: listing::decode_status(
            &row.try_get::<String, _>("status").map_err(storage_error)?,
        )?,
        state: serde_json::from_str(&state)?,
        lease_holder: row.try_get("lease_holder").map_err(storage_error)?,
        fencing: Some(decode_u64(
            row.try_get("fencing_generation").map_err(storage_error)?,
            "fencing_generation",
        )?),
        created_at: text_datetime(row, "created_at")?,
        updated_at: text_datetime(row, "updated_at")?,
    })
}

/// Clamp the lease TTL (≥1s, ≤24h) so a zero/absurd TTL cannot make a
/// lease instantly dead or effectively eternal.
fn normalized_ttl(ttl: Duration) -> Duration {
    Duration::from_secs_f64(ttl.as_secs_f64().clamp(1.0, 86_400.0))
}

/// Insert a `Created` execution row inside an existing transaction.
///
/// Called by both `ExecutionStore::create` (via a single-statement tx) and
/// the dedup compose so the `port_executions` INSERT shape is defined exactly
/// once.  A unique-violation maps to `StorageError::Duplicate` so both call
/// sites propagate it uniformly.
pub(super) async fn insert_created_execution(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    scope: &Scope,
    id: &str,
    workflow_id: &str,
    initial_state: &serde_json::Value,
) -> Result<(), StorageError> {
    crate::execution_state::ensure_execution_state_size(initial_state)?;
    let state = serde_json::to_string(initial_state)?;
    let created_at = MicrosInstant::now();
    let ts = listing::encode_text_instant(created_at);
    let res = sqlx::query(
        "INSERT INTO port_executions \
         (id, workspace_id, org_id, workflow_id, status, state, version, \
          fencing_generation, created_at, updated_at, created_at_us) \
         VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?, ?, ?)",
    )
    .bind(id)
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .bind(workflow_id)
    .bind(ExecutionListingStatus::Created.as_str())
    .bind(&state)
    .bind(&ts)
    .bind(&ts)
    .bind(created_at.as_micros())
    .execute(&mut **tx)
    .await;
    match res {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
            Err(StorageError::Duplicate {
                entity: "execution",
                detail: format!("execution {id} already exists"),
            })
        },
        Err(e) => Err(storage_error(e)),
    }
}

#[async_trait::async_trait]
impl ExecutionStore for SqliteExecutionStore {
    async fn record_execution_admission_refusal(
        &self,
        refusal: &nebula_storage_port::store::ExecutionAdmissionRefusal<'_>,
    ) -> Result<nebula_storage_port::store::ExecutionAdmissionRefusalOutcome, StorageError> {
        super::control_turn::record_admission(&self.pool, refusal).await
    }

    fn backend_kind(&self) -> nebula_storage_port::StorageBackendKind {
        nebula_storage_port::StorageBackendKind::Sqlite
    }

    async fn create(
        &self,
        scope: &Scope,
        id: &str,
        workflow_id: &str,
        initial_state: serde_json::Value,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        insert_created_execution(&mut tx, scope, id, workflow_id, &initial_state).await?;
        tx.commit().await.map_err(storage_error)?;
        tracing::debug!(
            target: "nebula_storage::sqlite",
            execution_id = id,
            workflow_id,
            "execution created"
        );
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ExecutionRecord>, StorageError> {
        // Scope mismatch or absent → `None`: an existence-preserving miss.
        sqlx::query(
            "SELECT id, workspace_id, org_id, workflow_id, status, \
                    CASE WHEN length(CAST(state AS BLOB)) <= ? THEN state END AS state, \
                    version, lease_holder, fencing_generation, created_at, updated_at \
             FROM port_executions \
             WHERE id = ? AND workspace_id = ? AND org_id = ?",
        )
        .bind(crate::execution_state::MAX_PERSISTED_EXECUTION_STATE_BYTES)
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .as_ref()
        .map(decode_execution)
        .transpose()
    }

    async fn commit(&self, batch: TransitionBatch) -> Result<TransitionOutcome, StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        let outcome = commit_locked(&mut tx, &batch).await?;
        tx.commit().await.map_err(storage_error)?;
        Ok(outcome)
    }

    async fn acquire_lease(
        &self,
        scope: &Scope,
        id: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<FencingToken>, StorageError> {
        let ttl_ms = i64::try_from(normalized_ttl(ttl).as_millis()).unwrap_or(i64::MAX);
        // The database decides live-vs-expired and stamps the deadline, in one
        // statement. SQLite serves one host, so this is not about skew between
        // replicas — it is about there being exactly one rule for lease time
        // across all three backends, so a lease cannot mean something different
        // depending on which adapter is wired.
        let new_generation: Option<i64> = sqlx::query_scalar(
            "UPDATE port_executions \
             SET lease_holder = ?, \
                 lease_expires_at_ms = \
                     CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) + ?, \
                 fencing_generation = fencing_generation + 1 \
             WHERE id = ? AND workspace_id = ? AND org_id = ? \
               AND (lease_expires_at_ms IS NULL \
                    OR lease_expires_at_ms \
                       < CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)) \
             RETURNING fencing_generation",
        )
        .bind(holder)
        .bind(ttl_ms)
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;

        if let Some(generation) = new_generation {
            return Ok(Some(FencingToken::from_generation(generation as u64)));
        }

        // Zero rows means either a live lease or no such row; only a live lease
        // is `Ok(None)`.
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM port_executions \
             WHERE id = ? AND workspace_id = ? AND org_id = ?",
        )
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        if exists.is_none() {
            return Err(StorageError::not_found("execution", id));
        }
        Ok(None)
    }

    async fn renew_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: FencingToken,
        ttl: Duration,
    ) -> Result<bool, StorageError> {
        let ttl_ms = i64::try_from(normalized_ttl(ttl).as_millis()).unwrap_or(i64::MAX);
        let res = sqlx::query(
            "UPDATE port_executions \
             SET lease_expires_at_ms = \
                 CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) + ? \
             WHERE id = ? AND workspace_id = ? AND org_id = ? \
               AND fencing_generation = ?",
        )
        .bind(ttl_ms)
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(token.generation() as i64)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(res.rows_affected() == 1)
    }

    async fn release_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: FencingToken,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE port_executions \
             SET lease_holder = NULL, lease_expires_at_ms = NULL \
             WHERE id = ? AND workspace_id = ? AND org_id = ? \
               AND fencing_generation = ?",
        )
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(token.generation() as i64)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(res.rows_affected() == 1)
    }

    async fn list_all_running(&self) -> Result<Vec<ExecutionRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, org_id, workflow_id, status, \
                    CASE WHEN length(CAST(state AS BLOB)) <= ? THEN state END AS state, version, \
                    lease_holder, fencing_generation, created_at, updated_at \
             FROM port_executions \
             WHERE status IN ('created', 'running', 'paused', 'cancelling')",
        )
        .bind(crate::execution_state::MAX_PERSISTED_EXECUTION_STATE_BYTES)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;
        rows.iter().map(decode_execution).collect()
    }

    async fn list_history(
        &self,
        scope: &Scope,
        query: &ExecutionHistoryQuery,
    ) -> Result<ExecutionHistoryPage, StorageError> {
        let mut sql = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT id, workflow_id, status, created_at_us, started_at, finished_at, updated_at \
             FROM port_executions WHERE workspace_id = ",
        );
        sql.push_bind(&scope.workspace_id)
            .push(" AND org_id = ")
            .push_bind(&scope.org_id);
        if let Some(workflow_id) = query.workflow_id() {
            sql.push(" AND workflow_id = ").push_bind(workflow_id);
        }
        if !query.statuses().is_all() {
            sql.push(" AND status IN (");
            let mut statuses = sql.separated(", ");
            for status in query.statuses().iter() {
                statuses.push_bind(status.as_str());
            }
            statuses.push_unseparated(")");
        }
        if let Some(bound) = query.created_after() {
            sql.push(" AND created_at_us >= ")
                .push_bind(bound.as_micros());
        }
        if let Some(bound) = query.created_before() {
            sql.push(" AND created_at_us < ")
                .push_bind(bound.as_micros());
        }
        if let Some(cursor) = query.cursor() {
            let key = cursor.created_at().as_micros();
            sql.push(" AND (created_at_us < ")
                .push_bind(key)
                .push(" OR (created_at_us = ")
                .push_bind(key)
                .push(" AND id < ")
                .push_bind(cursor.id())
                .push("))");
        }
        sql.push(" ORDER BY created_at_us DESC, id DESC LIMIT ")
            .push_bind(i64::from(query.fetch_limit()));
        let rows = sql
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(storage_error)?;
        let summaries = rows
            .into_iter()
            .map(|row| {
                let optional_instant = |column: &str| -> Result<_, StorageError> {
                    row.try_get::<Option<String>, _>(column)
                        .map_err(storage_error)?
                        .as_deref()
                        .map(listing::decode_text_instant)
                        .transpose()
                };
                Ok(ExecutionSummary {
                    id: row.try_get("id").map_err(storage_error)?,
                    workflow_id: row.try_get("workflow_id").map_err(storage_error)?,
                    status: listing::decode_status(
                        &row.try_get::<String, _>("status").map_err(storage_error)?,
                    )?,
                    created_at: listing::decode_sort_key(
                        row.try_get("created_at_us").map_err(storage_error)?,
                    )?,
                    started_at: optional_instant("started_at")?,
                    finished_at: optional_instant("finished_at")?,
                    updated_at: listing::decode_text_instant(
                        &row.try_get::<String, _>("updated_at")
                            .map_err(storage_error)?,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        Ok(ExecutionHistoryPage::from_overfetched(summaries, query))
    }

    async fn count(&self, scope: &Scope, workflow_id: Option<&str>) -> Result<u64, StorageError> {
        let row = match workflow_id {
            Some(wf) => {
                sqlx::query(
                    "SELECT COUNT(*) AS n FROM port_executions \
                     WHERE workspace_id = ? AND org_id = ? AND workflow_id = ?",
                )
                .bind(&scope.workspace_id)
                .bind(&scope.org_id)
                .bind(wf)
                .fetch_one(&self.pool)
                .await
            },
            None => {
                sqlx::query(
                    "SELECT COUNT(*) AS n FROM port_executions \
                     WHERE workspace_id = ? AND org_id = ?",
                )
                .bind(&scope.workspace_id)
                .bind(&scope.org_id)
                .fetch_one(&self.pool)
                .await
            },
        }
        .map_err(storage_error)?;
        Ok(row.try_get::<i64, _>("n").map_err(storage_error)? as u64)
    }
}

/// SQLite-backed idempotency guard. The mark key folds in the scope so a
/// cross-tenant probe cannot collide with another tenant's entry.
#[derive(Clone, Debug)]
pub struct SqliteIdempotencyGuard {
    pool: SqlitePool,
}

impl SqliteIdempotencyGuard {
    /// Wrap an existing pool whose schema was installed via
    /// [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

async fn apply_reference_transition(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    transition: nebula_storage_port::ExecutionReferenceTransition,
) -> Result<(), StorageError> {
    let reference_row = sqlx::query(
        "SELECT reference_state, rollback_window_id, retain_until_ms \
         FROM port_execution_revision_refs WHERE execution_id = ?",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage_error)?;

    // A missing row is a legacy execution that predates materialized
    // starts; terminal transitions on it are a no-op. An existing row
    // in an incompatible lifecycle must fail the whole commit.
    if let Some(reference_row) = reference_row {
        let state: String = reference_row
            .try_get("reference_state")
            .map_err(storage_error)?;
        match transition {
            nebula_storage_port::ExecutionReferenceTransition::ReleaseLive => {
                match state.as_str() {
                    "live" => {
                        sqlx::query(
                            "UPDATE port_execution_revision_refs \
                             SET reference_state = 'released', rollback_window_id = NULL, \
                                 retain_until_ms = NULL \
                             WHERE execution_id = ? AND reference_state = 'live'",
                        )
                        .bind(id)
                        .execute(&mut **tx)
                        .await
                        .map_err(storage_error)?;
                    },
                    "released" => {
                        let window: Option<Vec<u8>> = reference_row
                            .try_get("rollback_window_id")
                            .map_err(storage_error)?;
                        if window.is_some() {
                            return Err(StorageError::Internal(format!(
                                "terminal dereference: execution {id} reference is                                          released from a rollback window"
                            )));
                        }
                    },
                    _ => {
                        return Err(StorageError::Internal(format!(
                            "terminal dereference: execution {id} reference is in                                      incompatible state {state}"
                        )));
                    },
                }
            },
            nebula_storage_port::ExecutionReferenceTransition::RetainRollback {
                window_id,
                retain_until,
            } => {
                let stored_window: Option<Vec<u8>> = reference_row
                    .try_get("rollback_window_id")
                    .map_err(storage_error)?;
                let stored_until: Option<i64> = reference_row
                    .try_get("retain_until_ms")
                    .map_err(storage_error)?;
                match state.as_str() {
                    "live" => {
                        sqlx::query(
                            "UPDATE port_execution_revision_refs \
                             SET reference_state = 'rollback', rollback_window_id = ?, \
                                 retain_until_ms = ? \
                             WHERE execution_id = ? AND reference_state = 'live'",
                        )
                        .bind(window_id.as_slice())
                        .bind(retain_until.timestamp_millis())
                        .bind(id)
                        .execute(&mut **tx)
                        .await
                        .map_err(storage_error)?;
                    },
                    "rollback"
                        if stored_window.as_deref() == Some(window_id.as_slice())
                            && stored_until == Some(retain_until.timestamp_millis()) => {},
                    _ => {
                        return Err(StorageError::Internal(format!(
                            "terminal dereference: execution {id} reference is in                                      incompatible state {state}"
                        )));
                    },
                }
            },
        }
    }

    Ok(())
}

pub(super) async fn commit_locked(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    batch: &TransitionBatch,
) -> Result<TransitionOutcome, StorageError> {
    crate::execution_state::ensure_execution_state_size(batch.new_state())?;
    let id = batch.execution_id().to_string();

    let row = sqlx::query(
        "SELECT version, fencing_generation FROM port_executions \
         WHERE id = ? AND workspace_id = ? AND org_id = ?",
    )
    .bind(&id)
    .bind(&batch.scope().workspace_id)
    .bind(&batch.scope().org_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage_error)?;

    let Some(row) = row else {
        // Unknown id or invisible cross-tenant row: never Apply.
        return Ok(TransitionOutcome::VersionConflict { actual: 0 });
    };
    let cur_version = row.try_get::<i64, _>("version").map_err(storage_error)? as u64;
    let cur_gen = row
        .try_get::<i64, _>("fencing_generation")
        .map_err(storage_error)? as u64;

    // Fencing gate first: a superseded token is rejected even on a
    // version match (zombie-runner closure, spec §4.1).
    if batch.fencing().generation() != cur_gen {
        tracing::warn!(
            target: "nebula_storage::sqlite",
            execution_id = %id,
            caller_generation = batch.fencing().generation(),
            current_generation = cur_gen,
            "commit fenced out: caller token superseded"
        );
        return Ok(TransitionOutcome::FencedOut);
    }
    if cur_version != batch.expected_version() {
        return Ok(TransitionOutcome::VersionConflict {
            actual: cur_version,
        });
    }

    let new_version = cur_version
        .checked_add(1)
        .filter(|value| i64::try_from(*value).is_ok())
        .ok_or_else(|| StorageError::Internal("execution version exhausted".into()))?;
    let new_state = serde_json::to_string(batch.new_state())?;
    let ts = listing::encode_text_instant(MicrosInstant::now());
    let projection = batch.listing();
    sqlx::query(
        "UPDATE port_executions SET state = ?, version = ?, updated_at = ?, \
                status = ?, started_at = ?, finished_at = ? \
         WHERE id = ? AND workspace_id = ? AND org_id = ?",
    )
    .bind(&new_state)
    .bind(new_version as i64)
    .bind(&ts)
    .bind(projection.status().as_str())
    .bind(projection.started_at().map(listing::encode_text_instant))
    .bind(projection.finished_at().map(listing::encode_text_instant))
    .bind(&id)
    .bind(&batch.scope().workspace_id)
    .bind(&batch.scope().org_id)
    .execute(&mut **tx)
    .await
    .map_err(storage_error)?;

    // Journal append: next seq for this execution.
    let next_seq: i64 = sqlx::query(
        "SELECT COALESCE(MAX(seq), 0) + 1 AS next FROM port_execution_journal \
         WHERE execution_id = ?",
    )
    .bind(&id)
    .fetch_one(&mut **tx)
    .await
    .map_err(storage_error)?
    .try_get("next")
    .map_err(storage_error)?;
    for (offset, je) in batch.journal().iter().enumerate() {
        let payload = serde_json::to_string(&je.payload)?;
        sqlx::query(
            "INSERT INTO port_execution_journal (execution_id, seq, payload) \
             VALUES (?, ?, ?)",
        )
        .bind(&id)
        .bind(next_seq + offset as i64)
        .bind(&payload)
        .execute(&mut **tx)
        .await
        .map_err(storage_error)?;
    }

    // Outbox append: raw 16-byte ULID id (no UTF-8-of-ULID hack).
    for msg in batch.outbox() {
        let resume_target_json: Option<String> = msg
            .resume_target
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO port_control_queue \
             (id, execution_id, workspace_id, org_id, command, status, \
              w3c_traceparent, reclaim_count, resume_target) \
             VALUES (?, ?, ?, ?, ?, 'Pending', ?, ?, ?)",
        )
        .bind(msg.id.as_slice())
        .bind(&msg.execution_id)
        .bind(&msg.scope.workspace_id)
        .bind(&msg.scope.org_id)
        .bind(msg.command.as_str())
        .bind(msg.w3c_traceparent.as_deref())
        .bind(i64::from(msg.reclaim_count))
        .bind(resume_target_json)
        .execute(&mut **tx)
        .await
        .map_err(storage_error)?;
    }

    // Insert resume-token rows in the same transaction as the
    // state/outbox/journal writes.  ON CONFLICT(execution_id, node_key)
    // DO NOTHING ensures a crash re-drive that re-parks the same node
    // does NOT mint a duplicate live token.
    for token_row in batch.resume_tokens() {
        sqlx::query(
            "INSERT INTO port_resume_tokens \
             (token_hash, workspace_id, org_id, execution_id, node_key, \
              wait_kind, callback_label, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(execution_id, node_key) DO NOTHING",
        )
        .bind(token_row.token_hash.as_bytes())
        .bind(&token_row.scope.workspace_id)
        .bind(&token_row.scope.org_id)
        .bind(&token_row.execution_id)
        .bind(&token_row.node_key)
        .bind(token_row.wait_kind.as_str())
        .bind(&token_row.callback_label)
        .bind(&token_row.created_at)
        .bind(token_row.expires_at.as_deref())
        .execute(&mut **tx)
        .await
        .map_err(storage_error)?;
    }

    if let Some(transition) = batch.reference_transition() {
        apply_reference_transition(tx, &id, transition).await?;
    }

    tracing::debug!(
        target: "nebula_storage::sqlite",
        execution_id = %id,
        new_version,
        "commit applied (state + outbox + journal + resume_tokens + reference_transition in one tx)"
    );
    Ok(TransitionOutcome::Applied { new_version })
}

#[async_trait::async_trait]
impl IdempotencyGuard for SqliteIdempotencyGuard {
    async fn check_and_mark(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
        attempt: u32,
    ) -> Result<bool, StorageError> {
        let key = format!(
            "{}:{}:{execution_id}:{node_id}:{attempt}",
            scope.workspace_id, scope.org_id
        );
        // First-writer-wins: INSERT OR IGNORE then check rows_affected.
        let res = sqlx::query("INSERT OR IGNORE INTO port_idempotency_marks (mark_key) VALUES (?)")
            .bind(&key)
            .execute(&self.pool)
            .await
            .map_err(storage_error)?;
        Ok(res.rows_affected() == 1)
    }
}
