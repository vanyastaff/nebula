//! Execution-state size bounds on the PostgreSQL adapter.
//!
//! PostgreSQL evidence binary: every case needs a live database and fails loudly
//! without one, so a green run always means the cases ran. Excluded from the
//! default nextest profile and collected by the CI `postgres-conformance` job.

#![cfg(feature = "postgres")]

use nebula_storage_port::store::ExecutionStore;
use nebula_storage_port::{Scope, StorageError};

const MAX_EXECUTION_STATE_BYTES: usize = 64 * 1024 * 1024;

#[path = "support/postgres_schema.rs"]
mod postgres_schema;

#[tokio::test]
async fn postgres_uses_its_canonical_json_size_for_writes_and_bounded_reads() {
    // An evidence binary never passes without a database: absence is a loud
    // failure, not a silent skip.
    let url = std::env::var("DATABASE_URL").expect(
        "execution_state_bounds_postgres needs a live PostgreSQL: DATABASE_URL is unset or not Unicode",
    );
    let pool = postgres_schema::connect_with_private_schema(&url, "execution_state_bound")
        .await
        .unwrap();
    nebula_storage::postgres::init_schema(&pool).await.unwrap();
    let store = nebula_storage::postgres::PgExecutionStore::new(pool.clone());
    let scope = Scope::new("state-bound-workspace", "state-bound-organization");

    let exact = serde_json::Value::String("x".repeat(MAX_EXECUTION_STATE_BYTES - 2));
    store
        .create(&scope, "exact", "workflow", exact)
        .await
        .unwrap();
    assert!(store.get(&scope, "exact").await.unwrap().is_some());

    // Compact serde JSON is exactly at the limit; JSONB adds one space after
    // the object colon. The database-side write predicate must reject it.
    let canonical_overflow = serde_json::json!({
        "a": "x".repeat(MAX_EXECUTION_STATE_BYTES - 8),
    });
    assert!(matches!(
        store
            .create(&scope, "canonical-overflow", "workflow", canonical_overflow)
            .await,
        Err(StorageError::Serialization(_))
    ));

    let oversized = serde_json::Value::String("x".repeat(MAX_EXECUTION_STATE_BYTES - 1));
    let timestamp = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO port_executions \
         (id, workspace_id, org_id, workflow_id, status, state, version, \
          fencing_generation, created_at, updated_at) \
         VALUES ($1, $2, $3, 'workflow', 'Created', $4, 0, 0, $5, $5)",
    )
    .bind("oversized")
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .bind(oversized)
    .bind(timestamp)
    .execute(&pool)
    .await
    .unwrap();

    assert!(matches!(
        store.get(&scope, "oversized").await,
        Err(StorageError::Serialization(_))
    ));
}
