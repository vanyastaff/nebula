//! Operation-ledger conformance for the SQLite deployment backend.
//!
//! Every case runs against a fresh in-memory database whose schema comes from
//! the ordered migration catalog, so the adapter is exercised against exactly
//! the `CHECK` constraints migration 0045 installs.

#![cfg(feature = "sqlite")]

#[macro_use]
#[path = "support/operation_ledger_oracle.rs"]
mod oracle;

use std::str::FromStr;

use nebula_storage::sqlite::{SqliteOperationLedger, init_schema};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// An isolated in-memory database with the ordered migration catalog applied.
///
/// The shared cache keeps every pooled connection on the same database; a
/// private `:memory:` connection would give each one its own empty schema.
async fn fresh_pool() -> SqlitePool {
    let database = format!("nebula-ledger-{}", uuid::Uuid::new_v4());
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
    pool
}

async fn ledger() -> Option<SqliteOperationLedger> {
    Some(SqliteOperationLedger::new(fresh_pool().await))
}

operation_ledger_conformance_suite!(ledger());
