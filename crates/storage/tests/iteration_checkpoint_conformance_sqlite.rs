//! Iteration-checkpoint conformance for the SQLite deployment backend.
//!
//! Every case runs against a fresh in-memory database whose schema comes from
//! the ordered migration catalog, so the adapter is exercised against exactly
//! the `CHECK` constraints migration 0062 installs.

#![cfg(feature = "sqlite")]

#[macro_use]
#[path = "support/iteration_checkpoint_oracle.rs"]
mod oracle;

use std::str::FromStr;

use nebula_storage::sqlite::{SqliteCheckpointStore, SqliteExecutionStore, init_schema};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// An isolated in-memory database with the ordered migration catalog applied.
///
/// The shared cache keeps every pooled connection on the same database; a
/// private `:memory:` connection would give each one its own empty schema.
async fn fresh_pool() -> SqlitePool {
    let database = format!("nebula-checkpoint-{}", uuid::Uuid::new_v4());
    let url = format!("sqlite:file:{database}?mode=memory&cache=shared");
    let options = SqliteConnectOptions::from_str(&url)
        .expect("in-memory SQLite URL must parse")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .expect("connect to in-memory SQLite");
    init_schema(&pool)
        .await
        .expect("apply the ordered SQLite migration catalog");
    pool
}

async fn store() -> Option<(SqliteCheckpointStore, SqliteExecutionStore)> {
    let pool = fresh_pool().await;
    Some((
        SqliteCheckpointStore::new(pool.clone()),
        SqliteExecutionStore::new(pool),
    ))
}

iteration_checkpoint_conformance_suite!(store());

/// The migration's `CHECK`s hold even against a writer that bypasses the
/// adapter: a row outside the record's bounds is refused by the schema.
#[tokio::test]
async fn the_schema_refuses_rows_outside_the_record_bounds() {
    use nebula_storage_port::store::ExecutionStore;

    let pool = fresh_pool().await;
    let scope = oracle::scope();
    let execution = oracle::execution_id(0xC0);
    SqliteExecutionStore::new(pool.clone())
        .create(
            &scope,
            &execution,
            "workflow",
            serde_json::json!({"status":"Created"}),
        )
        .await
        .unwrap();
    let insert = |iteration: i64, state: Vec<u8>, digest: Vec<u8>| {
        let pool = pool.clone();
        let scope = scope.clone();
        let execution = execution.clone();
        async move {
            sqlx::query(
                "INSERT INTO port_iteration_checkpoints \
                 (workspace_id, org_id, execution_id, node_key, action_key, action_version, \
                  iteration, state, state_digest, resume_delay_ms, attested_positions, \
                  attempt_generation, fencing_generation, written_at_ms) \
                 VALUES (?, ?, ?, 'node', 'action', '1.0.0', ?, ?, ?, NULL, 0, 0, 0, 0)",
            )
            .bind(&scope.workspace_id)
            .bind(&scope.org_id)
            .bind(&execution)
            .bind(iteration)
            .bind(state)
            .bind(digest)
            .execute(&pool)
            .await
        }
    };
    assert!(
        insert(0, b"{}".to_vec(), vec![0; 32]).await.is_err(),
        "iteration 0"
    );
    assert!(
        insert(10_001, b"{}".to_vec(), vec![0; 32]).await.is_err(),
        "iteration past the cap"
    );
    assert!(
        insert(1, b"{}".to_vec(), vec![0; 31]).await.is_err(),
        "short digest"
    );
    assert!(
        insert(1, vec![b'x'; 1_048_577], vec![0; 32]).await.is_err(),
        "state past 1 MiB"
    );
    assert!(insert(1, b"{}".to_vec(), vec![0; 32]).await.is_ok());
}
