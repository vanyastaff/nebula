//! Exact dispatch identity must never be manufactured for preexisting rows.

#[cfg(feature = "sqlite")]
mod sqlite {
    use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

    async fn legacy_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        MIGRATOR.run_to(45, &pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn every_legacy_status_rejects_atomically_without_schema_or_data_change() {
        for status in ["Pending", "Processing", "Dispatched", "Failed"] {
            let pool = legacy_pool().await;
            sqlx::query("INSERT INTO port_job_dispatch_queue (id, execution_id, workspace_id, org_id, command, status, required_plugin_key, payload, claim_generation) VALUES (?, 'execution', 'workspace', 'org', 'Start', ?, 'plugin', '{\"preserved\":true}', 7)")
                .bind([0x11_u8;16].as_slice()).bind(status).execute(&pool).await.unwrap();
            let schema_before: Vec<(String, String)> = sqlx::query_as(
                "SELECT name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
            assert!(
                MIGRATOR.run(&pool).await.is_err(),
                "must reject {status} legacy row"
            );
            let schema_after: Vec<(String, String)> = sqlx::query_as(
                "SELECT name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
            assert_eq!(schema_after, schema_before);
            let row = sqlx::query(
                "SELECT status, payload, claim_generation FROM port_job_dispatch_queue",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(row.get::<String, _>("status"), status);
            assert_eq!(row.get::<String, _>("payload"), "{\"preserved\":true}");
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
        let column = sqlx::query("SELECT \"notnull\", dflt_value FROM pragma_table_info('port_job_dispatch_queue') WHERE name = 'required_worker_flavor_id'").fetch_one(&pool).await.unwrap();
        assert_eq!(column.get::<i64, _>("notnull"), 1);
        assert!(column.get::<Option<String>, _>("dflt_value").is_none());
        let insert = "INSERT INTO port_job_dispatch_queue (id, execution_id, workspace_id, org_id, command, required_plugin_key, required_worker_flavor_id) VALUES (?, 'execution', 'workspace', 'org', 'Start', 'plugin', ?)";
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
        assert!(
            sqlx::query(insert)
                .bind([0x11_u8; 16].as_slice())
                .bind("a".repeat(32))
                .execute(&pool)
                .await
                .is_err()
        );
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
}

#[cfg(feature = "postgres")]
#[path = "support/postgres_schema.rs"]
mod postgres_schema;

#[cfg(feature = "postgres")]
mod postgres {
    use sqlx::{PgPool, Row};
    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");

    async fn legacy_pool() -> Option<PgPool> {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(std::env::VarError::NotPresent) => {
                assert!(
                    std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                    "required PostgreSQL migration evidence needs DATABASE_URL"
                );
                return None;
            },
            Err(std::env::VarError::NotUnicode(_)) => {
                panic!("configured PostgreSQL URL must be valid Unicode")
            },
        };
        let pool =
            super::postgres_schema::connect_with_private_schema(&url, "nebula_dispatch_migration")
                .await
                .unwrap();
        MIGRATOR.run_to(45, &pool).await.unwrap();
        Some(pool)
    }

    #[tokio::test]
    async fn every_legacy_status_rejects_atomically_without_schema_or_data_change() {
        for status in ["Pending", "Processing", "Dispatched", "Failed"] {
            let Some(pool) = legacy_pool().await else {
                return;
            };
            sqlx::query("INSERT INTO port_job_dispatch_queue (id, execution_id, workspace_id, org_id, command, status, required_plugin_key, payload, claim_generation) VALUES ($1, 'execution', 'workspace', 'org', 'Start', $2, 'plugin', '{\"preserved\":true}', 7)")
                .bind([0x11_u8;16].as_slice()).bind(status).execute(&pool).await.unwrap();
            let schema_before: Vec<(String, String)> = sqlx::query_as("SELECT column_name, data_type FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = 'port_job_dispatch_queue' ORDER BY ordinal_position").fetch_all(&pool).await.unwrap();
            assert!(
                MIGRATOR.run(&pool).await.is_err(),
                "must reject {status} legacy row"
            );
            let schema_after: Vec<(String, String)> = sqlx::query_as("SELECT column_name, data_type FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = 'port_job_dispatch_queue' ORDER BY ordinal_position").fetch_all(&pool).await.unwrap();
            assert_eq!(schema_after, schema_before);
            let row = sqlx::query(
                "SELECT status, payload, claim_generation FROM port_job_dispatch_queue",
            )
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
        let Some(pool) = legacy_pool().await else {
            return;
        };
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
}
