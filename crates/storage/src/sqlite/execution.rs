//! SQLite `ExecutionStore` + `IdempotencyGuard` over `executions` and the
//! rows recorded beneath an execution.
//!
//! `commit` runs the §12.2 triple inside one `BEGIN IMMEDIATE` transaction:
//! a single-writer lock is taken up front so the CAS + fencing check + state
//! write + outbox append + journal append either all land or none do. Scope
//! is enforced as `WHERE org_id = ? AND workspace_id = ?` on every query so a
//! cross-tenant `get` yields `None` and a cross-tenant `commit` can never
//! Apply. Instants are INTEGER microseconds since the Unix epoch.

use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_storage_port::dto::ExecutionRecord;
use nebula_storage_port::store::{ExecutionStore, IdempotencyGuard};
use nebula_storage_port::{
    ExecutionHistoryPage, ExecutionHistoryQuery, ExecutionListingStatus, ExecutionSummary,
    FencingToken, MicrosInstant, Scope, StorageError, TransitionBatch, TransitionOutcome,
};
use sqlx::{Row, SqlitePool};

use crate::execution_listing as listing;
use crate::sql_error::{
    decode_u64, encode_u64, foreign_key_not_found, storage_error, storage_error_for,
};

/// Columns of one execution row, `state` NULLed past the size cap (`?1`).
const EXECUTION_COLUMNS: &str = "id, org_id, workspace_id, workflow_id, status, \
     CASE WHEN length(CAST(state AS BLOB)) <= ?1 THEN state END AS state, \
     version, lease_holder, fencing_generation, created_at, updated_at";

/// The database clock in microseconds since the Unix epoch.
const NOW_MICROS: &str = "CAST((julianday('now') - 2440587.5) * 86400000000.0 AS INTEGER)";

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

/// A NOT NULL instant column.
fn instant(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<MicrosInstant, StorageError> {
    listing::decode_sort_key(row.try_get(column).map_err(storage_error)?)
}

/// A nullable instant column.
fn optional_instant(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<Option<MicrosInstant>, StorageError> {
    row.try_get::<Option<i64>, _>(column)
        .map_err(storage_error)?
        .map(listing::decode_sort_key)
        .transpose()
}

/// Decode one execution row selected as [`EXECUTION_COLUMNS`] — `state`
/// already NULLed by the size cap.
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
        state: serde_json::from_str(&state)
            .map_err(|_| StorageError::Corrupt("column `state` is not the expected JSON".into()))?,
        lease_holder: row.try_get("lease_holder").map_err(storage_error)?,
        fencing: Some(decode_u64(
            row.try_get("fencing_generation").map_err(storage_error)?,
            "fencing_generation",
        )?),
        created_at: instant(row, "created_at")?.to_datetime(),
        updated_at: instant(row, "updated_at")?.to_datetime(),
    })
}

/// Clamp the lease TTL (≥1s, ≤24h) so a zero/absurd TTL cannot make a
/// lease instantly dead or effectively eternal; in microseconds.
fn normalized_ttl_micros(ttl: Duration) -> i64 {
    let clamped = Duration::from_secs_f64(ttl.as_secs_f64().clamp(1.0, 86_400.0));
    // At most 86 400 000 000 µs: always within `i64`.
    i64::try_from(clamped.as_micros()).unwrap_or(86_400_000_000)
}

fn encode_generation(token: FencingToken) -> Result<i64, StorageError> {
    encode_u64(token.generation(), "fencing_generation")
}

/// Insert a `Created` execution row inside an existing transaction.
///
/// Called by both `ExecutionStore::create` and start materialization so the
/// insert shape is defined exactly once. The workflow must be live (the
/// caller's `BEGIN IMMEDIATE` serializes the check with an archive); a
/// missing or deleted workflow is `NotFound`. A taken id is
/// `Duplicate { entity: "execution" }`.
pub(super) async fn insert_created_execution(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    scope: &Scope,
    id: &str,
    workflow_id: &str,
    initial_state: &serde_json::Value,
) -> Result<(), StorageError> {
    crate::execution_state::ensure_execution_state_size(initial_state)?;
    let workflow = sqlx::query(
        "SELECT id FROM workflows \
         WHERE org_id = ? AND workspace_id = ? AND id = ? AND deleted_at IS NULL",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(workflow_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage_error)?;
    if workflow.is_none() {
        return Err(StorageError::not_found("workflow", workflow_id));
    }
    let state = serde_json::to_string(initial_state)?;
    let now = MicrosInstant::now().as_micros();
    sqlx::query(
        "INSERT INTO executions \
         (org_id, workspace_id, id, workflow_id, status, state, version, \
          fencing_generation, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?, ?)",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(id)
    .bind(workflow_id)
    .bind(ExecutionListingStatus::Created.as_str())
    .bind(&state)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(|error| storage_error_for("execution", error))?;
    Ok(())
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
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
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
        let sql = format!(
            "SELECT {EXECUTION_COLUMNS} FROM executions \
             WHERE org_id = ?2 AND workspace_id = ?3 AND id = ?4"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(crate::execution_state::MAX_PERSISTED_EXECUTION_STATE_BYTES)
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .bind(id)
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
        // The database decides live-vs-expired and stamps the deadline, in one
        // statement. SQLite serves one host, so this is not about skew between
        // replicas — it is about there being exactly one rule for lease time
        // across all three backends, so a lease cannot mean something different
        // depending on which adapter is wired.
        let sql = format!(
            "UPDATE executions \
             SET lease_holder = ?, lease_expires_at = {NOW_MICROS} + ?, \
                 fencing_generation = fencing_generation + 1 \
             WHERE org_id = ? AND workspace_id = ? AND id = ? \
               AND (lease_expires_at IS NULL OR lease_expires_at < {NOW_MICROS}) \
             RETURNING fencing_generation"
        );
        let new_generation: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(holder)
            .bind(normalized_ttl_micros(ttl))
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage_error)?;

        if let Some(generation) = new_generation {
            return Ok(Some(FencingToken::from_generation(decode_u64(
                generation,
                "fencing_generation",
            )?)));
        }

        // Zero rows means either a live lease or no such row; only a live lease
        // is `Ok(None)`.
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM executions WHERE org_id = ? AND workspace_id = ? AND id = ?",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(id)
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
        let sql = format!(
            "UPDATE executions SET lease_expires_at = {NOW_MICROS} + ? \
             WHERE org_id = ? AND workspace_id = ? AND id = ? AND fencing_generation = ?"
        );
        let res = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(normalized_ttl_micros(ttl))
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .bind(id)
            .bind(encode_generation(token)?)
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
            "UPDATE executions SET lease_holder = NULL, lease_expires_at = NULL \
             WHERE org_id = ? AND workspace_id = ? AND id = ? AND fencing_generation = ?",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(id)
        .bind(encode_generation(token)?)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(res.rows_affected() == 1)
    }

    async fn list_all_running(&self) -> Result<Vec<ExecutionRecord>, StorageError> {
        let sql = format!(
            "SELECT {EXECUTION_COLUMNS} FROM executions \
             WHERE status IN ('created', 'running', 'paused', 'cancelling')"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
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
            "SELECT id, workflow_id, status, created_at, started_at, finished_at, updated_at \
             FROM executions WHERE org_id = ",
        );
        sql.push_bind(&scope.org_id)
            .push(" AND workspace_id = ")
            .push_bind(&scope.workspace_id);
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
            sql.push(" AND created_at >= ").push_bind(bound.as_micros());
        }
        if let Some(bound) = query.created_before() {
            sql.push(" AND created_at < ").push_bind(bound.as_micros());
        }
        if let Some(cursor) = query.cursor() {
            let key = cursor.created_at().as_micros();
            sql.push(" AND (created_at < ")
                .push_bind(key)
                .push(" OR (created_at = ")
                .push_bind(key)
                .push(" AND id < ")
                .push_bind(cursor.id())
                .push("))");
        }
        sql.push(" ORDER BY created_at DESC, id DESC LIMIT ")
            .push_bind(i64::from(query.fetch_limit()));
        let rows = sql
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(storage_error)?;
        let summaries = rows
            .iter()
            .map(|row| {
                Ok(ExecutionSummary {
                    id: row.try_get("id").map_err(storage_error)?,
                    workflow_id: row.try_get("workflow_id").map_err(storage_error)?,
                    status: listing::decode_status(
                        &row.try_get::<String, _>("status").map_err(storage_error)?,
                    )?,
                    created_at: instant(row, "created_at")?,
                    started_at: optional_instant(row, "started_at")?,
                    finished_at: optional_instant(row, "finished_at")?,
                    updated_at: instant(row, "updated_at")?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        Ok(ExecutionHistoryPage::from_overfetched(summaries, query))
    }

    async fn count(&self, scope: &Scope, workflow_id: Option<&str>) -> Result<u64, StorageError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM executions \
             WHERE org_id = ?1 AND workspace_id = ?2 AND (?3 IS NULL OR workflow_id = ?3)",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(workflow_id)
        .fetch_one(&self.pool)
        .await
        .map_err(storage_error)?;
        decode_u64(n, "count")
    }
}

/// SQLite-backed idempotency guard over `idempotency_marks`.
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
        "SELECT reference_state, rollback_window_id, retain_until \
         FROM execution_revision_references WHERE execution_id = ?",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage_error)?;

    // An execution created without a materialized start holds no catalog
    // reference; terminal transitions on it are a no-op. An existing row in
    // an incompatible lifecycle must fail the whole commit.
    let Some(reference_row) = reference_row else {
        return Ok(());
    };
    let state: String = reference_row
        .try_get("reference_state")
        .map_err(storage_error)?;
    match transition {
        nebula_storage_port::ExecutionReferenceTransition::ReleaseLive => match state.as_str() {
            "live" => {
                sqlx::query(
                    "UPDATE execution_revision_references \
                     SET reference_state = 'released', rollback_window_id = NULL, \
                         retain_until = NULL \
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
                        "terminal dereference: execution {id} reference is \
                         released from a rollback window"
                    )));
                }
            },
            _ => {
                return Err(StorageError::Internal(format!(
                    "terminal dereference: execution {id} reference is in \
                     incompatible state {state}"
                )));
            },
        },
        nebula_storage_port::ExecutionReferenceTransition::RetainRollback {
            window_id,
            retain_until,
        } => {
            let retain_until = MicrosInstant::floor(retain_until).as_micros();
            let stored_window: Option<Vec<u8>> = reference_row
                .try_get("rollback_window_id")
                .map_err(storage_error)?;
            let stored_until: Option<i64> = reference_row
                .try_get("retain_until")
                .map_err(storage_error)?;
            match state.as_str() {
                "live" => {
                    sqlx::query(
                        "UPDATE execution_revision_references \
                         SET reference_state = 'rollback', rollback_window_id = ?, \
                             retain_until = ? \
                         WHERE execution_id = ? AND reference_state = 'live'",
                    )
                    .bind(window_id.as_slice())
                    .bind(retain_until)
                    .bind(id)
                    .execute(&mut **tx)
                    .await
                    .map_err(storage_error)?;
                },
                "rollback"
                    if stored_window.as_deref() == Some(window_id.as_slice())
                        && stored_until == Some(retain_until) => {},
                _ => {
                    return Err(StorageError::Internal(format!(
                        "terminal dereference: execution {id} reference is in \
                         incompatible state {state}"
                    )));
                },
            }
        },
    }
    Ok(())
}

pub(super) async fn commit_locked(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    batch: &TransitionBatch,
) -> Result<TransitionOutcome, StorageError> {
    crate::execution_state::ensure_execution_state_size(batch.new_state())?;
    let id = batch.execution_id().to_string();
    let scope = batch.scope();

    let row = sqlx::query(
        "SELECT version, fencing_generation FROM executions \
         WHERE org_id = ? AND workspace_id = ? AND id = ?",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(&id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage_error)?;

    let Some(row) = row else {
        // Unknown id or invisible cross-tenant row: never Apply.
        return Ok(TransitionOutcome::VersionConflict { actual: 0 });
    };
    let cur_version = decode_u64(
        row.try_get::<i64, _>("version").map_err(storage_error)?,
        "version",
    )?;
    let cur_gen = decode_u64(
        row.try_get::<i64, _>("fencing_generation")
            .map_err(storage_error)?,
        "fencing_generation",
    )?;

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
    let projection = batch.listing();
    sqlx::query(
        "UPDATE executions SET state = ?, version = ?, updated_at = ?, \
                status = ?, started_at = ?, finished_at = ? \
         WHERE org_id = ? AND workspace_id = ? AND id = ?",
    )
    .bind(&new_state)
    .bind(encode_u64(new_version, "version")?)
    .bind(MicrosInstant::now().as_micros())
    .bind(projection.status().as_str())
    .bind(projection.started_at().map(MicrosInstant::as_micros))
    .bind(projection.finished_at().map(MicrosInstant::as_micros))
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(&id)
    .execute(&mut **tx)
    .await
    .map_err(storage_error)?;

    // Journal append: next seq for this execution.
    let next_seq: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM execution_journal WHERE execution_id = ?",
    )
    .bind(&id)
    .fetch_one(&mut **tx)
    .await
    .map_err(storage_error)?;
    for (offset, je) in batch.journal().iter().enumerate() {
        let seq = i64::try_from(offset)
            .ok()
            .and_then(|offset| next_seq.checked_add(offset))
            .ok_or_else(|| StorageError::Internal("execution journal sequence exhausted".into()))?;
        let payload = serde_json::to_string(&je.payload)?;
        sqlx::query(
            "INSERT INTO execution_journal (org_id, workspace_id, execution_id, seq, payload) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&id)
        .bind(seq)
        .bind(&payload)
        .execute(&mut **tx)
        .await
        .map_err(storage_error)?;
    }

    // Outbox append: raw 16-byte ULID id.
    for msg in batch.outbox() {
        let resume_target_json: Option<String> = msg
            .resume_target
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(StorageError::from)?;
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
    // state/outbox/journal writes. ON CONFLICT (execution_id, node_key)
    // DO NOTHING ensures a crash re-drive that re-parks the same node
    // does NOT mint a duplicate live token.
    for token_row in batch.resume_tokens() {
        let created_at = parse_token_instant(&token_row.created_at, "created_at")?;
        let expires_at = token_row
            .expires_at
            .as_deref()
            .map(|value| parse_token_instant(value, "expires_at"))
            .transpose()?;
        sqlx::query(
            "INSERT INTO resume_tokens \
             (org_id, workspace_id, execution_id, token_hash, node_key, \
              wait_kind, callback_label, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (execution_id, node_key) DO NOTHING",
        )
        .bind(&token_row.scope.org_id)
        .bind(&token_row.scope.workspace_id)
        .bind(&token_row.execution_id)
        .bind(token_row.token_hash.as_bytes())
        .bind(&token_row.node_key)
        .bind(token_row.wait_kind.as_str())
        .bind(&token_row.callback_label)
        .bind(created_at)
        .bind(expires_at)
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

/// A resume-token instant the engine produced as RFC 3339, as stored
/// microseconds.
fn parse_token_instant(value: &str, field: &str) -> Result<i64, StorageError> {
    DateTime::parse_from_rfc3339(value)
        .map(|instant| MicrosInstant::floor(instant.with_timezone(&Utc)).as_micros())
        .map_err(|_| StorageError::InvalidInput(format!("resume token {field} is not RFC 3339")))
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
        // First writer wins: the insert lands for exactly one caller. A mark
        // for a missing execution is `NotFound` (OR IGNORE does not cover
        // foreign keys).
        let res = sqlx::query(
            "INSERT OR IGNORE INTO idempotency_marks \
             (org_id, workspace_id, execution_id, node_key, attempt) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(execution_id)
        .bind(node_id)
        .bind(i64::from(attempt))
        .execute(&self.pool)
        .await
        .map_err(|error| foreign_key_not_found(error, "execution", execution_id))?;
        Ok(res.rows_affected() == 1)
    }
}
