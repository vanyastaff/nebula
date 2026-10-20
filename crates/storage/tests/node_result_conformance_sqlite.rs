//! Node-result conformance for the SQLite deployment backend.
//!
//! Every case runs against a fresh in-memory database whose schema comes from
//! the ordered migration catalog, so the adapter is exercised against exactly
//! the keys, foreign keys and `CHECK` constraints `0013_node_results.sql`
//! installs.

#![cfg(feature = "sqlite")]

#[macro_use]
#[path = "support/node_result_oracle.rs"]
mod oracle;

#[path = "support/execution_parents.rs"]
mod execution_parents;

use std::str::FromStr;

use execution_parents::SeedExecutionParents;
use nebula_storage::sqlite::{SqliteExecutionStore, SqliteNodeResultStore, init_schema};
use nebula_storage_port::StorageError;
use nebula_storage_port::store::NodeResultStore;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// An isolated in-memory database with the ordered migration catalog applied.
///
/// The shared cache keeps every pooled connection on the same database; a
/// private `:memory:` connection would give each one its own empty schema.
async fn fresh_pool() -> SqlitePool {
    let database = format!("nebula-noderes-{}", uuid::Uuid::new_v4());
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
    for scope in [oracle::scope(), oracle::other_scope()] {
        pool.seed_execution_parents(&scope, oracle::WORKFLOW).await;
    }
    pool
}

async fn store() -> Option<(SqliteNodeResultStore, SqliteExecutionStore)> {
    let pool = fresh_pool().await;
    Some((
        SqliteNodeResultStore::new(pool.clone()),
        SqliteExecutionStore::new(pool),
    ))
}

node_result_conformance_suite!(store());
node_result_relational_suite!(store());

/// Insert a raw row into `table` behind the adapter's back.
async fn insert_raw(
    pool: &SqlitePool,
    table: &str,
    execution: &str,
    payload: &str,
    schema_version: i64,
) -> Result<(), sqlx::Error> {
    let scope = oracle::scope();
    let (columns, node) = if table == "workflow_inputs" {
        ("", "")
    } else {
        ("node_key, ", "'a', ")
    };
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {table} \
         (org_id, workspace_id, execution_id, {columns}kind_tag, payload, schema_version) \
         VALUES (?, ?, ?, {node}'Value', ?, ?)"
    )))
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(execution)
    .bind(payload)
    .bind(schema_version)
    .execute(pool)
    .await
    .map(|_| ())
}

/// A row a newer binary wrote is never returned: every read of it fails
/// closed, the bulk loads included.
#[tokio::test]
async fn a_stored_newer_schema_version_fails_closed() {
    let pool = fresh_pool().await;
    let scope = oracle::scope();
    let execution =
        oracle::create_execution(&SqliteExecutionStore::new(pool.clone()), &scope, 0xC0, 0).await;
    for table in ["node_outputs", "node_results", "workflow_inputs"] {
        insert_raw(&pool, table, &execution, "{}", 2).await.unwrap();
    }
    let store = SqliteNodeResultStore::new(pool);
    let unknown = |error: StorageError| {
        assert!(
            matches!(
                error,
                StorageError::UnknownSchemaVersion { found: 2, max: 1 }
            ),
            "{error:?}"
        );
    };
    unknown(
        store
            .load_node_output(&scope, &execution, "a")
            .await
            .unwrap_err(),
    );
    unknown(
        store
            .load_node_result(&scope, &execution, "a")
            .await
            .unwrap_err(),
    );
    unknown(
        store
            .load_all_node_outputs(&scope, &execution)
            .await
            .unwrap_err(),
    );
    unknown(
        store
            .load_all_results(&scope, &execution)
            .await
            .unwrap_err(),
    );
    unknown(
        store
            .get_workflow_input(&scope, &execution)
            .await
            .unwrap_err(),
    );
}

/// Purging an execution purges its node results with it.
#[tokio::test]
async fn purging_the_execution_removes_its_rows() {
    let pool = fresh_pool().await;
    let scope = oracle::scope();
    let execution =
        oracle::create_execution(&SqliteExecutionStore::new(pool.clone()), &scope, 0xC2, 0).await;
    let store = SqliteNodeResultStore::new(pool.clone());
    let record = oracle::record("Value", serde_json::json!(1));
    store
        .save_node_output(&scope, &execution, "a", record.clone())
        .await
        .unwrap();
    store
        .save_node_result(&scope, &execution, "a", record.clone())
        .await
        .unwrap();
    store
        .set_workflow_input(&scope, &execution, record)
        .await
        .unwrap();
    sqlx::query("DELETE FROM executions WHERE id = ?")
        .bind(&execution)
        .execute(&pool)
        .await
        .unwrap();
    for table in ["node_outputs", "node_results", "workflow_inputs"] {
        let left: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM {table} WHERE execution_id = ?"
        )))
        .bind(&execution)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(left, 0, "{table} rows cascade with their execution");
    }
}

/// The migration's `CHECK`s hold even against a writer that bypasses the
/// adapter.
#[tokio::test]
async fn the_schema_refuses_rows_outside_the_record_bounds() {
    let pool = fresh_pool().await;
    let scope = oracle::scope();
    let execution =
        oracle::create_execution(&SqliteExecutionStore::new(pool.clone()), &scope, 0xC1, 0).await;
    for table in ["node_outputs", "node_results", "workflow_inputs"] {
        assert!(
            insert_raw(&pool, table, &execution, "{not json", 1)
                .await
                .is_err(),
            "{table}: payload must be JSON"
        );
        assert!(
            insert_raw(&pool, table, &execution, "{}", -1)
                .await
                .is_err(),
            "{table}: negative schema version"
        );
        assert!(
            insert_raw(&pool, table, &execution, "{}", i64::from(u32::MAX) + 1)
                .await
                .is_err(),
            "{table}: schema version past u32"
        );
        assert!(
            insert_raw(&pool, table, "exe-missing", "{}", 1)
                .await
                .is_err(),
            "{table}: a row needs its execution"
        );
        assert!(insert_raw(&pool, table, &execution, "{}", 1).await.is_ok());
    }
}
