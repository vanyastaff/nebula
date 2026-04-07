//! Postgres-backed implementation of [`ExecutionRepo`](crate::ExecutionRepo).
//!
//! Uses the `executions`, `node_outputs`, `execution_journal`, and
//! `idempotency_keys` tables created by the SQL migrations in `./migrations`.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::types::Json;
use sqlx::{Pool, Postgres};

use crate::execution_repo::{ExecutionRepo, ExecutionRepoError};
use nebula_core::{ExecutionId, NodeId, WorkflowId};

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

/// Convert a domain `ExecutionId` to the sqlx UUID type.
fn execution_to_uuid(id: ExecutionId) -> sqlx::types::Uuid {
    sqlx::types::Uuid::from_bytes(*id.get().as_bytes())
}

/// Convert a sqlx UUID back to a domain `ExecutionId`.
fn execution_from_uuid(uuid: sqlx::types::Uuid) -> ExecutionId {
    ExecutionId::from_bytes(*uuid.as_bytes())
}

/// Convert a domain `WorkflowId` to the sqlx UUID type.
fn workflow_to_uuid(id: WorkflowId) -> sqlx::types::Uuid {
    sqlx::types::Uuid::from_bytes(*id.get().as_bytes())
}

/// Convert a domain `NodeId` to the sqlx UUID type.
fn node_to_uuid(id: NodeId) -> sqlx::types::Uuid {
    sqlx::types::Uuid::from_bytes(*id.get().as_bytes())
}

/// Convert a sqlx UUID back to a domain `NodeId`.
fn node_from_uuid(uuid: sqlx::types::Uuid) -> NodeId {
    NodeId::from_bytes(*uuid.as_bytes())
}

/// Map a sqlx error to an [`ExecutionRepoError`].
fn map_err(err: sqlx::Error) -> ExecutionRepoError {
    ExecutionRepoError::Connection(err.to_string())
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
        let uuid = execution_to_uuid(id);
        let row = sqlx::query_as::<_, (i64, Json<serde_json::Value>)>(
            "SELECT version, state FROM executions WHERE id = $1",
        )
        .bind(uuid)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(|(version, state)| (version as u64, state.0)))
    }

    async fn transition(
        &self,
        id: ExecutionId,
        expected_version: u64,
        new_state: serde_json::Value,
    ) -> Result<bool, ExecutionRepoError> {
        let uuid = execution_to_uuid(id);
        let result = sqlx::query(
            "UPDATE executions \
             SET version = $3 + 1, state = $2, updated_at = NOW() \
             WHERE id = $1 AND version = $3",
        )
        .bind(uuid)
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
        let uuid = execution_to_uuid(id);
        let rows = sqlx::query_as::<_, (Json<serde_json::Value>,)>(
            "SELECT entry FROM execution_journal \
             WHERE execution_id = $1 \
             ORDER BY created_at, id",
        )
        .bind(uuid)
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
        let uuid = execution_to_uuid(id);
        sqlx::query(
            "INSERT INTO execution_journal (execution_id, entry) \
             VALUES ($1, $2)",
        )
        .bind(uuid)
        .bind(Json(&entry))
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(())
    }

    async fn acquire_lease(
        &self,
        id: ExecutionId,
        holder: String,
        ttl: Duration,
    ) -> Result<bool, ExecutionRepoError> {
        let uuid = execution_to_uuid(id);
        let secs = ttl_seconds(ttl);
        let result = sqlx::query(
            "UPDATE executions \
             SET lease_holder = $2, \
                 lease_expires_at = NOW() + make_interval(secs => $3) \
             WHERE id = $1 \
               AND (lease_holder IS NULL OR lease_expires_at < NOW())",
        )
        .bind(uuid)
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
        let uuid = execution_to_uuid(id);
        let secs = ttl_seconds(ttl);
        let result = sqlx::query(
            "UPDATE executions \
             SET lease_expires_at = NOW() + make_interval(secs => $3) \
             WHERE id = $1 AND lease_holder = $2",
        )
        .bind(uuid)
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
        let uuid = execution_to_uuid(id);
        let result = sqlx::query(
            "UPDATE executions \
             SET lease_holder = NULL, lease_expires_at = NULL \
             WHERE id = $1 AND lease_holder = $2",
        )
        .bind(uuid)
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
        let exec_uuid = execution_to_uuid(id);
        let wf_uuid = workflow_to_uuid(workflow_id);
        let result = sqlx::query(
            "INSERT INTO executions (id, workflow_id, version, state) \
             VALUES ($1, $2, 1, $3)",
        )
        .bind(exec_uuid)
        .bind(wf_uuid)
        .bind(Json(&state))
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
                // A unique-constraint violation guarantees the row exists, so
                // fetch its current version to provide an accurate Conflict error.
                let actual_version = sqlx::query_scalar::<_, i64>(
                    "SELECT version FROM executions WHERE id = $1",
                )
                .bind(exec_uuid)
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
            }
            Err(err) => Err(map_err(err)),
        }
    }

    async fn save_node_output(
        &self,
        execution_id: ExecutionId,
        node_id: NodeId,
        attempt: u32,
        output: serde_json::Value,
    ) -> Result<(), ExecutionRepoError> {
        let eid = execution_to_uuid(execution_id);
        let nid = node_to_uuid(node_id);
        let attempt_i32 = i32::try_from(attempt).map_err(|_| {
            ExecutionRepoError::Internal(format!(
                "attempt value {attempt} exceeds i32::MAX"
            ))
        })?;
        sqlx::query(
            "INSERT INTO node_outputs (execution_id, node_id, attempt, output) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (execution_id, node_id, attempt) \
             DO UPDATE SET output = EXCLUDED.output",
        )
        .bind(eid)
        .bind(nid)
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
        node_id: NodeId,
    ) -> Result<Option<serde_json::Value>, ExecutionRepoError> {
        let eid = execution_to_uuid(execution_id);
        let nid = node_to_uuid(node_id);
        let row = sqlx::query_as::<_, (Json<serde_json::Value>,)>(
            "SELECT output FROM node_outputs \
             WHERE execution_id = $1 AND node_id = $2 \
             ORDER BY attempt DESC \
             LIMIT 1",
        )
        .bind(eid)
        .bind(nid)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(|(output,)| output.0))
    }

    async fn load_all_outputs(
        &self,
        execution_id: ExecutionId,
    ) -> Result<HashMap<NodeId, serde_json::Value>, ExecutionRepoError> {
        let eid = execution_to_uuid(execution_id);
        let rows = sqlx::query_as::<_, (sqlx::types::Uuid, Json<serde_json::Value>)>(
            "SELECT DISTINCT ON (node_id) node_id, output \
             FROM node_outputs \
             WHERE execution_id = $1 \
             ORDER BY node_id, attempt DESC",
        )
        .bind(eid)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(rows
            .into_iter()
            .map(|(uuid, output)| (node_from_uuid(uuid), output.0))
            .collect())
    }

    async fn list_running(&self) -> Result<Vec<ExecutionId>, ExecutionRepoError> {
        let rows = sqlx::query_as::<_, (sqlx::types::Uuid,)>(
            "SELECT id FROM executions \
             WHERE state->>'status' IN ('created', 'running', 'paused', 'cancelling')",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(rows
            .into_iter()
            .map(|(uuid,)| execution_from_uuid(uuid))
            .collect())
    }

    async fn count(&self, workflow_id: Option<WorkflowId>) -> Result<u64, ExecutionRepoError> {
        let count: i64 = match workflow_id {
            Some(wid) => {
                let uuid = workflow_to_uuid(wid);
                sqlx::query_scalar("SELECT COUNT(*) FROM executions WHERE workflow_id = $1")
                    .bind(uuid)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(map_err)?
            }
            None => sqlx::query_scalar("SELECT COUNT(*) FROM executions")
                .fetch_one(&self.pool)
                .await
                .map_err(map_err)?,
        };

        Ok(count as u64)
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
        node_id: NodeId,
    ) -> Result<(), ExecutionRepoError> {
        let eid = execution_to_uuid(execution_id);
        let nid = node_to_uuid(node_id);
        sqlx::query(
            "INSERT INTO idempotency_keys (key, execution_id, node_id) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (key) DO NOTHING",
        )
        .bind(key)
        .bind(eid)
        .bind(nid)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(())
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

        repo.create(id, wf_id, state.clone())
            .await
            .expect("create");

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

        repo.create(id, wf_id, state.clone())
            .await
            .expect("create");

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
            }
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
        let node_id = NodeId::new();
        let output = serde_json::json!({"result": 42});

        repo.create(exec_id, wf_id, serde_json::json!({}))
            .await
            .expect("create");
        repo.save_node_output(exec_id, node_id, 0, output.clone())
            .await
            .expect("save output");

        let got = repo
            .load_node_output(exec_id, node_id)
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
        let node_id = NodeId::new();

        repo.create(exec_id, wf_id, serde_json::json!({}))
            .await
            .expect("create");
        repo.save_node_output(exec_id, node_id, 0, serde_json::json!("attempt0"))
            .await
            .expect("save attempt 0");
        repo.save_node_output(exec_id, node_id, 1, serde_json::json!("attempt1"))
            .await
            .expect("save attempt 1");

        let all = repo
            .load_all_outputs(exec_id)
            .await
            .expect("load all");
        assert_eq!(all.get(&node_id), Some(&serde_json::json!("attempt1")));
    }

    #[tokio::test]
    async fn pg_idempotency_check_and_mark() {
        let Some(repo) = pg_exec_repo().await else {
            return;
        };
        let key = format!("test-idempotency-{}", ExecutionId::new());
        let exec_id = ExecutionId::new();
        let node_id = NodeId::new();

        assert!(
            !repo.check_idempotency(&key).await.expect("check before"),
            "key should not exist yet"
        );

        repo.mark_idempotent(&key, exec_id, node_id)
            .await
            .expect("first mark");

        assert!(
            repo.check_idempotency(&key).await.expect("check after first mark"),
            "key should exist after marking"
        );

        // Second mark with same key should be idempotent (ON CONFLICT DO NOTHING).
        repo.mark_idempotent(&key, exec_id, node_id)
            .await
            .expect("second mark should not fail");

        assert!(
            repo.check_idempotency(&key).await.expect("check after second mark"),
            "key should still exist after second mark"
        );
    }
}
