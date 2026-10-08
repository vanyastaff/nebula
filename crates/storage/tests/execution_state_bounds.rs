//! Execution-state size bounds on the SQLite adapter. The PostgreSQL arm lives in
//! `execution_state_bounds_postgres` (an evidence binary that needs a live database).

#![cfg(feature = "sqlite")]

#[path = "support/execution_parents.rs"]
mod execution_parents;

use execution_parents::SeedExecutionParents;
use nebula_storage_port::store::ExecutionStore;
use nebula_storage_port::{Scope, StorageError};

const MAX_EXECUTION_STATE_BYTES: usize = 64 * 1024 * 1024;

#[tokio::test]
async fn sqlite_accepts_the_limit_and_rejects_an_oversized_raw_row_on_read() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    let store = nebula_storage::sqlite::SqliteExecutionStore::new(pool.clone());
    let scope = Scope::new("state-bound-workspace", "state-bound-organization");
    pool.seed_execution_parents(&scope, "workflow").await;

    let exact = serde_json::Value::String("x".repeat(MAX_EXECUTION_STATE_BYTES - 2));
    store
        .create(&scope, "exact", "workflow", exact)
        .await
        .unwrap();
    assert!(store.get(&scope, "exact").await.unwrap().is_some());

    let oversized = format!("\"{}\"", "x".repeat(MAX_EXECUTION_STATE_BYTES - 1));
    let timestamp = chrono::Utc::now().timestamp_micros();
    sqlx::query(
        "INSERT INTO executions \
         (id, workspace_id, org_id, workflow_id, status, state, version, \
          fencing_generation, created_at, updated_at) \
         VALUES (?, ?, ?, 'workflow', 'created', ?, 0, 0, ?, ?)",
    )
    .bind("oversized")
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .bind(oversized)
    .bind(timestamp)
    .bind(timestamp)
    .execute(&pool)
    .await
    .unwrap();

    assert!(matches!(
        store.get(&scope, "oversized").await,
        Err(StorageError::Serialization(_))
    ));
}
