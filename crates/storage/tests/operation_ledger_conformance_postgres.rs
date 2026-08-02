//! Operation-ledger conformance for the PostgreSQL deployment backend.
//!
//! PostgreSQL is a deployment backend, so its absence is a job failure, never a
//! silent substitution. With `NEBULA_REQUIRE_POSTGRES=1` and no `DATABASE_URL`,
//! every case fails; without it a developer without a database sees the cases
//! report the backend was unreachable and assert nothing.

#![cfg(feature = "postgres")]

#[macro_use]
#[path = "support/operation_ledger_oracle.rs"]
mod oracle;

use nebula_storage::postgres::{PgOperationLedger, init_schema};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::OnceCell;

static SCHEMA_READY: OnceCell<()> = OnceCell::const_new();

/// Connect to `DATABASE_URL` and apply the ordered migration catalog, or report
/// that PostgreSQL is unreachable.
///
/// The oracle folds a per-process namespace into every execution identity, so
/// cases share one database without meeting an earlier run's slots.
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
        })
        .await;
    Some(pool)
}

async fn ledger() -> Option<PgOperationLedger> {
    pool().await.map(PgOperationLedger::new)
}

operation_ledger_conformance_suite!(ledger());
