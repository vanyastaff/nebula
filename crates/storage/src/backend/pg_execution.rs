//! Postgres-backed implementation of [`ExecutionRepo`](crate::ExecutionRepo).
//!
//! Uses the `executions`, `node_outputs`, `execution_journal`, and
//! `idempotency_keys` tables created by the SQL migrations in `./migrations`.

use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;
#[cfg(all(test, feature = "postgres"))]
use nebula_core::node_key;
use nebula_core::{ExecutionId, NodeKey, WorkflowId};
use sqlx::{Pool, Postgres, types::Json};

use crate::execution_repo::{
    ExecutionRepo, ExecutionRepoError, MAX_SUPPORTED_RESULT_SCHEMA_VERSION, NodeResultRecord,
};

/// Postgres-backed execution repository.
#[derive(Clone, Debug)]
pub struct PgExecutionRepo {
    pool: Pool<Postgres>,
}

impl PgExecutionRepo {
    /// Create a new [`PgExecutionRepo`] from an existing connection pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    /// Returns a reference to the underlying connection pool.
    #[must_use]
    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }
}

/// Map a sqlx error to an [`ExecutionRepoError`].
fn map_err(err: sqlx::Error) -> ExecutionRepoError {
    ExecutionRepoError::Connection(err.to_string())
}

fn map_journal_err(err: sqlx::Error, id: ExecutionId) -> ExecutionRepoError {
    if let Some(db_err) = err.as_database_error()
        && db_err.code().as_deref() == Some("23503")
    {
        return ExecutionRepoError::not_found("execution", id.to_string());
    }
    map_err(err)
}

fn map_idempotency_err(err: sqlx::Error, execution_id: ExecutionId) -> ExecutionRepoError {
    if let Some(db_err) = err.as_database_error()
        && db_err.code().as_deref() == Some("23503")
    {
        return ExecutionRepoError::not_found("execution", execution_id.to_string());
    }
    map_err(err)
}

/// Compute lease TTL as whole seconds, clamping to a safe range.
///
/// Returns the number of seconds as `f64` for use in
/// `NOW() + make_interval(secs => $N)`.
fn ttl_seconds(ttl: Duration) -> f64 {
    let secs = ttl.as_secs_f64();
    // Cap at 24 hours, floor at 1 second.
    secs.clamp(1.0, 86_400.0)
}

#[async_trait]
impl ExecutionRepo for PgExecutionRepo {
    async fn get_state(
        &self,
        id: ExecutionId,
    ) -> Result<Option<(u64, serde_json::Value)>, ExecutionRepoError> {
        let row = sqlx::query_as::<_, (i64, Json<serde_json::Value>)>(
            "SELECT version, state FROM executions WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(|(version, state)| (version.max(0) as u64, state.0)))
    }

    async fn transition(
        &self,
        id: ExecutionId,
        expected_version: u64,
        new_state: serde_json::Value,
    ) -> Result<bool, ExecutionRepoError> {
        let result = sqlx::query(
            "UPDATE executions \
             SET version = $3 + 1, state = $2, updated_at = NOW() \
             WHERE id = $1 AND version = $3",
        )
        .bind(id)
        .bind(Json(&new_state))
        .bind(expected_version as i64)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_journal(
        &self,
        id: ExecutionId,
    ) -> Result<Vec<serde_json::Value>, ExecutionRepoError> {
        let rows = sqlx::query_as::<_, (Json<serde_json::Value>,)>(
            "SELECT entry FROM execution_journal \
             WHERE execution_id = $1 \
             ORDER BY created_at, id",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(rows.into_iter().map(|(entry,)| entry.0).collect())
    }

    async fn append_journal(
        &self,
        id: ExecutionId,
        entry: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        sqlx::query(
            "INSERT INTO execution_journal (execution_id, entry) \
             VALUES ($1, $2)",
        )
        .bind(id)
        .bind(Json(&entry))
        .execute(&self.pool)
        .await
        .map_err(|e| map_journal_err(e, id))?;

        Ok(())
    }

    async fn acquire_lease(
        &self,
        id: ExecutionId,
        holder: String,
        ttl: Duration,
    ) -> Result<bool, ExecutionRepoError> {
        let secs = ttl_seconds(ttl);
        let result = sqlx::query(
            "UPDATE executions \
             SET lease_holder = $2, \
                 lease_expires_at = NOW() + make_interval(secs => $3) \
             WHERE id = $1 \
               AND (lease_holder IS NULL OR lease_expires_at < NOW())",
        )
        .bind(id)
        .bind(&holder)
        .bind(secs)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(result.rows_affected() > 0)
    }

    async fn renew_lease(
        &self,
        id: ExecutionId,
        holder: &str,
        ttl: Duration,
    ) -> Result<bool, ExecutionRepoError> {
        let secs = ttl_seconds(ttl);
        let result = sqlx::query(
            "UPDATE executions \
             SET lease_expires_at = NOW() + make_interval(secs => $3) \
             WHERE id = $1 AND lease_holder = $2",
        )
        .bind(id)
        .bind(holder)
        .bind(secs)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(result.rows_affected() > 0)
    }

    async fn release_lease(
        &self,
        id: ExecutionId,
        holder: &str,
    ) -> Result<bool, ExecutionRepoError> {
        let result = sqlx::query(
            "UPDATE executions \
             SET lease_holder = NULL, lease_expires_at = NULL \
             WHERE id = $1 AND lease_holder = $2",
        )
        .bind(id)
        .bind(holder)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(result.rows_affected() > 0)
    }

    async fn create(
        &self,
        id: ExecutionId,
        workflow_id: WorkflowId,
        state: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        let result = sqlx::query(
            "INSERT INTO executions (id, workflow_id, version, state) \
             VALUES ($1, $2, 1, $3)",
        )
        .bind(id)
        .bind(workflow_id)
        .bind(Json(&state))
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
                // A unique-constraint violation guarantees the row exists, so
                // fetch its current version to provide an accurate Conflict error.
                let actual_version =
                    sqlx::query_scalar::<_, i64>("SELECT version FROM executions WHERE id = $1")
                        .bind(id)
                        .fetch_optional(&self.pool)
                        .await
                        .map_err(map_err)?
                        .ok_or_else(|| {
                            ExecutionRepoError::Internal(format!(
                                "execution {id} not found after unique-constraint violation"
                            ))
                        })
                        .and_then(|v| {
                            u64::try_from(v).map_err(|_| {
                                ExecutionRepoError::Internal(format!(
                                    "execution {id} version {v} overflows u64"
                                ))
                            })
                        })?;

                Err(ExecutionRepoError::Conflict {
                    entity: "execution".into(),
                    id: id.to_string(),
                    expected_version: 0,
                    actual_version,
                })
            },
            Err(err) => Err(map_err(err)),
        }
    }

    async fn save_node_output(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        attempt: u32,
        output: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        let attempt_i32 = i32::try_from(attempt).map_err(|_| {
            ExecutionRepoError::Internal(format!("attempt value {attempt} exceeds i32::MAX"))
        })?;
        sqlx::query(
            "INSERT INTO node_outputs (execution_id, node_id, attempt, output) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (execution_id, node_id, attempt) \
             DO UPDATE SET output = EXCLUDED.output",
        )
        .bind(execution_id)
        .bind(node_key)
        .bind(attempt_i32)
        .bind(Json(&output))
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(())
    }

    async fn load_node_output(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
    ) -> Result<Option<serde_json::Value>, ExecutionRepoError> {
        // `output` is nullable since ADR-0009 migration 0009; rows written
        // only via `save_node_result` carry NULL in the legacy column and
        // are skipped here.
        let row = sqlx::query_as::<_, (Json<serde_json::Value>,)>(
            "SELECT output FROM node_outputs \
             WHERE execution_id = $1 AND node_id = $2 AND output IS NOT NULL \
             ORDER BY attempt DESC \
             LIMIT 1",
        )
        .bind(execution_id)
        .bind(node_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(|(output,)| output.0))
    }

    async fn load_all_outputs(
        &self,
        execution_id: ExecutionId,
    ) -> Result<HashMap<NodeKey, serde_json::Value>, ExecutionRepoError> {
        let rows = sqlx::query_as::<_, (NodeKey, Json<serde_json::Value>)>(
            "SELECT DISTINCT ON (node_id) node_id, output \
             FROM node_outputs \
             WHERE execution_id = $1 AND output IS NOT NULL \
             ORDER BY node_id, attempt DESC",
        )
        .bind(execution_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(rows
            .into_iter()
            .map(|(node_key, output)| (node_key, output.0))
            .collect())
    }

    async fn list_running(&self) -> Result<Vec<ExecutionId>, ExecutionRepoError> {
        let rows = sqlx::query_as::<_, (ExecutionId,)>(
            "SELECT id FROM executions \
             WHERE state->>'status' IN ('created', 'running', 'paused', 'cancelling')",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    async fn count(&self, workflow_id: Option<WorkflowId>) -> Result<u64, ExecutionRepoError> {
        let count: i64 = match workflow_id {
            Some(wid) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM executions WHERE workflow_id = $1")
                    .bind(wid)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(map_err)?
            },
            None => sqlx::query_scalar("SELECT COUNT(*) FROM executions")
                .fetch_one(&self.pool)
                .await
                .map_err(map_err)?,
        };

        Ok(count.max(0) as u64)
    }

    async fn check_idempotency(&self, key: &str) -> Result<bool, ExecutionRepoError> {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM idempotency_keys WHERE key = $1)")
                .bind(key)
                .fetch_one(&self.pool)
                .await
                .map_err(map_err)?;

        Ok(exists)
    }

    async fn mark_idempotent(
        &self,
        key: &str,
        execution_id: ExecutionId,
        node_key: NodeKey,
    ) -> Result<(), ExecutionRepoError> {
        sqlx::query(
            "INSERT INTO idempotency_keys (key, execution_id, node_id) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (key) DO NOTHING",
        )
        .bind(key)
        .bind(execution_id)
        .bind(node_key)
        .execute(&self.pool)
        .await
        .map_err(|e| map_idempotency_err(e, execution_id))?;

        Ok(())
    }

    // ── ADR-0009: workflow input + node results (B1 foundation) ────────────

    async fn set_workflow_input(
        &self,
        execution_id: ExecutionId,
        input: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        let result =
            sqlx::query("UPDATE executions SET input = $2, updated_at = NOW() WHERE id = $1")
                .bind(execution_id)
                .bind(Json(&input))
                .execute(&self.pool)
                .await
                .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(ExecutionRepoError::not_found(
                "execution",
                execution_id.to_string(),
            ));
        }
        Ok(())
    }

    async fn get_workflow_input(
        &self,
        execution_id: ExecutionId,
    ) -> Result<Option<serde_json::Value>, ExecutionRepoError> {
        let row = sqlx::query_as::<_, (Option<Json<serde_json::Value>>,)>(
            "SELECT input FROM executions WHERE id = $1",
        )
        .bind(execution_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.and_then(|(input,)| input.map(|j| j.0)))
    }

    async fn save_node_result(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        attempt: u32,
        record: NodeResultRecord,
    ) -> Result<(), ExecutionRepoError> {
        let attempt_i32 = i32::try_from(attempt).map_err(|_| {
            ExecutionRepoError::Internal(format!("attempt value {attempt} exceeds i32::MAX"))
        })?;
        let version_i32 = i32::try_from(record.schema_version).map_err(|_| {
            ExecutionRepoError::Internal(format!(
                "result_schema_version {} exceeds i32::MAX",
                record.schema_version
            ))
        })?;

        // Preserves any existing `output` on conflict (set by `save_node_output`)
        // and updates only the new variant columns — legacy and new writers
        // compose on the same (execution_id, node_id, attempt) row.
        sqlx::query(
            "INSERT INTO node_outputs \
             (execution_id, node_id, attempt, output, result_schema_version, result_kind, result) \
             VALUES ($1, $2, $3, NULL, $4, $5, $6) \
             ON CONFLICT (execution_id, node_id, attempt) \
             DO UPDATE SET \
                 result_schema_version = EXCLUDED.result_schema_version, \
                 result_kind           = EXCLUDED.result_kind, \
                 result                = EXCLUDED.result",
        )
        .bind(execution_id)
        .bind(node_key)
        .bind(attempt_i32)
        .bind(version_i32)
        .bind(&record.kind)
        .bind(Json(&record.result))
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(())
    }

    async fn load_node_result(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
    ) -> Result<Option<NodeResultRecord>, ExecutionRepoError> {
        // Filter on `result IS NOT NULL` so rows written only by legacy
        // `save_node_output` are invisible to the new reader (they have
        // no variant information). Pre-migration rows also carry
        // `result_schema_version = 1` via DEFAULT but no `result`, and
        // are correctly skipped here.
        let row = sqlx::query_as::<_, (i32, String, Json<serde_json::Value>)>(
            "SELECT result_schema_version, result_kind, result FROM node_outputs \
             WHERE execution_id = $1 AND node_id = $2 AND result IS NOT NULL \
             ORDER BY attempt DESC \
             LIMIT 1",
        )
        .bind(execution_id)
        .bind(node_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        let Some((version_i32, kind, result)) = row else {
            return Ok(None);
        };
        let version = u32::try_from(version_i32).map_err(|_| {
            ExecutionRepoError::Internal(format!(
                "result_schema_version {version_i32} is negative in database"
            ))
        })?;
        if version > MAX_SUPPORTED_RESULT_SCHEMA_VERSION {
            return Err(ExecutionRepoError::UnknownSchemaVersion {
                version,
                max_supported: MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
            });
        }
        Ok(Some(NodeResultRecord::with_version(
            version, kind, result.0,
        )))
    }

    async fn load_all_results(
        &self,
        execution_id: ExecutionId,
    ) -> Result<HashMap<NodeKey, NodeResultRecord>, ExecutionRepoError> {
        let rows = sqlx::query_as::<_, (NodeKey, i32, String, Json<serde_json::Value>)>(
            "SELECT DISTINCT ON (node_id) node_id, result_schema_version, result_kind, result \
             FROM node_outputs \
             WHERE execution_id = $1 AND result IS NOT NULL \
             ORDER BY node_id, attempt DESC",
        )
        .bind(execution_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        let mut out = HashMap::with_capacity(rows.len());
        for (node_key, version_i32, kind, result) in rows {
            let version = u32::try_from(version_i32).map_err(|_| {
                ExecutionRepoError::Internal(format!(
                    "result_schema_version {version_i32} is negative in database"
                ))
            })?;
            if version > MAX_SUPPORTED_RESULT_SCHEMA_VERSION {
                return Err(ExecutionRepoError::UnknownSchemaVersion {
                    version,
                    max_supported: MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
                });
            }
            out.insert(
                node_key,
                NodeResultRecord::with_version(version, kind, result.0),
            );
        }
        Ok(out)
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;
    use crate::backend::postgres::PostgresStorage;

    /// Helper: create a `PgExecutionRepo` from `DATABASE_URL`, or skip the test.
    async fn pg_exec_repo() -> Option<PgExecutionRepo> {
        let url = std::env::var("DATABASE_URL").ok()?;
        let storage = PostgresStorage::new(url).await.expect("connect");
        storage.run_migrations().await.expect("migrations");
        Some(PgExecutionRepo::new(storage.pool().clone()))
    }

    #[tokio::test]
    async fn pg_create_and_get_state() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let id = ExecutionId::new();
        let wf_id = WorkflowId::new();
        let state = serde_json::json!({"status": "created"});

        repo.create(id, wf_id, state.clone()).await.expect("create");

        let (version, got) = repo.get_state(id).await.expect("get").expect("some");
        assert_eq!(version, 1);
        assert_eq!(got, state);
    }

    #[tokio::test]
    async fn pg_create_conflict_returns_actual_version() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let id = ExecutionId::new();
        let wf_id = WorkflowId::new();
        let state = serde_json::json!({"status": "created"});

        repo.create(id, wf_id, state.clone()).await.expect("create");

        let err = repo
            .create(id, wf_id, state)
            .await
            .expect_err("duplicate create");
        match err {
            ExecutionRepoError::Conflict {
                actual_version,
                expected_version,
                ..
            } => {
                assert_eq!(expected_version, 0);
                assert_eq!(actual_version, 1);
            },
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn pg_transition_cas_succeeds_and_bumps_version() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let id = ExecutionId::new();
        let wf_id = WorkflowId::new();
        let state1 = serde_json::json!({"status": "created"});
        let state2 = serde_json::json!({"status": "running"});

        repo.create(id, wf_id, state1).await.expect("create");

        let ok = repo
            .transition(id, 1, state2.clone())
            .await
            .expect("transition");
        assert!(ok, "CAS should succeed with correct version");

        let (version, got) = repo.get_state(id).await.expect("get").expect("some");
        assert_eq!(version, 2);
        assert_eq!(got, state2);
    }

    #[tokio::test]
    async fn pg_transition_cas_fails_on_wrong_version() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let id = ExecutionId::new();
        let wf_id = WorkflowId::new();
        let state = serde_json::json!({"status": "created"});

        repo.create(id, wf_id, state.clone()).await.expect("create");

        let ok = repo
            .transition(id, 99, state)
            .await
            .expect("transition (wrong version)");
        assert!(!ok, "CAS should fail with incorrect version");
    }

    #[tokio::test]
    async fn pg_save_and_load_node_output() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let exec_id = ExecutionId::new();
        let wf_id = WorkflowId::new();
        let node_key = node_key!("test_node");
        let output = serde_json::json!({"result": 42});

        repo.create(exec_id, wf_id, serde_json::json!({}))
            .await
            .expect("create");
        repo.save_node_output(exec_id, node_key.clone(), 0, output.clone())
            .await
            .expect("save output");

        let got = repo
            .load_node_output(exec_id, node_key.clone())
            .await
            .expect("load")
            .expect("some");
        assert_eq!(got, output);
    }

    #[tokio::test]
    async fn pg_load_all_outputs_returns_highest_attempt() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let exec_id = ExecutionId::new();
        let wf_id = WorkflowId::new();
        let node_key = node_key!("test_node");

        repo.create(exec_id, wf_id, serde_json::json!({}))
            .await
            .expect("create");
        repo.save_node_output(exec_id, node_key.clone(), 0, serde_json::json!("attempt0"))
            .await
            .expect("save attempt 0");
        repo.save_node_output(exec_id, node_key.clone(), 1, serde_json::json!("attempt1"))
            .await
            .expect("save attempt 1");

        let all = repo.load_all_outputs(exec_id).await.expect("load all");
        assert_eq!(
            all.get(&node_key.clone()),
            Some(&serde_json::json!("attempt1"))
        );
    }

    #[tokio::test]
    async fn pg_idempotency_check_and_mark() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let key = format!("test-idempotency-{}", ExecutionId::new());
        let exec_id = ExecutionId::new();
        let wf_id = WorkflowId::new();
        let node_key = node_key!("test_node");
        repo.create(exec_id, wf_id, serde_json::json!({"status":"created"}))
            .await
            .expect("create execution for idempotency");

        assert!(
            !repo.check_idempotency(&key).await.expect("check before"),
            "key should not exist yet"
        );

        repo.mark_idempotent(&key, exec_id, node_key.clone())
            .await
            .expect("first mark");

        assert!(
            repo.check_idempotency(&key)
                .await
                .expect("check after first mark"),
            "key should exist after marking"
        );

        // Second mark with same key should be idempotent (ON CONFLICT DO NOTHING).
        repo.mark_idempotent(&key, exec_id, node_key.clone())
            .await
            .expect("second mark should not fail");

        assert!(
            repo.check_idempotency(&key)
                .await
                .expect("check after second mark"),
            "key should still exist after second mark"
        );
    }

    // ── ADR-0009 B1: migration + round-trip + forward-compat ─────────────

    #[tokio::test]
    async fn pg_migration_creates_resume_persistence_columns() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };

        let executions_has_input: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM information_schema.columns
                 WHERE table_name = 'executions' AND column_name = 'input'
             )",
        )
        .fetch_one(repo.pool())
        .await
        .expect("query executions.input");
        assert!(
            executions_has_input,
            "executions.input column must exist after migration 0009"
        );

        let expected = ["result_schema_version", "result_kind", "result"];
        for col in expected {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                     SELECT 1 FROM information_schema.columns
                     WHERE table_name = 'node_outputs' AND column_name = $1
                 )",
            )
            .bind(col)
            .fetch_one(repo.pool())
            .await
            .unwrap_or_else(|e| panic!("query node_outputs.{col}: {e}"));
            assert!(exists, "node_outputs.{col} must exist after migration 0009");
        }

        let output_is_nullable: String = sqlx::query_scalar(
            "SELECT is_nullable FROM information_schema.columns
             WHERE table_name = 'node_outputs' AND column_name = 'output'",
        )
        .fetch_one(repo.pool())
        .await
        .expect("query output nullability");
        assert_eq!(
            output_is_nullable, "YES",
            "output must be nullable so save_node_result can omit it (ADR-0009)"
        );
    }

    #[tokio::test]
    async fn pg_workflow_input_round_trip() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({"status": "created"}))
            .await
            .expect("create");

        assert_eq!(
            repo.get_workflow_input(eid).await.expect("get empty"),
            None,
            "unset input must be None, never synthesized Null"
        );

        let payload = serde_json::json!({"trigger": "http", "body": [1, 2, 3]});
        repo.set_workflow_input(eid, payload.clone())
            .await
            .expect("set");
        assert_eq!(
            repo.get_workflow_input(eid).await.expect("get after set"),
            Some(payload),
        );
    }

    #[tokio::test]
    async fn pg_set_workflow_input_unknown_execution_returns_not_found() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let err = repo
            .set_workflow_input(ExecutionId::new(), serde_json::json!({}))
            .await
            .expect_err("missing execution must reject");
        assert!(matches!(
            err,
            ExecutionRepoError::NotFound { ref entity, .. } if entity == "execution"
        ));
    }

    #[tokio::test]
    async fn pg_save_and_load_node_result_round_trip() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({}))
            .await
            .expect("create");

        let nid = node_key!("branch_node");
        let result_json = serde_json::json!({
            "type": "Branch",
            "selected": "true",
            "output": {"Value": {"matched": true}},
            "alternatives": {"false": {"Value": {"matched": false}}},
        });
        let record = NodeResultRecord::new("Branch", result_json.clone());
        repo.save_node_result(eid, nid.clone(), 0, record)
            .await
            .expect("save result");

        let loaded = repo
            .load_node_result(eid, nid)
            .await
            .expect("load")
            .expect("some");
        assert_eq!(loaded.schema_version, MAX_SUPPORTED_RESULT_SCHEMA_VERSION);
        assert_eq!(loaded.kind, "Branch");
        assert_eq!(loaded.result, result_json);
    }

    #[tokio::test]
    async fn pg_save_node_result_composes_with_save_node_output_on_same_row() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({}))
            .await
            .expect("create");

        let nid = node_key!("dual_writer");
        let primary = serde_json::json!({"answer": 42});
        repo.save_node_output(eid, nid.clone(), 0, primary.clone())
            .await
            .expect("legacy save_node_output");

        let result_json = serde_json::json!({
            "type": "Success",
            "output": {"Value": {"answer": 42}},
        });
        repo.save_node_result(
            eid,
            nid.clone(),
            0,
            NodeResultRecord::new("Success", result_json.clone()),
        )
        .await
        .expect("save_node_result onto existing row");

        assert_eq!(
            repo.load_node_output(eid, nid.clone())
                .await
                .expect("legacy read")
                .expect("some"),
            primary,
            "save_node_result must not wipe legacy `output`"
        );
        let record = repo
            .load_node_result(eid, nid)
            .await
            .expect("new read")
            .expect("some");
        assert_eq!(record.result, result_json);
    }

    #[tokio::test]
    async fn pg_load_node_result_skips_legacy_rows_without_result() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({}))
            .await
            .expect("create");

        let nid = node_key!("legacy");
        repo.save_node_output(eid, nid.clone(), 0, serde_json::json!({"legacy": true}))
            .await
            .expect("legacy only");

        let loaded = repo.load_node_result(eid, nid).await.expect("load");
        assert_eq!(
            loaded, None,
            "legacy rows without persisted variant must be invisible to load_node_result"
        );
    }

    #[tokio::test]
    async fn pg_load_node_result_returns_latest_attempt() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({}))
            .await
            .expect("create");

        let nid = node_key!("retried");
        repo.save_node_result(
            eid,
            nid.clone(),
            0,
            NodeResultRecord::new("Success", serde_json::json!({"v": 0})),
        )
        .await
        .expect("attempt 0");
        repo.save_node_result(
            eid,
            nid.clone(),
            1,
            NodeResultRecord::new("Success", serde_json::json!({"v": 1})),
        )
        .await
        .expect("attempt 1");

        let loaded = repo
            .load_node_result(eid, nid)
            .await
            .expect("load")
            .expect("some");
        assert_eq!(loaded.result, serde_json::json!({"v": 1}));
    }

    #[tokio::test]
    async fn pg_load_node_result_forward_compat_unknown_schema_version() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({}))
            .await
            .expect("create");

        let nid = node_key!("future");
        let future = NodeResultRecord::with_version(
            MAX_SUPPORTED_RESULT_SCHEMA_VERSION + 1,
            "FutureVariant",
            serde_json::json!({"type": "FutureVariant"}),
        );
        repo.save_node_result(eid, nid.clone(), 0, future)
            .await
            .expect("save future record");

        let err = repo
            .load_node_result(eid, nid)
            .await
            .expect_err("unknown schema version must not fall back");
        match err {
            ExecutionRepoError::UnknownSchemaVersion {
                version,
                max_supported,
            } => {
                assert_eq!(version, MAX_SUPPORTED_RESULT_SCHEMA_VERSION + 1);
                assert_eq!(max_supported, MAX_SUPPORTED_RESULT_SCHEMA_VERSION);
            },
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn pg_load_all_results_ignores_future_version_on_non_latest_attempt() {
        // Mirror of the InMem regression: earlier attempt carries a future
        // schema_version, latest attempt is valid. DISTINCT ON (node_id)
        // ORDER BY attempt DESC must drop the stale record entirely, so
        // load_all_results succeeds instead of surfacing UnknownSchemaVersion.
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({}))
            .await
            .expect("create");

        let nid = node_key!("retried");
        repo.save_node_result(
            eid,
            nid.clone(),
            0,
            NodeResultRecord::with_version(
                MAX_SUPPORTED_RESULT_SCHEMA_VERSION + 7,
                "FutureStale",
                serde_json::json!({}),
            ),
        )
        .await
        .expect("stale future-version attempt");
        repo.save_node_result(
            eid,
            nid.clone(),
            1,
            NodeResultRecord::new("Success", serde_json::json!({"ok": true})),
        )
        .await
        .expect("fresh valid attempt");

        let all = repo
            .load_all_results(eid)
            .await
            .expect("latest attempt decodable; stale future-version row must be invisible");
        assert_eq!(all.len(), 1);
        assert_eq!(all[&nid].kind, "Success");
    }

    #[tokio::test]
    async fn pg_load_all_results_returns_latest_per_node() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let eid = ExecutionId::new();
        let wid = WorkflowId::new();
        repo.create(eid, wid, serde_json::json!({}))
            .await
            .expect("create");

        let n1 = node_key!("n1");
        let n2 = node_key!("n2");

        repo.save_node_result(
            eid,
            n1.clone(),
            0,
            NodeResultRecord::new("Success", serde_json::json!("n1_v0")),
        )
        .await
        .unwrap();
        repo.save_node_result(
            eid,
            n1.clone(),
            1,
            NodeResultRecord::new("Branch", serde_json::json!("n1_v1")),
        )
        .await
        .unwrap();
        repo.save_node_result(
            eid,
            n2.clone(),
            0,
            NodeResultRecord::new("Skip", serde_json::json!("n2_v0")),
        )
        .await
        .unwrap();

        let all = repo.load_all_results(eid).await.expect("load all");
        assert_eq!(all.len(), 2);
        assert_eq!(all[&n1].kind, "Branch");
        assert_eq!(all[&n1].result, serde_json::json!("n1_v1"));
        assert_eq!(all[&n2].kind, "Skip");
    }

    #[tokio::test]
    async fn pg_mark_idempotent_unknown_execution_returns_not_found() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let err = repo
            .mark_idempotent(
                &format!("test-idempotency-missing-{}", ExecutionId::new()),
                ExecutionId::new(),
                node_key!("test"),
            )
            .await
            .expect_err("missing execution should reject idempotency mark");
        assert!(matches!(
            err,
            ExecutionRepoError::NotFound { ref entity, .. } if entity == "execution"
        ));
    }
}
