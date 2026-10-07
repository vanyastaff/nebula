//! The SQLite and PostgreSQL catalogs, migrated to head, define the same
//! structural schema: same tables, columns, nullability, keys, index column
//! lists/uniqueness/partiality, and foreign keys with update/delete actions,
//! with types compared by storage family. Tables only PostgreSQL has are
//! listed below with the reason; any other difference fails.
//! Index expressions, partial predicates and CHECK expressions are not compared
//! by this structural model; their behavior is covered by conformance and
//! dedicated schema-constraint tests. This is not a proof of SQL equivalence.
//!
//! Requires `DATABASE_URL`; it runs in the PostgreSQL CI job and is excluded
//! from the default nextest filter.

#![cfg(all(feature = "sqlite", feature = "postgres"))]

use std::collections::BTreeSet;

mod support {
    pub(crate) mod postgres_schema;
    pub(crate) mod schema_model;
}

use support::{postgres_schema, schema_model};

/// Tables that exist only in PostgreSQL, each for a stated reason.
const POSTGRES_ONLY_TABLES: &[(&str, &str)] = &[
    (
        "rate_limits",
        "one SQLite process keeps rate limits in memory",
    ),
    (
        "rate_limit_reservations",
        "one SQLite process keeps rate limits in memory",
    ),
];

#[tokio::test]
async fn sqlite_and_postgres_catalogs_define_the_same_schema() {
    let url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must point at a disposable PostgreSQL database");

    let sqlite = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open SQLite");
    nebula_storage::sqlite::init_schema(&sqlite)
        .await
        .expect("migrate SQLite to head");

    let postgres = postgres_schema::connect_with_private_schema(&url, "nebula_parity")
        .await
        .expect("connect PostgreSQL");
    nebula_storage::postgres::init_schema(&postgres)
        .await
        .expect("migrate PostgreSQL to head");

    let postgres_only: BTreeSet<&str> = POSTGRES_ONLY_TABLES.iter().map(|(t, _)| *t).collect();
    let postgres_model = schema_model::postgres_schema(&postgres).await;
    let sqlite_model = schema_model::sqlite_schema(&sqlite).await;
    if let Some(path) = std::env::var_os("NEBULA_SCHEMA_SNAPSHOT_PATH") {
        std::fs::write(
            path,
            format!("postgres:\n{postgres_model:#?}\nsqlite:\n{sqlite_model:#?}\n"),
        )
        .expect("retain inspected schema before baseline conversion");
    }
    let differences = schema_model::differences(
        "postgres",
        &postgres_model,
        "sqlite",
        &sqlite_model,
        &postgres_only,
    );
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&postgres)
        .await
        .expect("private schema name");
    assert!(
        schema
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    );
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&postgres)
        .await
        .expect("remove parity test schema");
    postgres.close().await;
    sqlite.close().await;
    assert!(
        differences.is_empty(),
        "{} schema differences:\n{}",
        differences.len(),
        differences.join("\n")
    );
}
