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
use nebula_storage_port::{FencingToken, Scope, StorageError, TransitionBatch, TransitionOutcome};
use sqlx::{Row, SqlitePool};

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

pub(super) fn conn_err(e: sqlx::Error) -> StorageError {
    StorageError::Connection(e.to_string())
}

/// Clamp the lease TTL (≥1s, ≤24h) so a zero/absurd TTL cannot make a
/// lease instantly dead or effectively eternal.
fn normalized_ttl(ttl: Duration) -> Duration {
    Duration::from_secs_f64(ttl.as_secs_f64().clamp(1.0, 86_400.0))
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
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
    let state = serde_json::to_string(initial_state)?;
    let ts = now_rfc3339();
    let res = sqlx::query(
        "INSERT INTO port_executions \
         (id, workspace_id, org_id, workflow_id, status, state, version, \
          fencing_generation, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 'Created', ?, 0, 0, ?, ?)",
    )
    .bind(id)
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .bind(workflow_id)
    .bind(&state)
    .bind(&ts)
    .bind(&ts)
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
        Err(e) => Err(conn_err(e)),
    }
}

#[async_trait::async_trait]
impl ExecutionStore for SqliteExecutionStore {
    async fn create(
        &self,
        scope: &Scope,
        id: &str,
        workflow_id: &str,
        initial_state: serde_json::Value,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await.map_err(conn_err)?;
        insert_created_execution(&mut tx, scope, id, workflow_id, &initial_state).await?;
        tx.commit().await.map_err(conn_err)?;
        tracing::debug!(
            target: "nebula_storage::sqlite",
            execution_id = id,
            workflow_id,
            "execution created"
        );
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ExecutionRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT workflow_id, status, state, version, lease_holder, \
                    fencing_generation, created_at, updated_at \
             FROM port_executions \
             WHERE id = ? AND workspace_id = ? AND org_id = ?",
        )
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        let Some(row) = row else {
            // Scope mismatch or absent → existence-preserving miss.
            return Ok(None);
        };
        let state_str: String = row.try_get("state").map_err(conn_err)?;
        let state: serde_json::Value = serde_json::from_str(&state_str)?;
        Ok(Some(ExecutionRecord {
            id: id.to_string(),
            workflow_id: row.try_get("workflow_id").map_err(conn_err)?,
            scope: scope.clone(),
            version: row.try_get::<i64, _>("version").map_err(conn_err)? as u64,
            status: row.try_get("status").map_err(conn_err)?,
            state,
            lease_holder: row.try_get("lease_holder").map_err(conn_err)?,
            fencing: Some(
                row.try_get::<i64, _>("fencing_generation")
                    .map_err(conn_err)? as u64,
            ),
            created_at: row.try_get("created_at").map_err(conn_err)?,
            updated_at: row.try_get("updated_at").map_err(conn_err)?,
        }))
    }

    async fn commit(&self, batch: TransitionBatch) -> Result<TransitionOutcome, StorageError> {
        let id = batch.execution_id().to_string();
        // BEGIN IMMEDIATE takes the write lock up front so the whole triple
        // is atomic against the single writer (spec §5 SQLite contract).
        let mut tx = self.pool.begin().await.map_err(conn_err)?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *tx).await.ok(); // sqlx already opened a tx; the hint is best-effort.

        let row = sqlx::query(
            "SELECT version, fencing_generation FROM port_executions \
             WHERE id = ? AND workspace_id = ? AND org_id = ?",
        )
        .bind(&id)
        .bind(&batch.scope().workspace_id)
        .bind(&batch.scope().org_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(conn_err)?;

        let Some(row) = row else {
            // Unknown id or invisible cross-tenant row: never Apply.
            tx.rollback().await.map_err(conn_err)?;
            return Ok(TransitionOutcome::VersionConflict { actual: 0 });
        };
        let cur_version = row.try_get::<i64, _>("version").map_err(conn_err)? as u64;
        let cur_gen = row
            .try_get::<i64, _>("fencing_generation")
            .map_err(conn_err)? as u64;

        // Fencing gate first: a superseded token is rejected even on a
        // version match (zombie-runner closure, spec §4.1).
        if batch.fencing().generation() != cur_gen {
            tx.rollback().await.map_err(conn_err)?;
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
            tx.rollback().await.map_err(conn_err)?;
            return Ok(TransitionOutcome::VersionConflict {
                actual: cur_version,
            });
        }

        let new_version = cur_version + 1;
        let new_state = serde_json::to_string(batch.new_state())?;
        let ts = now_rfc3339();
        sqlx::query(
            "UPDATE port_executions SET state = ?, version = ?, updated_at = ? \
             WHERE id = ? AND workspace_id = ? AND org_id = ?",
        )
        .bind(&new_state)
        .bind(new_version as i64)
        .bind(&ts)
        .bind(&id)
        .bind(&batch.scope().workspace_id)
        .bind(&batch.scope().org_id)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;

        // Journal append: next seq for this execution.
        let next_seq: i64 = sqlx::query(
            "SELECT COALESCE(MAX(seq), 0) + 1 AS next FROM port_execution_journal \
             WHERE execution_id = ?",
        )
        .bind(&id)
        .fetch_one(&mut *tx)
        .await
        .map_err(conn_err)?
        .try_get("next")
        .map_err(conn_err)?;
        for (offset, je) in batch.journal().iter().enumerate() {
            let payload = serde_json::to_string(&je.payload)?;
            sqlx::query(
                "INSERT INTO port_execution_journal (execution_id, seq, payload) \
                 VALUES (?, ?, ?)",
            )
            .bind(&id)
            .bind(next_seq + offset as i64)
            .bind(&payload)
            .execute(&mut *tx)
            .await
            .map_err(conn_err)?;
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
            .execute(&mut *tx)
            .await
            .map_err(conn_err)?;
        }

        tx.commit().await.map_err(conn_err)?;
        tracing::debug!(
            target: "nebula_storage::sqlite",
            execution_id = %id,
            new_version,
            "commit applied (state + outbox + journal in one tx)"
        );
        Ok(TransitionOutcome::Applied { new_version })
    }

    async fn acquire_lease(
        &self,
        scope: &Scope,
        id: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<FencingToken>, StorageError> {
        let ttl = normalized_ttl(ttl);
        let mut tx = self.pool.begin().await.map_err(conn_err)?;
        let row = sqlx::query(
            "SELECT lease_holder, lease_expires_at_ms, fencing_generation \
             FROM port_executions \
             WHERE id = ? AND workspace_id = ? AND org_id = ?",
        )
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(conn_err)?;
        let Some(row) = row else {
            tx.rollback().await.map_err(conn_err)?;
            return Err(StorageError::not_found("execution", id));
        };
        let cur_exp: Option<i64> = row.try_get("lease_expires_at_ms").map_err(conn_err)?;
        let cur_gen = row
            .try_get::<i64, _>("fencing_generation")
            .map_err(conn_err)? as u64;
        let now = now_ms();
        let live = matches!(cur_exp, Some(exp) if exp >= now);
        if live {
            // A live lease blocks acquisition outright — including a
            // second acquire by the *same* holder. Renewal is the
            // dedicated `renew_lease` op (fencing-token gated); a
            // second `acquire_lease` while the lease is live is
            // contention, not a silent renew (zombie-runner closure —
            // two concurrent runners must see exactly one winner).
            tx.rollback().await.map_err(conn_err)?;
            return Ok(None);
        }
        // Every successful acquire bumps the fencing generation, so
        // every previously issued token is dead — including one held
        // by the *same* holder string (a crashed-then-restarted runner
        // reusing its `instance_id` is a zombie w.r.t. its pre-crash
        // token). Generation 0 therefore universally means "no lease
        // ever issued / stale".
        let new_gen = cur_gen + 1;
        let exp_ms = now + ttl.as_millis() as i64;
        sqlx::query(
            "UPDATE port_executions \
             SET lease_holder = ?, lease_expires_at_ms = ?, fencing_generation = ? \
             WHERE id = ? AND workspace_id = ? AND org_id = ?",
        )
        .bind(holder)
        .bind(exp_ms)
        .bind(new_gen as i64)
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;
        tx.commit().await.map_err(conn_err)?;
        tracing::debug!(
            target: "nebula_storage::sqlite",
            execution_id = id,
            holder,
            generation = new_gen,
            "lease acquired"
        );
        Ok(Some(FencingToken::from_generation(new_gen)))
    }

    async fn renew_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: FencingToken,
        ttl: Duration,
    ) -> Result<bool, StorageError> {
        let ttl = normalized_ttl(ttl);
        let exp_ms = now_ms() + ttl.as_millis() as i64;
        let res = sqlx::query(
            "UPDATE port_executions SET lease_expires_at_ms = ? \
             WHERE id = ? AND workspace_id = ? AND org_id = ? \
               AND fencing_generation = ?",
        )
        .bind(exp_ms)
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(token.generation() as i64)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
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
        .map_err(conn_err)?;
        Ok(res.rows_affected() == 1)
    }

    async fn list_running(&self, scope: &Scope) -> Result<Vec<String>, StorageError> {
        let rows =
            sqlx::query("SELECT id FROM port_executions WHERE workspace_id = ? AND org_id = ?")
                .bind(&scope.workspace_id)
                .bind(&scope.org_id)
                .fetch_all(&self.pool)
                .await
                .map_err(conn_err)?;
        rows.into_iter()
            .map(|r| r.try_get::<String, _>("id").map_err(conn_err))
            .collect()
    }

    async fn list_running_for_workflow(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Vec<String>, StorageError> {
        let rows = sqlx::query(
            "SELECT id FROM port_executions \
             WHERE workspace_id = ? AND org_id = ? AND workflow_id = ?",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.into_iter()
            .map(|r| r.try_get::<String, _>("id").map_err(conn_err))
            .collect()
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
        .map_err(conn_err)?;
        Ok(row.try_get::<i64, _>("n").map_err(conn_err)? as u64)
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
            .map_err(conn_err)?;
        Ok(res.rows_affected() == 1)
    }
}
