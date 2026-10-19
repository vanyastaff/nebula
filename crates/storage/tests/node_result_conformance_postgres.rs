//! Node-result conformance for the PostgreSQL deployment backend.
//!
//! PostgreSQL is a deployment backend, so its absence is a job failure, never a
//! silent substitution. With `NEBULA_REQUIRE_POSTGRES=1` and no `DATABASE_URL`,
//! every case fails; without it a developer without a database still sees every
//! case fail loudly naming the unreachable backend. A green run of this runner
//! therefore always means the cases ran against a live database.

#![cfg(feature = "postgres")]

#[macro_use]
#[path = "support/node_result_oracle.rs"]
mod oracle;

#[path = "support/execution_parents.rs"]
mod execution_parents;

use execution_parents::SeedExecutionParents;
use nebula_storage::postgres::{PgExecutionStore, PgNodeResultStore, init_schema};
use nebula_storage_port::StorageError;
use nebula_storage_port::store::NodeResultStore;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::OnceCell;

static SCHEMA_READY: OnceCell<()> = OnceCell::const_new();

/// Connect to `DATABASE_URL` and apply the ordered migration catalog, or return
/// `None`, which every case turns into a loud failure naming the backend.
///
/// The oracle folds a per-process namespace into every execution identity, so
/// cases share one database without meeting an earlier run's rows.
async fn pool() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            assert_ne!(
                std::env::var("NEBULA_REQUIRE_POSTGRES").as_deref(),
                Ok("1"),
                "DATABASE_URL must be set when NEBULA_REQUIRE_POSTGRES=1: \
                 PostgreSQL is a deployment backend and is never substituted"
            );
            return None;
        },
        Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
    };

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    SCHEMA_READY
        .get_or_init(|| async {
            init_schema(&pool)
                .await
                .expect("apply the ordered PostgreSQL migration catalog");
            for scope in [oracle::scope(), oracle::other_scope()] {
                pool.seed_execution_parents(&scope, oracle::WORKFLOW).await;
            }
        })
        .await;
    Some(pool)
}

async fn store() -> Option<(PgNodeResultStore, PgExecutionStore)> {
    pool().await.map(|pool| {
        (
            PgNodeResultStore::new(pool.clone()),
            PgExecutionStore::new(pool),
        )
    })
}

node_result_conformance_suite!(store());
node_result_relational_suite!(store());

async fn required_pool() -> PgPool {
    pool()
        .await
        .expect("backend unreachable — set DATABASE_URL to a disposable PostgreSQL database")
}

/// Insert a raw row into `table` behind the adapter's back.
async fn insert_raw(
    pool: &PgPool,
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
         VALUES ($1, $2, $3, {node}'Value', $4::jsonb, $5)"
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
    let pool = required_pool().await;
    let scope = oracle::scope();
    let execution =
        oracle::create_execution(&PgExecutionStore::new(pool.clone()), &scope, 0xC0, 0).await;
    for table in ["node_outputs", "node_results", "workflow_inputs"] {
        insert_raw(&pool, table, &execution, "{}", 2).await.unwrap();
    }
    let store = PgNodeResultStore::new(pool);
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
    let pool = required_pool().await;
    let scope = oracle::scope();
    let execution =
        oracle::create_execution(&PgExecutionStore::new(pool.clone()), &scope, 0xC2, 0).await;
    let store = PgNodeResultStore::new(pool.clone());
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
    sqlx::query("DELETE FROM executions WHERE id = $1")
        .bind(&execution)
        .execute(&pool)
        .await
        .unwrap();
    for table in ["node_outputs", "node_results", "workflow_inputs"] {
        let left: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM {table} WHERE execution_id = $1"
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
    let pool = required_pool().await;
    let scope = oracle::scope();
    let execution =
        oracle::create_execution(&PgExecutionStore::new(pool.clone()), &scope, 0xC1, 0).await;
    for table in ["node_outputs", "node_results", "workflow_inputs"] {
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
