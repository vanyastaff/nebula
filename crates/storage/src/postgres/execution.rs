//! Postgres `ExecutionStore` + `IdempotencyGuard`.
//!
//! `commit` runs the §12.2 triple in a real transaction: the target row is
//! locked with `SELECT … FOR UPDATE`, the fencing token is checked before
//! the version CAS, then state + journal + outbox are written and the tx
//! commits atomically. Scope is `WHERE workspace_id = $ AND org_id = $` on
//! every query so a cross-tenant `get` yields `None` and a cross-tenant
//! `commit` can never Apply.

use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_storage_port::dto::ExecutionRecord;
use nebula_storage_port::store::{ExecutionStore, IdempotencyGuard};
use nebula_storage_port::{FencingToken, Scope, StorageError, TransitionBatch, TransitionOutcome};
use sqlx::{PgPool, Row};

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

pub(super) fn conn_err(e: sqlx::Error) -> StorageError {
    StorageError::Connection(e.to_string())
}

/// Clamp the lease TTL (≥1s, ≤24h) so a zero/absurd TTL cannot make a
/// lease instantly dead or effectively eternal.
fn normalized_ttl(ttl: Duration) -> Duration {
    Duration::from_secs_f64(ttl.as_secs_f64().clamp(1.0, 86_400.0))
}

fn ttl_chrono(ttl: Duration) -> chrono::Duration {
    // `normalized_ttl` caps at 24h, well within chrono's range; the
    // fallback keeps this total instead of panicking.
    chrono::Duration::from_std(normalized_ttl(ttl))
        .unwrap_or_else(|_| chrono::Duration::seconds(86_400))
}

#[async_trait::async_trait]
impl ExecutionStore for PgExecutionStore {
    async fn create(
        &self,
        scope: &Scope,
        id: &str,
        workflow_id: &str,
        initial_state: serde_json::Value,
    ) -> Result<(), StorageError> {
        let now = Utc::now();
        let res = sqlx::query(
            "INSERT INTO port_executions \
             (id, workspace_id, org_id, workflow_id, status, state, version, \
              fencing_generation, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 'Created', $5, 0, 0, $6, $6)",
        )
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(workflow_id)
        .bind(&initial_state)
        .bind(now)
        .execute(&self.pool)
        .await;
        match res {
            Ok(_) => {
                tracing::debug!(
                    target: "nebula_storage::postgres",
                    execution_id = id,
                    workflow_id,
                    "execution created"
                );
                Ok(())
            },
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Err(StorageError::Duplicate {
                    entity: "execution",
                    detail: format!("execution {id} already exists"),
                })
            },
            Err(e) => Err(conn_err(e)),
        }
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ExecutionRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT workflow_id, status, state, version, lease_holder, \
                    fencing_generation, created_at, updated_at \
             FROM port_executions \
             WHERE id = $1 AND workspace_id = $2 AND org_id = $3",
        )
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let created: DateTime<Utc> = row.try_get("created_at").map_err(conn_err)?;
        let updated: DateTime<Utc> = row.try_get("updated_at").map_err(conn_err)?;
        Ok(Some(ExecutionRecord {
            id: id.to_string(),
            workflow_id: row.try_get("workflow_id").map_err(conn_err)?,
            scope: scope.clone(),
            version: row.try_get::<i64, _>("version").map_err(conn_err)? as u64,
            status: row.try_get("status").map_err(conn_err)?,
            state: row.try_get("state").map_err(conn_err)?,
            lease_holder: row.try_get("lease_holder").map_err(conn_err)?,
            fencing: Some(
                row.try_get::<i64, _>("fencing_generation")
                    .map_err(conn_err)? as u64,
            ),
            created_at: created.to_rfc3339(),
            updated_at: updated.to_rfc3339(),
        }))
    }

    async fn commit(&self, batch: TransitionBatch) -> Result<TransitionOutcome, StorageError> {
        let id = batch.execution_id().to_string();
        let mut tx = self.pool.begin().await.map_err(conn_err)?;

        // Lock the row for the duration of the tx so the CAS + fencing +
        // write triple is serializable against concurrent committers.
        let row = sqlx::query(
            "SELECT version, fencing_generation FROM port_executions \
             WHERE id = $1 AND workspace_id = $2 AND org_id = $3 FOR UPDATE",
        )
        .bind(&id)
        .bind(&batch.scope().workspace_id)
        .bind(&batch.scope().org_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(conn_err)?;

        let Some(row) = row else {
            tx.rollback().await.map_err(conn_err)?;
            return Ok(TransitionOutcome::VersionConflict { actual: 0 });
        };
        let cur_version = row.try_get::<i64, _>("version").map_err(conn_err)? as u64;
        let cur_gen = row
            .try_get::<i64, _>("fencing_generation")
            .map_err(conn_err)? as u64;

        if batch.fencing().generation() != cur_gen {
            tx.rollback().await.map_err(conn_err)?;
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
            tx.rollback().await.map_err(conn_err)?;
            return Ok(TransitionOutcome::VersionConflict {
                actual: cur_version,
            });
        }

        let new_version = cur_version + 1;
        let now = Utc::now();
        sqlx::query(
            "UPDATE port_executions SET state = $1, version = $2, updated_at = $3 \
             WHERE id = $4 AND workspace_id = $5 AND org_id = $6",
        )
        .bind(batch.new_state())
        .bind(new_version as i64)
        .bind(now)
        .bind(&id)
        .bind(&batch.scope().workspace_id)
        .bind(&batch.scope().org_id)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;

        let next_seq: i64 = sqlx::query(
            "SELECT COALESCE(MAX(seq), 0) + 1 AS next FROM port_execution_journal \
             WHERE execution_id = $1",
        )
        .bind(&id)
        .fetch_one(&mut *tx)
        .await
        .map_err(conn_err)?
        .try_get("next")
        .map_err(conn_err)?;
        for (offset, je) in batch.journal().iter().enumerate() {
            sqlx::query(
                "INSERT INTO port_execution_journal (execution_id, seq, payload) \
                 VALUES ($1, $2, $3)",
            )
            .bind(&id)
            .bind(next_seq + offset as i64)
            .bind(&je.payload)
            .execute(&mut *tx)
            .await
            .map_err(conn_err)?;
        }

        for msg in batch.outbox() {
            sqlx::query(
                "INSERT INTO port_control_queue \
                 (id, execution_id, workspace_id, org_id, command, status, \
                  w3c_traceparent, reclaim_count) \
                 VALUES ($1, $2, $3, $4, $5, 'Pending', $6, $7)",
            )
            .bind(msg.id.as_slice())
            .bind(&msg.execution_id)
            .bind(&msg.scope.workspace_id)
            .bind(&msg.scope.org_id)
            .bind(msg.command.as_str())
            .bind(msg.w3c_traceparent.as_deref())
            .bind(i32::try_from(msg.reclaim_count).unwrap_or(i32::MAX))
            .execute(&mut *tx)
            .await
            .map_err(conn_err)?;
        }

        tx.commit().await.map_err(conn_err)?;
        tracing::debug!(
            target: "nebula_storage::postgres",
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
        let mut tx = self.pool.begin().await.map_err(conn_err)?;
        let row = sqlx::query(
            "SELECT lease_holder, lease_expires_at, fencing_generation \
             FROM port_executions \
             WHERE id = $1 AND workspace_id = $2 AND org_id = $3 FOR UPDATE",
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
        let cur_holder: Option<String> = row.try_get("lease_holder").map_err(conn_err)?;
        let cur_exp: Option<DateTime<Utc>> = row.try_get("lease_expires_at").map_err(conn_err)?;
        let cur_gen = row
            .try_get::<i64, _>("fencing_generation")
            .map_err(conn_err)? as u64;
        let now = Utc::now();
        let live = matches!(cur_exp, Some(exp) if exp >= now);
        if live && cur_holder.as_deref() != Some(holder) {
            tx.rollback().await.map_err(conn_err)?;
            return Ok(None);
        }
        let taking_over = cur_holder.as_deref() != Some(holder);
        let new_gen = if taking_over { cur_gen + 1 } else { cur_gen };
        let new_exp = now + ttl_chrono(ttl);
        sqlx::query(
            "UPDATE port_executions \
             SET lease_holder = $1, lease_expires_at = $2, fencing_generation = $3 \
             WHERE id = $4 AND workspace_id = $5 AND org_id = $6",
        )
        .bind(holder)
        .bind(new_exp)
        .bind(new_gen as i64)
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;
        tx.commit().await.map_err(conn_err)?;
        tracing::debug!(
            target: "nebula_storage::postgres",
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
        let new_exp = Utc::now() + ttl_chrono(ttl);
        let res = sqlx::query(
            "UPDATE port_executions SET lease_expires_at = $1 \
             WHERE id = $2 AND workspace_id = $3 AND org_id = $4 \
               AND fencing_generation = $5",
        )
        .bind(new_exp)
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
             SET lease_holder = NULL, lease_expires_at = NULL \
             WHERE id = $1 AND workspace_id = $2 AND org_id = $3 \
               AND fencing_generation = $4",
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
            sqlx::query("SELECT id FROM port_executions WHERE workspace_id = $1 AND org_id = $2")
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
             WHERE workspace_id = $1 AND org_id = $2 AND workflow_id = $3",
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
                     WHERE workspace_id = $1 AND org_id = $2 AND workflow_id = $3",
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
                     WHERE workspace_id = $1 AND org_id = $2",
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

/// Postgres-backed idempotency guard. The mark key folds in the scope so a
/// cross-tenant probe cannot collide with another tenant's entry.
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

#[async_trait::async_trait]
impl IdempotencyGuard for PgIdempotencyGuard {
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
        // First-writer-wins: INSERT ... ON CONFLICT DO NOTHING then check
        // rows_affected (1 = we marked it, 0 = someone already had it).
        let res = sqlx::query(
            "INSERT INTO port_idempotency_marks (mark_key) VALUES ($1) \
             ON CONFLICT (mark_key) DO NOTHING",
        )
        .bind(&key)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(res.rows_affected() == 1)
    }
}
