//! Raw observations from fresh creation and populated-head reconnect — SQLite arm.
//!
//! The PostgreSQL arm lives in `ordered_migration_observations_postgres` (an
//! evidence binary that needs a live database).

#![cfg(feature = "sqlite")]

#[path = "support/ordered_migration_common.rs"]
mod common;

use common::{Value, assert_snapshot, hex, json, populate_head, retain};

mod sqlite {
    use super::*;
    use sqlx::{Row, SqlitePool, sqlite::SqliteConnectOptions};
    use std::{str::FromStr as _, time::Duration};

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

    async fn migrations(pool: &SqlitePool) -> Value {
        let rows = sqlx::query(
            "SELECT version, description, checksum, success FROM _sqlx_migrations ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "version": row.get::<i64, _>("version"),
                        "description": row.get::<String, _>("description"),
                        "checksum": hex(&row.get::<Vec<u8>, _>("checksum")),
                        "success": row.get::<bool, _>("success"),
                    })
                })
                .collect(),
        )
    }

    async fn sentinel(pool: &SqlitePool) -> Value {
        let execution = sqlx::query("SELECT id, workspace_id, org_id, workflow_id, status, state, version, fencing_generation FROM executions WHERE id = 'migration-sentinel'")
            .fetch_optional(pool).await.unwrap().map(|row| json!({
                "id": row.get::<String,_>("id"), "workspace_id": row.get::<String,_>("workspace_id"),
                "org_id": row.get::<String,_>("org_id"), "workflow_id": row.get::<String,_>("workflow_id"),
                "status": row.get::<String,_>("status"), "state": serde_json::from_str::<Value>(&row.get::<String,_>("state")).unwrap(),
                "version": row.get::<i64,_>("version"), "fencing_generation": row.get::<i64,_>("fencing_generation")
            }));
        let journal = sqlx::query("SELECT seq, payload FROM execution_journal WHERE execution_id = 'migration-sentinel' ORDER BY seq")
            .fetch_all(pool).await.unwrap().into_iter().map(|row| json!({
                "seq": row.get::<i64,_>("seq"), "payload": serde_json::from_str::<Value>(&row.get::<String,_>("payload")).unwrap()
            })).collect::<Vec<_>>();
        json!({"execution": execution, "journal": journal})
    }

    async fn migration_snapshot(pool: &SqlitePool, sequence: u8, stage: &str) -> Value {
        json!({
            "sequence": sequence,
            "kind": "migration_snapshot",
            "stage": stage,
            "migrations": migrations(pool).await,
            "sentinel": sentinel(pool).await,
        })
    }

    async fn observe(populated: bool) -> Value {
        let directory = tempfile::tempdir().unwrap();
        let options =
            SqliteConnectOptions::from_str(directory.path().join("migration.db").to_str().unwrap())
                .unwrap()
                .create_if_missing(true)
                .busy_timeout(Duration::from_secs(10));
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options.clone())
            .await
            .unwrap();
        nebula_storage::sqlite::init_schema(&pool).await.unwrap();
        if populated {
            populate_head(
                &nebula_storage::sqlite::SqliteTenantProvisioningStore::new(pool.clone()),
                &nebula_storage::sqlite::SqliteWorkflowStore::new(pool.clone()),
                &nebula_storage::sqlite::SqliteExecutionStore::new(pool.clone()),
            )
            .await;
        }
        let migrated = migration_snapshot(&pool, 0, "migrated").await;
        pool.close().await;
        let closed = json!({"sequence":1,"kind":"pool_closed"});
        let reopened = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap();
        let opened = migration_snapshot(&reopened, 2, "reopened").await;
        nebula_storage::sqlite::init_schema(&reopened)
            .await
            .unwrap();
        let reinitialized = migration_snapshot(&reopened, 3, "reinitialized").await;
        for snapshot in [&migrated, &opened, &reinitialized] {
            assert_snapshot(snapshot, &MIGRATOR, populated);
        }
        reopened.close().await;
        json!({
            "scenario": if populated { "populated-head" } else { "clean" },
            "events": [migrated, closed, opened, reinitialized],
        })
    }

    #[tokio::test]
    async fn clean_and_populated_head_observations() {
        let scenarios = vec![observe(false).await, observe(true).await];
        let version_pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let version: String = sqlx::query_scalar("SELECT sqlite_version()")
            .fetch_one(&version_pool)
            .await
            .unwrap();
        version_pool.close().await;
        retain(
            "NEBULA_ORDERED_MIGRATIONS_SQLITE_OBSERVATIONS_PATH",
            "sqlite",
            version,
            scenarios,
        );
    }
}
