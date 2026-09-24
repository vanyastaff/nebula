//! Resource status conformance for the PostgreSQL adapter.

#![cfg(feature = "postgres")]

#[macro_use]
#[path = "support/resource_status_oracle.rs"]
mod oracle;

use std::str::FromStr as _;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nebula_storage::postgres::{PgResourceStatusStore, init_schema};
use nebula_storage_port::dto::StatusWorkerId;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

struct PgTimeControl {
    pool: PgPool,
}

#[async_trait::async_trait]
impl oracle::ResourceStatusTimeControl for PgTimeControl {
    async fn pass(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    async fn expire_long_ago(&self, worker: &StatusWorkerId) {
        sqlx::query("UPDATE port_worker_heartbeats SET expires_at_ms = (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT - 7200000 WHERE worker_id = $1")
            .bind(worker.as_str())
            .execute(&self.pool)
            .await
            .expect("heartbeat backdate succeeds");
    }
}

fn unique_schema() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("nebula_resource_status_{}_{nanos}", std::process::id())
}

async fn store() -> Option<(PgResourceStatusStore, PgTimeControl)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            assert_ne!(
                std::env::var("NEBULA_REQUIRE_POSTGRES").as_deref(),
                Ok("1"),
                "DATABASE_URL must be set when NEBULA_REQUIRE_POSTGRES=1"
            );
            return None;
        },
        Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
    };
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    let schema = unique_schema();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create isolated schema");
    let options = PgConnectOptions::from_str(&url)
        .expect("parse DATABASE_URL")
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .expect("connect to isolated schema");
    init_schema(&pool)
        .await
        .expect("initialize isolated schema");
    Some((
        PgResourceStatusStore::new(pool.clone()),
        PgTimeControl { pool },
    ))
}

resource_status_conformance_suite!(store());
