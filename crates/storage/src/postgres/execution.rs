//! Postgres `ExecutionStore` + `IdempotencyGuard` over `executions` and the
//! rows recorded beneath an execution.
//!
//! `commit` runs the §12.2 triple in a real transaction: the target row is
//! locked with `SELECT … FOR UPDATE`, the fencing token is checked before
//! the version CAS, then state + journal + outbox are written and the tx
//! commits atomically. Scope is `WHERE org_id = $ AND workspace_id = $` on
//! every query so a cross-tenant `get` yields `None` and a cross-tenant
//! `commit` can never Apply.

use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_storage_port::dto::ExecutionRecord;
use nebula_storage_port::store::{ExecutionStore, IdempotencyGuard};
use nebula_storage_port::{
    ExecutionHistoryPage, ExecutionHistoryQuery, ExecutionListingStatus, ExecutionSummary,
    FencingToken, MicrosInstant, Scope, StorageError, TransitionBatch, TransitionOutcome,
};
use sqlx::{PgPool, Row};

use crate::execution_listing as listing;
use crate::sql_error::{
    decode_u64, encode_u64, foreign_key_not_found, storage_error, storage_error_for,
};

/// Columns of one execution row, `state` NULLed past the size cap (`$1`).
const EXECUTION_COLUMNS: &str = "id, org_id, workspace_id, workflow_id, status, \
     CASE WHEN octet_length(state::text) <= $1 THEN state END AS state, \
     version, lease_holder, fencing_generation, created_at, updated_at";

/// Insert a `Created` execution row inside an existing transaction.
///
/// Called by both `ExecutionStore::create` and start materialization so the
/// insert shape is defined exactly once. The workflow must be live: its row
/// is share-locked first, so a concurrent archive serializes with the insert,
/// and a missing or deleted workflow is `NotFound`. A taken id is
/// `Duplicate { entity: "execution" }`.
pub(super) async fn insert_created_execution(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    scope: &Scope,
    id: &str,
    workflow_id: &str,
    initial_state: &serde_json::Value,
) -> Result<(), StorageError> {
    crate::execution_state::ensure_execution_state_size(initial_state)?;
    let workflow = sqlx::query(
        "SELECT id FROM workflows \
         WHERE org_id = $1 AND workspace_id = $2 AND id = $3 AND deleted_at IS NULL \
         FOR SHARE",
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
    let now = MicrosInstant::now().to_datetime();
    let res = sqlx::query(
        "INSERT INTO executions \
         (org_id, workspace_id, id, workflow_id, status, state, version, \
          fencing_generation, created_at, updated_at) \
         SELECT $1, $2, $3, $4, $5, $6, 0, 0, $7, $7 \
         WHERE octet_length($6::jsonb::text) <= $8",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(id)
    .bind(workflow_id)
    .bind(ExecutionListingStatus::Created.as_str())
    .bind(initial_state)
    .bind(now)
    .bind(crate::execution_state::MAX_PERSISTED_EXECUTION_STATE_BYTES)
    .execute(&mut **tx)
    .await
    .map_err(|error| storage_error_for("execution", error))?;
    if res.rows_affected() == 1 {
        Ok(())
    } else {
        Err(crate::execution_state::oversized_execution_state())
    }
}

/// Postgres-backed execution aggregate. Wrap a pool whose schema was
/// installed via [`super::init_schema`].
#[derive(Clone, Debug)]
pub struct PgExecutionStore {
    pool: PgPool,
}

impl PgExecutionStore {
    /// Wrap an existing pool. The caller installs the port schema (see
    /// [`super::init_schema`]).
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Read a nullable `timestamptz` listing column.
fn optional_instant(
    row: &sqlx::postgres::PgRow,
    column: &str,
) -> Result<Option<MicrosInstant>, StorageError> {
    Ok(row
        .try_get::<Option<DateTime<Utc>>, _>(column)
        .map_err(storage_error)?
        .map(MicrosInstant::floor))
}

/// Clamp the lease TTL (≥1s, ≤24h) so a zero/absurd TTL cannot make a
/// lease instantly dead or effectively eternal; in milliseconds.
fn normalized_ttl_ms(ttl: Duration) -> i64 {
    let clamped = Duration::from_secs_f64(ttl.as_secs_f64().clamp(1.0, 86_400.0));
    // At most 86 400 000 ms: always within `i64`.
    i64::try_from(clamped.as_millis()).unwrap_or(86_400_000)
}

fn encode_generation(token: FencingToken) -> Result<i64, StorageError> {
    encode_u64(token.generation(), "fencing_generation")
}

/// Decode one execution row selected as [`EXECUTION_COLUMNS`] — `state`
/// already NULLed by the size cap.
fn decode_execution(row: &sqlx::postgres::PgRow) -> Result<ExecutionRecord, StorageError> {
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
        state: row
            .try_get::<Option<serde_json::Value>, _>("state")
            .map_err(storage_error)?
            .ok_or_else(crate::execution_state::oversized_execution_state)?,
        lease_holder: row.try_get("lease_holder").map_err(storage_error)?,
        fencing: Some(decode_u64(
            row.try_get("fencing_generation").map_err(storage_error)?,
            "fencing_generation",
        )?),
        created_at: row.try_get("created_at").map_err(storage_error)?,
        updated_at: row.try_get("updated_at").map_err(storage_error)?,
    })
}

#[async_trait::async_trait]
impl ExecutionStore for PgExecutionStore {
    async fn record_execution_admission_refusal(
        &self,
        refusal: &nebula_storage_port::store::ExecutionAdmissionRefusal<'_>,
    ) -> Result<nebula_storage_port::store::ExecutionAdmissionRefusalOutcome, StorageError> {
        super::control_turn::record_admission(&self.pool, refusal).await
    }

    fn backend_kind(&self) -> nebula_storage_port::StorageBackendKind {
        nebula_storage_port::StorageBackendKind::Postgres
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
            target: "nebula_storage::postgres",
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
             WHERE org_id = $2 AND workspace_id = $3 AND id = $4"
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
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let outcome = commit_locked(&mut tx, &batch).await?;
        match outcome {
            TransitionOutcome::Applied { .. } => tx.commit().await.map_err(storage_error)?,
            TransitionOutcome::FencedOut | TransitionOutcome::VersionConflict { .. } => {
                tx.rollback().await.map_err(storage_error)?;
            },
        }
        Ok(outcome)
    }

    async fn acquire_lease(
        &self,
        scope: &Scope,
        id: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<FencingToken>, StorageError> {
        // The database decides live-vs-expired and stamps the new deadline, in
        // one statement, from `clock_timestamp()`.
        //
        // Reading the clock in the client would make lease liveness depend on
        // each worker's own clock: a worker running fast would judge a healthy
        // peer's lease expired and fence it out, and a worker running slow
        // would mint a lease that is already dead. Neither is detectable from
        // the row. With every replica comparing against the same server clock,
        // skew stops being a correctness input.
        //
        // `clock_timestamp()` and not `now()`: the latter is transaction start
        // time, which would drift under a long transaction.
        let new_generation: Option<i64> = sqlx::query_scalar(
            "UPDATE executions \
             SET lease_holder = $1, \
                 lease_expires_at = clock_timestamp() + $2 * INTERVAL '1 millisecond', \
                 fencing_generation = fencing_generation + 1 \
             WHERE org_id = $3 AND workspace_id = $4 AND id = $5 \
               AND (lease_expires_at IS NULL OR lease_expires_at < clock_timestamp()) \
             RETURNING fencing_generation",
        )
        .bind(holder)
        .bind(normalized_ttl_ms(ttl))
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;

        // Every successful acquire bumps the fencing generation, so every
        // previously issued token is dead — including one held by the *same*
        // holder string (a crashed-then-restarted runner reusing its
        // `instance_id` is a zombie w.r.t. its pre-crash token). Generation 0
        // therefore universally means "no lease ever issued / stale".
        if let Some(generation) = new_generation {
            tracing::debug!(
                target: "nebula_storage::postgres",
                execution_id = id,
                holder,
                generation,
                "lease acquired"
            );
            return Ok(Some(FencingToken::from_generation(decode_u64(
                generation,
                "fencing_generation",
            )?)));
        }

        // Zero rows means either a live lease or no such row; the caller needs
        // them apart, and only a live lease is `Ok(None)`.
        let exists: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM executions WHERE org_id = $1 AND workspace_id = $2 AND id = $3",
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
        // A live lease blocks acquisition outright — including a second
        // acquire by the *same* holder. Renewal is the dedicated,
        // fencing-token-gated op; a second acquire while the lease is live is
        // contention, not a silent renew.
        Ok(None)
    }

    async fn renew_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: FencingToken,
        ttl: Duration,
    ) -> Result<bool, StorageError> {
        // Same server clock as `acquire_lease`, for the same reason: a renewal
        // stamped from a client clock could extend a lease past — or short of —
        // what every other replica believes.
        let res = sqlx::query(
            "UPDATE executions \
             SET lease_expires_at = clock_timestamp() + $1 * INTERVAL '1 millisecond' \
             WHERE org_id = $2 AND workspace_id = $3 AND id = $4 \
               AND fencing_generation = $5",
        )
        .bind(normalized_ttl_ms(ttl))
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
             WHERE org_id = $1 AND workspace_id = $2 AND id = $3 \
               AND fencing_generation = $4",
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
        let mut sql = sqlx::QueryBuilder::<sqlx::Postgres>::new(
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
            let statuses: Vec<&str> = query
                .statuses()
                .iter()
                .map(ExecutionListingStatus::as_str)
                .collect();
            sql.push(" AND status = ANY(").push_bind(statuses).push(")");
        }
        if let Some(bound) = query.created_after() {
            sql.push(" AND created_at >= ")
                .push_bind(bound.to_datetime());
        }
        if let Some(bound) = query.created_before() {
            sql.push(" AND created_at < ")
                .push_bind(bound.to_datetime());
        }
        if let Some(cursor) = query.cursor() {
            let key = cursor.created_at().to_datetime();
            sql.push(" AND (created_at < ")
                .push_bind(key)
                .push(" OR (created_at = ")
                .push_bind(key)
                .push(" AND id COLLATE \"C\" < ")
                .push_bind(cursor.id())
                .push("))");
        }
        sql.push(" ORDER BY created_at DESC, id COLLATE \"C\" DESC LIMIT ")
            .push_bind(i64::from(query.fetch_limit()));
        let rows = sql
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(storage_error)?;
        let summaries = rows
            .into_iter()
            .map(|row| {
                Ok(ExecutionSummary {
                    id: row.try_get("id").map_err(storage_error)?,
                    workflow_id: row.try_get("workflow_id").map_err(storage_error)?,
                    status: listing::decode_status(
                        &row.try_get::<String, _>("status").map_err(storage_error)?,
                    )?,
                    created_at: MicrosInstant::floor(
                        row.try_get("created_at").map_err(storage_error)?,
                    ),
                    started_at: optional_instant(&row, "started_at")?,
                    finished_at: optional_instant(&row, "finished_at")?,
                    updated_at: MicrosInstant::floor(
                        row.try_get("updated_at").map_err(storage_error)?,
                    ),
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        Ok(ExecutionHistoryPage::from_overfetched(summaries, query))
    }

    async fn count(&self, scope: &Scope, workflow_id: Option<&str>) -> Result<u64, StorageError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM executions \
             WHERE org_id = $1 AND workspace_id = $2 \
               AND ($3::text IS NULL OR workflow_id = $3)",
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

/// Postgres-backed idempotency guard over `idempotency_marks`.
#[derive(Clone, Debug)]
pub struct PgIdempotencyGuard {
    pool: PgPool,
}

impl PgIdempotencyGuard {
    /// Wrap an existing pool whose schema was installed via
    /// [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

async fn apply_reference_transition(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: &str,
    transition: nebula_storage_port::ExecutionReferenceTransition,
) -> Result<(), StorageError> {
    let reference_row = sqlx::query(
        "SELECT reference_state, rollback_window_id, retain_until \
         FROM execution_revision_references WHERE execution_id = $1",
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
                     WHERE execution_id = $1 AND reference_state = 'live'",
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
            // Stored at microsecond precision: compare at the same precision.
            let retain_until = MicrosInstant::floor(retain_until).to_datetime();
            let stored_window: Option<Vec<u8>> = reference_row
                .try_get("rollback_window_id")
                .map_err(storage_error)?;
            let stored_until: Option<DateTime<Utc>> = reference_row
                .try_get("retain_until")
                .map_err(storage_error)?;
            match state.as_str() {
                "live" => {
                    sqlx::query(
                        "UPDATE execution_revision_references \
                         SET reference_state = 'rollback', rollback_window_id = $1, \
                             retain_until = $2 \
                         WHERE execution_id = $3 AND reference_state = 'live'",
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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    batch: &TransitionBatch,
) -> Result<TransitionOutcome, StorageError> {
    crate::execution_state::ensure_execution_state_size(batch.new_state())?;
    let id = batch.execution_id().to_string();
    let scope = batch.scope();

    let row = sqlx::query(
        "SELECT version, fencing_generation FROM executions \
         WHERE org_id = $1 AND workspace_id = $2 AND id = $3 FOR UPDATE",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(&id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage_error)?;

    let Some(row) = row else {
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

    if batch.fencing().generation() != cur_gen {
        tracing::warn!(
            target: "nebula_storage::postgres",
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
    let update = sqlx::query(
        "UPDATE executions SET state = $1, version = $2, updated_at = $3, \
                status = $4, started_at = $5, finished_at = $6 \
         WHERE org_id = $7 AND workspace_id = $8 AND id = $9 \
           AND octet_length($1::jsonb::text) <= $10",
    )
    .bind(batch.new_state())
    .bind(encode_u64(new_version, "version")?)
    .bind(MicrosInstant::now().to_datetime())
    .bind(batch.listing().status().as_str())
    .bind(batch.listing().started_at().map(MicrosInstant::to_datetime))
    .bind(
        batch
            .listing()
            .finished_at()
            .map(MicrosInstant::to_datetime),
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(&id)
    .bind(crate::execution_state::MAX_PERSISTED_EXECUTION_STATE_BYTES)
    .execute(&mut **tx)
    .await
    .map_err(storage_error)?;
    if update.rows_affected() != 1 {
        return Err(crate::execution_state::oversized_execution_state());
    }

    let next_seq: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM execution_journal WHERE execution_id = $1",
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
        sqlx::query(
            "INSERT INTO execution_journal (org_id, workspace_id, execution_id, seq, payload) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&id)
        .bind(seq)
        .bind(&je.payload)
        .execute(&mut **tx)
        .await
        .map_err(storage_error)?;
    }

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
             VALUES ($1, $2, $3, $4, $5, 'Pending', $6, $7, $8)",
        )
        .bind(msg.id.as_slice())
        .bind(&msg.execution_id)
        .bind(&msg.scope.workspace_id)
        .bind(&msg.scope.org_id)
        .bind(msg.command.as_str())
        .bind(msg.w3c_traceparent.as_deref())
        .bind(i32::try_from(msg.reclaim_count).unwrap_or(i32::MAX))
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
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
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
        target: "nebula_storage::postgres",
        execution_id = %id,
        new_version,
        "commit applied (state + outbox + journal + resume_tokens + reference_transition in one tx)"
    );
    Ok(TransitionOutcome::Applied { new_version })
}

/// A resume-token instant the engine produced as RFC 3339, at storage
/// precision.
fn parse_token_instant(value: &str, field: &str) -> Result<DateTime<Utc>, StorageError> {
    DateTime::parse_from_rfc3339(value)
        .map(|instant| MicrosInstant::floor(instant.with_timezone(&Utc)).to_datetime())
        .map_err(|_| StorageError::InvalidInput(format!("resume token {field} is not RFC 3339")))
}

#[async_trait::async_trait]
impl IdempotencyGuard for PgIdempotencyGuard {
    async fn check_and_mark(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
        attempt: u32,
    ) -> Result<bool, StorageError> {
        // First writer wins: the insert lands for exactly one caller. A mark
        // for a missing execution is `NotFound`.
        let res = sqlx::query(
            "INSERT INTO idempotency_marks \
             (org_id, workspace_id, execution_id, node_key, attempt) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT DO NOTHING",
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
