//! Raw observations from clean and previous-supported migration paths.

#[cfg(any(feature = "sqlite", feature = "postgres"))]
use std::io::Write;

#[cfg(any(feature = "sqlite", feature = "postgres"))]
use serde_json::{Value, json};

#[cfg(any(feature = "sqlite", feature = "postgres"))]
const PREVIOUS_SUPPORTED: i64 = 45;
#[cfg(any(feature = "sqlite", feature = "postgres"))]
const CURRENT_HEAD: i64 = 50;

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            write!(output, "{byte:02x}").unwrap();
            output
        },
    )
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn retain(variable: &str, backend: &str, database_version: String, scenarios: Vec<Value>) {
    if let Some(path) = std::env::var_os(variable) {
        let path = std::path::Path::new(&path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let bytes = serde_json::to_vec_pretty(&json!({
            "producer_version": 2,
            "contract": "ordered-migrations",
            "scenario_inventory_version": 1,
            "backend": backend,
            "database_version": database_version,
            "previous_supported_version": PREVIOUS_SUPPORTED,
            "current_head": CURRENT_HEAD,
            "scenarios": scenarios,
        }))
        .unwrap();
        assert!(bytes.len() <= 512 * 1024);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .unwrap();
        file.write_all(&bytes).unwrap();
        file.sync_all().unwrap();
    }
}

#[cfg(feature = "sqlite")]
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
        let execution = sqlx::query("SELECT id, workspace_id, org_id, workflow_id, status, state, version, fencing_generation FROM port_executions WHERE id = 'migration-sentinel'")
            .fetch_optional(pool).await.unwrap().map(|row| json!({
                "id": row.get::<String,_>("id"), "workspace_id": row.get::<String,_>("workspace_id"),
                "org_id": row.get::<String,_>("org_id"), "workflow_id": row.get::<String,_>("workflow_id"),
                "status": row.get::<String,_>("status"), "state": serde_json::from_str::<Value>(&row.get::<String,_>("state")).unwrap(),
                "version": row.get::<i64,_>("version"), "fencing_generation": row.get::<i64,_>("fencing_generation")
            }));
        let journal = sqlx::query("SELECT seq, payload FROM port_execution_journal WHERE execution_id = 'migration-sentinel' ORDER BY seq")
            .fetch_all(pool).await.unwrap().into_iter().map(|row| json!({
                "seq": row.get::<i64,_>("seq"), "payload": serde_json::from_str::<Value>(&row.get::<String,_>("payload")).unwrap()
            })).collect::<Vec<_>>();
        json!({"execution": execution, "journal": journal})
    }

    async fn observe(previous: bool) -> Value {
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
        if previous {
            MIGRATOR.run_to(PREVIOUS_SUPPORTED, &pool).await.unwrap();
            sqlx::query("INSERT INTO port_executions (id, workspace_id, org_id, workflow_id, status, state, version, fencing_generation, created_at, updated_at) VALUES ('migration-sentinel','workspace','org','workflow','Running','{\"sentinel\":true}',7,11,'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')")
                .execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO port_execution_journal (execution_id, seq, payload) VALUES ('migration-sentinel',3,'{\"event\":\"preserved\"}')")
                .execute(&pool).await.unwrap();
        }
        nebula_storage::sqlite::init_schema(&pool).await.unwrap();
        let migrated = json!({"sequence":0,"kind":"migration_snapshot","stage":"migrated","migrations":migrations(&pool).await,"sentinel":sentinel(&pool).await});
        pool.close().await;
        let closed = json!({"sequence":1,"kind":"pool_closed"});
        let reopened = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap();
        let opened = json!({"sequence":2,"kind":"migration_snapshot","stage":"reopened","migrations":migrations(&reopened).await,"sentinel":sentinel(&reopened).await});
        nebula_storage::sqlite::init_schema(&reopened)
            .await
            .unwrap();
        let reinitialized = json!({"sequence":3,"kind":"migration_snapshot","stage":"reinitialized","migrations":migrations(&reopened).await,"sentinel":sentinel(&reopened).await});
        reopened.close().await;
        json!({"scenario":if previous {"previous-supported-version"} else {"clean"},"events":[migrated,closed,opened,reinitialized]})
    }

    #[tokio::test]
    async fn clean_and_previous_supported_observations() {
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

#[cfg(feature = "postgres")]
#[path = "support/postgres_schema.rs"]
mod postgres_schema;

#[cfg(feature = "postgres")]
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
        let execution = sqlx::query("SELECT id, workspace_id, org_id, workflow_id, status, state, version, fencing_generation FROM port_executions WHERE id = 'migration-sentinel'")
            .fetch_optional(pool).await.unwrap().map(|row| json!({
                "id":row.get::<String,_>("id"),"workspace_id":row.get::<String,_>("workspace_id"),"org_id":row.get::<String,_>("org_id"),
                "workflow_id":row.get::<String,_>("workflow_id"),"status":row.get::<String,_>("status"),"state":row.get::<Value,_>("state"),
                "version":row.get::<i64,_>("version"),"fencing_generation":row.get::<i64,_>("fencing_generation")
            }));
        let journal = sqlx::query("SELECT seq, payload FROM port_execution_journal WHERE execution_id = 'migration-sentinel' ORDER BY seq")
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
        let Ok(url) = std::env::var("DATABASE_URL") else {
            assert!(std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none());
            return;
        };
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
