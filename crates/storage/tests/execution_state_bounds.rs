#[cfg(any(feature = "sqlite", feature = "postgres"))]
use nebula_storage_port::store::ExecutionStore;
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use nebula_storage_port::{Scope, StorageError};

#[cfg(any(feature = "sqlite", feature = "postgres"))]
const MAX_EXECUTION_STATE_BYTES: usize = 64 * 1024 * 1024;

#[cfg(feature = "sqlite")]
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

    let exact = serde_json::Value::String("x".repeat(MAX_EXECUTION_STATE_BYTES - 2));
    store
        .create(&scope, "exact", "workflow", exact)
        .await
        .unwrap();
    assert!(store.get(&scope, "exact").await.unwrap().is_some());

    let oversized = format!("\"{}\"", "x".repeat(MAX_EXECUTION_STATE_BYTES - 1));
    let timestamp = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO port_executions \
         (id, workspace_id, org_id, workflow_id, status, state, version, \
          fencing_generation, created_at, updated_at) \
         VALUES (?, ?, ?, 'workflow', 'Created', ?, 0, 0, ?, ?)",
    )
    .bind("oversized")
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .bind(oversized)
    .bind(&timestamp)
    .bind(&timestamp)
    .execute(&pool)
    .await
    .unwrap();

    assert!(matches!(
        store.get(&scope, "oversized").await,
        Err(StorageError::Serialization(_))
    ));
}

#[cfg(feature = "postgres")]
#[path = "support/postgres_schema.rs"]
mod postgres_schema;

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_uses_its_canonical_json_size_for_writes_and_bounded_reads() {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => return,
        Err(std::env::VarError::NotUnicode(_)) => panic!("DATABASE_URL must be Unicode"),
    };
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
