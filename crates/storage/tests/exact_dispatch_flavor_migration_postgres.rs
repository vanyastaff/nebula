//! Exact dispatch identity must never be manufactured for preexisting rows —
//! PostgreSQL arm.
//!
//! PostgreSQL evidence binary: every case needs a live database and fails loudly
//! without one, so a green run always means the cases ran. Excluded from the
//! default nextest profile and collected by the CI `postgres-conformance` job.
//! The SQLite arm lives in `exact_dispatch_flavor_migration`.

#![cfg(feature = "postgres")]

use sqlx::{PgPool, Row};

#[path = "support/postgres_schema.rs"]
mod postgres_schema;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");

async fn legacy_pool() -> PgPool {
    // An evidence binary never passes without a database: absence is a loud
    // failure, not a silent skip.
    let url = std::env::var("DATABASE_URL").expect(
        "exact_dispatch_flavor_migration_postgres needs a live PostgreSQL: DATABASE_URL is unset or not Unicode",
    );
    let pool = postgres_schema::connect_with_private_schema(&url, "nebula_dispatch_migration")
        .await
        .unwrap();
    MIGRATOR.run_to(45, &pool).await.unwrap();
    pool
}

#[tokio::test]
async fn every_legacy_status_rejects_atomically_without_schema_or_data_change() {
    for status in ["Pending", "Processing", "Dispatched", "Failed"] {
        let pool = legacy_pool().await;
        sqlx::query("INSERT INTO port_job_dispatch_queue (id, execution_id, workspace_id, org_id, command, status, required_plugin_key, payload, claim_generation) VALUES ($1, 'execution', 'workspace', 'org', 'Start', $2, 'plugin', '{\"preserved\":true}', 7)")
            .bind([0x11_u8;16].as_slice()).bind(status).execute(&pool).await.unwrap();
        let schema_before: Vec<(String, String)> = sqlx::query_as("SELECT column_name, data_type FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = 'port_job_dispatch_queue' ORDER BY ordinal_position").fetch_all(&pool).await.unwrap();
        assert!(
            MIGRATOR.run(&pool).await.is_err(),
            "must reject {status} legacy row"
        );
        let schema_after: Vec<(String, String)> = sqlx::query_as("SELECT column_name, data_type FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = 'port_job_dispatch_queue' ORDER BY ordinal_position").fetch_all(&pool).await.unwrap();
        assert_eq!(schema_after, schema_before);
        let row =
            sqlx::query("SELECT status, payload, claim_generation FROM port_job_dispatch_queue")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.get::<String, _>("status"), status);
        assert_eq!(
            row.get::<serde_json::Value, _>("payload"),
            serde_json::json!({"preserved":true})
        );
        assert_eq!(row.get::<i64, _>("claim_generation"), 7);
        let head: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(head, 45);
    }
}

#[tokio::test]
async fn empty_upgrade_requires_well_formed_identity_without_default() {
    let pool = legacy_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    let column = sqlx::query("SELECT is_nullable, column_default FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = 'port_job_dispatch_queue' AND column_name = 'required_worker_flavor_id'").fetch_one(&pool).await.unwrap();
    assert_eq!(column.get::<String, _>("is_nullable"), "NO");
    assert!(column.get::<Option<String>, _>("column_default").is_none());
    let insert = "INSERT INTO port_job_dispatch_queue (id, execution_id, workspace_id, org_id, command, required_plugin_key, required_worker_flavor_id) VALUES ($1, 'execution', 'workspace', 'org', 'Start', 'plugin', $2)";
    for invalid in [
        None::<Vec<u8>>,
        Some(vec![]),
        Some(vec![0; 31]),
        Some(vec![0; 33]),
    ] {
        assert!(
            sqlx::query(insert)
                .bind([0x11_u8; 16].as_slice())
                .bind(invalid)
                .execute(&pool)
                .await
                .is_err()
        );
    }
    sqlx::query(insert)
        .bind([0x11_u8; 16].as_slice())
        .bind(vec![0x22_u8; 32])
        .execute(&pool)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM port_job_dispatch_queue")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}
