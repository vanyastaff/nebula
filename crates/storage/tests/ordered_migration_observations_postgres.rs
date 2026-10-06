//! Raw observations from clean and previous-supported migration paths —
//! PostgreSQL arm.
//!
//! PostgreSQL evidence binary: every case needs a live database and fails loudly
//! without one, so a green run always means the cases ran. Excluded from the
//! default nextest profile and collected by the CI `postgres-conformance` job and
//! the runtime-authority evidence step. The SQLite arm lives in
//! `ordered_migration_observations`.

#![cfg(feature = "postgres")]

#[path = "support/ordered_migration_common.rs"]
mod common;

use common::{PREVIOUS_SUPPORTED, Value, hex, json, retain};

#[path = "support/postgres_schema.rs"]
mod postgres_schema;

mod postgres {
    use super::*;
    use sqlx::{PgPool, Row};

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");

    async fn migrations(pool: &PgPool) -> Value {
        Value::Array(sqlx::query("SELECT version, description, checksum, success FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool).await.unwrap().into_iter().map(|row| json!({
                "version":row.get::<i64,_>("version"), "description":row.get::<String,_>("description"),
                "checksum":hex(&row.get::<Vec<u8>,_>("checksum")), "success":row.get::<bool,_>("success")
            })).collect())
    }

    async fn sentinel(pool: &PgPool) -> Value {
        let execution = sqlx::query("SELECT id, workspace_id, org_id, workflow_id, status, state, version, fencing_generation FROM executions WHERE id = 'migration-sentinel'")
            .fetch_optional(pool).await.unwrap().map(|row| json!({
                "id":row.get::<String,_>("id"),"workspace_id":row.get::<String,_>("workspace_id"),"org_id":row.get::<String,_>("org_id"),
                "workflow_id":row.get::<String,_>("workflow_id"),"status":row.get::<String,_>("status"),"state":row.get::<Value,_>("state"),
                "version":row.get::<i64,_>("version"),"fencing_generation":row.get::<i64,_>("fencing_generation")
            }));
        let journal = sqlx::query("SELECT seq, payload FROM execution_journal WHERE execution_id = 'migration-sentinel' ORDER BY seq")
            .fetch_all(pool).await.unwrap().into_iter().map(|row| json!({"seq":row.get::<i64,_>("seq"),"payload":row.get::<Value,_>("payload")})).collect::<Vec<_>>();
        json!({"execution":execution,"journal":journal})
    }

    async fn observe(url: &str, previous: bool) -> Value {
        let pool = postgres_schema::connect_with_private_schema(url, "ordered_migration")
            .await
            .unwrap();
        if previous {
            MIGRATOR.run_to(PREVIOUS_SUPPORTED, &pool).await.unwrap();
            sqlx::query("INSERT INTO port_executions (id, workspace_id, org_id, workflow_id, status, state, version, fencing_generation, created_at, updated_at) VALUES ('migration-sentinel','workspace','org','workflow','Running','{\"sentinel\":true}'::jsonb,7,11,'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')").execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO port_execution_journal (execution_id, seq, payload) VALUES ('migration-sentinel',3,'{\"event\":\"preserved\"}'::jsonb)").execute(&pool).await.unwrap();
        }
        nebula_storage::postgres::init_schema(&pool).await.unwrap();
        let migrated = json!({"sequence":0,"kind":"migration_snapshot","stage":"migrated","migrations":migrations(&pool).await,"sentinel":sentinel(&pool).await});
        let schema: String = sqlx::query_scalar("SELECT current_schema()")
            .fetch_one(&pool)
            .await
            .unwrap();
        pool.close().await;
        let closed = json!({"sequence":1,"kind":"pool_closed"});
        let options = url
            .parse::<sqlx::postgres::PgConnectOptions>()
            .unwrap()
            .options([("search_path", schema)]);
        let reopened = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap();
        let opened = json!({"sequence":2,"kind":"migration_snapshot","stage":"reopened","migrations":migrations(&reopened).await,"sentinel":sentinel(&reopened).await});
        nebula_storage::postgres::init_schema(&reopened)
            .await
            .unwrap();
        let reinitialized = json!({"sequence":3,"kind":"migration_snapshot","stage":"reinitialized","migrations":migrations(&reopened).await,"sentinel":sentinel(&reopened).await});
        reopened.close().await;
        json!({"scenario":if previous {"previous-supported-version"} else {"clean"},"events":[migrated,closed,opened,reinitialized]})
    }

    #[tokio::test]
    async fn clean_and_previous_supported_observations() {
        // An evidence binary never passes without a database: absence is a loud
        // failure, not a silent skip.
        let url = std::env::var("DATABASE_URL").expect(
            "ordered_migration_observations_postgres needs a live PostgreSQL: DATABASE_URL is unset or not Unicode",
        );
        let scenarios = vec![observe(&url, false).await, observe(&url, true).await];
        let pool = PgPool::connect(&url).await.unwrap();
        let version: String = sqlx::query_scalar("SHOW server_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        pool.close().await;
        retain(
            "NEBULA_ORDERED_MIGRATIONS_POSTGRES_OBSERVATIONS_PATH",
            "postgresql",
            version,
            scenarios,
        );
    }
}
