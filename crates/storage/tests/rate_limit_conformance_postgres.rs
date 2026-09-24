//! Rate-limit store conformance for the PostgreSQL adapter: the same kit the
//! in-process store passes, against a real server clock and real row locks.

#![cfg(feature = "postgres")]

use std::str::FromStr as _;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nebula_resilience::rate_limiter::gcra::conformance;
use nebula_storage::postgres::{PgLimitStore, init_schema};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

fn unique_schema() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("nebula_rate_limit_{}_{nanos}", std::process::id())
}

async fn store() -> Option<PgLimitStore> {
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
        // More connections than the kit's concurrent callers contend for,
        // so the contention case exercises row locks, not pool queueing.
        .max_connections(16)
        .connect_with(options)
        .await
        .expect("connect to isolated schema");
    init_schema(&pool)
        .await
        .expect("initialize isolated schema");
    Some(PgLimitStore::new(pool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_limit_store_passes_the_conformance_kit() {
    let Some(store) = store().await else {
        panic!(
            "postgres_limit_store_passes_the_conformance_kit: backend unreachable — the case \
             cannot run and must fail rather than pass unchecked; reach the backend (set \
             DATABASE_URL for postgres) or run without this feature"
        );
    };
    conformance::run_all(Arc::new(store)).await;
}
